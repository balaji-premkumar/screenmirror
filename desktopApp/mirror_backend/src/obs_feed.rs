/// OBS Feed Module
///
/// Manages a raw POSIX shared memory segment (`/mirror_obs_feed`) that an OBS
/// Studio source plugin can map and read decoded video frames from.
///
/// The layout is intentionally simple and crate-independent so that the
/// companion C plugin can open the same segment with a plain `shm_open`+`mmap`.
///
/// Memory layout:
///   [24B FrameHeader] [width * height * 4 bytes BGRA pixel data]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use once_cell::sync::Lazy;
use crate::receiver::log_event;

// ── Toggle ──────────────────────────────────────────────────
static OBS_ENABLED: AtomicBool = AtomicBool::new(false);

pub fn set_enabled(enabled: bool) {
    OBS_ENABLED.store(enabled, Ordering::Relaxed);
    log_event("INFO", "OBS", "feed", &format!("OBS shared memory feed {}", if enabled { "ENABLED" } else { "DISABLED" }));
}

pub fn is_enabled() -> bool {
    OBS_ENABLED.load(Ordering::Relaxed)
}

// ── Shared memory handle ────────────────────────────────────
struct ObsShmem {
    ptr: *mut u8,
    size: usize,
    fd: i32,
}

unsafe impl Send for ObsShmem {}
unsafe impl Sync for ObsShmem {}

static OBS_SHMEM: Lazy<Mutex<Option<ObsShmem>>> = Lazy::new(|| Mutex::new(None));

#[cfg(target_os = "linux")]
const SHM_NAME: &[u8] = b"/mirror_obs_feed\0";

/// Initialise the OBS shared memory segment.
/// Called once from `init_mirror()`.
pub fn init(width: u32, height: u32) -> bool {
    #[cfg(target_os = "linux")]
    {
        let header_size = std::mem::size_of::<crate::FrameHeader>();
        let data_size = (width * height * 4) as usize;
        let total = header_size + data_size;

        unsafe {
            // Remove stale segment from a previous crash
            libc::shm_unlink(SHM_NAME.as_ptr() as *const libc::c_char);

            let fd = libc::shm_open(
                SHM_NAME.as_ptr() as *const libc::c_char,
                libc::O_CREAT | libc::O_RDWR,
                0o666,
            );
            if fd < 0 {
                log_event("ERROR", "OBS", "shmem", "shm_open failed");
                return false;
            }

            if libc::ftruncate(fd, total as libc::off_t) != 0 {
                log_event("ERROR", "OBS", "shmem", "ftruncate failed");
                libc::close(fd);
                return false;
            }

            let ptr = libc::mmap(
                std::ptr::null_mut(),
                total,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                fd,
                0,
            );

            if ptr == libc::MAP_FAILED {
                log_event("ERROR", "OBS", "shmem", "mmap failed");
                libc::close(fd);
                return false;
            }

            // Zero out the memory so OBS sees an empty frame initially
            std::ptr::write_bytes(ptr as *mut u8, 0, total);

            if let Ok(mut shmem) = OBS_SHMEM.lock() {
                *shmem = Some(ObsShmem {
                    ptr: ptr as *mut u8,
                    size: total,
                    fd,
                });
            }
        }

        log_event("SUCCESS", "OBS", "shmem", &format!(
            "OBS feed shared memory initialised: {}x{} ({} bytes) at /dev/shm/mirror_obs_feed",
            width, height, total
        ));
        true
    }

    #[cfg(not(target_os = "linux"))]
    {
        log_event("WARN", "OBS", "shmem", "OBS shared memory feed only supported on Linux");
        false
    }
}

/// Write a decoded frame to the OBS shared memory segment.
/// Called from `write_frame_to_obs()` in lib.rs when the OBS feed is enabled.
pub fn write_frame(data: *const u8, len: usize, width: u32, height: u32, timestamp: u64) {
    if !is_enabled() { return; }

    if let Ok(shmem) = OBS_SHMEM.lock() {
        if let Some(ref shm) = *shmem {
            unsafe {
                let header = crate::FrameHeader {
                    magic: *b"MIRR",
                    width,
                    height,
                    timestamp,
                };
                let header_size = std::mem::size_of::<crate::FrameHeader>();
                std::ptr::copy_nonoverlapping(
                    &header as *const crate::FrameHeader as *const u8,
                    shm.ptr,
                    header_size,
                );
                let data_ptr = shm.ptr.add(header_size);
                let copy_len = len.min((width * height * 4) as usize);
                std::ptr::copy_nonoverlapping(data, data_ptr, copy_len);
            }
        }
    }
}

/// Clean up the shared memory segment on shutdown.
pub fn cleanup() {
    #[cfg(target_os = "linux")]
    {
        if let Ok(mut shmem) = OBS_SHMEM.lock() {
            if let Some(shm) = shmem.take() {
                unsafe {
                    libc::munmap(shm.ptr as *mut libc::c_void, shm.size);
                    libc::close(shm.fd);
                    libc::shm_unlink(SHM_NAME.as_ptr() as *const libc::c_char);
                }
                log_event("INFO", "OBS", "shmem", "OBS feed shared memory released");
            }
        }
    }
}

// ── OBS Detection & Plugin Management ───────────────────────

/// Check whether OBS Studio is installed on this system.
pub fn check_obs_installed() -> bool {
    #[cfg(target_os = "linux")]
    {
        // Native package
        if std::process::Command::new("which").arg("obs")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status().map(|s| s.success()).unwrap_or(false) {
            return true;
        }
        // Flatpak
        if std::process::Command::new("flatpak")
            .args(["info", "com.obsproject.Studio"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status().map(|s| s.success()).unwrap_or(false) {
            return true;
        }
        // Snap
        if std::process::Command::new("snap")
            .args(["list", "obs-studio"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status().map(|s| s.success()).unwrap_or(false) {
            return true;
        }
        false
    }
    #[cfg(target_os = "windows")]
    {
        let paths = [
            r"C:\Program Files\obs-studio",
            r"C:\Program Files (x86)\obs-studio",
        ];
        paths.iter().any(|p| std::path::Path::new(p).exists())
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    false
}

/// Find the user-level OBS plugin directory.
pub fn get_obs_plugin_dir() -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        let home = std::env::var("HOME").unwrap_or_default();

        // Flatpak install
        let flatpak_config = format!("{}/.var/app/com.obsproject.Studio/config/obs-studio", home);
        if std::path::Path::new(&flatpak_config).exists() {
            return Some(format!("{}/plugins", flatpak_config));
        }

        // Native install — use ~/.config/obs-studio/ (OBS 28+ default)
        let config_dir = format!("{}/.config/obs-studio", home);
        if std::path::Path::new(&config_dir).exists() {
            return Some(format!("{}/plugins", config_dir));
        }

        // Legacy path
        let legacy_dir = format!("{}/.obs-studio", home);
        if std::path::Path::new(&legacy_dir).exists() {
            return Some(format!("{}/plugins", legacy_dir));
        }

        // If OBS is installed but hasn't been run yet, use the modern default
        if check_obs_installed() {
            return Some(format!("{}/plugins", config_dir));
        }

        None
    }
    #[cfg(not(target_os = "linux"))]
    None
}

/// Check whether our OBS plugin is already installed.
pub fn check_plugin_installed() -> bool {
    if let Some(plugin_dir) = get_obs_plugin_dir() {
        let so_path = format!("{}/mirror-source/bin/64bit/mirror-source.so", plugin_dir);
        std::path::Path::new(&so_path).exists()
    } else {
        false
    }
}

/// Build and install the OBS plugin.
/// Returns 0 on success, -1 on failure.
pub fn install_plugin(project_root: &str) -> i32 {
    log_event("INFO", "OBS", "install", "Starting OBS plugin build & install...");

    let plugin_dir = match get_obs_plugin_dir() {
        Some(d) => d,
        None => {
            log_event("ERROR", "OBS", "install", "Cannot find OBS plugin directory");
            return -1;
        }
    };

    let source_dir = format!("{}/obs_plugin", project_root);
    let build_dir = format!("{}/build", source_dir);

    // Create build directory
    let _ = std::fs::create_dir_all(&build_dir);

    // Check for libobs-dev
    #[cfg(target_os = "linux")]
    {
        // Check if pre-compiled exists
        let precompiled = format!("{}/mirror-source.so", build_dir);
        if !std::path::Path::new(&precompiled).exists() {
            log_event("ERROR", "OBS", "install", "Pre-compiled plugin (mirror-source.so) not found in build directory.");
            return -1;
        }

        // Install to OBS plugin directory
        let install_dir = format!("{}/mirror-source/bin/64bit", plugin_dir);
        if std::fs::create_dir_all(&install_dir).is_err() {
            log_event("ERROR", "OBS", "install", "Failed to create plugin install directory");
            return -1;
        }

        let dst = format!("{}/mirror-source.so", install_dir);
        if std::fs::copy(&precompiled, &dst).is_err() {
            log_event("ERROR", "OBS", "install", "Failed to copy plugin binary");
            return -1;
        }

        log_event("SUCCESS", "OBS", "install", &format!("Plugin installed to {}", dst));
    }

    0
}

/// Check whether ffplay is available (bundled or system).
pub fn check_ffplay_available(project_root: &str) -> bool {
    let bundled = format!("{}/bin/ffplay", project_root);
    if std::path::Path::new(&bundled).exists() {
        return true;
    }

    std::process::Command::new("which").arg("ffplay")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}
