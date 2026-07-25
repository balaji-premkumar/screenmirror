use once_cell::sync::Lazy;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

pub mod audio;
pub mod decoder;
pub mod demuxer;
pub mod framepool;
pub mod metrics;
pub mod obs_feed;
pub mod pipeline;
pub mod player;
pub mod receiver;
pub mod shared_mem;
#[cfg(target_os = "windows")]
pub mod windows_driver;

/// Session generation counter used for cooperative shutdown.
///
/// Every background thread captures the generation at spawn time and exits
/// its loop as soon as the global value no longer matches. Unlike a boolean
/// flag that gets reset, a monotonically increasing generation can never be
/// "missed" by a thread that was blocked in a syscall when stop was called.
pub static SESSION_GEN: AtomicU64 = AtomicU64::new(0);

pub fn current_gen() -> u64 {
    SESSION_GEN.load(Ordering::Acquire)
}

pub fn session_alive(my_gen: u64) -> bool {
    SESSION_GEN.load(Ordering::Acquire) == my_gen
}

/// Bumped whenever a new USB streaming session begins. The decoder watches it
/// so it can flush reference frames at a stream discontinuity instead of
/// decoding the new stream against the old one's references.
pub static STREAM_EPOCH: AtomicU64 = AtomicU64::new(0);

pub fn stream_epoch() -> u64 {
    STREAM_EPOCH.load(Ordering::Acquire)
}

/// Monotonic nanoseconds since process start.
static EPOCH: Lazy<Instant> = Lazy::new(Instant::now);

/// A real clock for the frame header the OBS plugin reads.
///
/// This field used to carry `FRAME_COUNTER` — a frame index, not a time — so
/// anything downstream treating it as a timestamp was reading nonsense. It is
/// still a *receive* time, not a capture time: honest end-to-end latency needs
/// a sender-side timestamp on the wire (see ISSUES.md item 1).
pub fn now_nanos() -> u64 {
    EPOCH.elapsed().as_nanos() as u64
}

/// Encoded-video ingress queue (USB thread → decoder thread).
/// 32 packets ≈ 250 ms at 120 fps — enough to absorb scheduler jitter
/// without adding perceptible latency.
pub static VIDEO_QUEUE: Lazy<Arc<pipeline::VideoQueue>> =
    Lazy::new(|| Arc::new(pipeline::VideoQueue::new(32)));

/// Triple-buffer SHM writer for the OBS plugin. Set once at init.
static TRIPLE_BUFFER: Lazy<Mutex<Option<Arc<shared_mem::TripleBufferManager>>>> =
    Lazy::new(|| Mutex::new(None));

static INITIALIZED: Lazy<Mutex<bool>> = Lazy::new(|| Mutex::new(false));

// ── Lifecycle ───────────────────────────────────────────────

#[no_mangle]
pub extern "C" fn init_mirror(_width: u32, _height: u32) -> i32 {
    let mut inited = INITIALIZED.lock().unwrap_or_else(|e| e.into_inner());
    if *inited {
        return 0; // idempotent — never spawn duplicate threads
    }

    let trbuff = match shared_mem::TripleBufferManager::create("obs_mirror_buffer") {
        Ok(t) => Arc::new(t),
        Err(e) => {
            receiver::log_event(
                "ERROR",
                "SYSTEM",
                "init",
                &format!("Shared memory init failed: {e}"),
            );
            return -1;
        }
    };

    if let Ok(mut tb) = TRIPLE_BUFFER.lock() {
        *tb = Some(trbuff);
    }

    obs_feed::init_audio();

    let gen = current_gen();
    decoder::start_decoder_thread(VIDEO_QUEUE.clone(), gen);
    receiver::start_usb_listener_thread(gen);

    *inited = true;
    0
}

#[no_mangle]
pub extern "C" fn stop_mirror() -> i32 {
    // Bumping the generation makes every session thread exit its loop at the
    // next iteration — no flag reset race, no sleep required.
    SESSION_GEN.fetch_add(1, Ordering::AcqRel);

    VIDEO_QUEUE.clear();
    player::stop();
    obs_feed::set_enabled(false);
    obs_feed::cleanup();

    if let Ok(mut tb) = TRIPLE_BUFFER.lock() {
        *tb = None;
    }
    if let Ok(mut inited) = INITIALIZED.lock() {
        *inited = false;
    }

    receiver::log_event(
        "SUCCESS",
        "SYSTEM",
        "shutdown",
        "Mirroring session stopped and cleaned up.",
    );
    0
}

// ── Sinks ───────────────────────────────────────────────────

/// Does anything still need a *decoded* frame?
///
/// Only the OBS shared-memory feed consumes decoded BGRA. The player gets an
/// HEVC passthrough stream and decodes it inside ffplay, so it only needs the
/// decoder long enough to learn the frame size for the Matroska header.
pub fn needs_decoded_frames() -> bool {
    obs_feed::is_enabled() || player::needs_dimensions()
}

// ── Frame delivery (decoder thread → OBS SHM) ──────────────

/// Deliver a decoded BGRA frame. Takes ownership of `buffer` (a pooled
/// allocation) and returns it to the pool when done.
///
/// `decode_started` is when the packet left the ingress queue, so the recorded
/// latency covers decode + colour conversion + sink write. The old figure
/// timed only this function's own body — essentially the memcpy — and reported
/// it to the UI as "pipeline latency".
pub fn deliver_frame(buffer: Vec<u8>, width: u32, height: u32, decode_started: Instant) {
    // Lets a pending player session fill in the Matroska track header.
    player::note_dimensions(width, height);

    // OBS shared memory (only when the user enabled the feed — skips an
    // 8 MB memcpy per frame otherwise). The mutex below is only taken on this
    // branch, and never on a preview-only session.
    if obs_feed::is_enabled() {
        let tb = TRIPLE_BUFFER
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if let Some(tb) = tb {
            let _ = tb.write_frame(width, height, now_nanos(), &buffer);
        }
    }

    if let Ok(mut m) = metrics::METRICS.lock() {
        m.record_frame(buffer.len(), decode_started.elapsed().as_millis() as u64);
    }

    framepool::release(buffer);
}

// ── Packet ingress (USB thread → decoder queue) ─────────────

/// Move a demuxed video packet into the decode queue. Never blocks.
pub fn push_video_packet(data: Vec<u8>) {
    let dropped = VIDEO_QUEUE.push(data);
    if dropped > 0 {
        if let Ok(mut m) = metrics::METRICS.lock() {
            for _ in 0..dropped {
                m.record_drop();
            }
        }
    }
}

/// Legacy C ABI entry point (copies). Prefer `push_video_packet`.
#[no_mangle]
pub extern "C" fn push_packet(data: *const u8, len: usize) -> i32 {
    if data.is_null() {
        return -1;
    }
    let slice = unsafe { std::slice::from_raw_parts(data, len) };
    push_video_packet(slice.to_vec());
    0
}

// ── FFI: status, config, devices, logs, metrics ─────────────

// ── Player (ffplay) ─────────────────────────────────────────

/// Start playback in a child ffplay window (video *and* audio).
/// Returns 0 on success, -1 when ffplay could not be launched.
#[no_mangle]
pub extern "C" fn start_player(project_root: *const libc::c_char) -> i32 {
    let root = unsafe {
        if project_root.is_null() {
            ""
        } else {
            std::ffi::CStr::from_ptr(project_root).to_str().unwrap_or("")
        }
    };
    player::start(root)
}

#[no_mangle]
pub extern "C" fn stop_player() -> i32 {
    player::stop();
    0
}

/// 0 = stopped, 1 = starting (waiting for a keyframe), 2 = playing.
#[no_mangle]
pub extern "C" fn get_player_state() -> i32 {
    player::state()
}

#[no_mangle]
pub extern "C" fn check_obs_installed() -> i32 {
    obs_feed::check_obs_installed() as i32
}

/// Is OBS Studio actually running? The UI gates the feed toggle on this.
#[no_mangle]
pub extern "C" fn check_obs_running() -> i32 {
    obs_feed::check_obs_running() as i32
}

#[no_mangle]
pub extern "C" fn check_obs_plugin_installed() -> i32 {
    obs_feed::check_plugin_installed() as i32
}

#[no_mangle]
pub extern "C" fn check_ffplay_available(project_root: *const libc::c_char) -> i32 {
    let root = unsafe {
        if project_root.is_null() {
            ""
        } else {
            std::ffi::CStr::from_ptr(project_root).to_str().unwrap_or("")
        }
    };
    obs_feed::check_ffplay_available(root) as i32
}

#[no_mangle]
pub extern "C" fn get_obs_plugin_dir() -> *mut libc::c_char {
    let dir = obs_feed::get_obs_plugin_dir().unwrap_or_default();
    let c_str = std::ffi::CString::new(dir).unwrap_or_default();
    c_str.into_raw()
}

#[no_mangle]
pub extern "C" fn install_obs_plugin(project_root: *const libc::c_char) -> i32 {
    let root = unsafe {
        if project_root.is_null() {
            "."
        } else {
            std::ffi::CStr::from_ptr(project_root).to_str().unwrap_or(".")
        }
    };
    obs_feed::install_plugin(root)
}

#[no_mangle]
pub extern "C" fn toggle_obs_feed(enabled: i32) {
    obs_feed::set_enabled(enabled != 0);
}

#[no_mangle]
pub extern "C" fn trigger_manual_handshake(vid: u16, pid: u16) -> i32 {
    if let Ok(mut fd) = receiver::FORCE_DISCONNECT.lock() {
        *fd = false;
    }
    receiver::trigger_manual_handshake(vid, pid)
}

#[no_mangle]
pub extern "C" fn toggle_auto_reconnect(enabled: i32) {
    if let Ok(mut fd) = receiver::FORCE_DISCONNECT.lock() {
        *fd = false;
    }
    let mut auto = receiver::AUTO_RECONNECT_ENABLED
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    *auto = enabled != 0;
}

#[no_mangle]
pub extern "C" fn force_disconnect() -> i32 {
    if let Ok(mut flag) = receiver::FORCE_DISCONNECT.lock() {
        *flag = true;
        if let Ok(mut auto) = receiver::AUTO_RECONNECT_ENABLED.lock() {
            *auto = false;
        }
        if let Ok(mut m) = metrics::METRICS.lock() {
            m.reset();
        }
        return 0;
    }
    -1
}

#[no_mangle]
pub extern "C" fn sync_config(json: *const libc::c_char) -> i32 {
    unsafe {
        if json.is_null() {
            return -1;
        }
        let c_str = std::ffi::CStr::from_ptr(json);
        if let Ok(s) = c_str.to_str() {
            // Parse the command properly instead of substring matching.
            let command = serde_json::from_str::<serde_json::Value>(s)
                .ok()
                .and_then(|v| v.get("command").and_then(|c| c.as_str()).map(String::from));

            if let Ok(mut config) = receiver::PENDING_CONFIG.lock() {
                *config = Some(s.to_string());
            }

            match command.as_deref() {
                Some("start") => {
                    if let Ok(mut auto) = receiver::AUTO_RECONNECT_ENABLED.lock() {
                        *auto = true;
                    }
                }
                Some("stop") => {
                    if let Ok(mut auto) = receiver::AUTO_RECONNECT_ENABLED.lock() {
                        *auto = false;
                    }
                }
                _ => {}
            }
            return 0;
        }
    }
    -1
}

#[no_mangle]
pub extern "C" fn get_devices() -> *mut libc::c_char {
    let list = receiver::DISCOVERED_DEVICES
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let combined = list.join(",");
    let c_str = std::ffi::CString::new(combined.replace('\0', "")).unwrap_or_default();
    c_str.into_raw()
}

#[no_mangle]
pub extern "C" fn get_structured_logs() -> *mut libc::c_char {
    let logs = receiver::LOG_BUFFER.lock().unwrap_or_else(|e| e.into_inner());
    let json = serde_json::to_string(&*logs).unwrap_or_else(|_| "[]".to_string());
    let c_str = std::ffi::CString::new(json.replace('\0', "")).unwrap_or_default();
    c_str.into_raw()
}

#[no_mangle]
pub extern "C" fn get_new_logs() -> *mut libc::c_char {
    let new_logs = receiver::get_new_logs();
    let json = serde_json::to_string(&new_logs).unwrap_or_else(|_| "[]".to_string());
    let c_str = std::ffi::CString::new(json.replace('\0', "")).unwrap_or_default();
    c_str.into_raw()
}

#[no_mangle]
pub extern "C" fn get_metrics() -> *mut libc::c_char {
    let mut manager = metrics::METRICS.lock().unwrap_or_else(|e| e.into_inner());
    let snapshot = manager.get_snapshot(VIDEO_QUEUE.len(), VIDEO_QUEUE.capacity());
    let json = serde_json::to_string(&snapshot).unwrap_or_else(|_| "{}".to_string());
    let c_str = std::ffi::CString::new(json.replace('\0', "")).unwrap_or_default();
    c_str.into_raw()
}

#[no_mangle]
pub extern "C" fn get_status() -> i32 {
    if receiver::is_streaming() {
        1
    } else {
        0
    }
}

#[no_mangle]
pub extern "C" fn get_buffer_size() -> i32 {
    VIDEO_QUEUE.len() as i32
}

#[no_mangle]
pub extern "C" fn free_string(s: *mut libc::c_char) {
    unsafe {
        if s.is_null() {
            return;
        }
        let _ = std::ffi::CString::from_raw(s);
    }
}

// ── Platform: drivers & permissions ─────────────────────────

#[no_mangle]
pub extern "C" fn check_driver_status() -> i32 {
    #[cfg(target_os = "linux")]
    {
        let path_primary = std::path::Path::new("/etc/udev/rules.d/51-android-aoa.rules");
        let path_legacy = std::path::Path::new("/etc/udev/rules.d/99-android-mirror.rules");
        if path_primary.exists() || path_legacy.exists() {
            1
        } else {
            0
        }
    }
    #[cfg(target_os = "windows")]
    {
        windows_driver::check_driver_status()
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    1
}

#[no_mangle]
pub extern "C" fn install_windows_driver() -> i32 {
    #[cfg(target_os = "windows")]
    {
        windows_driver::install_driver()
    }
    #[cfg(not(target_os = "windows"))]
    {
        0
    }
}

#[no_mangle]
pub extern "C" fn setup_linux_permissions() -> i32 {
    #[cfg(target_os = "linux")]
    {
        use std::io::Write;
        use std::process::{Command, Stdio};

        const RULE_PATH: &str = "/etc/udev/rules.d/51-android-aoa.rules";

        if std::path::Path::new(RULE_PATH).exists() {
            return 0;
        }

        // Scope the rule to the Android accessory PIDs with uaccess
        // (session-local access) instead of a world-writable 0666 node.
        // Only the AOA product range is matched — an unqualified vendor match
        // would hand every device from that vendor to the local session.
        const CONTENT: &str = "\
# ScreenMirror — Android Open Accessory access for the active session\n\
SUBSYSTEM==\"usb\", ATTR{idVendor}==\"18d1\", ATTR{idProduct}==\"2d0?\", TAG+=\"uaccess\"\n";

        receiver::log_event(
            "INFO",
            "DRIVER",
            "setup",
            "Requesting OS permissions via pkexec...",
        );

        // The rule is streamed to the elevated shell on stdin and written by
        // root itself. Staging it in a shared directory first (the old
        // `/tmp/51-android-aoa.rules` + `cp`) let any local user pre-create or
        // swap that path between the write and the copy, so root would install
        // attacker-supplied udev rules — and udev rules can carry RUN+=, which
        // makes that arbitrary code execution as root.
        //
        // RULE_PATH is a compile-time constant, so nothing user-controlled
        // reaches the shell.
        let script = format!(
            "umask 022 && cat > {RULE_PATH} && \
             udevadm control --reload-rules && udevadm trigger"
        );

        let child = Command::new("pkexec")
            .arg("sh")
            .arg("-c")
            .arg(&script)
            .stdin(Stdio::piped())
            .spawn();

        let mut child = match child {
            Ok(c) => c,
            Err(e) => {
                receiver::log_event(
                    "ERROR",
                    "DRIVER",
                    "setup",
                    &format!("Could not launch pkexec: {e}"),
                );
                return -1;
            }
        };

        if let Some(mut stdin) = child.stdin.take() {
            if stdin.write_all(CONTENT.as_bytes()).is_err() {
                let _ = child.kill();
                let _ = child.wait();
                return -1;
            }
            // Close the pipe so `cat` sees EOF and the script can proceed.
            drop(stdin);
        }

        match child.wait() {
            Ok(status) if status.success() && std::path::Path::new(RULE_PATH).exists() => {
                receiver::log_event(
                    "SUCCESS",
                    "DRIVER",
                    "setup",
                    "Udev rules installed. Replug the device to activate them.",
                );
                1
            }
            _ => {
                receiver::log_event(
                    "ERROR",
                    "DRIVER",
                    "setup",
                    "Udev rule installation failed or was cancelled at the prompt.",
                );
                -1
            }
        }
    }
    #[cfg(not(target_os = "linux"))]
    0
}
