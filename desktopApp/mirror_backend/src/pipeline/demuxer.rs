//! Reassembles framed packets from arbitrary USB bulk reads.
//!
//! A bulk transfer splits and coalesces without regard for frame boundaries:
//! one read can carry half a header, and the next can carry the rest plus two
//! whole frames. This module keeps a reassembly buffer and hands out only
//! complete frames.
//!
//! The frame layout itself lives in `mirror-protocol`, which the mobile sender
//! also depends on. Before that crate existed this file had its own copy of
//! the magic bytes, the header size and the type tags — nothing tied them to
//! the writer, and a mismatch would show up as a stalled stream rather than a
//! build failure.

use crate::log_event;
use bytes::{Buf, BytesMut};
use mirror_i18n::codes;
use mirror_protocol::{FrameHeader, HeaderError, PacketType, HEADER_SIZE, MAGIC, MAX_PAYLOAD_SIZE};

/// Hard cap on the reassembly buffer before we give up and resynchronise.
const MAX_BUFFER_SIZE: usize = 2 * MAX_PAYLOAD_SIZE;

/// Only the first kilobyte of discarded bytes is reported, so a badly
/// desynchronised link does not bury every other event in the log.
const DISCARD_LOG_LIMIT: u64 = 1024;

/// How often to report progress, in frames.
const PROGRESS_EVERY: u64 = 2000;

/// A fully reassembled frame extracted from the USB byte stream.
pub struct DemuxedFrame {
    /// What the payload contains.
    pub packet_type: PacketType,
    /// The payload, header stripped.
    pub data: Vec<u8>,
}

/// Stream demuxer. Feed it bytes, take out frames.
pub struct Demuxer {
    buffer: BytesMut,
    frames_video: u64,
    frames_audio: u64,
    bytes_discarded: u64,
}

impl Default for Demuxer {
    fn default() -> Self {
        Self::new()
    }
}

impl Demuxer {
    /// Creates a demuxer with a 256 KB reassembly buffer.
    pub fn new() -> Self {
        Demuxer {
            buffer: BytesMut::with_capacity(256 * 1024),
            frames_video: 0,
            frames_audio: 0,
            bytes_discarded: 0,
        }
    }

    /// Feeds raw bytes from a USB bulk read.
    ///
    /// Returns every frame that became complete — none, one, or many.
    pub fn feed(&mut self, data: &[u8]) -> Vec<DemuxedFrame> {
        self.buffer.extend_from_slice(data);

        let mut frames = Vec::new();

        // Ends when no magic is left in the buffer — wait for more data then.
        while let Some(magic_pos) = mirror_protocol::find_magic(&self.buffer) {
            if magic_pos > 0 {
                self.bytes_discarded += magic_pos as u64;
                if self.bytes_discarded <= DISCARD_LOG_LIMIT {
                    log_event!(codes::DEMUX_RESYNC_DISCARDED, "bytes" => magic_pos);
                }
                self.buffer.advance(magic_pos);
            }

            let header = match FrameHeader::parse(&self.buffer) {
                Ok(h) => h,
                Err(HeaderError::Short) => break, // Wait for the rest.
                Err(HeaderError::UnknownType(byte)) => {
                    log_event!(codes::DEMUX_FRAME_UNKNOWN_TYPE, "type" => format!("0x{byte:02X}"));
                    self.buffer.advance(MAGIC.len());
                    continue;
                }
                Err(HeaderError::PayloadTooLarge(len)) => {
                    log_event!(codes::DEMUX_FRAME_OVERSIZED, "bytes" => len);
                    self.buffer.advance(MAGIC.len());
                    continue;
                }
                Err(HeaderError::BadMagic) => {
                    // find_magic just said the magic is here, so this is
                    // unreachable in practice. Resynchronising anyway is the
                    // safe branch: assuming it cannot happen risks a spin.
                    self.buffer.advance(MAGIC.len());
                    continue;
                }
            };

            if self.buffer.len() < header.frame_len() {
                break; // Partial payload — wait for more data.
            }

            let payload = self.buffer[HEADER_SIZE..header.frame_len()].to_vec();
            self.buffer.advance(header.frame_len());

            match header.packet_type {
                PacketType::Video => self.frames_video += 1,
                PacketType::Audio => self.frames_audio += 1,
            }
            self.report_progress(header.packet_type, payload.len());

            frames.push(DemuxedFrame {
                packet_type: header.packet_type,
                data: payload,
            });
        }

        // Prevent unbounded growth from corrupt or non-framed data.
        if self.buffer.len() > MAX_BUFFER_SIZE {
            log_event!(codes::DEMUX_BUFFER_OVERFLOW, "bytes" => self.buffer.len());
            self.buffer.clear();
        }

        frames
    }

    fn report_progress(&self, packet_type: PacketType, payload_len: usize) {
        let total = self.frames_video + self.frames_audio;
        if total == 1 {
            let kind = match packet_type {
                PacketType::Video => "video",
                PacketType::Audio => "audio",
            };
            log_event!(codes::DEMUX_FRAME_FIRST, "kind" => kind, "bytes" => payload_len);
        } else if total.is_multiple_of(PROGRESS_EVERY) && self.frames_video > 0 {
            log_event!(
                codes::DEMUX_PROGRESS,
                "frames" => total,
                "video" => self.frames_video,
                "audio" => self.frames_audio,
                "discarded" => self.bytes_discarded,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(packet_type: PacketType, payload: &[u8]) -> Vec<u8> {
        let mut buf = Vec::new();
        mirror_protocol::write_frame(&mut buf, packet_type, payload);
        buf
    }

    #[test]
    fn extracts_a_single_complete_frame() {
        let mut demuxer = Demuxer::new();
        let frames = demuxer.feed(&frame(PacketType::Video, b"hello"));
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].packet_type, PacketType::Video);
        assert_eq!(frames[0].data, b"hello");
    }

    #[test]
    fn reassembles_a_frame_split_across_reads() {
        let mut demuxer = Demuxer::new();
        let packet = frame(PacketType::Audio, b"abc");

        // Six bytes is mid-header: magic and type are in, the length is not.
        assert!(demuxer.feed(&packet[0..6]).is_empty());

        let frames = demuxer.feed(&packet[6..]);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].packet_type, PacketType::Audio);
        assert_eq!(frames[0].data, b"abc");
    }

    #[test]
    fn reassembles_one_byte_at_a_time() {
        // The pathological split. Nothing should emerge until the last byte.
        let mut demuxer = Demuxer::new();
        let packet = frame(PacketType::Video, b"payload");
        for byte in &packet[..packet.len() - 1] {
            assert!(demuxer.feed(&[*byte]).is_empty());
        }
        let frames = demuxer.feed(&packet[packet.len() - 1..]);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"payload");
    }

    #[test]
    fn extracts_several_frames_from_one_read() {
        let mut demuxer = Demuxer::new();
        let mut packet = frame(PacketType::Video, b"ab");
        packet.extend_from_slice(&frame(PacketType::Audio, b"xyz"));

        let frames = demuxer.feed(&packet);
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].packet_type, PacketType::Video);
        assert_eq!(frames[1].packet_type, PacketType::Audio);
    }

    #[test]
    fn resynchronises_past_leading_garbage() {
        let mut demuxer = Demuxer::new();
        let mut packet = vec![0x00, 0xDE, 0xAD, 0x11, 0x22];
        packet.extend_from_slice(&frame(PacketType::Video, b"ok"));

        let frames = demuxer.feed(&packet);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"ok");
    }

    #[test]
    fn skips_a_corrupt_header_and_recovers_the_next_frame() {
        let mut demuxer = Demuxer::new();
        // A frame whose type tag is invalid, immediately followed by a good one.
        let mut packet = frame(PacketType::Video, b"junk");
        packet[4] = 0x7F;
        packet.extend_from_slice(&frame(PacketType::Audio, b"good"));

        let frames = demuxer.feed(&packet);
        assert_eq!(frames.len(), 1, "the corrupt frame must not be emitted");
        assert_eq!(frames[0].packet_type, PacketType::Audio);
        assert_eq!(frames[0].data, b"good");
    }

    #[test]
    fn an_oversized_length_does_not_stall_the_stream() {
        // Without the length check this header makes the demuxer wait for a
        // payload that is never coming, and every later frame is lost behind
        // it.
        let mut demuxer = Demuxer::new();
        let mut packet = frame(PacketType::Video, b"x");
        packet[5..9].copy_from_slice(&(MAX_PAYLOAD_SIZE as u32 + 1).to_le_bytes());
        packet.extend_from_slice(&frame(PacketType::Video, b"after"));

        let frames = demuxer.feed(&packet);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].data, b"after");
    }

    #[test]
    fn a_zero_length_payload_is_a_valid_frame() {
        let mut demuxer = Demuxer::new();
        let frames = demuxer.feed(&frame(PacketType::Audio, b""));
        assert_eq!(frames.len(), 1);
        assert!(frames[0].data.is_empty());
    }

    #[test]
    fn a_flood_of_non_framed_data_clears_rather_than_growing_without_bound() {
        let mut demuxer = Demuxer::new();
        let junk = vec![0x11u8; 1024 * 1024];
        for _ in 0..(MAX_BUFFER_SIZE / junk.len()) + 2 {
            assert!(demuxer.feed(&junk).is_empty());
        }
        assert!(
            demuxer.buffer.len() <= MAX_BUFFER_SIZE,
            "buffer grew to {} bytes",
            demuxer.buffer.len()
        );
    }
}
