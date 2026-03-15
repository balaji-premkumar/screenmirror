use byteorder::{ReadBytesExt, LittleEndian};
use std::io::Cursor;
use crate::receiver::log_event;

/// Magic header that the mobile muxer prepends to every frame.
/// Must match the mobile's Muxer::frame_packet() magic bytes exactly.
const MAGIC: [u8; 4] = [0xDE, 0xAD, 0xBE, 0xEF];

/// Minimum header size: 4B magic + 1B type + 4B length = 9 bytes
const HEADER_SIZE: usize = 9;

/// Maximum payload size we'll accept (64 MB) — anything larger is corrupt
const MAX_PAYLOAD_SIZE: usize = 64 * 1024 * 1024;

/// Frame type tags — mirrors the mobile's PacketType enum
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FrameType {
    Video, // 0x01
    Audio, // 0x02
}

/// A fully reassembled frame extracted from the USB byte stream
pub struct DemuxedFrame {
    pub frame_type: FrameType,
    pub data: Vec<u8>,
}

/// Stream demuxer that reassembles framed packets from arbitrary USB bulk reads.
///
/// The mobile muxer wraps each encoded frame in:
///   [4B magic: 0xDEADBEEF][1B type][4B payload length LE][NB payload]
///
/// USB bulk transfers can split a single frame across multiple reads,
/// or deliver multiple frames in one read. This demuxer handles both cases
/// by maintaining an internal reassembly buffer.
pub struct Demuxer {
    buffer: Vec<u8>,
    stats_frames_video: u64,
    stats_frames_audio: u64,
    stats_bytes_discarded: u64,
}

impl Demuxer {
    pub fn new() -> Self {
        Demuxer {
            buffer: Vec::with_capacity(256 * 1024), // Pre-allocate 256KB
            stats_frames_video: 0,
            stats_frames_audio: 0,
            stats_bytes_discarded: 0,
        }
    }

    /// Feed raw bytes from a USB bulk read into the demuxer.
    /// Returns a Vec of fully reassembled frames (may be 0, 1, or many).
    pub fn feed(&mut self, data: &[u8]) -> Vec<DemuxedFrame> {
        self.buffer.extend_from_slice(data);

        let mut frames = Vec::new();

        loop {
            // Try to find the magic header in the buffer
            let magic_pos = match self.find_magic() {
                Some(pos) => pos,
                None => break, // No magic found — wait for more data
            };

            // If there are stray bytes before the magic, discard them
            if magic_pos > 0 {
                self.stats_bytes_discarded += magic_pos as u64;
                if self.stats_bytes_discarded <= 1024 {
                    // Only log the first few to avoid spam
                    log_event("WARN", "DEMUX", "pipeline",
                        &format!("Discarded {} bytes before magic header", magic_pos));
                }
                self.buffer.drain(0..magic_pos);
            }

            // Need at least the full header to proceed
            if self.buffer.len() < HEADER_SIZE {
                break; // Wait for more data
            }

            // Parse the header
            let frame_type_byte = self.buffer[4];
            let frame_type = match frame_type_byte {
                0x01 => FrameType::Video,
                0x02 => FrameType::Audio,
                _ => {
                    // Invalid frame type — skip past this magic and try again
                    log_event("WARN", "DEMUX", "pipeline",
                        &format!("Unknown frame type: 0x{:02X}, skipping", frame_type_byte));
                    self.buffer.drain(0..4); // Skip past the magic
                    continue;
                }
            };

            // Read the payload length (4 bytes LE at offset 5)
            let payload_len = {
                let mut cursor = Cursor::new(&self.buffer[5..9]);
                cursor.read_u32::<LittleEndian>().unwrap() as usize
            };

            // Sanity check: reject impossibly large frames
            if payload_len > MAX_PAYLOAD_SIZE {
                log_event("ERROR", "DEMUX", "pipeline",
                    &format!("Frame claims {} bytes — exceeds max, likely corrupt. Skipping.", payload_len));
                self.buffer.drain(0..4); // Skip past the magic
                continue;
            }

            // Check if we have the full frame payload yet
            let total_frame_size = HEADER_SIZE + payload_len;
            if self.buffer.len() < total_frame_size {
                break; // Partial frame — wait for more data
            }

            // Extract the payload
            let payload = self.buffer[HEADER_SIZE..total_frame_size].to_vec();

            // Remove the consumed frame from the buffer
            self.buffer.drain(0..total_frame_size);

            // Update stats
            match frame_type {
                FrameType::Video => self.stats_frames_video += 1,
                FrameType::Audio => self.stats_frames_audio += 1,
            }

            // Log periodically
            let total = self.stats_frames_video + self.stats_frames_audio;
            if total == 1 {
                log_event("SUCCESS", "DEMUX", "pipeline",
                    &format!("First {:?} frame demuxed: {} bytes", frame_type, payload.len()));
            } else if total % 500 == 0 {
                log_event("INFO", "DEMUX", "pipeline",
                    &format!("Demuxed {} frames (V:{} A:{}, discarded {} bytes)",
                        total, self.stats_frames_video, self.stats_frames_audio,
                        self.stats_bytes_discarded));
            }

            frames.push(DemuxedFrame {
                frame_type,
                data: payload,
            });
        }

        // Prevent unbounded buffer growth from corrupt/non-framed data
        if self.buffer.len() > MAX_PAYLOAD_SIZE {
            log_event("ERROR", "DEMUX", "pipeline",
                &format!("Buffer grew to {} bytes without finding valid frame — clearing", self.buffer.len()));
            self.buffer.clear();
        }

        frames
    }

    /// Scan the buffer for the 4-byte magic sequence.
    /// Returns the byte offset of the first occurrence, or None.
    fn find_magic(&self) -> Option<usize> {
        self.buffer.windows(4).position(|w| w == MAGIC)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_single_complete_frame() {
        let mut demuxer = Demuxer::new();
        // Build a video frame: magic + type(0x01) + length(5) + "hello"
        let mut packet = vec![0xDE, 0xAD, 0xBE, 0xEF, 0x01];
        packet.extend_from_slice(&5u32.to_le_bytes());
        packet.extend_from_slice(b"hello");

        let frames = demuxer.feed(&packet);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].frame_type, FrameType::Video);
        assert_eq!(frames[0].data, b"hello");
    }

    #[test]
    fn test_split_across_reads() {
        let mut demuxer = Demuxer::new();
        let mut packet = vec![0xDE, 0xAD, 0xBE, 0xEF, 0x02];
        packet.extend_from_slice(&3u32.to_le_bytes());
        packet.extend_from_slice(b"abc");

        // Feed first 6 bytes (partial header + partial length)
        let frames = demuxer.feed(&packet[0..6]);
        assert_eq!(frames.len(), 0);

        // Feed the rest
        let frames = demuxer.feed(&packet[6..]);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].frame_type, FrameType::Audio);
        assert_eq!(frames[0].data, b"abc");
    }

    #[test]
    fn test_multiple_frames_in_one_read() {
        let mut demuxer = Demuxer::new();
        let mut packet = Vec::new();

        // Frame 1: video
        packet.extend_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF, 0x01]);
        packet.extend_from_slice(&2u32.to_le_bytes());
        packet.extend_from_slice(b"ab");

        // Frame 2: audio
        packet.extend_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF, 0x02]);
        packet.extend_from_slice(&3u32.to_le_bytes());
        packet.extend_from_slice(b"xyz");

        let frames = demuxer.feed(&packet);
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].frame_type, FrameType::Video);
        assert_eq!(frames[1].frame_type, FrameType::Audio);
    }
}
