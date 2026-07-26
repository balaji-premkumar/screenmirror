//! Wire format shared by the Mirror mobile sender and the desktop receiver.
//!
//! Before this crate existed the framing lived in two places — the mobile
//! `muxer.rs` wrote it and the desktop `demuxer.rs` parsed it — each with its
//! own copy of the magic bytes, the header size and the type tags. Nothing
//! stopped one side from being edited without the other, and a mismatch
//! surfaces as a silent stream stall rather than a compile error. Both sides
//! now depend on this crate, so the format is defined exactly once.
//!
//! # Frame layout
//!
//! ```text
//! ┌────────────┬──────────┬──────────────────┬───────────────┐
//! │ 4B magic   │ 1B type  │ 4B length (LE)   │ N bytes       │
//! │ DE AD BE EF│ 01 or 02 │ payload length   │ payload       │
//! └────────────┴──────────┴──────────────────┴───────────────┘
//! ```
//!
//! USB bulk transfers split and coalesce arbitrarily, so a reader must be able
//! to resynchronise on the magic and to handle a header arriving without its
//! payload. [`find_magic`] and [`FrameHeader::parse`] are the two primitives a
//! reader needs; [`write_header`] is the one a writer needs.

#![forbid(unsafe_code)]

pub mod hevc;

/// Marks the start of every frame on the wire.
///
/// Chosen to be improbable in HEVC payload data, not to be cryptographically
/// meaningful: a reader that finds it still validates the type tag and length
/// before trusting the frame.
pub const MAGIC: [u8; 4] = [0xDE, 0xAD, 0xBE, 0xEF];

/// Bytes before the payload: 4 magic + 1 type + 4 length.
pub const HEADER_SIZE: usize = 9;

/// Largest payload a reader will accept.
///
/// A 4K HEVC keyframe at a high bitrate is a couple of megabytes and an audio
/// block is a few kilobytes, so this is generous for real traffic. It exists to
/// bound the damage from a corrupt length field: without it, one bad header
/// makes the reader buffer until the claimed size arrives, and a frame that
/// large is never coming.
pub const MAX_PAYLOAD_SIZE: usize = 8 * 1024 * 1024;

/// Bumped whenever the layout above changes incompatibly.
///
/// It is not currently transmitted — the AOA handshake strings carry the
/// version the two sides negotiate on. It is defined here so that when a
/// version byte is added there is one place that already owns the number.
pub const PROTOCOL_VERSION: u16 = 1;

/// What a frame carries.
///
/// The discriminants are the bytes on the wire; do not renumber them without
/// bumping [`PROTOCOL_VERSION`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum PacketType {
    /// HEVC (H.265) in Annex-B byte-stream format.
    Video = 0x01,
    /// Interleaved 32-bit float PCM, mono, 48 kHz, little-endian.
    Audio = 0x02,
}

impl PacketType {
    /// The byte written to the wire for this type.
    #[inline]
    #[must_use]
    pub const fn as_byte(self) -> u8 {
        self as u8
    }

    /// Reads a type tag, returning `None` for any byte this version does not
    /// define. A reader treats `None` as corruption and resynchronises rather
    /// than guessing.
    #[inline]
    #[must_use]
    pub const fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            0x01 => Some(PacketType::Video),
            0x02 => Some(PacketType::Audio),
            _ => None,
        }
    }
}

/// A parsed frame header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameHeader {
    /// What the payload contains.
    pub packet_type: PacketType,
    /// Payload length in bytes, already validated against [`MAX_PAYLOAD_SIZE`].
    pub payload_len: usize,
}

impl FrameHeader {
    /// Total bytes this frame occupies on the wire, header included.
    #[inline]
    #[must_use]
    pub const fn frame_len(&self) -> usize {
        HEADER_SIZE + self.payload_len
    }

    /// Parses a header from a buffer positioned at the magic bytes.
    ///
    /// `bytes` may be longer than a header — trailing data is ignored — but it
    /// must be at least [`HEADER_SIZE`] long, otherwise [`HeaderError::Short`]
    /// tells the caller to wait for more of the stream rather than to
    /// resynchronise.
    ///
    /// # Errors
    ///
    /// See [`HeaderError`]. Every variant except `Short` means the bytes are
    /// not a valid header and the reader should skip past this magic.
    pub fn parse(bytes: &[u8]) -> Result<Self, HeaderError> {
        if bytes.len() < HEADER_SIZE {
            return Err(HeaderError::Short);
        }
        if bytes[..4] != MAGIC {
            return Err(HeaderError::BadMagic);
        }
        let packet_type =
            PacketType::from_byte(bytes[4]).ok_or(HeaderError::UnknownType(bytes[4]))?;
        let payload_len = u32::from_le_bytes([bytes[5], bytes[6], bytes[7], bytes[8]]) as usize;
        if payload_len > MAX_PAYLOAD_SIZE {
            return Err(HeaderError::PayloadTooLarge(payload_len));
        }
        Ok(FrameHeader {
            packet_type,
            payload_len,
        })
    }
}

/// Why a header could not be parsed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeaderError {
    /// Fewer than [`HEADER_SIZE`] bytes available. Not corruption — the caller
    /// should wait for more data and try again.
    Short,
    /// The buffer does not start at the magic bytes.
    BadMagic,
    /// The type tag is not one this protocol version defines.
    UnknownType(u8),
    /// The length field exceeds [`MAX_PAYLOAD_SIZE`].
    PayloadTooLarge(usize),
}

impl core::fmt::Display for HeaderError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            HeaderError::Short => write!(f, "incomplete frame header"),
            HeaderError::BadMagic => write!(f, "buffer does not start at the frame magic"),
            HeaderError::UnknownType(b) => write!(f, "unknown packet type 0x{b:02X}"),
            HeaderError::PayloadTooLarge(n) => {
                write!(
                    f,
                    "payload claims {n} bytes, over the {MAX_PAYLOAD_SIZE} limit"
                )
            }
        }
    }
}

impl std::error::Error for HeaderError {}

/// Appends a frame header to `out`.
///
/// The caller appends the payload itself, which lets a sender write straight
/// into a pooled buffer without an intermediate copy.
///
/// # Panics
///
/// Panics if `payload_len` exceeds [`MAX_PAYLOAD_SIZE`]. A sender that has
/// produced an oversized payload has a bug that the receiver would otherwise
/// see only as an unexplained stream resynchronisation.
pub fn write_header(out: &mut Vec<u8>, packet_type: PacketType, payload_len: usize) {
    assert!(
        payload_len <= MAX_PAYLOAD_SIZE,
        "payload of {payload_len} bytes exceeds the protocol maximum of {MAX_PAYLOAD_SIZE}"
    );
    out.extend_from_slice(&MAGIC);
    out.push(packet_type.as_byte());
    out.extend_from_slice(&(payload_len as u32).to_le_bytes());
}

/// Writes a complete frame — header then payload — into `out`.
///
/// `out` is cleared first, so a pooled buffer can be reused directly.
///
/// # Panics
///
/// Panics if `payload` is longer than [`MAX_PAYLOAD_SIZE`]; see
/// [`write_header`].
pub fn write_frame(out: &mut Vec<u8>, packet_type: PacketType, payload: &[u8]) {
    out.clear();
    out.reserve(HEADER_SIZE + payload.len());
    write_header(out, packet_type, payload.len());
    out.extend_from_slice(payload);
}

/// Finds the first occurrence of [`MAGIC`] in `haystack`.
///
/// Returns `None` when the magic does not appear, *and* when it might still
/// appear in bytes that have not arrived: a partial magic at the very end of
/// the buffer is reported as absent so the caller keeps those bytes and
/// re-scans once more data lands.
#[must_use]
pub fn find_magic(haystack: &[u8]) -> Option<usize> {
    if haystack.len() < MAGIC.len() {
        return None;
    }
    let mut offset = 0;
    while let Some(pos) = memchr::memchr(MAGIC[0], &haystack[offset..]) {
        let start = offset + pos;
        if haystack.len() - start < MAGIC.len() {
            // A truncated magic. Treat it as not-found so the caller waits for
            // the rest instead of discarding a header that is mid-flight.
            return None;
        }
        if haystack[start..start + MAGIC.len()] == MAGIC {
            return Some(start);
        }
        offset = start + 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(packet_type: PacketType, payload: &[u8]) -> Vec<u8> {
        let mut buf = Vec::new();
        write_frame(&mut buf, packet_type, payload);
        buf
    }

    #[test]
    fn header_layout_is_magic_type_then_le_length() {
        let f = frame(PacketType::Video, b"abc");
        assert_eq!(&f[0..4], &MAGIC);
        assert_eq!(f[4], 0x01);
        assert_eq!(u32::from_le_bytes([f[5], f[6], f[7], f[8]]), 3);
        assert_eq!(&f[9..], b"abc");
    }

    #[test]
    fn round_trips_through_parse() {
        for (ty, payload) in [
            (PacketType::Video, &b"video payload"[..]),
            (PacketType::Audio, &b"audio"[..]),
            (PacketType::Audio, &b""[..]),
        ] {
            let f = frame(ty, payload);
            let h = FrameHeader::parse(&f).expect("valid header");
            assert_eq!(h.packet_type, ty);
            assert_eq!(h.payload_len, payload.len());
            assert_eq!(h.frame_len(), f.len());
            assert_eq!(&f[HEADER_SIZE..h.frame_len()], payload);
        }
    }

    #[test]
    fn short_buffer_is_distinguishable_from_corruption() {
        let f = frame(PacketType::Video, b"abc");
        // One byte short of a header must not read as corruption: the caller
        // has to know to wait rather than to skip past the magic.
        assert_eq!(FrameHeader::parse(&f[..8]), Err(HeaderError::Short));
        assert!(FrameHeader::parse(&f).is_ok());
    }

    #[test]
    fn rejects_unknown_type_and_oversized_length() {
        let mut f = frame(PacketType::Video, b"abc");
        f[4] = 0x7F;
        assert_eq!(FrameHeader::parse(&f), Err(HeaderError::UnknownType(0x7F)));

        let mut f = frame(PacketType::Video, b"abc");
        f[5..9].copy_from_slice(&(MAX_PAYLOAD_SIZE as u32 + 1).to_le_bytes());
        assert_eq!(
            FrameHeader::parse(&f),
            Err(HeaderError::PayloadTooLarge(MAX_PAYLOAD_SIZE + 1))
        );
    }

    #[test]
    fn find_magic_skips_leading_noise() {
        let mut buf = vec![0x00, 0xDE, 0xAD, 0x00, 0xDE];
        buf.extend_from_slice(&frame(PacketType::Audio, b"x"));
        // The 0xDE at index 1 and index 4 are decoys; the real magic is at 5.
        assert_eq!(find_magic(&buf), Some(5));
    }

    #[test]
    fn find_magic_waits_on_a_truncated_magic() {
        // 0xDE 0xAD 0xBE with the final byte still in flight. Reporting Some
        // here would make the caller parse a header that does not exist yet;
        // reporting None keeps the bytes buffered.
        assert_eq!(find_magic(&[0x11, 0xDE, 0xAD, 0xBE]), None);
    }

    #[test]
    fn packet_type_byte_mapping_is_stable() {
        // These values are on the wire. Changing them without bumping
        // PROTOCOL_VERSION silently breaks every deployed sender.
        assert_eq!(PacketType::Video.as_byte(), 0x01);
        assert_eq!(PacketType::Audio.as_byte(), 0x02);
        assert_eq!(PacketType::from_byte(0x01), Some(PacketType::Video));
        assert_eq!(PacketType::from_byte(0x02), Some(PacketType::Audio));
        assert_eq!(PacketType::from_byte(0x00), None);
        assert_eq!(PacketType::from_byte(0x03), None);
    }

    #[test]
    #[should_panic(expected = "exceeds the protocol maximum")]
    fn writing_an_oversized_payload_panics_at_the_sender() {
        let mut buf = Vec::new();
        write_header(&mut buf, PacketType::Video, MAX_PAYLOAD_SIZE + 1);
    }
}
