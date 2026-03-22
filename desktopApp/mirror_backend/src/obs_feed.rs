use crate::receiver::log_event;
use once_cell::sync::Lazy;
/// OBS Feed Module
///
/// Manages a raw POSIX shared memory segment (`/mirror_obs_feed`) that an OBS
/// Studio source plugin can map and read decoded video frames from.
///
/// The layout is intentionally simple and crate-independent so that the
/// companion C plugin can open the same segment with a plain `shm_open`+`mmap`.
///
/// Memory layout:
///   [24B FrameHeader] [pixel data up to MAX_FRAME_WIDTH * MAX_FRAME_HEIGHT * 4 bytes]
///
/// The SHM is allocated large enough for any frame up to 4K. The FrameHeader
/// contains the actual width/height so the consumer knows how much data to read.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

// ── Toggle ──────────────────────────────────────────────────
static OBS_ENABLED: AtomicBool = AtomicBool::new(false);

pub fn set_enabled(enabled: bool) {
    OBS_ENABLED.store(enabled, Ordering::Relaxed);
    log_event(
        "INFO",
        "OBS",
        "feed",
        &format!(
            "OBS shared memory feed {}",
            if enabled { "ENABLED" } else { "DISABLED" }
        ),
    );
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
static AUDIO_SHMEM: Lazy<Mutex<Option<ObsShmem>>> = Lazy::new(|| Mutex::new(None));

#[cfg(target_os = "linux")]
const SHM_NAME: &[u8] = b"/mirror_obs_feed\0";

#[cfg(target_os = "linux")]
const AUDIO_SHM_NAME: &[u8] = b"/mirror_obs_audio\0";

#[repr(C)]
struct AudioShmHeader {
    magic: [u8; 4],
    head: u32,
    // float data[96000] follows
}

const AUDIO_BUFFER_SAMPLES: usize = 96000;
const AUDIO_SHM_SIZE: usize = std::mem::size_of::<AudioShmHeader>() + (AUDIO_BUFFER_SAMPLES * 4);

// Max frame size we'll ever need to handle (4K UHD)
const MAX_FRAME_WIDTH: usize = 3840;
const MAX_FRAME_HEIGHT: usize = 2160;
const MAX_PIXEL_DATA: usize = MAX_FRAME_WIDTH * MAX_FRAME_HEIGHT * 4;

/// Initialise the OBS shared memory segment.
/// Called once from `init_mirror()`.
pub fn init(width: u32, height: u32) -> bool {
    #[cfg(target_os = "linux")]
    {
        let header_size = std::mem::size_of::<crate::FrameHeader>();
        // Allocate enough for any frame up to MAX resolution, not just the init dimensions.
        // This prevents overflow when the phone sends portrait or higher-res frames.
        let total = header_size + MAX_PIXEL_DATA;

        unsafe {
            // Video SHM
            libc::shm_unlink(SHM_NAME.as_ptr() as *const libc::c_char);
            let fd = libc::shm_open(SHM_NAME.as_ptr() as *const libc::c_char, libc::O_CREAT | libc::O_RDWR, 0o666);
            if fd >= 0 {
                libc::ftruncate(fd, total as libc::off_t);
                let ptr = libc::mmap(std::ptr::null_mut(), total, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_SHARED, fd, 0);
                if ptr != libc::MAP_FAILED {
                    std::ptr::write_bytes(ptr as *mut u8, 0, total);
                    if let Ok(mut shmem) = OBS_SHMEM.lock() {
                        *shmem = Some(ObsShmem { ptr: ptr as *mut u8, size: total, fd });
                    }
                }
            }

            // Audio SHM
            libc::shm_unlink(AUDIO_SHM_NAME.as_ptr() as *const libc::c_char);
            let afd = libc::shm_open(AUDIO_SHM_NAME.as_ptr() as *const libc::c_char, libc::O_CREAT | libc::O_RDWR, 0o666);
            if afd >= 0 {
                libc::ftruncate(afd, AUDIO_SHM_SIZE as libc::off_t);
                let aptr = libc::mmap(std::ptr::null_mut(), AUDIO_SHM_SIZE, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_SHARED, afd, 0);
                if aptr != libc::MAP_FAILED {
                    std::ptr::write_bytes(aptr as *mut u8, 0, AUDIO_SHM_SIZE);
                    let hdr = aptr as *mut AudioShmHeader;
                    (*hdr).magic = *b"MIRA";
                    (*hdr).head = 0;
                    if let Ok(mut shmem) = AUDIO_SHMEM.lock() {
                        *shmem = Some(ObsShmem { ptr: aptr as *mut u8, size: AUDIO_SHM_SIZE, fd: afd });
                    }
                }
            }
        }

        log_event(
            "SUCCESS",
            "OBS",
            "shmem",
            &format!("OBS feed shared memory initialised: {}x{}", width, height),
        );
        true
    }

    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}

pub fn write_audio(samples: &[f32]) {
    if !is_enabled() {
        return;
    }

    if let Ok(shmem_opt) = AUDIO_SHMEM.lock() {
        if let Some(ref shm) = *shmem_opt {
            unsafe {
                let hdr = shm.ptr as *mut AudioShmHeader;
                let data_ptr = (shm.ptr.add(std::mem::size_of::<AudioShmHeader>())) as *mut f32;
                
                let mut head = (*hdr).head as usize;
                
                for &sample in samples {
                    *data_ptr.add(head) = sample;
                    head = (head + 1) % AUDIO_BUFFER_SAMPLES;
                }
                
                // Write head atomically (or close enough for our lock-free needs)
                std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::Release);
                (*hdr).head = head as u32;
            }
        }
    }
}

/// Write a decoded frame to the OBS shared memory segment.
/// Called from `write_frame_to_obs()` in lib.rs when the OBS feed is enabled.
pub fn write_frame(data: *const u8, len: usize, width: u32, height: u32, format: u32, timestamp: u64) {
    if !is_enabled() {
        return;
    }

    if let Ok(shmem) = OBS_SHMEM.lock() {
        if let Some(ref shm) = *shmem {
            let header_size = std::mem::size_of::<crate::FrameHeader>();
            let pixel_data_needed = (width as usize) * (height as usize) * 4;
            let required = header_size + pixel_data_needed;

            // Safety: only write if the frame fits in our pre-allocated SHM
            if required > shm.size {
                return; // Frame too large, skip it rather than overflow
            }

            unsafe {
                let header = crate::FrameHeader {
                    magic: *b"MIRR",
                    width,
                    height,
                    format,
                    timestamp,
                };
                std::ptr::copy_nonoverlapping(
                    &header as *const crate::FrameHeader as *const u8,
                    shm.ptr,
                    header_size,
                );
                let data_ptr = shm.ptr.add(header_size);
                let copy_len = len.min(pixel_data_needed);
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

const PLUGIN_VERSION: &str = "1.1.0";

/// Check whether our OBS plugin is already installed and up to date.
pub fn check_plugin_installed() -> bool {
    if let Some(plugin_dir) = get_obs_plugin_dir() {
        let base_path = format!("{}/mirror-source", plugin_dir);
        let so_path = format!("{}/bin/64bit/mirror-source.so", base_path);
        let version_path = format!("{}/version.txt", base_path);

        if !std::path::Path::new(&so_path).exists() {
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

/// Build and install the OBS plugin.
/// Returns 0 on success, -1 on failure.
pub fn install_plugin(project_root: &str) -> i32 {
    log_event(
        "INFO",
        "OBS",
        "install",
        &format!("Starting OBS plugin build & install (v{})...", PLUGIN_VERSION),
    );

    let plugin_dir = match get_obs_plugin_dir() {
        Some(d) => d,
        None => {
            log_event(
                "ERROR",
                "OBS",
                "install",
                "Cannot find OBS plugin directory",
            );
            return -1;
        }
    };

    let source_dir = format!("{}/obs_plugin", project_root);
    let build_dir = format!("{}/build", source_dir);

    // Ensure build directory exists
    let _ = std::fs::create_dir_all(&build_dir);

    #[cfg(target_os = "linux")]
    {
        // 1. Try to find a pre-bundled plugin in the bin/ directory
        let bundled_so = format!("{}/bin/mirror-source.so", project_root);
        let precompiled_dev = format!("{}/build/mirror-source.so", source_dir);
        
        let plugin_src = if std::path::Path::new(&bundled_so).exists() {
            Some(bundled_so)
        } else if std::path::Path::new(&precompiled_dev).exists() {
            Some(precompiled_dev.clone())
        } else {
            None
        };

        if plugin_src.is_none() {
             log_event("INFO", "OBS", "install", "Plugin not found in bin/, attempting local compile...");
             let status = std::process::Command::new("gcc")
                .args([
                    "-shared", "-fPIC", 
                    "-o", &precompiled_dev,
                    &format!("{}/mirror_source.c", source_dir),
                    "-I/usr/include/obs", "-lobs", "-lrt"
                ])
                .status();
                
             if status.is_err() || !status.unwrap().success() {
                log_event("ERROR", "OBS", "install", "Failed to compile plugin locally. Is libobs-dev installed?");
                return -1;
             }
        }

        let final_src = plugin_src.unwrap_or(precompiled_dev);

        // Install to OBS plugin directory
        let base_install_dir = format!("{}/mirror-source", plugin_dir);
        let bin_install_dir = format!("{}/bin/64bit", base_install_dir);
        
        if std::fs::create_dir_all(&bin_install_dir).is_err() {
            log_event(
                "ERROR",
                "OBS",
                "install",
                "Failed to create plugin install directory",
            );
            return -1;
        }

        let dst = format!("{}/mirror-source.so", bin_install_dir);
        log_event("INFO", "OBS", "install", &format!("Copying plugin from {} to {}", final_src, dst));
        if let Err(e) = std::fs::copy(&final_src, &dst) {
            log_event("ERROR", "OBS", "install", &format!("Failed to copy plugin binary: {}", e));
            return -1;
        }

        // Write version file
        let version_path = format!("{}/version.txt", base_install_dir);
        if let Err(e) = std::fs::write(&version_path, PLUGIN_VERSION) {
             log_event("WARN", "OBS", "install", &format!("Failed to write version file to {}: {}", version_path, e));
        }

        log_event(
            "SUCCESS",
            "OBS",
            "install",
            &format!("Plugin v{} installed to {}", PLUGIN_VERSION, dst),
        );
    }

    0
}


/// Check whether native preview is available (now always true as it's built-in).
pub fn check_ffplay_available(_project_root: &str) -> bool {
    true
}
