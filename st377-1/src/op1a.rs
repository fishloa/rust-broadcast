//! OP1a Operational Pattern — SMPTE ST 378:2004 / ST 377-1:2019 §A.2
//! (`docs/st377-1.md`): identification helpers for the "single item,
//! single package" operational pattern that nearly all real MXF files
//! use.
//!
//! The OP1a Universal Label is 16 bytes (SMPTE-RP 224 registered):
//!
//! | Bytes  | Value          | Meaning               |
//! |--------|----------------|-----------------------|
//! | 1-4    | `06.0E.2B.34`  | SMPTE UL prefix       |
//! | 5-8    | `04.01.01.01`  | Registry: Labels      |
//! | 9-10   | `0D.01`        | Organization: AAF     |
//! | 11-12  | `02.01`        | Application: MXF OPs  |
//! | 13-14  | `01.01`        | OP1a base bytes       |
//! | 15     | qualifier      | bitfield (see below)  |
//! | 16     | `0x00`         | reserved              |
//!
//! Byte 15 qualifier bits:
//! - bit 0: external essence (0 = internal, default)
//! - bit 1: non-streamable (0 = streamable, default)
//! - bit 2: multi-track (0 = single-track, default)

use crate::types::UlBytes;

/// Bytes 1-14 of the OP1a UL (everything except the qualifier byte 15
/// and the reserved byte 16).
pub const OP1A_UL_PREFIX: [u8; 14] = [
    0x06, 0x0E, 0x2B, 0x34, 0x04, 0x01, 0x01, 0x01, 0x0D, 0x01, 0x02, 0x01, 0x01, 0x01,
];

/// Qualifier bit flags for the OP1a UL's byte 15.
///
/// Default (all bits clear) = internal essence, streamable, single
/// track — the most common case.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Op1aQualifier(u8);

impl Op1aQualifier {
    /// Bit 0: true if essence is stored external to the MXF file.
    #[must_use]
    pub fn external_essence(self) -> bool {
        self.0 & 0x01 != 0
    }

    /// Bit 1: true if the file is not streamable (requires random
    /// access to play).
    #[must_use]
    pub fn non_streamable(self) -> bool {
        self.0 & 0x02 != 0
    }

    /// Bit 2: true if the Material Package has more than one Track.
    #[must_use]
    pub fn multi_track(self) -> bool {
        self.0 & 0x04 != 0
    }

    /// Set the external-essence bit.
    #[must_use]
    pub fn with_external_essence(mut self) -> Self {
        self.0 |= 0x01;
        self
    }

    /// Set the non-streamable bit.
    #[must_use]
    pub fn with_non_streamable(mut self) -> Self {
        self.0 |= 0x02;
        self
    }

    /// Set the multi-track bit.
    #[must_use]
    pub fn with_multi_track(mut self) -> Self {
        self.0 |= 0x04;
        self
    }

    /// The raw qualifier byte value.
    #[must_use]
    pub fn to_byte(self) -> u8 {
        self.0
    }

    /// Build from a raw qualifier byte.
    #[must_use]
    pub fn from_byte(b: u8) -> Self {
        Op1aQualifier(b)
    }
}

/// True if `operational_pattern` is an OP1a UL (bytes 1-14 match
/// [`OP1A_UL_PREFIX`], byte 16 ignored).
#[must_use]
pub fn is_op1a(operational_pattern: &UlBytes) -> bool {
    operational_pattern[..14] == OP1A_UL_PREFIX
}

/// Build a complete 16-byte OP1a UL with the given qualifier flags.
#[must_use]
pub fn op1a_ul(qualifier: Op1aQualifier) -> UlBytes {
    let mut ul = [0u8; 16];
    ul[..14].copy_from_slice(&OP1A_UL_PREFIX);
    ul[14] = qualifier.to_byte();
    // byte 15 (index 15) is reserved, left as 0x00.
    ul
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_op1a_ul() {
        let ul = op1a_ul(Op1aQualifier::default());
        assert!(is_op1a(&ul));
        let q = Op1aQualifier::from_byte(ul[14]);
        assert!(!q.external_essence());
        assert!(!q.non_streamable());
        assert!(!q.multi_track());
    }

    #[test]
    fn qualifier_bits() {
        let q = Op1aQualifier::default()
            .with_external_essence()
            .with_multi_track();
        assert!(q.external_essence());
        assert!(!q.non_streamable());
        assert!(q.multi_track());
        assert_eq!(q.to_byte(), 0x05);
    }

    #[test]
    fn is_op1a_false_for_other_ops() {
        let mut ul = op1a_ul(Op1aQualifier::default());
        ul[13] = 0x02; // change to something else
        assert!(!is_op1a(&ul));
    }

    #[test]
    fn round_trip_qualifier() {
        for byte in 0..=0x07 {
            let q = Op1aQualifier::from_byte(byte);
            let ul = op1a_ul(q);
            assert!(is_op1a(&ul));
            assert_eq!(ul[14], byte);
        }
    }
}
