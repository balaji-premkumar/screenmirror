//! USB discovery, the AOA handshake, and the streaming session.
//!
//! This is the only module that talks to libusb. It hands demuxed video
//! packets to `pipeline` and audio straight to `sinks`, and reports what it is
//! doing through `telemetry::log`.

use crate::log_event;
use mirror_i18n::codes;
use once_cell::sync::Lazy;
use rusb::{Context as RusbContext, DeviceHandle, UsbContext};
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

fn perform_aoa_handshake(handle: &mut DeviceHandle<RusbContext>) -> Result<(), rusb::Error> {
    let timeout = Duration::from_secs(1);
    let mut buf = [0u8; 2];

    log_event!(codes::AOA_HANDSHAKE_REQUESTING_VERSION);

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
                log_event!(codes::AOA_HANDSHAKE_ATTEMPT_FAILED, "attempt" => i + 1, "error" => format!("{e:?}"));
                std::thread::sleep(Duration::from_millis(500));
            }
        }
    }

    if protocol < 1 {
        log_event!(codes::AOA_HANDSHAKE_REFUSED);
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
            Ok(_) => log_event!(codes::AOA_HANDSHAKE_STRING_SET, "index" => i, "value" => s),
            Err(e) => {
                log_event!(codes::AOA_HANDSHAKE_STRING_FAILED, "index" => i, "error" => format!("{e:?}"));
                return Err(e);
            }
        }
    }

    log_event!(codes::AOA_HANDSHAKE_SWITCHING);
    handle.write_control(0x40, 53, 0, 0, &[], timeout)?;
    Ok(())
}

/// Guard to ensure STREAMING_ACTIVE is reset on thread exit/panic
struct StreamingActiveGuard;
impl Drop for StreamingActiveGuard {
    fn drop(&mut self) {
        let mut active = STREAMING_ACTIVE.lock().unwrap_or_else(|e| e.into_inner());
        *active = false;
        log_event!(codes::USB_STREAMING_SESSION_RESET);
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

        log_event!(codes::USB_STREAMING_THREAD_STARTED);

        let handle = match device.open() {
            Ok(h) => h,
            Err(e) => {
                log_event!(codes::USB_STREAMING_OPEN_FAILED, "error" => format!("{e:?}"));
                return;
            }
        };

        let _ = handle.set_auto_detach_kernel_driver(true);
        if let Err(e) = handle.claim_interface(0) {
            log_event!(codes::USB_STREAMING_CLAIM_FAILED, "error" => format!("{e:?}"));
            return;
        }

        log_event!(codes::USB_STREAMING_LINK_ESTABLISHED);

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
            log_event!(codes::USB_STREAMING_DEFAULT_ENDPOINT);
        }

        let mut buf = vec![0u8; 1024 * 1024]; // 1MB read buffer
        let mut demuxer = crate::pipeline::demuxer::Demuxer::new();
        let mut last_activity = Instant::now();
        // Short timeout so pending config commands and shutdown signals are
        // observed quickly; at 120 fps data arrives every ~8 ms anyway, so
        // the timeout only fires when the link is idle.
        let timeout_duration = Duration::from_millis(100);

        loop {
            if !crate::session_alive(my_gen) {
                log_event!(codes::USB_STREAMING_THREAD_STOPPING);
                break;
            }

            // 1. Check for user-triggered disconnect
            if let Ok(mut fd) = FORCE_DISCONNECT.lock() {
                if *fd {
                    *fd = false;
                    log_event!(codes::USB_STREAMING_USER_DISCONNECT);
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

                log_event!(codes::USB_STREAMING_CONFIG_SENDING, "bytes" => data.len());
                match handle.write_bulk(endpoint_out, &data, Duration::from_millis(1000)) {
                    Ok(n) => log_event!(codes::USB_STREAMING_CONFIG_SENT, "bytes" => n),
                    Err(e) => {
                        log_event!(codes::USB_STREAMING_CONFIG_FAILED, "error" => format!("{e:?}"));
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
                    if let Ok(mut m) = crate::telemetry::metrics::METRICS.lock() {
                        m.record_usb_bytes(len);
                    }

                    let frames = demuxer.feed(&buf[..len]);
                    for frame in frames {
                        match frame.packet_type {
                            mirror_protocol::PacketType::Video => {
                                // ffplay gets the encoded stream verbatim; it
                                // is a no-op (one atomic load) when no player
                                // session is running.
                                crate::sinks::player::push_video(&frame.data);
                                // Moves the Vec — no copy on this path.
                                crate::push_video_packet(frame.data);
                            }
                            mirror_protocol::PacketType::Audio => {
                                crate::sinks::push_audio(&frame.data);
                            }
                        }
                    }
                }
                Ok(_) | Err(rusb::Error::Timeout) => {
                    if last_activity.elapsed() >= Duration::from_secs(5) {
                        log_event!(codes::USB_STREAMING_INACTIVITY_TIMEOUT);
                        break;
                    }
                }
                Err(e) => {
                    log_event!(codes::USB_STREAMING_READ_FAILED, "error" => format!("{e:?}"));
                    break;
                }
            }
        }

        let _ = handle.release_interface(0);
        log_event!(codes::USB_STREAMING_THREAD_ENDED);
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
    log_event!(codes::FFI_HANDSHAKE_REQUESTED, "vid" => format!("{target_vid:04X}"), "pid" => format!("{target_pid:04X}"));

    // Manual trigger re-enables auto-reconnect for this device re-enumeration
    if let Ok(mut auto) = AUTO_RECONNECT_ENABLED.lock() {
        *auto = true;
    }

    let my_gen = crate::current_gen();
    std::thread::spawn(move || {
        let context = match RusbContext::new() {
            Ok(c) => c,
            Err(e) => {
                log_event!(codes::FFI_HANDSHAKE_CONTEXT_FAILED, "error" => format!("{e:?}"));
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
                            log_event!(codes::FFI_HANDSHAKE_ALREADY_ACCESSORY);
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
                                    log_event!(codes::FFI_HANDSHAKE_FAILED, "error" => format!("{e:?}"));
                                } else {
                                    log_event!(codes::FFI_HANDSHAKE_SWITCHING);
                                    drop(handle);
                                    wait_for_aoa_reenumeration(&context);
                                }
                                return;
                            }
                            Err(e) => {
                                log_event!(codes::FFI_HANDSHAKE_OPEN_FAILED, "error" => format!("{e:?}"));
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
                        log_event!(codes::REENUM_ACCESSORY_FOUND);
                        return 0;
                    }
                }
            }
        }
    }
    log_event!(codes::REENUM_TIMEOUT);
    -4
}

pub fn start_usb_listener_thread(my_gen: u64) {
    std::thread::spawn(move || {
        let context = match RusbContext::new() {
            Ok(c) => c,
            Err(e) => {
                log_event!(codes::SYSTEM_DISCOVERY_CONTEXT_FAILED, "error" => format!("{e:?}"));
                return;
            }
        };
        log_event!(codes::SYSTEM_DISCOVERY_STARTED);

        let mut info_cache: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();

        loop {
            if !crate::session_alive(my_gen) {
                log_event!(codes::SYSTEM_DISCOVERY_STOPPING);
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
                    log_event!(codes::SYSTEM_DISCOVERY_POLL_FAILED, "error" => format!("{e:?}"));
                }
            }
            std::thread::sleep(Duration::from_secs(2));
        }
    });
}
