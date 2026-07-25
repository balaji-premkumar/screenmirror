use once_cell::sync::Lazy;
use rusb::{Context as RusbContext, DeviceHandle, UsbContext};
use serde::Serialize;
use std::io::Write;
use std::sync::mpsc;
use std::sync::Mutex;
use std::time::{Duration, Instant};

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
        std::path::PathBuf::from(home)
            .join(".mirror_stream")
            .join("logs")
    } else {
        std::env::temp_dir().join("mirror_stream").join("logs")
    }
}

/// Rotate the log once it passes this size, keeping one previous generation.
/// Without this `mirror_rust.log.json` grew without limit for the lifetime of
/// the install.
const LOG_ROTATE_BYTES: u64 = 5 * 1024 * 1024;
/// Bounded so a burst of errors cannot grow the channel without limit. Logs
/// are diagnostics: dropping a few under pressure beats unbounded memory on
/// the streaming threads.
const LOG_CHANNEL_CAPACITY: usize = 4096;

fn open_log_file(log_path: &std::path::Path) -> Option<std::fs::File> {
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
        .ok()
}

/// Dedicated log writer thread. Streaming threads only push into a channel;
/// all file I/O (open/append/flush) happens here, off the hot path.
static LOG_TX: Lazy<Mutex<mpsc::SyncSender<LogEntry>>> = Lazy::new(|| {
    let (tx, rx) = mpsc::sync_channel::<LogEntry>(LOG_CHANNEL_CAPACITY);
    std::thread::spawn(move || {
        let log_dir = get_log_dir();
        let _ = std::fs::create_dir_all(&log_dir);
        let log_path = log_dir.join("mirror_rust.log.json");
        let rotated = log_dir.join("mirror_rust.log.json.1");

        let mut file = open_log_file(&log_path);
        let mut written: u64 = std::fs::metadata(&log_path).map(|m| m.len()).unwrap_or(0);

        while let Ok(entry) = rx.recv() {
            let Ok(json) = serde_json::to_string(&entry) else {
                continue;
            };
            if let Some(f) = file.as_mut() {
                if writeln!(f, "{}", json).is_ok() {
                    written += json.len() as u64 + 1;
                }
            }
            if written >= LOG_ROTATE_BYTES {
                // Close the handle before renaming — Windows refuses to
                // rename a file that still has an open handle.
                drop(file.take());
                let _ = std::fs::rename(&log_path, &rotated);
                file = open_log_file(&log_path);
                written = 0;
            }
        }
    });
    Mutex::new(tx)
});

pub fn log_event(level: &str, module: &str, thread: &str, message: &str) {
    let entry = LogEntry {
        timestamp: chrono::Local::now().format("%H:%M:%S%.3f").to_string(),
        level: level.to_string(),
        module: module.to_string(),
        thread: thread.to_string(),
        message: message.to_string(),
    };

    // Hand off to the writer thread — no file I/O here, and try_send so a
    // backed-up writer can never block a streaming thread.
    if let Ok(tx) = LOG_TX.lock() {
        let _ = tx.try_send(entry.clone());
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
    let logs = LOG_BUFFER.lock().unwrap_or_else(|e| e.into_inner());
    let mut cursor = LOG_CURSOR.lock().unwrap_or_else(|e| e.into_inner());
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

    log_event(
        "INFO",
        "AOA",
        "handshake",
        "Requesting AOA Protocol version...",
    );

    let mut protocol = 0;
    // Attempt multiple variants for picky devices
    for i in 0..5 {
        match handle.read_control(0xC0, 51, 0, 0, &mut buf, timeout) {
            Ok(_) => {
                protocol = u16::from_le_bytes(buf);
                if protocol >= 1 {
                    break;
                }
            }
            Err(e) => {
                log_event(
                    "WARN",
                    "AOA",
                    "handshake",
                    &format!("Handshake attempt {} failed: {:?}", i + 1, e),
                );
                std::thread::sleep(Duration::from_millis(500));
            }
        }
    }

    if protocol < 1 {
        log_event(
            "ERROR",
            "AOA",
            "handshake",
            "Device refused AOA (v0). Possible MTP lock or accessory already active.",
        );
        return Err(rusb::Error::NotSupported);
    }

    let strings = [
        "BalajiProjects",     // Index 0: manufacturer
        "MirrorReceiver",     // Index 1: model
        "Mirroring Stream",   // Index 2: description
        "1.0",                // Index 3: version
        "https://github.com", // Index 4: URI
        "12345678",           // Index 5: serial
    ];
    for (i, s) in strings.iter().enumerate() {
        match handle.write_control(0x40, 52, 0, i as u16, s.as_bytes(), timeout) {
            Ok(_) => log_event(
                "INFO",
                "AOA",
                "handshake",
                &format!("String {} set: \"{}\"", i, s),
            ),
            Err(e) => {
                log_event(
                    "ERROR",
                    "AOA",
                    "handshake",
                    &format!("Failed to set string {}: {:?}", i, e),
                );
                return Err(e);
            }
        }
    }

    log_event(
        "SUCCESS",
        "AOA",
        "handshake",
        "Switching device to Accessory Mode...",
    );
    handle.write_control(0x40, 53, 0, 0, &[], timeout)?;
    Ok(())
}

/// Guard to ensure STREAMING_ACTIVE is reset on thread exit/panic
struct StreamingActiveGuard;
impl Drop for StreamingActiveGuard {
    fn drop(&mut self) {
        let mut active = STREAMING_ACTIVE.lock().unwrap_or_else(|e| e.into_inner());
        *active = false;
        log_event(
            "WARN",
            "USB",
            "streaming",
            "Session guard dropped: Link state reset.",
        );
    }
}

fn start_streaming_loop(device: rusb::Device<RusbContext>, my_gen: u64) {
    // Check if a session is already active
    {
        if let Ok(mut active) = STREAMING_ACTIVE.lock() {
            if *active {
                return;
            }
            *active = true; // Set active IMMEDIATELY to prevent UI flickering
        }
    }

    // Tells the decoder this is a fresh bitstream, so it flushes references
    // from the previous session instead of decoding against them.
    crate::STREAM_EPOCH.fetch_add(1, std::sync::atomic::Ordering::AcqRel);

    std::thread::spawn(move || {
        let _guard = StreamingActiveGuard;

        log_event("INFO", "USB", "streaming", "Starting USB session thread...");

        let handle = match device.open() {
            Ok(h) => h,
            Err(e) => {
                log_event(
                    "ERROR",
                    "USB",
                    "streaming",
                    &format!("Open failed: {:?}. Connection abandoned.", e),
                );
                return;
            }
        };

        let _ = handle.set_auto_detach_kernel_driver(true);
        if let Err(e) = handle.claim_interface(0) {
            log_event(
                "ERROR",
                "USB",
                "streaming",
                &format!("Claim failed: {:?}. Device busy.", e),
            );
            return;
        }

        log_event(
            "SUCCESS",
            "USB",
            "streaming",
            "Mobile link established. Interface 0 claimed.",
        );

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
                                }
                                (rusb::Direction::Out, rusb::TransferType::Bulk) => {
                                    endpoint_out = edesc.address();
                                    found_out = true;
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
        }

        if !found_out {
            log_event(
                "WARN",
                "USB",
                "streaming",
                "Specific OUT endpoint not detected, using default 0x02.",
            );
        }

        let mut buf = vec![0u8; 1024 * 1024]; // 1MB read buffer
        let mut demuxer = crate::demuxer::Demuxer::new();
        let mut last_activity = Instant::now();
        // Short timeout so pending config commands and shutdown signals are
        // observed quickly; at 120 fps data arrives every ~8 ms anyway, so
        // the timeout only fires when the link is idle.
        let timeout_duration = Duration::from_millis(100);

        loop {
            if !crate::session_alive(my_gen) {
                log_event(
                    "INFO",
                    "USB",
                    "streaming",
                    "Streaming loop received termination signal.",
                );
                break;
            }

            // 1. Check for user-triggered disconnect
            if let Ok(mut fd) = FORCE_DISCONNECT.lock() {
                if *fd {
                    *fd = false;
                    log_event("WARN", "USB", "streaming", "User disconnect triggered.");
                    break;
                }
            }

            // 2. Flush pending config commands (Critical for 'Start' button)
            let mut current_config = None;
            if let Ok(mut pending) = PENDING_CONFIG.lock() {
                current_config = pending.take();
            }

            if let Some(config_json) = current_config {
                let mut data = config_json.as_bytes().to_vec();
                data.push(0); // Null terminator

                log_event(
                    "INFO",
                    "USB",
                    "streaming",
                    &format!("Sending config: {} bytes", data.len()),
                );
                match handle.write_bulk(endpoint_out, &data, Duration::from_millis(1000)) {
                    Ok(n) => log_event(
                        "SUCCESS",
                        "USB",
                        "streaming",
                        &format!("Config sent ({} bytes)", n),
                    ),
                    Err(e) => {
                        log_event(
                            "ERROR",
                            "USB",
                            "streaming",
                            &format!("Config write error: {:?}", e),
                        );
                        // Re-queue
                        if let Ok(mut pending) = PENDING_CONFIG.lock() {
                            if pending.is_none() {
                                *pending = Some(config_json);
                            }
                        }
                    }
                }
            }

            // 3. Stream data from USB
            match handle.read_bulk(endpoint_in, &mut buf, timeout_duration) {
                Ok(len) if len > 0 => {
                    last_activity = Instant::now();
                    if let Ok(mut m) = crate::metrics::METRICS.lock() {
                        m.record_usb_bytes(len);
                    }

                    let frames = demuxer.feed(&buf[..len]);
                    for frame in frames {
                        match frame.frame_type {
                            crate::demuxer::FrameType::Video => {
                                // ffplay gets the encoded stream verbatim; it
                                // is a no-op (one atomic load) when no player
                                // session is running.
                                crate::player::push_video(&frame.data);
                                // Moves the Vec — no copy on this path.
                                crate::push_video_packet(frame.data);
                            }
                            crate::demuxer::FrameType::Audio => {
                                crate::audio::push_audio(&frame.data);
                            }
                        }
                    }
                }
                Ok(_) | Err(rusb::Error::Timeout) => {
                    if last_activity.elapsed() >= Duration::from_secs(5) {
                        log_event(
                            "ERROR",
                            "USB",
                            "streaming",
                            "Inactivity timeout: mobile disconnected.",
                        );
                        break;
                    }
                }
                Err(e) => {
                    log_event(
                        "ERROR",
                        "USB",
                        "streaming",
                        &format!("Fatal Read Error: {:?}. Closing link.", e),
                    );
                    break;
                }
            }
        }

        let _ = handle.release_interface(0);
        log_event("INFO", "USB", "streaming", "Session thread ended.");
    });
}

fn get_device_info(device: &rusb::Device<RusbContext>) -> Option<String> {
    if let Ok(handle) = device.open() {
        if let Ok(langs) = handle.read_languages(Duration::from_millis(200)) {
            if let Some(lang) = langs.first() {
                if let Ok(desc) = device.device_descriptor() {
                    let mfg = handle
                        .read_manufacturer_string(*lang, &desc, Duration::from_millis(200))
                        .unwrap_or_default();
                    let prod = handle
                        .read_product_string(*lang, &desc, Duration::from_millis(200))
                        .unwrap_or_default();
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
    log_event(
        "INFO",
        "FFI",
        "handshake",
        &format!("CLI Handshake for {:04X}:{:04X}", target_vid, target_pid),
    );

    // Manual trigger re-enables auto-reconnect for this device re-enumeration
    if let Ok(mut auto) = AUTO_RECONNECT_ENABLED.lock() {
        *auto = true;
    }

    let my_gen = crate::current_gen();
    std::thread::spawn(move || {
        let context = match RusbContext::new() {
            Ok(c) => c,
            Err(e) => {
                log_event(
                    "ERROR",
                    "FFI",
                    "handshake",
                    &format!("Context Error: {:?}", e),
                );
                return;
            }
        };

        if let Ok(devices) = context.devices() {
            for device in devices.iter() {
                if let Ok(desc) = device.device_descriptor() {
                    if desc.vendor_id() == target_vid && desc.product_id() == target_pid {
                        // If it's already an accessory, connect immediately instead
                        // of waiting for the next discovery poll.
                        if target_vid == 0x18D1 && (0x2D00..=0x2D05).contains(&target_pid) {
                            log_event(
                                "INFO",
                                "FFI",
                                "handshake",
                                "Device is already an accessory, connecting directly.",
                            );
                            if let Ok(mut auto) = AUTO_RECONNECT_ENABLED.lock() {
                                *auto = true;
                            }
                            start_streaming_loop(device, my_gen);
                            return;
                        }

                        match device.open() {
                            Ok(mut handle) => {
                                let _ = handle.set_auto_detach_kernel_driver(true);
                                let _ = handle.reset();
                                std::thread::sleep(Duration::from_millis(500));
                                if let Err(e) = perform_aoa_handshake(&mut handle) {
                                    log_event(
                                        "ERROR",
                                        "FFI",
                                        "handshake",
                                        &format!("Handshake failed: {:?}", e),
                                    );
                                } else {
                                    log_event(
                                        "SUCCESS",
                                        "FFI",
                                        "handshake",
                                        "Switching to accessory mode...",
                                    );
                                    drop(handle);
                                    wait_for_aoa_reenumeration(&context);
                                }
                                return;
                            }
                            Err(e) => {
                                log_event(
                                    "ERROR",
                                    "FFI",
                                    "handshake",
                                    &format!("Open error: {:?}", e),
                                );
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
    log_event(
        "ERROR",
        "RE-ENUM",
        "handshake",
        "Device re-enumeration timeout.",
    );
    -4
}

pub fn start_usb_listener_thread(my_gen: u64) {
    std::thread::spawn(move || {
        let context = match RusbContext::new() {
            Ok(c) => c,
            Err(e) => {
                log_event(
                    "ERROR",
                    "SYSTEM",
                    "discovery",
                    &format!("Fatal Rust Context Error: {:?}", e),
                );
                return;
            }
        };
        log_event(
            "INFO",
            "SYSTEM",
            "discovery",
            "Engine background scanning loop active.",
        );

        let mut info_cache: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();

        loop {
            if !crate::session_alive(my_gen) {
                log_event(
                    "INFO",
                    "SYSTEM",
                    "discovery",
                    "Discovery thread received termination signal.",
                );
                break;
            }

            let mut candidates = Vec::new();
            let streaming = is_streaming();

            match context.devices() {
                Ok(devices) => {
                    for device in devices.iter() {
                        let desc = match device.device_descriptor() {
                            Ok(d) => d,
                            Err(_) => continue,
                        };
                        let vid = desc.vendor_id();
                        let pid = desc.product_id();
                        let device_key = format!("{:04X}:{:04X}_{:?}", vid, pid, device.address());

                        if vid == 0x18D1 && (0x2D00..=0x2D05).contains(&pid) {
                            // It's an accessory.
                            let info = if streaming {
                                info_cache
                                    .get(&device_key)
                                    .cloned()
                                    .unwrap_or_else(|| "AOA Accessory".to_string())
                            } else {
                                let info = get_device_info(&device)
                                    .unwrap_or_else(|| "AOA Accessory".to_string());
                                if info_cache.len() >= 64 {
                                    info_cache.clear();
                                }
                                info_cache.insert(device_key.clone(), info.clone());
                                info
                            };

                            candidates.push(format!("Accessory|{}|{:04X}:{:04X}", info, vid, pid));

                            if let Ok(auto) = AUTO_RECONNECT_ENABLED.lock() {
                                if *auto {
                                    start_streaming_loop(device, my_gen);
                                }
                            }
                        } else {
                            let mut android_candidate = false;
                            if let Ok(config) = device.active_config_descriptor() {
                                for intf in config.interfaces() {
                                    for alt in intf.descriptors() {
                                        if alt.class_code() == 0xFF {
                                            android_candidate = true;
                                            break;
                                        }
                                    }
                                }
                            }
                            if android_candidate {
                                let info = if let Some(cached) = info_cache.get(&device_key) {
                                    cached.clone()
                                } else {
                                    let info = get_device_info(&device)
                                        .unwrap_or_else(|| "Android Device".to_string());
                                    // Keys include the bus address, which
                                    // changes on every re-enumeration, so the
                                    // map would otherwise grow for the life of
                                    // the process.
                                    if info_cache.len() >= 64 {
                                        info_cache.clear();
                                    }
                                    info_cache.insert(device_key.clone(), info.clone());
                                    info
                                };
                                candidates.push(format!("Phone|{}|{:04X}:{:04X}", info, vid, pid));
                            }
                        }
                    }
                    // Only update list if we successfully polled the bus
                    if let Ok(mut list) = DISCOVERED_DEVICES.lock() {
                        *list = candidates;
                    }
                }
                Err(e) => {
                    log_event(
                        "WARN",
                        "SYSTEM",
                        "discovery",
                        &format!("USB Bus poll failed: {:?}", e),
                    );
                }
            }
            std::thread::sleep(Duration::from_secs(2));
        }
    });
}
