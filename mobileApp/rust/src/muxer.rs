use mirror_protocol::{hevc, PacketType, HEADER_SIZE};
use once_cell::sync::Lazy;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

/// Video packets dropped on queue overflow (reported in mobile metrics).
pub static DROPPED_VIDEO_FRAMES: AtomicU64 = AtomicU64::new(0);

static BUFFER_POOL: Lazy<Mutex<Vec<Vec<u8>>>> = Lazy::new(|| {
    let mut pool = Vec::with_capacity(8);
    for _ in 0..8 {
        pool.push(Vec::with_capacity(1024 * 1024)); // 1MB initial
    }
    Mutex::new(pool)
});

/// Scan an Annex-B HEVC packet for parameter sets (VPS/SPS/PPS).
///
/// Used to preserve codec config packets across overflow drops and muxer
/// restarts. The scan itself lives in `mirror-protocol` because the desktop
/// receiver needs the same walker to build its `hvcC` record; keeping two
/// copies is how the two ends came to disagree about trailing start codes.
pub fn contains_csd(data: &[u8]) -> bool {
    hevc::contains_parameter_sets(data)
}

/// A framed packet plus whether it carries parameter sets. Computed once on
/// the way in: the overflow path used to re-scan every one of the 180 queued
/// payloads while holding the queue lock, on the MediaCodec drain thread.
struct QueuedVideo {
    data: Vec<u8>,
    is_csd: bool,
    is_key: bool,
}

struct QueueInner {
    video: VecDeque<QueuedVideo>,
    audio: VecDeque<Vec<u8>>,
    /// Set after a drop left no keyframe to resume from. Until one arrives,
    /// delta frames are discarded: sending them would only paint garbage on
    /// the desktop, which has lost the frames they reference.
    need_key: bool,
    closed: bool,
}

/// Separate audio/video queues feeding the USB writer.
///
/// Both push paths are non-blocking (drop-oldest on overflow): the MediaCodec
/// drain thread and the audio capture thread must never stall on USB
/// backpressure — that is what caused encoder pacing collapse at 120 fps.
/// The USB writer drains audio first (small, latency-sensitive), then video.
pub struct UsbQueues {
    inner: Mutex<QueueInner>,
    cond: Condvar,
}

/// ~1.5 s of video at 120 fps. If USB falls this far behind, the link is
/// effectively down and dropping (keeping CSD) is the right call.
/// Video frames held before the queue starts shedding load. Kept small on
/// purpose: every queued frame is latency, and a stream that is behind should
/// jump to the present rather than replay the past (8 frames ≈ 133 ms at 60 fps).
const VIDEO_QUEUE_CAP: usize = 8;
/// ~1.3 s of audio at 48 kHz / 1024-sample blocks.
const AUDIO_QUEUE_CAP: usize = 64;

impl UsbQueues {
    pub fn new() -> Arc<Self> {
        Arc::new(UsbQueues {
            inner: Mutex::new(QueueInner {
                video: VecDeque::with_capacity(VIDEO_QUEUE_CAP),
                audio: VecDeque::with_capacity(AUDIO_QUEUE_CAP),
                need_key: false,
                closed: false,
            }),
            cond: Condvar::new(),
        })
    }

    fn push_video(&self, pkt: Vec<u8>, info: hevc::PacketInfo) {
        let (is_csd, is_key) = (info.has_parameter_sets, info.has_keyframe);
        let mut q = self.inner.lock().unwrap_or_else(|e| e.into_inner());

        if q.need_key {
            if is_key {
                q.need_key = false;
            } else if !is_csd {
                Muxer::release_buffer(pkt);
                DROPPED_VIDEO_FRAMES.fetch_add(1, Ordering::Relaxed);
                return;
            }
        }

        if q.video.len() >= VIDEO_QUEUE_CAP {
            // Dropping one delta frame corrupts every frame after it until the
            // next keyframe, so shed the whole stale run instead: everything
            // before the newest keyframe (the incoming packet counts). Bare
            // parameter sets survive; the decoder needs them to restart.
            let boundary = if is_key {
                q.video.len()
            } else {
                q.video
                    .iter()
                    .rposition(|p| p.is_key)
                    .unwrap_or(q.video.len())
            };
            if boundary == q.video.len() && !is_key {
                q.need_key = true; // nothing to resume from yet
            }
            let mut idx = 0;
            let mut dropped = 0;
            q.video.retain(|p| {
                let keep = idx >= boundary || (p.is_csd && !p.is_key);
                idx += 1;
                dropped += u64::from(!keep);
                keep
            });
            DROPPED_VIDEO_FRAMES.fetch_add(dropped, Ordering::Relaxed);
            if q.need_key && !is_csd {
                // The incoming delta frame is as unusable as the ones just shed.
                DROPPED_VIDEO_FRAMES.fetch_add(1, Ordering::Relaxed);
                return;
            }
        }
        q.video.push_back(QueuedVideo {
            data: pkt,
            is_csd,
            is_key,
        });
        drop(q);
        self.cond.notify_one();
    }

    fn push_audio(&self, pkt: Vec<u8>) {
        let mut q = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        while q.audio.len() >= AUDIO_QUEUE_CAP {
            if let Some(dropped) = q.audio.pop_front() {
                Muxer::release_buffer(dropped);
            }
        }
        q.audio.push_back(pkt);
        drop(q);
        self.cond.notify_one();
    }

    /// Blocking pop for the USB writer: audio first, then video.
    /// Returns None on timeout or when the queues have been closed.
    pub fn pop_next(&self, timeout: Duration) -> PopResult {
        let mut q = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if q.audio.is_empty() && q.video.is_empty() && !q.closed {
            let (guard, _res) = self
                .cond
                .wait_timeout(q, timeout)
                .unwrap_or_else(|e| e.into_inner());
            q = guard;
        }
        if let Some(pkt) = q.audio.pop_front() {
            return PopResult::Packet(pkt);
        }
        if let Some(pkt) = q.video.pop_front() {
            return PopResult::Packet(pkt.data);
        }
        if q.closed {
            PopResult::Closed
        } else {
            PopResult::Timeout
        }
    }

    pub fn close(&self) {
        let mut q = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        q.closed = true;
        q.video.clear();
        q.audio.clear();
        drop(q);
        self.cond.notify_all();
    }
}

pub enum PopResult {
    Packet(Vec<u8>),
    Timeout,
    Closed,
}

/// Muxer that frames video and audio packets and feeds the USB write queues.
pub struct Muxer {
    queues: Arc<UsbQueues>,
}

impl Muxer {
    pub fn new(queues: Arc<UsbQueues>) -> Self {
        Muxer { queues }
    }

    /// Push raw H.265 / HEVC encoded video data into the mux pipeline.
    /// Never blocks (drop-oldest on overflow).
    pub fn push_video(&mut self, h265_data: &[u8]) {
        // Scanned on the raw payload before framing, so the queue never has
        // to look at packet bytes again.
        let info = hevc::scan_packet(h265_data);
        let frame = Self::frame_packet_pooled(PacketType::Video, h265_data);
        self.queues.push_video(frame, info);
    }

    /// Push raw PCM (f32 LE mono 48 kHz) audio data into the mux pipeline.
    /// Never blocks — safe to call from the audio capture thread.
    pub fn push_audio(&mut self, pcm_data: &[u8]) {
        let frame = Self::frame_packet_pooled(PacketType::Audio, pcm_data);
        self.queues.push_audio(frame);
    }

    fn frame_packet_pooled(ptype: PacketType, data: &[u8]) -> Vec<u8> {
        let mut buf = if let Ok(mut pool) = BUFFER_POOL.lock() {
            pool.pop()
                .unwrap_or_else(|| Vec::with_capacity(data.len() + HEADER_SIZE))
        } else {
            Vec::with_capacity(data.len() + HEADER_SIZE)
        };

        // Clears and reserves for us, so a pooled buffer is reused in place.
        mirror_protocol::write_frame(&mut buf, ptype, data);
        buf
    }

    /// Returns a buffer to the pool after use
    pub fn release_buffer(buf: Vec<u8>) {
        if let Ok(mut pool) = BUFFER_POOL.lock() {
            if pool.len() < 16 {
                pool.push(buf);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn framed(ptype: PacketType, payload: &[u8]) -> Vec<u8> {
        Muxer::frame_packet_pooled(ptype, payload)
    }

    #[test]
    fn pooled_framing_matches_the_shared_protocol() {
        // The layout itself is covered in mirror-protocol. What matters here
        // is that the pooled path produces byte-identical output to the plain
        // one — a reused buffer that was not cleared would append instead.
        let f = framed(PacketType::Video, b"abc");
        let mut expected = Vec::new();
        mirror_protocol::write_frame(&mut expected, PacketType::Video, b"abc");
        assert_eq!(f, expected);

        // Recycle the buffer and frame something shorter through it.
        Muxer::release_buffer(f);
        let f2 = framed(PacketType::Audio, b"z");
        let mut expected2 = Vec::new();
        mirror_protocol::write_frame(&mut expected2, PacketType::Audio, b"z");
        assert_eq!(f2, expected2);
    }

    #[test]
    fn audio_has_priority() {
        let q = UsbQueues::new();
        let mut m = Muxer::new(q.clone());
        m.push_video(b"vvvv");
        m.push_audio(b"aaaa");
        match q.pop_next(Duration::from_millis(10)) {
            PopResult::Packet(p) => assert_eq!(p[4], PacketType::Audio as u8),
            _ => panic!("expected packet"),
        }
        match q.pop_next(Duration::from_millis(10)) {
            PopResult::Packet(p) => assert_eq!(p[4], PacketType::Video as u8),
            _ => panic!("expected packet"),
        }
    }

    #[test]
    fn overflow_drops_oldest_but_keeps_csd() {
        let q = UsbQueues::new();
        let mut m = Muxer::new(q.clone());
        // CSD packet: VPS NAL (type 32)
        let csd = [0u8, 0, 0, 1, 32 << 1, 0x01];
        m.push_video(&csd);
        for _ in 0..VIDEO_QUEUE_CAP + 5 {
            m.push_video(b"frame");
        }
        // First packet out must still be the CSD
        match q.pop_next(Duration::from_millis(10)) {
            PopResult::Packet(p) => assert!(contains_csd(&p[HEADER_SIZE..])),
            _ => panic!("expected packet"),
        }
    }

    fn nal(t: u8) -> Vec<u8> {
        vec![0, 0, 0, 1, t << 1, 1]
    }

    fn drain(q: &UsbQueues) -> Vec<u8> {
        let mut types = Vec::new();
        while let PopResult::Packet(p) = q.pop_next(Duration::from_millis(1)) {
            types.push(p[HEADER_SIZE + 4] >> 1);
        }
        types
    }

    #[test]
    fn overflow_sheds_the_stale_run_up_to_the_newest_keyframe() {
        let q = UsbQueues::new();
        let mut m = Muxer::new(q.clone());
        m.push_video(&nal(19)); // old IDR
        for _ in 0..VIDEO_QUEUE_CAP - 2 {
            m.push_video(&nal(1));
        }
        m.push_video(&nal(19)); // newest IDR
        m.push_video(&nal(1)); // overflow: drop everything before the newest IDR
        assert_eq!(drain(&q), vec![19, 1]);
    }

    #[test]
    fn no_keyframe_to_resume_from_discards_deltas_until_one_arrives() {
        let q = UsbQueues::new();
        let mut m = Muxer::new(q.clone());
        for _ in 0..VIDEO_QUEUE_CAP + 1 {
            m.push_video(&nal(1));
        }
        m.push_video(&nal(1)); // still waiting for a keyframe
        m.push_video(&nal(19));
        m.push_video(&nal(1));
        assert_eq!(drain(&q), vec![19, 1]);
    }

    #[test]
    fn detects_csd_3byte_start_code() {
        assert!(contains_csd(&[0, 0, 1, 33 << 1, 0]));
        assert!(!contains_csd(&[0, 0, 1, 1 << 1, 0]));
    }

    #[test]
    fn closed_queue_reports_closed() {
        let q = UsbQueues::new();
        q.close();
        match q.pop_next(Duration::from_millis(10)) {
            PopResult::Closed => {}
            _ => panic!("expected Closed"),
        }
    }
}
