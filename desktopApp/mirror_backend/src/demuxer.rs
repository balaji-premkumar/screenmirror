use crate::receiver::log_event;
use byteorder::{LittleEndian, ReadBytesExt};
use bytes::{Buf, BytesMut};
use std::io::Cursor;
use memchr;

/// Magic header that the mobile muxer prepends to every frame.
/// Must match the mobile's Muxer::frame_packet magic.
const MAGIC: [u8; 4] = [0xDE, 0xAD, 0xBE, 0xEF];

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FrameType {
    Video = 0x01,
    Audio = 0x02,
}

pub struct RawFrame {
    pub frame_type: FrameType,
    pub data: Vec<u8>,
}

pub struct Demuxer {
    buffer: BytesMut,
}

impl Demuxer {
    pub fn new() -> Self {
        Demuxer {
            buffer: BytesMut::with_capacity(2 * 1024 * 1024), // 2MB initial
        }
    }

    /// Feed a chunk of raw USB data into the demuxer.
    /// Returns any complete frames found in the stream.
    pub fn feed(&mut self, chunk: &[u8]) -> Vec<RawFrame> {
        self.buffer.extend_from_slice(chunk);
        let mut frames = Vec::new();

        loop {
            // 1. Find magic header
            let offset = match self.find_magic() {
                Some(off) => off,
                None => {
                    // No magic found. If buffer is huge, keep last 3 bytes (might be partial magic)
                    if self.buffer.len() > 64 * 1024 {
                        let keep = self.buffer.len().saturating_sub(3);
                        self.buffer.advance(keep);
                    }
                    break;
                }
            };

            // Discard data before magic
            if offset > 0 {
                self.buffer.advance(offset);
            }

            // 2. Check if we have enough for header: [4B magic][1B type][4B length] = 9 bytes
            if self.buffer.len() < 9 {
                break;
            }

            // 3. Peek at length without advancing yet
            let mut cursor = Cursor::new(&self.buffer[5..9]);
            let frame_len = cursor.read_u32::<LittleEndian>().unwrap_or(0) as usize;

            // 4. Validate reasonable frame size (max 8MB for 4K keyframes)
            if frame_len > 8 * 1024 * 1024 {
                log_event(
                    "ERROR",
                    "DEMUXER",
                    "parse",
                    &format!("Corrupt frame length detected: {} bytes. Resetting.", frame_len),
                );
                self.buffer.advance(4); // Skip this magic and try again
                continue;
            }

            // 5. Check if entire frame is in buffer
            if self.buffer.len() < 9 + frame_len {
                break;
            }

            // 6. Extract frame
            let ptype_byte = self.buffer[4];
            let frame_type = match ptype_byte {
                0x01 => FrameType::Video,
                0x02 => FrameType::Audio,
                _ => {
                    // Should not happen if protocol is followed
                    self.buffer.advance(4);
                    continue;
                }
            };

            // Advance past header
            self.buffer.advance(9);
            
            // Take the data
            let data = self.buffer.split_to(frame_len).to_vec();

            frames.push(RawFrame { frame_type, data });
        }

        frames
    }

    /// Scan the buffer for the 4-byte magic sequence.
    /// Returns the byte offset of the first occurrence, or None.
    fn find_magic(&self) -> Option<usize> {
        if self.buffer.len() < 4 {
            return None;
        }

        // Use memchr to find the first byte of the magic (0xDE), then check subsequent bytes.
        // This is significantly faster than windows(4).position() for large buffers.
        let mut offset = 0;
        while let Some(pos) = memchr::memchr(MAGIC[0], &self.buffer[offset..]) {
            let start = offset + pos;
            if self.buffer.len() - start < 4 {
                return None;
            }
            if &self.buffer[start..start + 4] == MAGIC {
                return Some(start);
            }
            offset = start + 1;
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_demuxer_basic() {
        let mut demuxer = Demuxer::new();
        let mut packet = Vec::new();
        // Magic
        packet.extend_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF]);
        // Type Video
        packet.push(0x01);
        // Length 4
        packet.extend_from_slice(&[4, 0, 0, 0]);
        // Data
        packet.extend_from_slice(b"test");

        let frames = demuxer.feed(&packet);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].frame_type, FrameType::Video);
        assert_eq!(frames[0].data, b"test");
    }

    #[test]
    fn test_demuxer_fragmented() {
        let mut demuxer = Demuxer::new();
        // Header
        demuxer.feed(&[0xDE, 0xAD, 0xBE, 0xEF, 0x01, 2, 0, 0, 0]);
        // Partial data
        let frames = demuxer.feed(&[b'h']);
        assert_eq!(frames.len(), 0);
        // Rest of data
        let frames = demuxer.feed(&[b'i']);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"hi");
    }

    #[test]
    fn test_demuxer_garbage_skip() {
        let mut demuxer = Demuxer::new();
        let mut packet = Vec::new();
        packet.extend_from_slice(b"garbage");
        packet.extend_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF, 0x02, 3, 0, 0, 0]);
        packet.extend_from_slice(b"abc");
        
        // Second frame immediately
        packet.extend_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF, 0x01, 3, 0, 0, 0]);
        packet.extend_from_slice(b"xyz");

        let frames = demuxer.feed(&packet);
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].frame_type, FrameType::Audio);
        assert_eq!(frames[1].frame_type, FrameType::Video);
    }
}
