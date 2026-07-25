use crate::api::METRICS;
use crate::muxer::{contains_csd, Muxer, PopResult, UsbQueues};
use once_cell::sync::Lazy;
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

// Global references for coordinating the pipeline
pub static USB_ACTIVE: Lazy<Mutex<bool>> = Lazy::new(|| Mutex::new(false));
pub static USB_HANDLE: Lazy<Mutex<Option<i32>>> = Lazy::new(|| Mutex::new(None));

// Global Muxer reference — used by the JNI bridge to push encoded video and
// captured audio from the Android side.
pub static GLOBAL_MUXER: Lazy<Mutex<Option<Arc<Mutex<Muxer>>>>> = Lazy::new(|| Mutex::new(None));

/// Cache of the most recent codec-config packet (VPS/SPS/PPS). MediaCodec
/// emits it exactly once at encoder start; if it arrives before the muxer
/// exists (or a new USB session starts), we replay it so the desktop decoder
/// can always initialise.
static LAST_CSD: Lazy<Mutex<Option<Vec<u8>>>> = Lazy::new(|| Mutex::new(None));

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
    // Remember parameter sets so they can be replayed on (re)connect.
    if contains_csd(data) {
        if let Ok(mut csd) = LAST_CSD.lock() {
            *csd = Some(data.to_vec());
        }
    }

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

pub fn push_audio_to_muxer(data: &[u8]) -> bool {
    if let Ok(guard) = GLOBAL_MUXER.lock() {
        if let Some(ref muxer_arc) = *guard {
            if let Ok(mut muxer) = muxer_arc.lock() {
                muxer.push_audio(data);
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

/// Write the whole buffer, handling short writes and EAGAIN/EINTR.
/// A partial `write()` on a saturated bulk endpoint used to silently truncate
/// frames and desynchronise the stream — the desktop then discarded bytes
/// until the next magic header (visible as burst frame drops).
fn write_all(fd: i32, data: &[u8]) -> std::io::Result<()> {
    let mut offset = 0;
    while offset < data.len() {
        let written = unsafe {
            libc::write(
                fd,
                data.as_ptr().add(offset) as *const libc::c_void,
                data.len() - offset,
            )
        };
        if written > 0 {
            offset += written as usize;
        } else {
            let err = std::io::Error::last_os_error();
            match err.raw_os_error() {
                Some(libc::EAGAIN) | Some(libc::EINTR) => {
                    std::thread::sleep(Duration::from_millis(1));
                }
                _ => return Err(err),
            }
        }
    }
    Ok(())
}

pub fn start_usb_loop(fd: i32) {
    // Guard against multiple starts
    {
        let mut active = USB_ACTIVE.lock().unwrap_or_else(|e| e.into_inner());
        if *active {
            // A session already owns a descriptor. This one came from
            // detachFd(), so Rust is the only owner — closing it here rather
            // than returning is what keeps duplicate attach events (the
            // MethodChannel callback *and* the getInitialAccessory poll) from
            // leaking an fd apiece until the process hits EMFILE.
            let existing = *USB_HANDLE.lock().unwrap_or_else(|e| e.into_inner());
            if existing != Some(fd) {
                unsafe {
                    libc::close(fd);
                }
            }
            return;
        }
        *active = true;
    }

    {
        let mut h = USB_HANDLE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(old_fd) = *h {
            if old_fd != fd {
                unsafe {
                    libc::close(old_fd);
                }
            }
        }
        *h = Some(fd);
    }

    set_state(AoaState::Connected);

    // Shared queues: created here so both threads and the JNI bridge (via
    // GLOBAL_MUXER) reference the same session.
    let queues = UsbQueues::new();
    let muxer = Arc::new(Mutex::new(Muxer::new(queues.clone())));

    {
        let mut global = GLOBAL_MUXER.lock().unwrap_or_else(|e| e.into_inner());
        *global = Some(muxer.clone());
    }

    // Replay cached codec config so the desktop decoder can initialise even
    // if the encoder started before this USB session.
    if let Ok(csd) = LAST_CSD.lock() {
        if let Some(ref csd_data) = *csd {
            if let Ok(mut m) = muxer.lock() {
                m.push_video(csd_data);
            }
        }
    }

    // ── Reader thread: control messages from the desktop ────────────────
    let read_fd = fd;
    let reader_queues = queues.clone();
    // Lets the writer wait for this thread to stop touching the fd before it
    // closes it. Closing an fd out from under a blocked reader is not just
    // untidy: the descriptor number is immediately reusable, so the reader can
    // wake up reading an unrelated file that another thread has since opened.
    let reader_done = Arc::new(AtomicBool::new(false));
    let reader_done_signal = reader_done.clone();

    std::thread::spawn(move || {
        // Signals completion even if this thread panics.
        struct DoneGuard(Arc<AtomicBool>);
        impl Drop for DoneGuard {
            fn drop(&mut self) {
                self.0.store(true, AtomicOrdering::Release);
            }
        }
        let _done = DoneGuard(reader_done_signal);

        let mut buf = [0u8; 4096];
        // Config messages are NUL-terminated JSON, but a single read() may
        // return a partial message or several at once — accumulate and split.
        let mut acc: Vec<u8> = Vec::with_capacity(4096);
        loop {
            {
                let active = USB_ACTIVE.lock().unwrap_or_else(|e| e.into_inner());
                if !*active {
                    break;
                }
            }

            // Wait for readability rather than blocking in read(), so the
            // shutdown flag above is observed within one poll interval.
            let mut pfd = libc::pollfd {
                fd: read_fd,
                events: libc::POLLIN,
                revents: 0,
            };
            let ready = unsafe { libc::poll(&mut pfd, 1, 100) };
            if ready == 0 {
                continue; // timeout — re-check the shutdown flag
            }
            if ready < 0 {
                let err = std::io::Error::last_os_error();
                if err.raw_os_error() == Some(libc::EINTR) {
                    continue;
                }
                break;
            }
            if pfd.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 {
                break; // accessory detached
            }
            if pfd.revents & libc::POLLIN == 0 {
                continue;
            }

            let len =
                unsafe { libc::read(read_fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
            if len > 0 {
                acc.extend_from_slice(&buf[..len as usize]);
                // Extract every complete NUL-terminated message
                while let Some(pos) = acc.iter().position(|&b| b == 0) {
                    let msg: Vec<u8> = acc.drain(..=pos).collect();
                    if let Ok(text) = std::str::from_utf8(&msg[..msg.len() - 1]) {
                        let trimmed = text.trim();
                        if trimmed.is_empty() {
                            continue;
                        }
                        if let Ok(mut lock) = crate::api::LATEST_CONFIG.lock() {
                            // Dart drains this by polling; if the app is
                            // backgrounded and stops polling, cap the backlog
                            // rather than growing without limit.
                            if lock.len() >= 32 {
                                lock.remove(0);
                            }
                            lock.push(trimmed.to_string());
                        }
                    }
                }
                // Garbage guard: a stream that never NUL-terminates is not
                // a config channel — don't grow forever.
                if acc.len() > 64 * 1024 {
                    acc.clear();
                }
            } else if len < 0 {
                let err = std::io::Error::last_os_error();
                if err.raw_os_error() == Some(libc::EAGAIN)
                    || err.kind() == std::io::ErrorKind::WouldBlock
                {
                    std::thread::sleep(Duration::from_millis(100));
                    continue;
                }
                break;
            } else {
                break;
            }
        }
        let mut active = USB_ACTIVE.lock().unwrap_or_else(|e| e.into_inner());
        *active = false;
        reader_queues.close();
    });

    // ── Writer thread: drains the queues onto the accessory fd ──────────
    std::thread::spawn(move || {
        let _m_guard = MetricsGuard;
        set_state(AoaState::Streaming);

        let mut last_tick = Instant::now();
        let mut bytes_sent: usize = 0;
        let mut frames_sent: u64 = 0;

        loop {
            match queues.pop_next(Duration::from_millis(100)) {
                PopResult::Packet(frame) => {
                    let frame_len = frame.len();
                    let frame_type_video = frame_len > 9 && frame[4] == 0x01;

                    let result = write_all(fd, &frame);
                    Muxer::release_buffer(frame);

                    if let Err(e) = result {
                        set_state(AoaState::Error(format!("USB write failed: {}", e)));
                        break;
                    }

                    bytes_sent += frame_len;
                    if frame_type_video {
                        frames_sent += 1;
                    }

                    let now = Instant::now();
                    let elapsed = now.duration_since(last_tick);
                    if elapsed >= Duration::from_secs(1) {
                        if let Ok(mut m) = METRICS.lock() {
                            m.throughput_mbps =
                                (bytes_sent as f64 * 8.0) / (1_000_000.0 * elapsed.as_secs_f64());
                            m.fps_actual = frames_sent as f64 / elapsed.as_secs_f64();
                            m.dropped_frames = crate::muxer::DROPPED_VIDEO_FRAMES
                                .load(std::sync::atomic::Ordering::Relaxed);
                        }
                        bytes_sent = 0;
                        frames_sent = 0;
                        last_tick = now;
                    }
                }
                PopResult::Timeout => {
                    let active = USB_ACTIVE.lock().unwrap_or_else(|e| e.into_inner());
                    if !*active {
                        break;
                    }
                }
                PopResult::Closed => {
                    set_state(AoaState::Error("Muxer pipeline closed".to_string()));
                    break;
                }
            }
        }

        // ── Session teardown ──
        {
            let mut global = GLOBAL_MUXER.lock().unwrap_or_else(|e| e.into_inner());
            *global = None;
        }
        {
            let mut active = USB_ACTIVE.lock().unwrap_or_else(|e| e.into_inner());
            *active = false;
        }
        {
            // Rust owns the fd (Kotlin hands it over via detachFd) — single
            // close here, never on the Java side.
            //
            // The reader thread polls this same descriptor, so wait for it to
            // finish before closing. One poll interval is 100 ms; give it a
            // generous multiple, then close regardless so a wedged reader
            // cannot leak the fd forever.
            let deadline = Instant::now() + Duration::from_millis(1000);
            while !reader_done.load(AtomicOrdering::Acquire) && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(10));
            }

            let mut h = USB_HANDLE.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(stored_fd) = h.take() {
                unsafe {
                    libc::close(stored_fd);
                }
            }
        }
        set_state(AoaState::Idle);
    });
}
