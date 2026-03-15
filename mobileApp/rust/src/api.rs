use flutter_rust_bridge::frb;
use anyhow::{Result, Context};
use std::sync::Mutex;
use once_cell::sync::Lazy;
use serde::Serialize;

#[frb(init)]
pub fn init_app() {
    flutter_rust_bridge::setup_default_user_utils();
}

pub(crate) static USB_BUFFER: Lazy<Mutex<CircularBuffer>> = Lazy::new(|| {
    Mutex::new(CircularBuffer::new(1024 * 1024 * 5))
});

#[derive(Serialize, Clone)]
pub struct MobileMetrics {
    pub throughput_mbps: f64,
    pub encoding_latency_ms: u64,
    pub fps_actual: f64,
}

pub static METRICS: Lazy<Mutex<MobileMetrics>> = Lazy::new(|| Mutex::new(MobileMetrics {
    throughput_mbps: 0.0,
    encoding_latency_ms: 0,
    fps_actual: 0.0,
}));

#[frb(ignore)]
pub struct CircularBuffer {
    pub data: Vec<u8>,
    pub head: usize,
    pub tail: usize,
    pub size: usize,
}

impl CircularBuffer {
    fn new(size: usize) -> Self {
        Self { data: vec![0; size], head: 0, tail: 0, size }
    }
    pub fn push(&mut self, packet: &[u8]) -> bool {
        let packet_len = packet.len();
        if packet_len > self.size { return false; }
        let available = if self.head >= self.tail { self.size - (self.head - self.tail) } else { self.tail - self.head };
        if packet_len > available { return false; }
        for &byte in packet { self.data[self.head] = byte; self.head = (self.head + 1) % self.size; }
        true
    }
}

#[frb(ignore)]
pub fn push_to_usb(data: Vec<u8>) -> bool {
    // Try pushing through the Muxer pipeline first (preferred path)
    if crate::usb_loop::push_video_to_muxer(&data) {
        return true;
    }
    // Fallback to CircularBuffer if muxer isn't ready yet
    let mut buffer = USB_BUFFER.lock().unwrap();
    buffer.push(&data)
}

pub static LATEST_CONFIG: Lazy<Mutex<Option<String>>> = Lazy::new(|| Mutex::new(None));

#[frb(sync)]
pub fn poll_config() -> Option<String> {
    if let Ok(mut config) = LATEST_CONFIG.lock() {
        return config.take();
    }
    None
}

/// Returns the current USB connection state as a string.
/// Dart polls this to detect disconnection even if the Android broadcast is missed.
#[frb(sync)]
pub fn get_connection_state() -> String {
    if let Ok(state) = crate::usb_loop::AOA_STATE.lock() {
        match &*state {
            crate::usb_loop::AoaState::Idle => "idle".to_string(),
            crate::usb_loop::AoaState::WaitingForHost => "waiting".to_string(),
            crate::usb_loop::AoaState::Connected => "connected".to_string(),
            crate::usb_loop::AoaState::Streaming => "streaming".to_string(),
            crate::usb_loop::AoaState::Error(msg) => format!("error:{}", msg),
        }
    } else {
        "unknown".to_string()
    }
}

pub fn get_mobile_metrics() -> String {
    let m = METRICS.lock().unwrap();
    serde_json::to_string(&*m).unwrap_or_else(|_| "{}".to_string())
}

/// Start the USB streaming pipeline using the Android-provided AOA file descriptor.
/// This is called from Dart when the Android framework detects USB accessory attachment
/// and passes the native file descriptor through the MethodChannel.
pub fn start_usb_streaming(fd: i32) -> Result<String> {
    crate::usb_loop::start_usb_loop(fd);
    Ok("USB streaming pipeline started".to_string())
}

/// Legacy AOA handshake — kept for reference but not used on Android
/// (Android framework handles AOA negotiation, not the app)
pub fn start_aoa() -> Result<String> {
    // On Android, AOA is handled by the system. 
    // The app receives an accessory FD via intent/MethodChannel.
    // This function now just returns success for compatibility.
    Ok("AOA mode managed by Android framework".to_string())
}

pub fn greet(name: String) -> String { format!("Hello, {name}!") }
