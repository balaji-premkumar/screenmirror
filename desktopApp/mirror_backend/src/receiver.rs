use rusb::{Context as RusbContext, DeviceHandle, UsbContext};
use std::time::Duration;
use crate::push_packet;
use std::sync::Mutex;
use once_cell::sync::Lazy;
use std::fs::OpenOptions;
use std::io::Write;
use serde::Serialize;

pub static DISCOVERED_DEVICES: Lazy<Mutex<Vec<String>>> = Lazy::new(|| Mutex::new(Vec::new()));
static STREAMING_ACTIVE: Lazy<Mutex<bool>> = Lazy::new(|| Mutex::new(false));
pub static PENDING_CONFIG: Lazy<Mutex<Option<String>>> = Lazy::new(|| Mutex::new(None));
pub static FORCE_DISCONNECT: Lazy<Mutex<bool>> = Lazy::new(|| Mutex::new(false));
pub static AUTO_RECONNECT_ENABLED: Lazy<Mutex<bool>> = Lazy::new(|| Mutex::new(true));

/// Public accessor for the streaming state, used by get_status() in lib.rs
pub fn is_streaming() -> bool {
    *STREAMING_ACTIVE.lock().unwrap_or_else(|e| e.into_inner())
}

pub static LOG_BUFFER: Lazy<Mutex<Vec<LogEntry>>> = Lazy::new(|| Mutex::new(Vec::new()));
// Cursor to track which logs have been sent to the UI
static LOG_CURSOR: Lazy<Mutex<usize>> = Lazy::new(|| Mutex::new(0));

#[derive(Serialize, Clone)]
pub struct LogEntry {
    pub timestamp: String,
    pub level: String,
    pub module: String,
    pub thread: String,
    pub message: String,
}

/// Returns the platform-appropriate log directory path
fn get_log_dir() -> std::path::PathBuf {
    if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
        std::path::PathBuf::from(home).join(".mirror_stream").join("logs")
    } else {
        std::path::PathBuf::from("/tmp").join("mirror_stream").join("logs")
    }
}

pub fn log_event(level: &str, module: &str, thread: &str, message: &str) {
    let entry = LogEntry {
        timestamp: chrono::Local::now().format("%H:%M:%S%.3f").to_string(),
        level: level.to_string(),
        module: module.to_string(),
        thread: thread.to_string(),
        message: message.to_string(),
    };

    // Write to file log using dynamic path
    let log_dir = get_log_dir();
    let log_path = log_dir.join("mirror_rust.log.json");
    if let Ok(json) = serde_json::to_string(&entry) {
        let _ = std::fs::create_dir_all(&log_dir);
        if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&log_path) {
            let _ = writeln!(file, "{}", json);
        }
    }

    if let Ok(mut logs) = LOG_BUFFER.lock() {
        logs.push(entry);
        // Keep a generous buffer to allow UI to catch up, but cap at 500
        if logs.len() > 500 {
            // Trim the oldest 250, adjust cursor accordingly
            logs.drain(0..250);
            if let Ok(mut cursor) = LOG_CURSOR.lock() {
                *cursor = cursor.saturating_sub(250);
            }
        }
    }
}

/// Returns only NEW logs since the last call, enabling incremental/live updates
pub fn get_new_logs() -> Vec<LogEntry> {
    let logs = LOG_BUFFER.lock().unwrap();
    let mut cursor = LOG_CURSOR.lock().unwrap();
    let start = *cursor;
    let end = logs.len();
    if start >= end {
        return Vec::new();
    }
    let new_logs: Vec<LogEntry> = logs[start..end].to_vec();
    *cursor = end;
    new_logs
}

fn perform_aoa_handshake(handle: &mut DeviceHandle<RusbContext>) -> Result<(), rusb::Error> {
    let timeout = Duration::from_secs(1);
    let mut buf = [0u8; 2];

    log_event("INFO", "AOA", "handshake", "Requesting AOA Protocol version...");
    
    let mut protocol = 0;
    // Attempt multiple variants for picky devices
    for i in 0..5 {
        // Variant 1: standard index 0
        match handle.read_control(0xC0, 51, 0, 0, &mut buf, timeout) {
            Ok(_) => {
                protocol = u16::from_le_bytes(buf);
                if protocol >= 1 { break; }
            }
            Err(e) => {
                log_event("WARN", "AOA", "handshake", &format!("Handshake attempt {} failed: {:?}", i+1, e));
                std::thread::sleep(Duration::from_millis(500));
            }
        }
    }

    if protocol < 1 { 
        log_event("ERROR", "AOA", "handshake", "Device refused AOA (v0). Possible MTP lock or accessory already active.");
        return Err(rusb::Error::NotSupported); 
    }

    let strings = [
        "BalajiProjects",    // Index 0: manufacturer
        "MirrorReceiver",    // Index 1: model
        "Mirroring Stream",  // Index 2: description
        "1.0",               // Index 3: version
        "https://github.com",// Index 4: URI
        "12345678"           // Index 5: serial
    ];
    for (i, s) in strings.iter().enumerate() {
        match handle.write_control(0x40, 52, 0, i as u16, s.as_bytes(), timeout) {
            Ok(_) => log_event("INFO", "AOA", "handshake", &format!("String {} set: \"{}\"", i, s)),
            Err(e) => {
                log_event("ERROR", "AOA", "handshake", &format!("Failed to set string {}: {:?}", i, e));
                return Err(e);
            }
        }
    }

    log_event("SUCCESS", "AOA", "handshake", "Switching device to Accessory Mode...");
    handle.write_control(0x40, 53, 0, 0, &[], timeout)?;
    Ok(())
}

/// Guard to ensure STREAMING_ACTIVE is reset on thread exit/panic
struct StreamingActiveGuard;
impl Drop for StreamingActiveGuard {
    fn drop(&mut self) {
        if let Ok(mut active) = STREAMING_ACTIVE.lock() {
            *active = false;
        } else if let Err(e) = STREAMING_ACTIVE.lock() {
            *e.into_inner() = false;
        }
        log_event("WARN", "USB", "streaming", "Session guard dropped: Link state reset.");
    }
}

fn start_streaming_loop(device: rusb::Device<RusbContext>) {
    // Check if a session is already active
    {
        if let Ok(active) = STREAMING_ACTIVE.lock() {
            if *active { return; }
        }
    }

    std::thread::spawn(move || {
        let _guard = StreamingActiveGuard;
        {
            if let Ok(mut active) = STREAMING_ACTIVE.lock() {
                *active = true;
            }
        }

        log_event("INFO", "USB", "streaming", "Opening USB pipeline to device...");
        
        let mut handle = match device.open() {
            Ok(h) => h,
            Err(e) => {
                log_event("ERROR", "USB", "streaming", &format!("Open failed: {:?}. Is another udev/adb claiming it?", e));
                return;
            }
        };

        let _ = handle.set_auto_detach_kernel_driver(true);
        if let Err(e) = handle.claim_interface(0) {
            log_event("ERROR", "USB", "streaming", &format!("Claim failed: {:?}. ADB may be active.", e));
            return;
        }

        log_event("SUCCESS", "USB", "streaming", "Mobile link established. Pipeline active.");
        
        let mut endpoint_in = 0x81;
        let mut endpoint_out = 0x02;
        let mut found_out = false;

        if let Ok(config) = device.active_config_descriptor() {
            for interface in config.interfaces() {
                if interface.number() == 0 {
                    for idesc in interface.descriptors() {
                        for edesc in idesc.endpoint_descriptors() {
                            match (edesc.direction(), edesc.transfer_type()) {
                                (rusb::Direction::In, rusb::TransferType::Bulk) => {
                                    endpoint_in = edesc.address();
                                    log_event("INFO", "USB", "streaming", &format!("Inbound EP: 0x{:02X}", endpoint_in));
                                }
                                (rusb::Direction::Out, rusb::TransferType::Bulk) => {
                                    endpoint_out = edesc.address();
                                    found_out = true;
                                    log_event("INFO", "USB", "streaming", &format!("Outbound EP: 0x{:02X}", endpoint_out));
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
        }

        if !found_out {
            log_event("WARN", "USB", "streaming", "No specific OUT endpoint found, using default 0x02.");
        }

        let mut buf = vec![0u8; 1024 * 1024]; // 1MB read buffer for high-bitrate video
        let mut demuxer = crate::demuxer::Demuxer::new();
        let mut idle_seconds = 0;

        loop {
            // Check for termination flags
            if let Ok(mut fd) = FORCE_DISCONNECT.lock() {
                if *fd {
                    *fd = false;
                    log_event("WARN", "USB", "streaming", "User disconnect triggered.");
                    break;
                }
            }

            // Flush pending config commands
            if let Ok(mut pending) = PENDING_CONFIG.lock() {
                if let Some(config_json) = pending.take() {
                    let data = config_json.as_bytes();
                    log_event("INFO", "USB", "streaming", &format!("Syncing config: {} bytes", data.len()));
                    match handle.write_bulk(endpoint_out, data, Duration::from_millis(500)) {
                        Ok(n) => log_event("SUCCESS", "USB", "streaming", &format!("Config write success ({} bytes)", n)),
                        Err(e) => log_event("ERROR", "USB", "streaming", &format!("Sync error: {:?}", e)),
                    }
                }
            }

            // Stream data from USB
            match handle.read_bulk(endpoint_in, &mut buf, Duration::from_millis(1000)) {
                Ok(len) if len > 0 => {
                    idle_seconds = 0;
                    let frames = demuxer.feed(&buf[..len]);
                    for frame in frames {
                        if matches!(frame.frame_type, crate::demuxer::FrameType::Video) {
                            push_packet(frame.data.as_ptr(), frame.data.len());
                        }
                    }
                }
                Ok(_) => {}
                Err(rusb::Error::Timeout) => {
                    idle_seconds += 1;
                    if idle_seconds >= 5 {
                        log_event("ERROR", "USB", "streaming", "Inactivity timeout: mobile disconnected.");
                        break;
                    }
                }
                Err(e) => {
                    log_event("ERROR", "USB", "streaming", &format!("USB Read Error: {:?}", e));
                    break;
                }
            }
        }

        let _ = handle.release_interface(0);
        log_event("INFO", "USB", "streaming", "Cleanup complete. Thread exiting.");
    });
}

fn get_device_info(device: &rusb::Device<RusbContext>) -> Option<String> {
    if let Ok(handle) = device.open() {
        if let Ok(langs) = handle.read_languages(Duration::from_millis(200)) {
            if let Some(lang) = langs.first() {
                if let Ok(desc) = device.device_descriptor() {
                    let mfg = handle.read_manufacturer_string(*lang, &desc, Duration::from_millis(200)).unwrap_or_default();
                    let prod = handle.read_product_string(*lang, &desc, Duration::from_millis(200)).unwrap_or_default();
                    if !mfg.is_empty() || !prod.is_empty() {
                        return Some(format!("{} {}", mfg, prod).trim().to_string());
                    }
                }
            }
        }
    }
    None
}

pub fn trigger_manual_handshake(target_vid: u16, target_pid: u16) -> i32 {
    log_event("INFO", "FFI", "handshake", &format!("CLI Handshake for {:04X}:{:04X}", target_vid, target_pid));
    
    // Manual trigger re-enables auto-reconnect for this device re-enumeration
    if let Ok(mut auto) = AUTO_RECONNECT_ENABLED.lock() {
        *auto = true;
    }

    std::thread::spawn(move || {
        let context = match RusbContext::new() {
            Ok(c) => c,
            Err(e) => {
                log_event("ERROR", "FFI", "handshake", &format!("Context Error: {:?}", e));
                return;
            }
        };

        if let Ok(devices) = context.devices() {
            for device in devices.iter() {
                if let Ok(desc) = device.device_descriptor() {
                    if desc.vendor_id() == target_vid && desc.product_id() == target_pid {
                        match device.open() {
                            Ok(mut handle) => {
                                let _ = handle.set_auto_detach_kernel_driver(true);
                                let _ = handle.reset();
                                std::thread::sleep(Duration::from_millis(500));
                                if let Err(e) = perform_aoa_handshake(&mut handle) {
                                    log_event("ERROR", "FFI", "handshake", &format!("Handshake failed: {:?}", e));
                                } else {
                                    log_event("SUCCESS", "FFI", "handshake", "Switching to accessory mode...");
                                    drop(handle);
                                    wait_for_aoa_reenumeration(&context);
                                }
                                return;
                            }
                            Err(e) => {
                                log_event("ERROR", "FFI", "handshake", &format!("Open error: {:?}", e));
                                return;
                            }
                        }
                    }
                }
            }
        }
    });
    0
}

fn wait_for_aoa_reenumeration(context: &RusbContext) -> i32 {
    for _ in 0..15 {
        std::thread::sleep(Duration::from_millis(500));
        if let Ok(devices) = context.devices() {
            for device in devices.iter() {
                if let Ok(desc) = device.device_descriptor() {
                    let vid = desc.vendor_id();
                    let pid = desc.product_id();
                    if vid == 0x18D1 && (0x2D00..=0x2D05).contains(&pid) {
                        log_event("SUCCESS", "RE-ENUM", "handshake", "AOA Accessory found.");
                        return 0;
                    }
                }
            }
        }
    }
    log_event("ERROR", "RE-ENUM", "handshake", "Device re-enumeration timeout.");
    -4
}

pub fn start_usb_listener_thread() {
    std::thread::spawn(move || {
        let context = match RusbContext::new() {
            Ok(c) => c,
            Err(e) => {
                log_event("ERROR", "SYSTEM", "discovery", &format!("Fatal Rust Context Error: {:?}", e));
                return;
            }
        };
        log_event("INFO", "SYSTEM", "discovery", "Engine background scanning loop active.");
        
        let mut info_cache: std::collections::HashMap<String, String> = std::collections::HashMap::new();

        loop {
            let mut candidates = Vec::new();
            let streaming = is_streaming();

            if let Ok(devices) = context.devices() {
                for device in devices.iter() {
                    let desc = match device.device_descriptor() { Ok(d) => d, Err(_) => continue };
                    let vid = desc.vendor_id();
                    let pid = desc.product_id();
                    let device_key = format!("{:04X}:{:04X}_{:?}", vid, pid, device.address());

                    if vid == 0x18D1 && (0x2D00..=0x2D05).contains(&pid) {
                        // It's an accessory. 
                        let info = if streaming {
                            // Don't disturb the stream by opening the device if we are already active
                            info_cache.get(&device_key).cloned().unwrap_or_else(|| "AOA Accessory".to_string())
                        } else {
                            let info = get_device_info(&device).unwrap_or_else(|| "AOA Accessory".to_string());
                            info_cache.insert(device_key.clone(), info.clone());
                            info
                        };

                        candidates.push(format!("Accessory|{}|{:04X}:{:04X}", info, vid, pid));
                        
                        if let Ok(auto) = AUTO_RECONNECT_ENABLED.lock() {
                            if *auto {
                                start_streaming_loop(device);
                            }
                        }
                    } else {
                        let mut android_candidate = false;
                        if let Ok(config) = device.active_config_descriptor() {
                            for intf in config.interfaces() {
                                for alt in intf.descriptors() {
                                    if alt.class_code() == 0xFF { android_candidate = true; break; }
                                }
                            }
                        }
                        if android_candidate {
                            let info = if let Some(cached) = info_cache.get(&device_key) {
                                cached.clone()
                            } else {
                                let info = get_device_info(&device).unwrap_or_else(|| "Android Device".to_string());
                                info_cache.insert(device_key.clone(), info.clone());
                                info
                            };
                            candidates.push(format!("Phone|{}|{:04X}:{:04X}", info, vid, pid));
                        }
                    }
                }
            }
            if let Ok(mut list) = DISCOVERED_DEVICES.lock() { *list = candidates; }
            std::thread::sleep(Duration::from_secs(2));
        }
    });
}
