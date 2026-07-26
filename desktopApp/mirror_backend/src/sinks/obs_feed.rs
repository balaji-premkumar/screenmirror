//! OBS Feed Module
//!
//! Video frames reach OBS through the triple-buffer segment owned by
//! `shared_mem::TripleBufferManager` (os id `obs_mirror_buffer`) — the OBS
//! plugin (`obs_plugin/mirror_source.c`) maps that segment directly.
//!
//! This module owns the *audio* ring shared with the plugin and the
//! OBS detection / plugin installation helpers.
//!
//! Audio SHM layout (32-byte header, must match `struct audio_shm_header`
//! in obs_plugin/mirror_source.c):
//!   offset 0   magic "MIRA"
//!   offset 4   _pad0    u32
//!   offset 8   written  u64  total samples ever written (atomic, monotonic)
//!   offset 16  session  u64  changes on every app start
//!   offset 24  _pad1    u64
//!   offset 32  float samples[96000]  (mono f32 @ 48 kHz ≈ 2 s)
//!
//! `written` is a running total rather than a ring index on purpose: a reader
//! that falls more than one buffer behind can compare it against its own
//! consumed count and *know* it was lapped. With a bare write index that
//! situation is indistinguishable from normal progress, so the reader would
//! silently emit garbage.

use crate::log_event;
use mirror_i18n::codes;
use once_cell::sync::Lazy;
use std::sync::atomic::{fence, AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;

// ── Toggle ──────────────────────────────────────────────────
static OBS_ENABLED: AtomicBool = AtomicBool::new(false);

pub fn set_enabled(enabled: bool) {
    OBS_ENABLED.store(enabled, Ordering::Relaxed);
    log_event!(codes::OBS_FEED_TOGGLED, "state" => if enabled { "enabled" } else { "disabled" });
}

pub fn is_enabled() -> bool {
    OBS_ENABLED.load(Ordering::Relaxed)
}

// ── Audio shared memory ─────────────────────────────────────

pub const AUDIO_BUFFER_SAMPLES: usize = 96000;
const AUDIO_HEADER_SIZE: usize = 32;
const AUDIO_SHM_SIZE: usize = AUDIO_HEADER_SIZE + AUDIO_BUFFER_SAMPLES * 4;
const AUDIO_OFF_WRITTEN: usize = 8;
const AUDIO_OFF_SESSION: usize = 16;

struct AudioShmem {
    ptr: *mut u8,
    #[cfg(unix)]
    fd: i32,
    #[cfg(target_os = "windows")]
    handle: win::HANDLE,
}

unsafe impl Send for AudioShmem {}

static AUDIO_SHMEM: Lazy<Mutex<Option<AudioShmem>>> = Lazy::new(|| Mutex::new(None));

// POSIX shared memory works the same on macOS as on Linux, and the OBS plugin
// already takes the POSIX branch there — only this side was gated, which left
// macOS with video but silently no audio.
#[cfg(unix)]
const AUDIO_SHM_NAME: &[u8] = b"/mirror_obs_audio\0";

#[cfg(target_os = "windows")]
mod win {
    pub type HANDLE = *mut core::ffi::c_void;

    #[link(name = "kernel32")]
    extern "system" {
        pub fn CreateFileMappingA(
            h_file: HANDLE,
            attrs: *mut core::ffi::c_void,
            protect: u32,
            size_high: u32,
            size_low: u32,
            name: *const u8,
        ) -> HANDLE;
        pub fn MapViewOfFile(
            h: HANDLE,
            access: u32,
            off_high: u32,
            off_low: u32,
            size: usize,
        ) -> *mut core::ffi::c_void;
        pub fn UnmapViewOfFile(addr: *const core::ffi::c_void) -> i32;
        pub fn CloseHandle(h: HANDLE) -> i32;
    }

    pub const PAGE_READWRITE: u32 = 0x04;
    pub const FILE_MAP_ALL_ACCESS: u32 = 0xF001F;
    pub const INVALID_HANDLE_VALUE: HANDLE = -1isize as HANDLE;
    /// Mapping name — the OBS plugin opens the same name with OpenFileMappingA.
    pub const AUDIO_MAP_NAME: &[u8] = b"mirror_obs_audio\0";
}

/// Create the audio SHM ring. Called once from `init_mirror()`.
pub fn init_audio() -> bool {
    #[cfg(unix)]
    unsafe {
        libc::shm_unlink(AUDIO_SHM_NAME.as_ptr() as *const libc::c_char);
        let fd = libc::shm_open(
            AUDIO_SHM_NAME.as_ptr() as *const libc::c_char,
            libc::O_CREAT | libc::O_RDWR,
            0o600,
        );
        if fd < 0 {
            log_event!(codes::OBS_SHMEM_AUDIO_OPEN_FAILED);
            return false;
        }
        // The OBS process runs as the same user; 0600 keeps other local
        // users from reading the mirrored audio.
        libc::ftruncate(fd, AUDIO_SHM_SIZE as libc::off_t);
        let ptr = libc::mmap(
            std::ptr::null_mut(),
            AUDIO_SHM_SIZE,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_SHARED,
            fd,
            0,
        );
        if ptr == libc::MAP_FAILED {
            libc::close(fd);
            return false;
        }
        let base = ptr as *mut u8;
        std::ptr::write_bytes(base, 0, AUDIO_SHM_SIZE);
        std::ptr::copy_nonoverlapping(b"MIRA".as_ptr(), base, 4);
        // Published last: a reader that sees the magic must also see a session
        // id it can compare against.
        *(base.add(AUDIO_OFF_SESSION) as *mut u64) = audio_session_id();
        if let Ok(mut shmem) = AUDIO_SHMEM.lock() {
            *shmem = Some(AudioShmem { ptr: base, fd });
        }
        log_event!(codes::OBS_SHMEM_AUDIO_READY);
        true
    }

    #[cfg(target_os = "windows")]
    unsafe {
        let handle = win::CreateFileMappingA(
            win::INVALID_HANDLE_VALUE,
            std::ptr::null_mut(),
            win::PAGE_READWRITE,
            0,
            AUDIO_SHM_SIZE as u32,
            win::AUDIO_MAP_NAME.as_ptr(),
        );
        if handle.is_null() {
            log_event!(codes::OBS_SHMEM_AUDIO_MAP_FAILED);
            return false;
        }
        let ptr = win::MapViewOfFile(handle, win::FILE_MAP_ALL_ACCESS, 0, 0, AUDIO_SHM_SIZE);
        if ptr.is_null() {
            win::CloseHandle(handle);
            return false;
        }
        let base = ptr as *mut u8;
        std::ptr::write_bytes(base, 0, AUDIO_SHM_SIZE);
        std::ptr::copy_nonoverlapping(b"MIRA".as_ptr(), base, 4);
        *(base.add(AUDIO_OFF_SESSION) as *mut u64) = audio_session_id();
        if let Ok(mut shmem) = AUDIO_SHMEM.lock() {
            *shmem = Some(AudioShmem { ptr: base, handle });
        }
        log_event!(codes::OBS_SHMEM_AUDIO_READY);
        true
    }

    #[cfg(not(any(unix, target_os = "windows")))]
    {
        log_event!(codes::OBS_SHMEM_UNSUPPORTED);
        false
    }
}

fn audio_session_id() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(1)
        | 1
}

pub fn write_audio(samples: &[f32]) {
    if !is_enabled() || samples.is_empty() {
        return;
    }

    if let Ok(shmem_opt) = AUDIO_SHMEM.lock() {
        if let Some(ref shm) = *shmem_opt {
            unsafe {
                let written_atomic = &*(shm.ptr.add(AUDIO_OFF_WRITTEN) as *const AtomicU64);
                let data_ptr = shm.ptr.add(AUDIO_HEADER_SIZE) as *mut f32;

                // Only this thread writes, so a relaxed load of our own
                // counter is enough to find the cursor.
                let written = written_atomic.load(Ordering::Relaxed);
                let mut head = (written % AUDIO_BUFFER_SAMPLES as u64) as usize;
                for &sample in samples {
                    *data_ptr.add(head) = sample;
                    head += 1;
                    if head == AUDIO_BUFFER_SAMPLES {
                        head = 0;
                    }
                }

                // Samples must be visible before the count that advertises them.
                fence(Ordering::Release);
                written_atomic.store(written + samples.len() as u64, Ordering::Release);
            }
        }
    }
}

/// Release the audio segment on shutdown.
pub fn cleanup() {
    if let Ok(mut shmem) = AUDIO_SHMEM.lock() {
        if let Some(shm) = shmem.take() {
            #[cfg(unix)]
            unsafe {
                libc::munmap(shm.ptr as *mut libc::c_void, AUDIO_SHM_SIZE);
                libc::close(shm.fd);
                libc::shm_unlink(AUDIO_SHM_NAME.as_ptr() as *const libc::c_char);
            }
            #[cfg(target_os = "windows")]
            unsafe {
                win::UnmapViewOfFile(shm.ptr as *const core::ffi::c_void);
                win::CloseHandle(shm.handle);
            }
            #[cfg(not(any(unix, target_os = "windows")))]
            let _ = shm;
            log_event!(codes::OBS_SHMEM_AUDIO_RELEASED);
        }
    }
}

// ── OBS Detection & Plugin Management ───────────────────────

/// Check whether OBS Studio is installed on this system.
pub fn check_obs_installed() -> bool {
    #[cfg(target_os = "linux")]
    {
        // Native package
        if std::process::Command::new("which")
            .arg("obs")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
        {
            return true;
        }
        // Flatpak
        if std::process::Command::new("flatpak")
            .args(["info", "com.obsproject.Studio"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
        {
            return true;
        }
        // Snap
        if std::process::Command::new("snap")
            .args(["list", "obs-studio"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
        {
            return true;
        }
        false
    }
    #[cfg(target_os = "windows")]
    {
        let mut paths = vec![
            r"C:\Program Files\obs-studio".to_string(),
            r"C:\Program Files (x86)\obs-studio".to_string(),
        ];
        if let Ok(pf) = std::env::var("ProgramFiles") {
            paths.push(format!(r"{pf}\obs-studio"));
        }
        paths.iter().any(|p| std::path::Path::new(p).exists())
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    false
}

/// Is OBS Studio running right now?
///
/// Gating the "Direct to OBS" toggle on this keeps the app from offering a
/// feed nothing is reading. The result is cached: this is polled from the UI
/// status tick, and spawning a process enumerator twice a second is not free.
pub fn check_obs_running() -> bool {
    const TTL: std::time::Duration = std::time::Duration::from_secs(3);

    static CACHE: Lazy<Mutex<Option<(bool, std::time::Instant)>>> = Lazy::new(|| Mutex::new(None));

    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((value, when)) = *cache {
        if when.elapsed() < TTL {
            return value;
        }
    }
    let value = probe_obs_running();
    *cache = Some((value, std::time::Instant::now()));
    value
}

fn probe_obs_running() -> bool {
    use std::process::{Command, Stdio};

    #[cfg(target_os = "linux")]
    {
        // Native package (obs / obs64) and the Flatpak wrapper process.
        for args in [
            vec!["-x", "obs"],
            vec!["-x", "obs64"],
            vec!["-f", "com.obsproject.Studio"],
        ] {
            if Command::new("pgrep")
                .args(&args)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
            {
                return true;
            }
        }
        false
    }
    #[cfg(target_os = "windows")]
    {
        for image in ["obs64.exe", "obs32.exe"] {
            let out = Command::new("tasklist")
                .args(["/NH", "/FI", &format!("IMAGENAME eq {image}")])
                .output();
            if let Ok(out) = out {
                if String::from_utf8_lossy(&out.stdout)
                    .to_lowercase()
                    .contains(&image.to_lowercase())
                {
                    return true;
                }
            }
        }
        false
    }
    #[cfg(target_os = "macos")]
    {
        Command::new("pgrep")
            .args(["-x", "OBS"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
    false
}

/// Find the user-level OBS plugin directory.
///
/// The per-platform paths live in `crate::platform`; this only decides what to
/// do when OBS has never been run. Installing into the default location is
/// still useful there — OBS reads the directory at startup, so the plugin is
/// picked up the first time the user launches it.
pub fn get_obs_plugin_dir() -> Option<String> {
    crate::platform::obs_plugin_dir()
        .or_else(|| {
            check_obs_installed()
                .then(crate::platform::default_obs_plugin_dir)
                .flatten()
        })
        .map(|p| p.to_string_lossy().into_owned())
}

/// Bump this whenever the SHM header layout changes — forces old plugin binaries
/// to be replaced on next app launch.
/// 3.0.0 — audio SHM header grew from 8 to 32 bytes and swapped the bare
/// write index for a monotonic sample count, so a 2.x plugin binary would
/// misread the ring entirely. The version file forces it to be replaced.
const PLUGIN_VERSION: &str = "3.0.0";

const PLUGIN_BINARY: &str = crate::platform::obs_plugin_filename();

/// Check whether our OBS plugin is already installed and up to date.
pub fn check_plugin_installed() -> bool {
    if let Some(plugin_dir) = get_obs_plugin_dir() {
        let base_path = format!("{}/mirror-source", plugin_dir);
        let bin_path = format!("{}/bin/64bit/{}", base_path, PLUGIN_BINARY);
        let version_path = format!("{}/version.txt", base_path);

        if !std::path::Path::new(&bin_path).exists() {
            return false;
        }

        // Check version
        if let Ok(installed_version) = std::fs::read_to_string(version_path) {
            if installed_version.trim() == PLUGIN_VERSION {
                return true;
            }
        }
        false
    } else {
        false
    }
}

/// Build (Linux) or copy (Windows) and install the OBS plugin.
/// Returns 0 on success, -1 on failure.
pub fn install_plugin(project_root: &str) -> i32 {
    log_event!(codes::OBS_INSTALL_STARTED, "version" => PLUGIN_VERSION);

    let plugin_dir = match get_obs_plugin_dir() {
        Some(d) => d,
        None => {
            log_event!(codes::OBS_INSTALL_DIR_NOT_FOUND);
            return -1;
        }
    };

    let source_dir = format!("{}/obs_plugin", project_root);
    let build_dir = format!("{}/build", source_dir);
    let _ = std::fs::create_dir_all(&build_dir);

    // 1. Look for a pre-bundled plugin binary
    let bundled = format!("{}/bin/{}", project_root, PLUGIN_BINARY);
    let precompiled_dev = format!("{}/{}", build_dir, PLUGIN_BINARY);

    let mut plugin_src = if std::path::Path::new(&bundled).exists() {
        Some(bundled)
    } else if std::path::Path::new(&precompiled_dev).exists() {
        Some(precompiled_dev.clone())
    } else {
        None
    };

    // 2. Linux only: compile locally when no binary is bundled
    #[cfg(target_os = "linux")]
    if plugin_src.is_none() {
        log_event!(codes::OBS_INSTALL_COMPILING);
        let status = std::process::Command::new("gcc")
            .args([
                "-shared",
                "-fPIC",
                "-O2",
                "-o",
                &precompiled_dev,
                &format!("{}/mirror_source.c", source_dir),
                "-I/usr/include/obs",
                "-lobs",
                "-lrt",
                "-lpthread",
            ])
            .status();

        if status.map(|s| s.success()).unwrap_or(false) {
            plugin_src = Some(precompiled_dev.clone());
        } else {
            log_event!(codes::OBS_INSTALL_COMPILE_FAILED);
            return -1;
        }
    }

    let final_src = match plugin_src {
        Some(p) => p,
        None => {
            log_event!(codes::OBS_INSTALL_BINARY_MISSING);
            return -1;
        }
    };

    // Install to the OBS plugin directory
    let base_install_dir = format!("{}/mirror-source", plugin_dir);
    let bin_install_dir = format!("{}/bin/64bit", base_install_dir);

    if std::fs::create_dir_all(&bin_install_dir).is_err() {
        log_event!(codes::OBS_INSTALL_MKDIR_FAILED);
        return -1;
    }

    let dst = format!("{}/{}", bin_install_dir, PLUGIN_BINARY);
    log_event!(codes::OBS_INSTALL_COPYING, "from" => &final_src, "to" => &dst);
    if let Err(e) = std::fs::copy(&final_src, &dst) {
        log_event!(codes::OBS_INSTALL_COPY_FAILED, "error" => e);
        return -1;
    }

    // Write version file
    let version_path = format!("{}/version.txt", base_install_dir);
    if let Err(e) = std::fs::write(&version_path, PLUGIN_VERSION) {
        log_event!(codes::OBS_INSTALL_VERSION_WRITE_FAILED, "path" => &version_path, "error" => e);
    }

    log_event!(codes::OBS_INSTALL_COMPLETE, "version" => PLUGIN_VERSION, "path" => &dst);

    0
}

/// Checks that this file and `obs_plugin/mirror_source.c` still agree about
/// the bytes they share.
///
/// Separate from the `unix`-only tests below because a layout mismatch is
/// equally fatal on Windows, and reading a source file needs no platform
/// support at all.
///
/// The C side has `_Static_assert`s for its own struct sizes, which catch a
/// C-only mistake. What neither side could catch alone is the two drifting
/// apart — and a drift here does not crash: OBS reads a plausible-looking
/// number from the wrong offset and renders silence or noise.
#[cfg(test)]
mod layout {
    use super::{AUDIO_HEADER_SIZE, AUDIO_OFF_SESSION, AUDIO_OFF_WRITTEN};

    /// The plugin source, read at compile time so the test cannot go stale by
    /// pointing at a file that has moved.
    const PLUGIN_SOURCE: &str = include_str!("../../../obs_plugin/mirror_source.c");

    /// Field sizes, in declaration order, of `struct audio_shm_header`.
    fn c_audio_header_fields() -> Vec<(String, usize)> {
        let start = PLUGIN_SOURCE
            .find("struct audio_shm_header {")
            .expect("mirror_source.c no longer declares struct audio_shm_header");
        let body_start = PLUGIN_SOURCE[start..].find('{').unwrap() + start + 1;
        let body_end = PLUGIN_SOURCE[body_start..].find('}').unwrap() + body_start;

        let mut fields = Vec::new();
        for line in PLUGIN_SOURCE[body_start..body_end].lines() {
            // Drop comments and whitespace, keep the declaration.
            let code = line.split("/*").next().unwrap_or("").trim();
            let Some(decl) = code.strip_suffix(';') else {
                continue;
            };
            let decl = decl.replace("volatile", "");
            let mut parts = decl.split_whitespace().collect::<Vec<_>>();
            let Some(name) = parts.pop() else { continue };
            let ty = parts.join(" ");

            let size = match ty.as_str() {
                "uint64_t" => 8,
                "uint32_t" => 4,
                "char" if name.ends_with("[4]") => 4,
                other => panic!("unhandled C type {other:?} in audio_shm_header"),
            };
            fields.push((name.to_string(), size));
        }
        fields
    }

    #[test]
    fn the_audio_header_matches_the_plugin() {
        let fields = c_audio_header_fields();
        assert!(!fields.is_empty(), "parsed no fields out of the C struct");

        let mut offsets = std::collections::HashMap::new();
        let mut offset = 0usize;
        for (name, size) in &fields {
            offsets.insert(name.as_str(), offset);
            offset += size;
        }

        assert_eq!(
            offset, AUDIO_HEADER_SIZE,
            "the C header is {offset} bytes but Rust writes {AUDIO_HEADER_SIZE}; \
             fields parsed: {fields:?}"
        );
        assert_eq!(
            offsets.get("written").copied(),
            Some(AUDIO_OFF_WRITTEN),
            "`written` is at a different offset in C than Rust writes it"
        );
        assert_eq!(
            offsets.get("session").copied(),
            Some(AUDIO_OFF_SESSION),
            "`session` is at a different offset in C than Rust writes it"
        );
    }

    #[test]
    fn the_plugin_still_asserts_its_own_sizes() {
        // If these are ever removed, a C-side struct change stops being caught
        // at compile time and this module becomes the only guard.
        assert!(PLUGIN_SOURCE.contains("sizeof(struct audio_shm_header) == 32"));
        assert!(PLUGIN_SOURCE.contains("sizeof(struct mpro_frame_header) == 64"));
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    /// Read the ring exactly the way obs_plugin/mirror_source.c does.
    /// Returns (samples_delivered, overrun_detected).
    fn read_like_plugin(consumed: &mut u64, out: &mut Vec<f32>) -> bool {
        let shm = AUDIO_SHMEM.lock().unwrap();
        let base = shm.as_ref().unwrap().ptr;
        unsafe {
            let written =
                (&*(base.add(AUDIO_OFF_WRITTEN) as *const AtomicU64)).load(Ordering::Acquire);
            let data = base.add(AUDIO_HEADER_SIZE) as *const f32;

            let mut avail = written - *consumed;
            let mut overrun = false;
            if avail > AUDIO_BUFFER_SAMPLES as u64 {
                overrun = true;
                *consumed = written - AUDIO_BUFFER_SAMPLES as u64;
                avail = AUDIO_BUFFER_SAMPLES as u64;
            }

            let start = (*consumed % AUDIO_BUFFER_SAMPLES as u64) as usize;
            let total = avail as usize;
            let first = total.min(AUDIO_BUFFER_SAMPLES - start);
            for i in 0..first {
                out.push(*data.add(start + i));
            }
            for i in 0..(total - first) {
                out.push(*data.add(i));
            }
            *consumed += total as u64;
            overrun
        }
    }

    fn written_count() -> u64 {
        let shm = AUDIO_SHMEM.lock().unwrap();
        let base = shm.as_ref().unwrap().ptr;
        unsafe { (&*(base.add(AUDIO_OFF_WRITTEN) as *const AtomicU64)).load(Ordering::Acquire) }
    }

    #[test]
    fn ring_round_trips_and_reports_overrun() {
        assert!(init_audio(), "audio SHM init failed");
        set_enabled(true);

        // Header magic is what the plugin gates on.
        {
            let shm = AUDIO_SHMEM.lock().unwrap();
            let base = shm.as_ref().unwrap().ptr;
            let magic = unsafe { std::slice::from_raw_parts(base, 4) };
            assert_eq!(magic, b"MIRA");
            let session = unsafe { *(base.add(AUDIO_OFF_SESSION) as *const u64) };
            assert_ne!(session, 0, "session id must be set");
        }

        let mut consumed = written_count();
        let mut got: Vec<f32> = Vec::new();

        // ── Normal flow: everything arrives, in order ──
        let batch: Vec<f32> = (0..1000).map(|i| i as f32).collect();
        write_audio(&batch);
        assert_eq!(written_count(), consumed + 1000);
        assert!(!read_like_plugin(&mut consumed, &mut got));
        assert_eq!(got, batch, "samples must survive the ring round trip");

        // ── Wrap: a batch straddling the end of the ring ──
        got.clear();
        let big: Vec<f32> = (0..AUDIO_BUFFER_SAMPLES)
            .map(|i| (i % 997) as f32)
            .collect();
        // Two writes so the second necessarily wraps past the end.
        write_audio(&big[..AUDIO_BUFFER_SAMPLES - 500]);
        write_audio(&big[AUDIO_BUFFER_SAMPLES - 500..]);
        assert!(!read_like_plugin(&mut consumed, &mut got));
        assert_eq!(got.len(), AUDIO_BUFFER_SAMPLES);
        assert_eq!(got, big, "wrapped read must reassemble in order");

        // ── Overrun: writer laps a reader that stopped consuming ──
        got.clear();
        let stale_cursor = written_count();
        let mut lagging = stale_cursor;
        for _ in 0..3 {
            write_audio(&big); // three full buffers with no reads
        }
        let overrun = read_like_plugin(&mut lagging, &mut got);
        assert!(overrun, "lapping the reader must be detected, not silent");
        assert_eq!(
            got.len(),
            AUDIO_BUFFER_SAMPLES,
            "resyncs to the newest buffer"
        );
        assert_eq!(lagging, written_count(), "reader caught up to the writer");

        set_enabled(false);
        cleanup();
    }
}

/// Check whether ffplay is available (bundled or system).
pub fn check_ffplay_available(project_root: &str) -> bool {
    #[cfg(target_os = "windows")]
    let bundled = format!(r"{}\bin\ffplay.exe", project_root);
    #[cfg(not(target_os = "windows"))]
    let bundled = format!("{}/bin/ffplay", project_root);

    if std::path::Path::new(&bundled).exists() {
        return true;
    }

    #[cfg(target_os = "windows")]
    let finder = ("where", "ffplay");
    #[cfg(not(target_os = "windows"))]
    let finder = ("which", "ffplay");

    std::process::Command::new(finder.0)
        .arg(finder.1)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}
