//! Annex-B HEVC helpers used by both ends of the link.
//!
//! The sender needs to know whether a packet carries parameter sets so it can
//! protect it from being dropped on queue overflow; the receiver needs to pull
//! those same parameter sets out to build the `hvcC` record Matroska requires.
//! Both had their own byte-stream walker, subtly different from the other —
//! one treated a trailing 3-byte start code as a NAL boundary and one did not.
//! There is one walker now, and both questions are answered from it.

/// NAL unit types that carry decoder configuration: VPS (32), SPS (33), PPS
/// (34). Everything a decoder needs before it can accept a coded frame.
const PARAMETER_SET_TYPES: core::ops::RangeInclusive<u8> = 32..=34;

/// NAL unit types for an IRAP picture — BLA, IDR and CRA (16 through 21).
/// A decoder can start here without any earlier frame.
const IRAP_TYPES: core::ops::RangeInclusive<u8> = 16..=21;

/// One NAL unit located inside an Annex-B byte stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NalUnit<'a> {
    /// The HEVC NAL unit type, taken from bits 1..6 of the first header byte.
    pub nal_type: u8,
    /// The unit's payload, start code excluded.
    pub bytes: &'a [u8],
}

impl NalUnit<'_> {
    /// Whether this unit is a VPS, SPS or PPS.
    #[inline]
    #[must_use]
    pub fn is_parameter_set(&self) -> bool {
        PARAMETER_SET_TYPES.contains(&self.nal_type)
    }

    /// Whether this unit is an IRAP picture — a point a decoder can start from.
    #[inline]
    #[must_use]
    pub fn is_irap(&self) -> bool {
        IRAP_TYPES.contains(&self.nal_type)
    }
}

/// What a video packet carries, from one pass over its NAL units.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct PacketInfo {
    /// Contains an IRAP picture (BLA/IDR/CRA).
    pub has_keyframe: bool,
    /// Contains parameter sets (VPS/SPS/PPS).
    pub has_parameter_sets: bool,
}

impl PacketInfo {
    /// Whether dropping this packet would cost the decoder more than one frame.
    ///
    /// A queue under pressure drops non-essential packets first: losing a
    /// P-frame shows as one glitch, but losing the parameter sets or the
    /// keyframe stalls decoding until the next one arrives.
    #[inline]
    #[must_use]
    pub fn is_essential(&self) -> bool {
        self.has_keyframe || self.has_parameter_sets
    }
}

/// Scans a packet for both keyframes and parameter sets in one pass.
///
/// One MediaCodec output buffer routinely holds VPS+SPS+PPS+IDR together, so
/// both answers come from the same walk.
#[must_use]
pub fn scan_packet(data: &[u8]) -> PacketInfo {
    let mut info = PacketInfo::default();
    for nal in nal_units(data) {
        if nal.is_irap() {
            info.has_keyframe = true;
        } else if nal.is_parameter_set() {
            info.has_parameter_sets = true;
        }
        if info.has_keyframe && info.has_parameter_sets {
            break;
        }
    }
    info
}

/// True if `data` begins a start code at `i` (`00 00 01` or `00 00 00 01`).
#[inline]
fn is_start_code(data: &[u8], i: usize) -> bool {
    i + 2 < data.len()
        && data[i] == 0
        && data[i + 1] == 0
        && (data[i + 2] == 1 || (data[i + 2] == 0 && i + 3 < data.len() && data[i + 3] == 1))
}

/// Walks the NAL units in an Annex-B byte stream.
///
/// Handles both 3- and 4-byte start codes, and tolerates leading garbage:
/// bytes before the first start code are skipped rather than treated as a
/// unit. A unit runs to the next start code, or to the end of `data`.
pub fn nal_units(data: &[u8]) -> NalUnits<'_> {
    NalUnits { data, cursor: 0 }
}

/// Iterator returned by [`nal_units`].
#[derive(Debug)]
pub struct NalUnits<'a> {
    data: &'a [u8],
    cursor: usize,
}

impl<'a> Iterator for NalUnits<'a> {
    type Item = NalUnit<'a>;

    fn next(&mut self) -> Option<NalUnit<'a>> {
        let data = self.data;
        while self.cursor + 3 < data.len() {
            if !is_start_code(data, self.cursor) {
                self.cursor += 1;
                continue;
            }
            // 3-byte start code puts the header at +3, 4-byte at +4.
            let start = if data[self.cursor + 2] == 1 {
                self.cursor + 3
            } else {
                self.cursor + 4
            };
            if start >= data.len() {
                break;
            }

            let mut scan = start;
            let end = loop {
                if scan + 2 >= data.len() {
                    break data.len();
                }
                if is_start_code(data, scan) {
                    break scan;
                }
                scan += 1;
            };

            // `end` can equal `start` only for a malformed stream; advancing to
            // at least start + 1 guarantees the cursor moves and we terminate.
            self.cursor = end.max(start + 1);
            return Some(NalUnit {
                nal_type: (data[start] >> 1) & 0x3F,
                bytes: &data[start..end],
            });
        }
        self.cursor = data.len();
        None
    }
}

/// Whether `data` carries any VPS, SPS or PPS.
///
/// The sender calls this on every video packet, so it stops at the first
/// parameter set rather than walking the whole frame.
#[must_use]
pub fn contains_parameter_sets(data: &[u8]) -> bool {
    nal_units(data).any(|n| n.is_parameter_set())
}

/// Collects every VPS, SPS and PPS in `data` into a fresh Annex-B stream, each
/// prefixed with a 4-byte start code.
///
/// Returns an empty vector when `data` has none, which the caller reads as
/// "not enough information to configure a decoder yet".
#[must_use]
pub fn extract_parameter_sets(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    for nal in nal_units(data).filter(NalUnit::is_parameter_set) {
        out.extend_from_slice(&[0, 0, 0, 1]);
        out.extend_from_slice(nal.bytes);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds an Annex-B stream from (nal_type, payload_len) pairs using a
    /// 4-byte start code.
    fn stream(units: &[(u8, usize)]) -> Vec<u8> {
        let mut out = Vec::new();
        for &(nal_type, len) in units {
            out.extend_from_slice(&[0, 0, 0, 1]);
            out.push(nal_type << 1);
            out.push(0x01); // second header byte
            out.extend(std::iter::repeat_n(0xAA, len));
        }
        out
    }

    #[test]
    fn walks_units_with_either_start_code_length() {
        let mut data = vec![0, 0, 1, 33 << 1, 0x01, 0xAA]; // 3-byte
        data.extend_from_slice(&[0, 0, 0, 1, 1 << 1, 0x01, 0xBB]); // 4-byte
        let types: Vec<u8> = nal_units(&data).map(|n| n.nal_type).collect();
        assert_eq!(types, vec![33, 1]);
    }

    #[test]
    fn skips_bytes_before_the_first_start_code() {
        let mut data = vec![0xFF, 0xFE, 0xFD];
        data.extend_from_slice(&stream(&[(34, 2)]));
        let types: Vec<u8> = nal_units(&data).map(|n| n.nal_type).collect();
        assert_eq!(types, vec![34]);
    }

    #[test]
    fn detects_parameter_sets_and_ignores_slices() {
        assert!(contains_parameter_sets(&stream(&[(32, 1)]))); // VPS
        assert!(contains_parameter_sets(&stream(&[(33, 1)]))); // SPS
        assert!(contains_parameter_sets(&stream(&[(34, 1)]))); // PPS
        assert!(!contains_parameter_sets(&stream(&[(1, 4)]))); // TRAIL_R
        assert!(!contains_parameter_sets(&stream(&[(31, 1), (35, 1)]))); // just outside
        assert!(!contains_parameter_sets(&[]));
    }

    #[test]
    fn extracts_only_parameter_sets_in_order() {
        let data = stream(&[(32, 1), (1, 8), (33, 2), (34, 1), (1, 4)]);
        let out = extract_parameter_sets(&data);
        let types: Vec<u8> = nal_units(&out).map(|n| n.nal_type).collect();
        assert_eq!(types, vec![32, 33, 34]);
        // Every emitted unit must carry a 4-byte start code, which is what the
        // hvcC builder on the receiving side assumes.
        assert_eq!(&out[0..4], &[0, 0, 0, 1]);
    }

    #[test]
    fn extraction_is_empty_when_there_is_nothing_to_extract() {
        assert!(extract_parameter_sets(&stream(&[(1, 16)])).is_empty());
        assert!(extract_parameter_sets(&[]).is_empty());
        assert!(extract_parameter_sets(&[0, 0, 1]).is_empty());
    }

    #[test]
    fn scan_packet_answers_both_questions_in_one_pass() {
        // The shape a MediaCodec output buffer actually has at a keyframe.
        let bundle = stream(&[(32, 1), (33, 2), (34, 1), (19, 32)]);
        let info = scan_packet(&bundle);
        assert!(info.has_keyframe);
        assert!(info.has_parameter_sets);
        assert!(info.is_essential());

        let plain = stream(&[(1, 64)]); // TRAIL_R
        let info = scan_packet(&plain);
        assert!(!info.has_keyframe);
        assert!(!info.has_parameter_sets);
        assert!(!info.is_essential());
    }

    #[test]
    fn irap_range_covers_bla_idr_and_cra_only() {
        for t in 16..=21 {
            assert!(scan_packet(&stream(&[(t, 1)])).has_keyframe, "type {t}");
        }
        for t in [15u8, 22, 23] {
            assert!(!scan_packet(&stream(&[(t, 1)])).has_keyframe, "type {t}");
        }
    }

    #[test]
    fn a_csd_only_packet_is_still_essential() {
        // Dropping this leaves the decoder unable to configure itself, even
        // though the packet carries no picture at all.
        let info = scan_packet(&stream(&[(33, 4)]));
        assert!(!info.has_keyframe);
        assert!(info.is_essential());
    }

    #[test]
    fn terminates_on_a_truncated_stream() {
        // Start code with no NAL header after it, and a lone trailing start
        // code. Both used to be ways to walk off the end or spin.
        assert!(extract_parameter_sets(&[0, 0, 0, 1]).is_empty());
        let mut data = stream(&[(33, 1)]);
        data.extend_from_slice(&[0, 0, 0, 1]);
        let types: Vec<u8> = nal_units(&data).map(|n| n.nal_type).collect();
        assert_eq!(types, vec![33]);
    }
}
