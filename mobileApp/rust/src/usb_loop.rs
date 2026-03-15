use crate::muxer::Muxer;
use crate::audio_capture::AudioCapture;
use crate::api::METRICS;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::mpsc;
use once_cell::sync::Lazy;
use std::time::{Instant, Duration};

// Global references for coordinating the pipeline
pub static USB_ACTIVE: Lazy<Mutex<bool>> = Lazy::new(|| Mutex::new(false));
pub static USB_HANDLE: Lazy<Mutex<Option<i32>>> = Lazy::new(|| Mutex::new(None));

// Global Muxer reference — used by JNI bridge to push video frames from MediaCodec
pub static GLOBAL_MUXER: Lazy<Mutex<Option<Arc<Mutex<Muxer>>>>> = Lazy::new(|| Mutex::new(None));

/// Information about the AOA state
#[allow(dead_code)]
pub enum AoaState {
    Idle,
    WaitingForHost,
    Connected,
    Streaming,
    Error(String),
}

pub static AOA_STATE: Lazy<Mutex<AoaState>> = Lazy::new(|| Mutex::new(AoaState::Idle));

fn set_state(state: AoaState) {
    if let Ok(mut s) = AOA_STATE.lock() {
        *s = state;
    }
}

pub fn push_video_to_muxer(data: &[u8]) -> bool {
    if let Ok(guard) = GLOBAL_MUXER.lock() {
        if let Some(ref muxer_arc) = *guard {
            if let Ok(mut muxer) = muxer_arc.lock() {
                muxer.push_video(data);
                return true;
            }
        }
    }
    false
}

struct MetricsGuard;
impl Drop for MetricsGuard {
    fn drop(&mut self) {
        if let Ok(mut m) = METRICS.lock() {
            m.throughput_mbps = 0.0;
            m.fps_actual = 0.0;
        }
    }
}

pub fn start_usb_loop(fd: i32) {
    // Guard against multiple starts
    {
        let mut active = USB_ACTIVE.lock().unwrap();
        if *active { return; }
        *active = true;
    }

    {
        let mut h = USB_HANDLE.lock().unwrap();
        *h = Some(fd);
    }

    set_state(AoaState::Connected);

    let read_fd = fd;
    std::thread::spawn(move || {
        let mut buf = [0u8; 1024];
        loop {
            // Check if we should still be active
            {
                let active = USB_ACTIVE.lock().unwrap();
                if !*active { break; }
            }

            let len = unsafe { libc::read(read_fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
            if len > 0 {
                if let Ok(json_str) = std::str::from_utf8(&buf[..len as usize]) {
                    if let Ok(mut lock) = crate::api::LATEST_CONFIG.lock() {
                        *lock = Some(json_str.to_string());
                    }
                }
            } else if len < 0 {
                let err = std::io::Error::last_os_error();
                if err.raw_os_error() == Some(libc::EAGAIN) || err.kind() == std::io::ErrorKind::WouldBlock {
                    std::thread::sleep(std::time::Duration::from_millis(100));
                    continue;
                }
                break;
            } else {
                break;
            }
        }
    });

    std::thread::spawn(move || {
        let _m_guard = MetricsGuard;
        let (frame_tx, frame_rx) = mpsc::channel::<Vec<u8>>();
        let muxer = Arc::new(Mutex::new(Muxer::new(frame_tx)));

        {
            let mut global = GLOBAL_MUXER.lock().unwrap();
            *global = Some(muxer.clone());
        }

        let muxer_clone = muxer.clone();
        let _audio_handle = AudioCapture::start(move |pcm_data: &[u8]| {
            if let Ok(mut m) = muxer_clone.lock() {
                m.push_audio(pcm_data);
            }
        });

        set_state(AoaState::Streaming);

        let mut last_tick = Instant::now();
        let mut bytes_sent = 0;
        let mut frames_sent = 0;

        loop {
            match frame_rx.recv_timeout(Duration::from_millis(500)) {
                Ok(frame) => {
                    let frame_len = frame.len();
                    let written = unsafe {
                        libc::write(fd, frame.as_ptr() as *const libc::c_void, frame_len)
                    };
                    if written <= 0 {
                        let errno = std::io::Error::last_os_error();
                        set_state(AoaState::Error(format!("USB write failed: {}", errno)));
                        break;
                    }

                    bytes_sent += written as usize;
                    if frame_len > 9 && frame[4] == 0x01 {
                        frames_sent += 1;
                    }

                    let now = Instant::now();
                    let elapsed = now.duration_since(last_tick);
                    if elapsed >= Duration::from_secs(1) {
                        if let Ok(mut m) = METRICS.lock() {
                            m.throughput_mbps = (bytes_sent as f64 * 8.0) / (1024.0 * 1024.0 * elapsed.as_secs_f64());
                            m.fps_actual = frames_sent as f64 / elapsed.as_secs_f64();
                        }
                        bytes_sent = 0;
                        frames_sent = 0;
                        last_tick = now;
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    let active = USB_ACTIVE.lock().unwrap();
                    if !*active { break; }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    set_state(AoaState::Error("Muxer pipeline closed".to_string()));
                    break;
                }
            }
        }

        {
            let mut global = GLOBAL_MUXER.lock().unwrap();
            *global = None;
        }
        unsafe { libc::close(fd); }
        {
            let mut active = USB_ACTIVE.lock().unwrap();
            *active = false;
        }
        set_state(AoaState::Idle);
    });
}
