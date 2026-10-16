//! The C ABI the desktop interface calls through `bun:ffi`.
//!
//! Everything `extern "C"` in this crate lives here, and nothing here does
//! real work — each function converts arguments, calls into a normal Rust
//! module, and converts the result back. Keeping that boundary in one file
//! means the surface the interface depends on can be read in one sitting, and
//! that adding a feature does not quietly add an exported symbol somewhere
//! else.
//!
//! # Conventions
//!
//! * `i32` returns: `0` success, negative failure, unless the doc comment says
//!   otherwise. Predicates return `1` for true and `0` for false.
//! * `*mut c_char` returns are heap-allocated here and **must** be released by
//!   the caller with [`free_string`]. `src/bun/index.ts` does this in the
//!   `readCString` helper's `finally` block.
//! * Every pointer argument may be null; each function checks.
//!
//! Adding a function here means adding it to the `dlopen` map in
//! `src/bun/index.ts` too — there is no generated binding, so the two lists
//! are kept in step by hand.

mod strings;

use crate::{pipeline, platform, sinks, telemetry, VIDEO_QUEUE};
use strings::{cstr_to_str, to_c_string};

// ── Lifecycle ───────────────────────────────────────────────

/// Starts the receiver. Returns 0 on success, -1 if shared memory failed.
///
/// The width and height arguments are historical: the frame size comes from
/// the stream, not from the caller. They are kept so the existing `dlopen`
/// signature in the interface stays valid.
#[no_mangle]
pub extern "C" fn init_mirror(_width: u32, _height: u32) -> i32 {
    match crate::init() {
        Ok(()) => 0,
        Err(e) => {
            crate::log_event!(mirror_i18n::codes::SYSTEM_INIT_SHARED_MEMORY_FAILED, "error" => e);
            -1
        }
    }
}

/// Stops the receiver and releases the USB interface and shared memory.
#[no_mangle]
pub extern "C" fn stop_mirror() -> i32 {
    crate::shutdown();
    0
}

// ── Packet ingress ──────────────────────────────────────────

/// Legacy C ABI entry point for pushing an encoded video packet (copies).
///
/// Nothing in the current interface calls this — packets arrive over USB, not
/// from the caller — but it is kept for out-of-tree tools that feed a capture
/// file in for debugging.
///
/// # Safety
///
/// `data` must point to at least `len` readable bytes, or be null.
#[no_mangle]
pub unsafe extern "C" fn push_packet(data: *const u8, len: usize) -> i32 {
    if data.is_null() {
        return -1;
    }
    let slice = std::slice::from_raw_parts(data, len);
    crate::push_video_packet(slice.to_vec());
    0
}

// ── Playback ────────────────────────────────────────────────

/// Starts playback in a child ffplay window, with video *and* audio.
///
/// Returns 0 on success, -1 when ffplay could not be launched. `project_root`
/// locates a bundled ffplay before falling back to one on `PATH`.
///
/// # Safety
///
/// `project_root` must be a valid NUL-terminated C string, or null.
#[no_mangle]
pub unsafe extern "C" fn start_player(project_root: *const libc::c_char) -> i32 {
    sinks::player::start(cstr_to_str(project_root, ""))
}

/// Stops playback and closes the player window.
#[no_mangle]
pub extern "C" fn stop_player() -> i32 {
    sinks::player::stop();
    0
}

/// 0 = stopped, 1 = starting (waiting for a keyframe), 2 = playing.
#[no_mangle]
pub extern "C" fn get_player_state() -> i32 {
    sinks::player::state()
}

// ── OBS ─────────────────────────────────────────────────────

/// 1 if OBS Studio is installed on this machine.
#[no_mangle]
pub extern "C" fn check_obs_installed() -> i32 {
    sinks::obs_feed::check_obs_installed() as i32
}

/// 1 if OBS Studio is running right now.
///
/// Cached for a few seconds inside the backend, so it is safe to ask on every
/// status poll. The interface gates the feed toggle on this: writing frames
/// nobody is reading wastes an 8 MB memcpy per frame.
#[no_mangle]
pub extern "C" fn check_obs_running() -> i32 {
    sinks::obs_feed::check_obs_running() as i32
}

/// 1 if the Mirror Source plugin is installed *and* at the current version.
#[no_mangle]
pub extern "C" fn check_obs_plugin_installed() -> i32 {
    sinks::obs_feed::check_plugin_installed() as i32
}

/// Where the OBS plugin directory is, or an empty string if unknown.
///
/// Free the result with [`free_string`].
#[no_mangle]
pub extern "C" fn get_obs_plugin_dir() -> *mut libc::c_char {
    to_c_string(sinks::obs_feed::get_obs_plugin_dir().unwrap_or_default())
}

/// Installs the OBS plugin. Returns 0 on success, -1 on failure.
///
/// # Safety
///
/// `project_root` must be a valid NUL-terminated C string, or null.
#[no_mangle]
pub unsafe extern "C" fn install_obs_plugin(project_root: *const libc::c_char) -> i32 {
    sinks::obs_feed::install_plugin(cstr_to_str(project_root, "."))
}

/// Turns the OBS shared-memory feed on or off.
#[no_mangle]
pub extern "C" fn toggle_obs_feed(enabled: i32) {
    sinks::obs_feed::set_enabled(enabled != 0);
}

/// 1 if an ffplay binary can be found, bundled or on `PATH`.
///
/// # Safety
///
/// `project_root` must be a valid NUL-terminated C string, or null.
#[no_mangle]
pub unsafe extern "C" fn check_ffplay_available(project_root: *const libc::c_char) -> i32 {
    sinks::obs_feed::check_ffplay_available(cstr_to_str(project_root, "")) as i32
}

// ── Connection control ──────────────────────────────────────

/// Puts a specific device into accessory mode.
#[no_mangle]
pub extern "C" fn trigger_manual_handshake(vid: u16, pid: u16) -> i32 {
    if let Ok(mut fd) = pipeline::receiver::FORCE_DISCONNECT.lock() {
        *fd = false;
    }
    pipeline::receiver::trigger_manual_handshake(vid, pid)
}

/// Enables or disables automatic reconnection to a known accessory.
#[no_mangle]
pub extern "C" fn toggle_auto_reconnect(enabled: i32) {
    if let Ok(mut fd) = pipeline::receiver::FORCE_DISCONNECT.lock() {
        *fd = false;
    }
    let mut auto = pipeline::receiver::AUTO_RECONNECT_ENABLED
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    *auto = enabled != 0;
}

/// Drops the current link and stops reconnecting until told otherwise.
#[no_mangle]
pub extern "C" fn force_disconnect() -> i32 {
    let Ok(mut flag) = pipeline::receiver::FORCE_DISCONNECT.lock() else {
        return -1;
    };
    *flag = true;
    if let Ok(mut auto) = pipeline::receiver::AUTO_RECONNECT_ENABLED.lock() {
        *auto = false;
    }
    if let Ok(mut m) = telemetry::metrics::METRICS.lock() {
        m.reset();
    }
    0
}

/// Queues a JSON settings message for delivery to the phone.
///
/// A `"start"` command also re-arms auto-reconnect and a `"stop"` disarms it,
/// so closing a session does not immediately reopen one.
///
/// # Safety
///
/// `json` must be a valid NUL-terminated C string, or null.
#[no_mangle]
pub unsafe extern "C" fn sync_config(json: *const libc::c_char) -> i32 {
    if json.is_null() {
        return -1;
    }
    let Some(s) = std::ffi::CStr::from_ptr(json).to_str().ok() else {
        return -1;
    };

    // Parsed rather than substring-matched: a resolution string containing the
    // word "stop" should not stop the stream.
    let command = serde_json::from_str::<serde_json::Value>(s)
        .ok()
        .and_then(|v| v.get("command").and_then(|c| c.as_str()).map(String::from));

    if let Ok(mut config) = pipeline::receiver::PENDING_CONFIG.lock() {
        *config = Some(s.to_string());
    }

    match command.as_deref() {
        Some("start") => {
            if let Ok(mut auto) = pipeline::receiver::AUTO_RECONNECT_ENABLED.lock() {
                *auto = true;
            }
        }
        Some("stop") => {
            if let Ok(mut auto) = pipeline::receiver::AUTO_RECONNECT_ENABLED.lock() {
                *auto = false;
            }
        }
        _ => {}
    }
    0
}

// ── Telemetry ───────────────────────────────────────────────

/// Comma-separated `type|name|vid:pid` triples for every discovered device.
///
/// Free the result with [`free_string`].
#[no_mangle]
pub extern "C" fn get_devices() -> *mut libc::c_char {
    let list = pipeline::receiver::DISCOVERED_DEVICES
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    to_c_string(list.join(","))
}

/// Every log entry this session, as a JSON array.
///
/// Free the result with [`free_string`].
#[no_mangle]
pub extern "C" fn get_structured_logs() -> *mut libc::c_char {
    let logs = telemetry::log::LOG_BUFFER
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    to_c_string(serde_json::to_string(&*logs).unwrap_or_else(|_| "[]".into()))
}

/// Log entries added since the last call, as a JSON array.
///
/// Each entry carries a `code` and `params` for translation, plus an English
/// `message` as a fallback. Free the result with [`free_string`].
#[no_mangle]
pub extern "C" fn get_new_logs() -> *mut libc::c_char {
    let new_logs = telemetry::log::take_new();
    to_c_string(serde_json::to_string(&new_logs).unwrap_or_else(|_| "[]".into()))
}

/// A JSON snapshot of throughput, latency, framerate and buffer health.
///
/// Free the result with [`free_string`].
#[no_mangle]
pub extern "C" fn get_metrics() -> *mut libc::c_char {
    let mut manager = telemetry::metrics::METRICS
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let snapshot = manager.get_snapshot(VIDEO_QUEUE.len(), VIDEO_QUEUE.capacity());
    to_c_string(serde_json::to_string(&snapshot).unwrap_or_else(|_| "{}".into()))
}

/// 1 while a phone is connected and streaming.
#[no_mangle]
pub extern "C" fn get_status() -> i32 {
    i32::from(pipeline::receiver::is_streaming())
}

/// How many encoded packets are waiting for the decoder.
#[no_mangle]
pub extern "C" fn get_buffer_size() -> i32 {
    VIDEO_QUEUE.len() as i32
}

/// Releases a string returned by any function in this module.
///
/// # Safety
///
/// `s` must be a pointer this module returned and has not already freed, or
/// null.
#[no_mangle]
pub unsafe extern "C" fn free_string(s: *mut libc::c_char) {
    if s.is_null() {
        return;
    }
    drop(std::ffi::CString::from_raw(s));
}

// ── Drivers and permissions ─────────────────────────────────

/// 1 when the OS already permits opening the accessory.
///
/// Cheap enough for the interface's twice-a-second status poll, and never
/// raises a prompt.
#[no_mangle]
pub extern "C" fn check_driver_status() -> i32 {
    platform::driver_status().as_i32()
}

/// Installs the WinUSB driver. No-op off Windows.
///
/// Retained under this name because `src/bun/index.ts` looks it up by symbol;
/// [`setup_linux_permissions`] is its Linux counterpart. Both raise a system
/// elevation prompt, so call only when [`check_driver_status`] returns 0.
#[no_mangle]
pub extern "C" fn install_windows_driver() -> i32 {
    if cfg!(target_os = "windows") {
        platform::install_driver().as_i32()
    } else {
        0
    }
}

/// Installs the udev rule that grants accessory access. No-op off Linux.
///
/// See [`install_windows_driver`] for why the two are separate symbols.
#[no_mangle]
pub extern "C" fn setup_linux_permissions() -> i32 {
    if cfg!(target_os = "linux") {
        platform::install_driver().as_i32()
    } else {
        0
    }
}
