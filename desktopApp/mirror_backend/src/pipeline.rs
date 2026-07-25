//! Video packet queue and HEVC bitstream helpers shared by the USB receiver
//! (producer) and the decoder thread (consumer).
//!
//! Design goals:
//! - The producer never blocks: on overflow we drop the oldest *droppable*
//!   packet (never CSD, never keyframes if avoidable) so the decoder can
//!   recover without waiting for the next IDR.
//! - The consumer blocks on a condvar instead of sleep-polling, so a frame
//!   is picked up the moment it arrives.

use std::collections::VecDeque;
use std::sync::{Condvar, Mutex};
use std::time::Duration;

/// Result of scanning an HEVC Annex-B packet for NAL units.
#[derive(Default, Clone, Copy)]
pub struct PacketInfo {
    /// Contains an IRAP picture (BLA/IDR/CRA, NAL types 16..=21)
    pub has_keyframe: bool,
    /// Contains parameter sets (VPS/SPS/PPS, NAL types 32..=34)
    pub has_csd: bool,
}

impl PacketInfo {
    pub fn is_essential(&self) -> bool {
        self.has_keyframe || self.has_csd
    }
}

/// Scan an Annex-B HEVC bitstream for NAL unit types.
/// Handles both 3-byte (00 00 01) and 4-byte (00 00 00 01) start codes and
/// packets that contain multiple NAL units (e.g. VPS+SPS+PPS+IDR in one
/// MediaCodec output buffer).
pub fn scan_packet(data: &[u8]) -> PacketInfo {
    let mut info = PacketInfo::default();
    let mut i = 0;
    while i + 3 < data.len() {
        // Find next start code
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
                // HEVC NAL header: forbidden_zero(1) | nal_unit_type(6) | ...
                let nal_type = (data[nal_start] >> 1) & 0x3F;
                match nal_type {
                    16..=21 => info.has_keyframe = true, // BLA/IDR/CRA (IRAP)
                    32..=34 => info.has_csd = true,      // VPS/SPS/PPS
                    _ => {}
                }
            }
            i = nal_start;
        } else {
            i += 1;
        }
    }
    info
}

/// A queued packet plus the result of scanning it, so the overflow path never
/// has to re-read packet bytes.
struct Queued {
    data: Vec<u8>,
    info: PacketInfo,
}

/// Bounded FIFO of encoded video packets with keyframe-aware overflow
/// handling and a blocking consumer side.
pub struct VideoQueue {
    inner: Mutex<VecDeque<Queued>>,
    cond: Condvar,
    cap: usize,
}

impl VideoQueue {
    pub fn new(cap: usize) -> Self {
        VideoQueue {
            inner: Mutex::new(VecDeque::with_capacity(cap)),
            cond: Condvar::new(),
            cap,
        }
    }

    /// Push a packet. Never blocks. Returns the number of packets dropped
    /// to make room (0 in the common case).
    pub fn push(&self, pkt: Vec<u8>) -> usize {
        // Scanned once here, outside the lock. The old code re-scanned every
        // queued packet's full payload on each overflow push, holding the
        // mutex the whole time — hundreds of MB/s of scanning on the USB
        // reader thread exactly when the pipeline was already struggling.
        let info = scan_packet(&pkt);

        let mut q = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let mut dropped = 0;
        while q.len() >= self.cap {
            // Prefer dropping the oldest packet that is neither CSD nor a
            // keyframe; if everything is essential, drop the oldest anyway.
            let victim = q
                .iter()
                .position(|p| !p.info.is_essential())
                .unwrap_or(0);
            q.remove(victim);
            dropped += 1;
        }
        q.push_back(Queued { data: pkt, info });
        drop(q);
        self.cond.notify_one();
        dropped
    }

    /// Block until a packet is available or the timeout elapses.
    pub fn pop_timeout(&self, timeout: Duration) -> Option<Vec<u8>> {
        let mut q = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if q.is_empty() {
            let (guard, _res) = self
                .cond
                .wait_timeout(q, timeout)
                .unwrap_or_else(|e| e.into_inner());
            q = guard;
        }
        q.pop_front().map(|p| p.data)
    }

    pub fn len(&self) -> usize {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    pub fn capacity(&self) -> usize {
        self.cap
    }

    pub fn clear(&self) {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nal(t: u8) -> Vec<u8> {
        // 4-byte start code + minimal NAL header
        vec![0, 0, 0, 1, t << 1, 0x01]
    }

    fn nal3(t: u8) -> Vec<u8> {
        // 3-byte start code variant
        vec![0, 0, 1, t << 1, 0x01]
    }

    #[test]
    fn detects_idr_with_4byte_start_code() {
        assert!(scan_packet(&nal(19)).has_keyframe);
        assert!(scan_packet(&nal(20)).has_keyframe);
    }

    #[test]
    fn detects_idr_with_3byte_start_code() {
        assert!(scan_packet(&nal3(19)).has_keyframe);
    }

    #[test]
    fn detects_csd_bundle() {
        let mut pkt = nal(32); // VPS
        pkt.extend(nal(33)); // SPS
        pkt.extend(nal(34)); // PPS
        pkt.extend(nal3(19)); // IDR with 3-byte start code
        let info = scan_packet(&pkt);
        assert!(info.has_csd);
        assert!(info.has_keyframe);
    }

    #[test]
    fn plain_frame_not_essential() {
        let info = scan_packet(&nal(1)); // TRAIL_R
        assert!(!info.is_essential());
    }

    #[test]
    fn overflow_preserves_essential_packets() {
        let q = VideoQueue::new(4);
        q.push(nal(32)); // CSD — must survive
        q.push(nal(1));
        q.push(nal(1));
        q.push(nal(1));
        let dropped = q.push(nal(1)); // overflow
        assert_eq!(dropped, 1);
        // CSD packet still at the front
        let first = q.pop_timeout(Duration::from_millis(10)).unwrap();
        assert!(scan_packet(&first).has_csd);
    }

    #[test]
    fn pop_blocks_then_receives() {
        use std::sync::Arc;
        let q = Arc::new(VideoQueue::new(4));
        let q2 = q.clone();
        let h = std::thread::spawn(move || q2.pop_timeout(Duration::from_secs(2)));
        std::thread::sleep(Duration::from_millis(50));
        q.push(nal(1));
        assert!(h.join().unwrap().is_some());
    }
}
