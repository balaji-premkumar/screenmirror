//! Getting the stream off the wire and into decoded frames.
//!
//! The stages run in order: `receiver` reads USB bulk transfers, `demuxer`
//! reassembles them into frames, this module's queue carries video packets
//! across a thread boundary, and `decoder` turns them into BGRA. `framepool`
//! recycles the buffers so a 60 fps session is not allocating 8 MB per frame.
//!
//! # Packet queue
//!
//! Shared by the USB receiver (producer) and the decoder thread (consumer).
//!
//! Design goals:
//! - The producer never blocks: on overflow we drop the oldest *droppable*
//!   packet (never CSD, never keyframes if avoidable) so the decoder can
//!   recover without waiting for the next IDR.
//! - The consumer blocks on a condvar instead of sleep-polling, so a frame
//!   is picked up the moment it arrives.

pub mod decoder;
pub mod demuxer;
pub mod framepool;
pub mod receiver;

use std::collections::VecDeque;
use std::sync::{Condvar, Mutex};
use std::time::Duration;

// The HEVC scan lives in `mirror-protocol`, next to the frame format. This
// file used to carry its own Annex-B walker — the third copy in the repo,
// alongside the mobile sender's and the player's — and the three had already
// drifted on how they treated a trailing start code.
pub use mirror_protocol::hevc::{scan_packet, PacketInfo};

/// A queued packet plus the result of scanning it, so the overflow path never
/// has to re-read packet bytes.
struct Queued {
    data: Vec<u8>,
    info: PacketInfo,
}

/// Bounded FIFO of encoded video packets with keyframe-aware overflow
/// handling and a blocking consumer side.
struct QueueState {
    packets: VecDeque<Queued>,
    /// Set after a drop left no keyframe to resume from. Until one arrives,
    /// delta frames are discarded rather than decoded against missing references.
    need_key: bool,
}

pub struct VideoQueue {
    inner: Mutex<QueueState>,
    cond: Condvar,
    cap: usize,
}

impl VideoQueue {
    pub fn new(cap: usize) -> Self {
        VideoQueue {
            inner: Mutex::new(QueueState {
                packets: VecDeque::with_capacity(cap),
                need_key: false,
            }),
            cond: Condvar::new(),
            cap,
        }
    }

    /// Push a packet. Never blocks. Returns the number of packets dropped
    /// to make room (0 in the common case).
    pub fn push(&self, pkt: Vec<u8>) -> usize {
        let info = scan_packet(&pkt);

        let mut q = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let mut dropped = 0;

        if q.need_key {
            if info.has_keyframe {
                q.need_key = false;
            } else if !info.has_parameter_sets {
                return 1;
            }
        }

        if q.packets.len() >= self.cap {
            // Dropping one delta frame corrupts every frame after it until the
            // next keyframe, so shed the whole stale run instead: everything
            // before the newest keyframe (the incoming packet counts). Bare
            // parameter sets survive; the decoder needs them to restart.
            let boundary = if info.has_keyframe {
                q.packets.len()
            } else {
                q.packets
                    .iter()
                    .rposition(|p| p.info.has_keyframe)
                    .unwrap_or(q.packets.len())
            };
            if boundary == q.packets.len() && !info.has_keyframe {
                q.need_key = true; // nothing to resume from yet
            }
            let mut idx = 0;
            q.packets.retain(|p| {
                let keep = idx >= boundary || (p.info.has_parameter_sets && !p.info.has_keyframe);
                idx += 1;
                dropped += usize::from(!keep);
                keep
            });
            if q.need_key && !info.has_parameter_sets {
                // The incoming delta frame is as unusable as the ones just shed.
                return dropped + 1;
            }
        }
        q.packets.push_back(Queued { data: pkt, info });
        drop(q);
        self.cond.notify_one();
        dropped
    }

    pub fn pop_timeout(&self, timeout: Duration) -> Option<Vec<u8>> {
        let mut q = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if q.packets.is_empty() {
            let (guard, _res) = self
                .cond
                .wait_timeout(q, timeout)
                .unwrap_or_else(|e| e.into_inner());
            q = guard;
        }
        q.packets.pop_front().map(|p| p.data)
    }

    /// How many packets are waiting. Reported to the interface as buffer depth.
    pub fn len(&self) -> usize {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .packets
            .len()
    }

    /// Whether the decoder has caught up with the receiver.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The queue's fixed capacity, used to report buffer health as a fraction.
    pub fn capacity(&self) -> usize {
        self.cap
    }

    pub fn clear(&self) {
        let mut q = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        q.packets.clear();
        q.need_key = false;
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
        assert!(info.has_parameter_sets);
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
        let dropped = q.push(nal(1)); // overflow: three deltas shed, incoming refused
        assert_eq!(dropped, 4);
        // CSD packet still at the front
        let first = q.pop_timeout(Duration::from_millis(10)).unwrap();
        assert!(scan_packet(&first).has_parameter_sets);
    }

    fn drain_types(q: &VideoQueue) -> Vec<u8> {
        let mut types = Vec::new();
        while let Some(p) = q.pop_timeout(Duration::from_millis(1)) {
            types.push(p[4] >> 1);
        }
        types
    }

    #[test]
    fn overflow_sheds_the_stale_run_up_to_the_newest_keyframe() {
        let q = VideoQueue::new(4);
        q.push(nal(19)); // old IDR
        q.push(nal(1));
        q.push(nal(19)); // newest IDR
        q.push(nal(1));
        let dropped = q.push(nal(1)); // overflow
        assert_eq!(dropped, 2);
        assert_eq!(drain_types(&q), vec![19, 1, 1]);
    }

    #[test]
    fn no_keyframe_to_resume_from_discards_deltas_until_one_arrives() {
        let q = VideoQueue::new(2);
        q.push(nal(1));
        q.push(nal(1));
        assert_eq!(q.push(nal(1)), 3); // sheds two, refuses the incoming one
        assert_eq!(q.push(nal(1)), 1); // still waiting for a keyframe
        q.push(nal(19));
        q.push(nal(1));
        assert_eq!(drain_types(&q), vec![19, 1]);
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
