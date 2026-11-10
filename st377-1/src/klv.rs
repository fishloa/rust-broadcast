//! KLV (Key-Length-Value) triplet as used by MXF — SMPTE ST 377-1:2019 §6.3
//! (`private/specs-verified/smpte/st377-1/st377-1.md`), the base framing
//! primitive every other KLV item in an MXF file rides on.
//!
//! The triplet and the walker are shared with every other KLV user in the
//! workspace and live in [`broadcast_common::klv`]; this module re-exports them
//! so the `st377_1::…` paths an MXF consumer already uses keep resolving, and
//! adds the Fill-item key test, which is MXF-specific.

pub use broadcast_common::klv::{KlvItem, collect_klv_items, walk_klv_items};

use crate::types::UlBytes;

/// The KLV Fill item key (§6.3.3), matching byte 8 (the version number) as
/// a wildcard per the spec's own note ("MXF decoders shall ignore the
/// version number byte ... when determining if a KLV key is the Fill item
/// key" — some early encoders wrote `0x01` there instead of the RP 210
/// value `0x02`).
pub const FILL_ITEM_KEY_PREFIX: [u8; 7] = [0x06, 0x0E, 0x2B, 0x34, 0x01, 0x01, 0x01];
/// Byte 8 (version) is a wildcard; bytes 9-16 of the Fill item key.
pub const FILL_ITEM_KEY_SUFFIX: [u8; 8] = [0x03, 0x01, 0x02, 0x10, 0x01, 0x00, 0x00, 0x00];

/// True if `key` is the KLV Fill item key (§6.3.3), ignoring byte 8 (the
/// version number) per the spec's own decoder rule.
#[must_use]
pub fn is_fill_item_key(key: &UlBytes) -> bool {
    key[..7] == FILL_ITEM_KEY_PREFIX && key[8..] == FILL_ITEM_KEY_SUFFIX
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fill_item_key_matches_ignoring_version_byte() {
        let rp210 = [
            0x06, 0x0E, 0x2B, 0x34, 0x01, 0x01, 0x01, 0x02, 0x03, 0x01, 0x02, 0x10, 0x01, 0x00,
            0x00, 0x00,
        ];
        let legacy = [
            0x06, 0x0E, 0x2B, 0x34, 0x01, 0x01, 0x01, 0x01, 0x03, 0x01, 0x02, 0x10, 0x01, 0x00,
            0x00, 0x00,
        ];
        assert!(is_fill_item_key(&rp210));
        assert!(is_fill_item_key(&legacy));
        assert!(!is_fill_item_key(&[0u8; 16]));
    }
}
