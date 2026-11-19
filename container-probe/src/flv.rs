//! FLV prober — Adobe Flash Video File Format Specification v10.1, Annex E
//! (transaction in `private/specs/adobe_flv_f4v_v10_1.pdf`; parse follows
//! `transmux/src/flv.rs`, which round-trips real fixtures).
//!
//! `"FLV"` at offset 0, a version byte, a `TypeFlags` byte whose reserved bits
//! must be clear (only bit 0 video / bit 2 audio are defined), and a
//! `DataOffset` header-size field that must be `>= 9` and point within the
//! region. Magic plus these structural header checks score `STRONG`.

use crate::{Confidence, Detail, Evidence, Outcome};

/// The FLV signature, `"FLV"` (Adobe FLV v10.1 §E.2).
const FLV_SIGNATURE: [u8; 3] = *b"FLV";
/// Minimum FLV header length in bytes: signature(3) + version(1) +
/// `TypeFlags`(1) + `DataOffset`(4) = 9 (§E.2).
const FLV_HEADER_LEN: usize = 9;
/// `TypeFlags` bit 2 — audio tags present (§E.2).
const TYPE_FLAG_AUDIO: u8 = 0x04;
/// `TypeFlags` bit 0 — video tags present (§E.2).
const TYPE_FLAG_VIDEO: u8 = 0x01;
/// Mask of the two defined `TypeFlags` bits; any other set bit is reserved and
/// must be zero in a conformant header.
const TYPE_FLAG_RESERVED_MASK: u8 = !(TYPE_FLAG_AUDIO | TYPE_FLAG_VIDEO);

/// `TypeFlags` is the byte immediately after the signature + version.
const TYPE_FLAGS_OFFSET: usize = 4;
/// `DataOffset` is the 4-byte big-endian field starting at byte 5.
const DATA_OFFSET_POS: usize = 5;

/// The registered FLV prober: signature + header-field checks over `limit`.
pub(crate) fn probe(data: &[u8], limit: usize) -> Outcome {
    debug_assert!(limit <= data.len(), "harness caps limit at data.len()");
    let region = &data[..limit];

    if let Some(outcome) = crate::signature_gate(region, &FLV_SIGNATURE, FLV_HEADER_LEN) {
        return outcome;
    }

    // Reserved bits in TypeFlags must be clear.
    let type_flags = region[TYPE_FLAGS_OFFSET];
    if type_flags & TYPE_FLAG_RESERVED_MASK != 0 {
        return Outcome::None;
    }
    // DataOffset (bytes 5..9) is the header size; it must be >= the 9-byte
    // header (so the field itself is sane and points at/after the header).
    let data_offset = u32::from_be_bytes([
        region[DATA_OFFSET_POS],
        region[DATA_OFFSET_POS + 1],
        region[DATA_OFFSET_POS + 2],
        region[DATA_OFFSET_POS + 3],
    ]) as usize;
    if data_offset < FLV_HEADER_LEN {
        return Outcome::None;
    }

    Outcome::Match(Evidence {
        confidence: Confidence::STRONG,
        detail: Detail::Flv {
            has_audio: type_flags & TYPE_FLAG_AUDIO != 0,
            has_video: type_flags & TYPE_FLAG_VIDEO != 0,
            data_offset: data_offset as u32,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_bytes(rel: &str) -> std::vec::Vec<u8> {
        test_fixtures::read(rel.trim_start_matches("fixtures/"))
    }

    /// Finding 4: an 8-byte prefix of a real FLV fixture (1 byte short of the
    /// 9-byte header) is `Insufficient`, not `Unknown`.
    #[test]
    fn short_prefix_is_insufficient() {
        let data = fixture_bytes("fixtures/flv/av.flv");
        let region = &data[..FLV_HEADER_LEN - 1];
        match probe(region, region.len()) {
            Outcome::Insufficient(need) => assert_eq!(need, FLV_HEADER_LEN),
            other => panic!("8-byte FLV prefix must be Insufficient(9), got {other:?}"),
        }
    }

    /// An FLV header with the given `TypeFlags` and big-endian `DataOffset`.
    fn header(flags: u8, data_offset: [u8; 4]) -> [u8; FLV_HEADER_LEN] {
        [
            b'F',
            b'L',
            b'V',
            0x01,
            flags,
            data_offset[0],
            data_offset[1],
            data_offset[2],
            data_offset[3],
        ]
    }

    /// The reported `(has_audio, has_video, data_offset)` from an FLV match.
    fn detail(outcome: Outcome) -> (bool, bool, u32) {
        match outcome {
            Outcome::Match(Evidence {
                detail:
                    Detail::Flv {
                        has_audio,
                        has_video,
                        data_offset,
                    },
                ..
            }) => (has_audio, has_video, data_offset),
            other => panic!("expected an FLV Detail match, got {other:?}"),
        }
    }

    /// `TypeFlags` bit 0 is video and bit 2 is audio (FLV v10.1 §E.2), each
    /// read independently of the other.
    #[test]
    fn type_flags_decode_audio_and_video_independently() {
        for (flags, has_audio, has_video) in [
            (0x00u8, false, false),
            (TYPE_FLAG_VIDEO, false, true),
            (TYPE_FLAG_AUDIO, true, false),
            (TYPE_FLAG_AUDIO | TYPE_FLAG_VIDEO, true, true),
        ] {
            let data = header(flags, [0x00, 0x00, 0x00, 0x09]);
            assert_eq!(
                detail(probe(&data, data.len())),
                (has_audio, has_video, 9),
                "TypeFlags {flags:#04x}"
            );
        }
    }

    /// `DataOffset` is the 4-byte big-endian header size (FLV v10.1 §E.2).
    #[test]
    fn data_offset_is_the_big_endian_header_size() {
        let data = header(TYPE_FLAG_VIDEO, [0x00, 0x00, 0x01, 0x00]);
        assert_eq!(detail(probe(&data, data.len())), (false, true, 256));
        let data = header(TYPE_FLAG_VIDEO, [0x01, 0x02, 0x03, 0x04]);
        assert_eq!(detail(probe(&data, data.len())), (false, true, 0x0102_0304));
    }

    /// A `DataOffset` smaller than the 9-byte header cannot point at/after the
    /// header and is rejected.
    #[test]
    fn data_offset_below_the_header_length_is_rejected() {
        let data = header(TYPE_FLAG_VIDEO, [0x00, 0x00, 0x00, 0x08]);
        assert!(
            matches!(probe(&data, data.len()), Outcome::None),
            "DataOffset 8 is shorter than the 9-byte header"
        );
        let data = header(TYPE_FLAG_VIDEO, [0x00, 0x00, 0x00, 0x09]);
        assert!(matches!(probe(&data, data.len()), Outcome::Match(_)));
    }

    /// Only bit 0 and bit 2 are defined in `TypeFlags`; a reserved bit set is
    /// not a conformant header (FLV v10.1 §E.2).
    #[test]
    fn reserved_type_flag_bits_are_rejected() {
        let data = header(0x02, [0x00, 0x00, 0x00, 0x09]);
        assert!(
            matches!(probe(&data, data.len()), Outcome::None),
            "reserved TypeFlags bit 1 must not be accepted"
        );
    }
}
