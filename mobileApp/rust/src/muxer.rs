use byteorder::{LittleEndian, WriteBytesExt};
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

/// Frame type tag bytes sent over the USB link
#[derive(Clone, Copy)]
#[repr(u8)]
pub enum PacketType {
    Video = 0x01,
    Audio = 0x02,
}

/// Scan an Annex-B HEVC packet for parameter sets (VPS/SPS/PPS).
/// Handles both 3- and 4-byte start codes. Used to preserve codec config
/// packets across overflow drops and muxer restarts.
pub fn contains_csd(data: &[u8]) -> bool {
    let mut i = 0;
    while i + 3 < data.len() {
        if data[i] == 0 && data[i + 1] == 0 {
            let nal_start = if data[i + 2] == 1 {
                i + 3
            } else if data[i + 2] == 0 && i + 3 < data.len() && data[i + 3] == 1 {
                i + 4
            } else {
                i += 1;
                continue;
            };
            if nal_start < data.len() {
                let nal_type = (data[nal_start] >> 1) & 0x3F;
                if (32..=34).contains(&nal_type) {
                    return true; // VPS/SPS/PPS
                }
            }
            i = nal_start;
        } else {
            i += 1;
        }
    }
    false
}

/// A framed packet plus whether it carries parameter sets. Computed once on
/// the way in: the overflow path used to re-scan every one of the 180 queued
/// payloads while holding the queue lock, on the MediaCodec drain thread.
struct QueuedVideo {
    data: Vec<u8>,
    is_csd: bool,
}

struct QueueInner {
    video: VecDeque<QueuedVideo>,
    audio: VecDeque<Vec<u8>>,
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
const VIDEO_QUEUE_CAP: usize = 180;
/// ~1.3 s of audio at 48 kHz / 1024-sample blocks.
const AUDIO_QUEUE_CAP: usize = 64;

impl UsbQueues {
    pub fn new() -> Arc<Self> {
        Arc::new(UsbQueues {
            inner: Mutex::new(QueueInner {
                video: VecDeque::with_capacity(VIDEO_QUEUE_CAP),
                audio: VecDeque::with_capacity(AUDIO_QUEUE_CAP),
                closed: false,
            }),
            cond: Condvar::new(),
        })
    }

    fn push_video(&self, pkt: Vec<u8>, is_csd: bool) {
        let mut q = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        while q.video.len() >= VIDEO_QUEUE_CAP {
            // Drop the oldest non-CSD packet.
            let victim = q.video.iter().position(|p| !p.is_csd).unwrap_or(0);
            if let Some(dropped) = q.video.remove(victim) {
                Muxer::release_buffer(dropped.data);
            }
            DROPPED_VIDEO_FRAMES.fetch_add(1, Ordering::Relaxed);
        }
        q.video.push_back(QueuedVideo { data: pkt, is_csd });
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
        let is_csd = contains_csd(h265_data);
        let frame = Self::frame_packet_pooled(PacketType::Video, h265_data);
        self.queues.push_video(frame, is_csd);
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
                .unwrap_or_else(|| Vec::with_capacity(data.len() + 9))
        } else {
            Vec::with_capacity(data.len() + 9)
        };

        buf.clear();
        buf.extend_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF]);
        buf.push(ptype as u8);
        buf.write_u32::<LittleEndian>(data.len() as u32).unwrap();
        buf.extend_from_slice(data);
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
    fn frame_header_layout() {
        let f = framed(PacketType::Video, b"abc");
        assert_eq!(&f[0..4], &[0xDE, 0xAD, 0xBE, 0xEF]);
        assert_eq!(f[4], 0x01);
        assert_eq!(u32::from_le_bytes([f[5], f[6], f[7], f[8]]), 3);
        assert_eq!(&f[9..], b"abc");
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
            PopResult::Packet(p) => assert!(contains_csd(&p[9..])),
            _ => panic!("expected packet"),
        }
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
