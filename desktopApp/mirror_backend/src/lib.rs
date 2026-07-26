//! Desktop receiver for the Mirror screen-sharing link.
//!
//! Built as a `cdylib` and loaded by the Electrobun shell through `bun:ffi`.
//! Every symbol the interface calls lives in [`ffi`]; nothing else in this
//! crate is `extern "C"`.
//!
//! # Layout
//!
//! ```text
//! ffi/        the C ABI the interface sees, and nothing else
//! pipeline/   USB in -> demux -> queue -> decode
//! sinks/      where the stream goes: ffplay, OBS. Both opt-in.
//! platform/   one file per OS, behind a single interface
//! telemetry/  the event log and the counters the interface polls
//! ```
//!
//! Two crates are shared with the mobile sender:
//! [`mirror_protocol`] defines the wire format so the two ends cannot drift,
//! and [`mirror_i18n`] defines the event codes so log lines can be translated
//! by whichever interface displays them.
//!
//! # Data flow
//!
//! ```text
//!  USB bulk read
//!       |
//!  pipeline::receiver ── audio ──────────────> sinks::push_audio
//!       |                                          |         |
//!  pipeline::demuxer                          player     obs_feed
//!       |                                     (ffplay)   (shm ring)
//!  push_video_packet ──> VIDEO_QUEUE
//!                             |
//!                       pipeline::decoder
//!                             |
//!                       deliver_frame ──────────> OBS triple buffer
//! ```
//!
//! Video reaches ffplay as an HEVC passthrough, not as decoded frames — see
//! [`needs_decoded_frames`].
//!
//! # Threading
//!
//! Three long-lived threads: USB discovery, the USB session, and the decoder.
//! None of them is joined on shutdown. Instead [`SESSION_GEN`] is bumped and
//! each thread notices at the top of its next iteration. A thread blocked in a
//! syscall when stop was called therefore cannot miss the signal, which a
//! boolean flag that gets reset can.

use once_cell::sync::Lazy;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

pub mod ffi;
pub mod pipeline;
pub mod platform;
pub mod sinks;
pub mod telemetry;

/// Session generation counter used for cooperative shutdown.
///
/// Every background thread captures the generation at spawn time and exits
/// its loop as soon as the global value no longer matches. Unlike a boolean
/// flag that gets reset, a monotonically increasing generation can never be
/// "missed" by a thread that was blocked in a syscall when stop was called.
pub static SESSION_GEN: AtomicU64 = AtomicU64::new(0);

/// The generation a thread should capture when it starts.
pub fn current_gen() -> u64 {
    SESSION_GEN.load(Ordering::Acquire)
}

/// Whether the session a thread was spawned for is still the current one.
pub fn session_alive(my_gen: u64) -> bool {
    SESSION_GEN.load(Ordering::Acquire) == my_gen
}

/// Bumped whenever a new USB streaming session begins. The decoder watches it
/// so it can flush reference frames at a stream discontinuity instead of
/// decoding the new stream against the old one's references.
pub static STREAM_EPOCH: AtomicU64 = AtomicU64::new(0);

/// The current stream epoch.
pub fn stream_epoch() -> u64 {
    STREAM_EPOCH.load(Ordering::Acquire)
}

/// Monotonic nanoseconds since process start.
static EPOCH: Lazy<Instant> = Lazy::new(Instant::now);

/// A real clock for the frame header the OBS plugin reads.
///
/// This field used to carry `FRAME_COUNTER` — a frame index, not a time — so
/// anything downstream treating it as a timestamp was reading nonsense. It is
/// still a *receive* time, not a capture time: honest end-to-end latency needs
/// a sender-side timestamp on the wire.
pub fn now_nanos() -> u64 {
    EPOCH.elapsed().as_nanos() as u64
}

/// Encoded-video ingress queue (USB thread → decoder thread).
///
/// 32 packets ≈ 250 ms at 120 fps — enough to absorb scheduler jitter without
/// adding perceptible latency.
pub static VIDEO_QUEUE: Lazy<Arc<pipeline::VideoQueue>> =
    Lazy::new(|| Arc::new(pipeline::VideoQueue::new(32)));

/// Triple-buffer shared-memory writer for the OBS plugin. Set once at init.
static TRIPLE_BUFFER: Lazy<Mutex<Option<Arc<sinks::shared_mem::TripleBufferManager>>>> =
    Lazy::new(|| Mutex::new(None));

static INITIALIZED: Lazy<Mutex<bool>> = Lazy::new(|| Mutex::new(false));

/// Starts the receiver: shared memory, the decoder thread, and USB discovery.
///
/// Idempotent — a second call while running is a no-op rather than a second
/// set of threads.
pub fn init() -> Result<(), String> {
    let mut inited = INITIALIZED.lock().unwrap_or_else(|e| e.into_inner());
    if *inited {
        return Ok(());
    }

    let trbuff = sinks::shared_mem::TripleBufferManager::create("obs_mirror_buffer")
        .map_err(|e| e.to_string())?;

    if let Ok(mut tb) = TRIPLE_BUFFER.lock() {
        *tb = Some(Arc::new(trbuff));
    }

    sinks::obs_feed::init_audio();

    let generation = current_gen();
    pipeline::decoder::start_decoder_thread(VIDEO_QUEUE.clone(), generation);
    pipeline::receiver::start_usb_listener_thread(generation);

    *inited = true;
    Ok(())
}

/// Stops everything and releases the USB interface and shared memory.
///
/// Only process exit calls this. It is deliberately not exposed as an RPC: no
/// path in the interface re-runs [`init`], so a "stop" button wired to it would
/// leave the app inert until restart.
pub fn shutdown() {
    // Bumping the generation makes every session thread exit its loop at the
    // next iteration — no flag reset race, no sleep required.
    SESSION_GEN.fetch_add(1, Ordering::AcqRel);

    VIDEO_QUEUE.clear();
    sinks::player::stop();
    sinks::obs_feed::set_enabled(false);
    sinks::obs_feed::cleanup();

    if let Ok(mut tb) = TRIPLE_BUFFER.lock() {
        *tb = None;
    }
    if let Ok(mut inited) = INITIALIZED.lock() {
        *inited = false;
    }

    log_event!(mirror_i18n::codes::SYSTEM_SHUTDOWN_COMPLETE);
}

/// Does anything still need a *decoded* frame?
///
/// Only the OBS shared-memory feed consumes decoded BGRA. The player gets an
/// HEVC passthrough stream and decodes it inside ffplay, so it only needs the
/// decoder long enough to learn the frame size for the Matroska header.
pub fn needs_decoded_frames() -> bool {
    sinks::obs_feed::is_enabled() || sinks::player::needs_dimensions()
}

/// Delivers a decoded BGRA frame.
///
/// Takes ownership of `buffer` (a pooled allocation) and returns it to the
/// pool when done.
///
/// `decode_started` is when the packet left the ingress queue, so the recorded
/// latency covers decode + colour conversion + sink write. The old figure timed
/// only this function's own body — essentially the memcpy — and reported it to
/// the interface as "pipeline latency".
pub fn deliver_frame(buffer: Vec<u8>, width: u32, height: u32, decode_started: Instant) {
    // Lets a pending player session fill in the Matroska track header.
    sinks::player::note_dimensions(width, height);

    // OBS shared memory, only when the user enabled the feed — that skips an
    // 8 MB memcpy per frame otherwise. The mutex below is taken on this branch
    // alone, and never on a playback-only session.
    if sinks::obs_feed::is_enabled() {
        let tb = TRIPLE_BUFFER
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if let Some(tb) = tb {
            let _ = tb.write_frame(width, height, now_nanos(), &buffer);
        }
    }

    if let Ok(mut m) = telemetry::metrics::METRICS.lock() {
        m.record_frame(buffer.len(), decode_started.elapsed().as_millis() as u64);
    }

    pipeline::framepool::release(buffer);
}

/// Moves a demuxed video packet into the decode queue. Never blocks.
pub fn push_video_packet(data: Vec<u8>) {
    let dropped = VIDEO_QUEUE.push(data);
    if dropped > 0 {
        if let Ok(mut m) = telemetry::metrics::METRICS.lock() {
            for _ in 0..dropped {
                m.record_drop();
            }
        }
    }
}
