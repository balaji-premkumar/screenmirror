use anyhow::{Context, Result};
use concurrent_queue::ConcurrentQueue;
use once_cell::sync::Lazy;
use shared_memory::{Shmem, ShmemConf};
use std::sync::Arc;
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub mod audio;
pub mod decoder;
pub mod demuxer;
pub mod metrics;
pub mod obs_feed;
pub mod receiver;
pub mod renderer;

#[repr(C)]
pub struct FrameHeader {
    pub magic: [u8; 4],
    pub width: u32,
    pub height: u32,
    pub format: u32, // 0 = BGRA, 1 = NV12, 2 = I420
    pub timestamp: u64,
}

pub struct MirrorState {
    pub shmem: Shmem,
    pub queue: Arc<ConcurrentQueue<Vec<u8>>>,
    pub width: u32,
    pub height: u32,
}

unsafe impl Send for MirrorState {}
unsafe impl Sync for MirrorState {}

pub static STATE: Lazy<Mutex<Option<MirrorState>>> = Lazy::new(|| Mutex::new(None));

#[no_mangle]
pub extern "C" fn open_native_preview(project_root: *const libc::c_char) -> i32 {
    let root = unsafe {
        if project_root.is_null() {
            ""
        } else {
            std::ffi::CStr::from_ptr(project_root)
                .to_str()
                .unwrap_or("")
        }
    };
    renderer::start_native_preview(root);
    0
}

// ── OBS & System Detection ──────────────────────────────────

#[no_mangle]
pub extern "C" fn check_obs_installed() -> i32 {
    if obs_feed::check_obs_installed() {
        1
    } else {
        0
    }
}

#[no_mangle]
pub extern "C" fn check_obs_plugin_installed() -> i32 {
    if obs_feed::check_plugin_installed() {
        1
    } else {
        0
    }
}

#[no_mangle]
pub extern "C" fn check_ffplay_available(project_root: *const libc::c_char) -> i32 {
    let root = unsafe {
        if project_root.is_null() {
            ""
        } else {
            std::ffi::CStr::from_ptr(project_root)
                .to_str()
                .unwrap_or("")
        }
    };
    if obs_feed::check_ffplay_available(root) {
        1
    } else {
        0
    }
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
            std::ffi::CStr::from_ptr(project_root)
                .to_str()
                .unwrap_or(".")
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
        // Also disable auto-reconnect for this session
        if let Ok(mut auto) = receiver::AUTO_RECONNECT_ENABLED.lock() {
            *auto = false;
        }
        // Reset metrics to starting point
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
            if let Ok(mut config) = receiver::PENDING_CONFIG.lock() {
                *config = Some(s.to_string());

                if s.contains("\"command\":\"start\"") {
                    if let Ok(mut auto) = receiver::AUTO_RECONNECT_ENABLED.lock() {
                        *auto = true;
                    }
                } else if s.contains("\"command\":\"stop\"") {
                    if let Ok(mut auto) = receiver::AUTO_RECONNECT_ENABLED.lock() {
                        *auto = false;
                    }
                }
                return 0;
            }
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
    let logs = receiver::LOG_BUFFER
        .lock()
        .unwrap_or_else(|e| e.into_inner());
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
    let snapshot = manager.get_snapshot();
    let json = serde_json::to_string(&snapshot).unwrap_or_else(|_| "{}".to_string());
    let c_str = std::ffi::CString::new(json.replace('\0', "")).unwrap_or_default();
    c_str.into_raw()
}

#[no_mangle]
pub extern "C" fn check_driver_status() -> i32 {
    #[cfg(target_os = "linux")]
    {
        // Check both filenames for compatibility:
        // - setup_udev.sh installs to 51-android-aoa.rules
        // - setup_linux_permissions() installs to 51-android-aoa.rules
        let path_primary = std::path::Path::new("/etc/udev/rules.d/51-android-aoa.rules");
        let path_legacy = std::path::Path::new("/etc/udev/rules.d/99-android-mirror.rules");
        if path_primary.exists() || path_legacy.exists() {
            1
        } else {
            0
        }
    }
    #[cfg(not(target_os = "linux"))]
    1
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

#[no_mangle]
pub extern "C" fn install_windows_driver() -> i32 {
    #[cfg(target_os = "windows")]
    {
        receiver::log_event(
            "WARN",
            "DRIVER",
            "setup",
            "Windows Driver Installation is not implemented. Please install the driver manually.",
        );
        0
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
        use std::process::Command;
        // Aligned with setup_udev.sh — both use 51-android-aoa.rules
        let rule_path = "/etc/udev/rules.d/51-android-aoa.rules";
        if std::path::Path::new(rule_path).exists() {
            return 0;
        }
        let content = "SUBSYSTEM==\"usb\", ATTR{idVendor}==\"18d1\", MODE=\"0666\"\n\
                       SUBSYSTEM==\"usb\", ATTR{idVendor}==\"2d95\", MODE=\"0666\"\n\
                       SUBSYSTEM==\"usb\", ATTR{idVendor}==\"04e8\", MODE=\"0666\"\n";
        match std::fs::write("/tmp/51-android-aoa.rules", content) {
            Ok(_) => {
                receiver::log_event(
                    "INFO",
                    "DRIVER",
                    "setup",
                    "Requesting OS permissions via pkexec...",
                );
                let _ = Command::new("pkexec")
                    .arg("cp")
                    .arg("/tmp/51-android-aoa.rules")
                    .arg(rule_path)
                    .status();
                let _ = Command::new("pkexec")
                    .arg("udevadm")
                    .arg("control")
                    .arg("--reload-rules")
                    .status();
                let _ = Command::new("pkexec")
                    .arg("udevadm")
                    .arg("trigger")
                    .status();

                // Re-verify after pkexec
                if std::path::Path::new(rule_path).exists() {
                    receiver::log_event(
                        "SUCCESS",
                        "DRIVER",
                        "setup",
                        "Udev rules installed successfully.",
                    );
                    return 1;
                }
                return -1;
            }

            Err(_) => return -1,
        }
    }
    #[cfg(not(target_os = "linux"))]
    0
}

#[no_mangle]
pub extern "C" fn init_mirror(width: u32, height: u32) -> i32 {
    let header_size = std::mem::size_of::<FrameHeader>();
    // Allocate enough for any frame up to 4K, not just the init dimensions.
    // Decoded frames from phones can be portrait or higher-res.
    let max_data_size = 3840 * 2160 * 4; // 4K UHD BGRA
    let total_size = header_size + max_data_size;
    let shmem = match ShmemConf::new()
        .size(total_size)
        .os_id("obs_mirror_buffer")
        .create()
    {
        Ok(s) => s,
        Err(_) => match ShmemConf::new().os_id("obs_mirror_buffer").open() {
            Ok(s) => s,
            Err(_) => return -1,
        },
    };
    let queue = Arc::new(ConcurrentQueue::bounded(2));
    decoder::start_decoder_thread(queue.clone());
    receiver::start_usb_listener_thread();

    // Initialise the OBS shared memory feed (separate from the internal shmem)
    obs_feed::init(width, height);

    if let Ok(mut state_lock) = STATE.lock() {
        *state_lock = Some(MirrorState {
            shmem,
            queue,
            width,
            height,
        });
    }
    0
}

#[no_mangle]
pub unsafe extern "C" fn write_frame_to_obs(data: *const u8, len: usize, width: u32, height: u32, timestamp: u64, format: u32) -> i32 {
    if let Ok(mut state_lock) = STATE.lock() {
        if let Some(state) = state_lock.as_mut() {
            let start = Instant::now();
            let header_size = std::mem::size_of::<FrameHeader>();

            let ptr = state.shmem.as_ptr();
            let header = FrameHeader {
                magic: *b"MIRR",
                width,
                height,
                format,
                timestamp,
            };
            std::ptr::copy_nonoverlapping(
                &header as *const FrameHeader as *const u8,
                ptr,
                header_size,
            );
            let data_ptr = ptr.add(header_size);
            let available = state.shmem.len() - header_size;
            let copy_len = len.min(available);
            std::ptr::copy_nonoverlapping(data, data_ptr, copy_len);

            let mut m = metrics::METRICS.lock().unwrap_or_else(|e| e.into_inner());
            m.record_frame(len, start.elapsed().as_millis() as u64);

            // Also push to OBS shared memory feed if enabled
            obs_feed::write_frame(data, len, width, height, format, timestamp);

            return 0;
        }
    }
    -1
}

#[no_mangle]
pub extern "C" fn push_packet(data: *const u8, len: usize) -> i32 {
    let slice = unsafe { std::slice::from_raw_parts(data, len) };

    if let Ok(mut state_lock) = STATE.lock() {
        if let Some(state) = state_lock.as_mut() {
            if state.queue.push(slice.to_vec()).is_err() {
                let mut m = metrics::METRICS.lock().unwrap_or_else(|e| e.into_inner());
                m.record_drop();
            }
            return 0;
        }
    }
    -1
}

#[no_mangle]
pub extern "C" fn get_status() -> i32 {
    // Return 1 only when a USB device is actively connected and streaming,
    // not just when the mirror state has been initialized.
    if STATE.lock().unwrap_or_else(|e| e.into_inner()).is_some() && receiver::is_streaming() {
        1
    } else {
        0
    }
}

#[no_mangle]
pub extern "C" fn get_buffer_size() -> i32 {
    if let Ok(state_lock) = STATE.lock() {
        if let Some(state) = state_lock.as_ref() {
            return state.queue.len() as i32;
        }
    }
    -1
}
