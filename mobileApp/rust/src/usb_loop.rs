use crate::muxer::Muxer;
use crate::audio_capture::AudioCapture;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::mpsc;
use once_cell::sync::Lazy;

// Global references for coordinating the pipeline
static USB_ACTIVE: Lazy<Mutex<bool>> = Lazy::new(|| Mutex::new(false));
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

/// Push video data from the JNI/MediaCodec encoder into the muxer.
/// Returns true if data was accepted, false if the pipeline isn't ready.
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

/// Start the USB AOA accessory mode listener.
/// This is called from the Dart side via FFI after RustLib.init().
///
/// The flow:
/// 1. Open the USB accessory file descriptor provided by Android framework
/// 2. Start the Muxer to receive encoded screen + audio data
/// 3. Start the AudioCapture engine
/// 4. In a hot loop, read interleaved frames from the Muxer and write to USB  
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
        // Create the muxer that interleaves H.265 video + AAC audio into a framed stream
        let (frame_tx, frame_rx) = mpsc::channel::<Vec<u8>>();
        
        let muxer = Arc::new(Mutex::new(Muxer::new(frame_tx)));

        // Store muxer globally so JNI bridge can push video frames into it
        {
            let mut global = GLOBAL_MUXER.lock().unwrap();
            *global = Some(muxer.clone());
        }

        let muxer_clone = muxer.clone();

        // Start audio capture and feed into muxer
        let _audio_handle = AudioCapture::start(move |pcm_data: &[u8]| {
            if let Ok(mut m) = muxer_clone.lock() {
                m.push_audio(pcm_data);
            }
        });

        set_state(AoaState::Streaming);

        // Hot write loop — read muxed frames and write them over USB fd
        // Uses raw POSIX write() for zero-copy performance
        loop {
            match frame_rx.recv() {
                Ok(frame) => {
                    let written = unsafe {
                        libc::write(fd, frame.as_ptr() as *const libc::c_void, frame.len())
                    };
                    if written <= 0 {
                        // USB disconnected or error
                        let errno = std::io::Error::last_os_error();
                        set_state(AoaState::Error(format!("USB write failed: {}", errno)));
                        break;
                    }
                }
                Err(_) => {
                    // Muxer channel closed
                    set_state(AoaState::Error("Muxer pipeline closed".to_string()));
                    break;
                }
            }
        }

        // Cleanup
        {
            let mut global = GLOBAL_MUXER.lock().unwrap();
            *global = None;
        }
        unsafe { libc::close(fd); }
        let mut active = USB_ACTIVE.lock().unwrap();
        *active = false;
        set_state(AoaState::Idle);
    });
}
