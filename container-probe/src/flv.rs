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

/// The registered FLV prober: signature + header-field checks over `limit`.
pub(crate) fn probe(data: &[u8], limit: usize) -> Outcome {
    debug_assert!(limit <= data.len(), "harness caps limit at data.len()");
    let region = &data[..limit];

    if region.len() < FLV_HEADER_LEN || region[..FLV_SIGNATURE.len()] != FLV_SIGNATURE {
        return Outcome::None;
    }

    // Reserved bits in TypeFlags must be clear.
    let type_flags = region[4];
    if type_flags & TYPE_FLAG_RESERVED_MASK != 0 {
        return Outcome::None;
    }
    // DataOffset (bytes 5..9) is the header size; it must be >= the 9-byte
    // header (so the field itself is sane and points at/after the header).
    let data_offset = u32::from_be_bytes([region[5], region[6], region[7], region[8]]) as usize;
    if data_offset < FLV_HEADER_LEN {
        return Outcome::None;
    }

    Outcome::Match(Evidence {
        confidence: Confidence::STRONG,
        detail: Detail::None,
    })
}
