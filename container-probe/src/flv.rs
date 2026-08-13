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

    if region.len() < FLV_HEADER_LEN {
        // Shorter than the 9-byte header the field checks need -> undecided.
        return Outcome::Insufficient(FLV_HEADER_LEN);
    }
    if region[..FLV_SIGNATURE.len()] != FLV_SIGNATURE {
        return Outcome::None;
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
        std::fs::read(std::format!("{}/../{}", env!("CARGO_MANIFEST_DIR"), rel))
            .unwrap_or_else(|e| panic!("failed to read {rel}: {e}"))
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
}
