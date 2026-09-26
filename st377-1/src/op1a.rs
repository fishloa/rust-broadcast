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
//! Byte 15 qualifier bits (`docs/st378-op1a.md` §6.4, Table "Bit"/"§6.4"/
//! "Meaning when set to 0" — 1-indexed bit *positions*, i.e. "bit 1" names
//! value `0x02`, not `0x01`; bit 0 (`0x01`) is a marker every real encoder
//! sets and is always written here):
//! - bit 0 (`0x01`): marker, always set (not a semantic flag)
//! - bit 1 (`0x02`): external essence (0 = internal, default)
//! - bit 2 (`0x04`): non-streamable (0 = streamable, default)
//! - bit 3 (`0x08`): multi-track (0 = single-track, default)
//!
//! Verified against the crate's own real `ffmpeg`-muxed fixture
//! (`tests/fixtures/op1a_mpeg2_pcm.mxf`, one interleaved MPEG-2 video + PCM
//! audio Essence Container): its qualifier byte is `0x09` (marker +
//! multi-track), matching a file whose essence is internal, streamable, and
//! carries more than one track (issue #1048).

use crate::types::UlBytes;

/// Bytes 1-14 of the OP1a UL (everything except the qualifier byte 15
/// and the reserved byte 16).
pub const OP1A_UL_PREFIX: [u8; 14] = [
    0x06, 0x0E, 0x2B, 0x34, 0x04, 0x01, 0x01, 0x01, 0x0D, 0x01, 0x02, 0x01, 0x01, 0x01,
];

/// Bit 0: marker every real encoder sets; not a semantic flag (§6.4).
const MARKER_BIT: u8 = 0x01;
/// Bit 1: external essence.
const EXTERNAL_ESSENCE_BIT: u8 = 0x02;
/// Bit 2: non-streamable.
const NON_STREAMABLE_BIT: u8 = 0x04;
/// Bit 3: multi-track.
const MULTI_TRACK_BIT: u8 = 0x08;

/// Qualifier bit flags for the OP1a UL's byte 15.
///
/// Default (all bits clear except the always-set marker) = internal
/// essence, streamable, single track — the most common case.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Op1aQualifier(u8);

impl Default for Op1aQualifier {
    fn default() -> Self {
        Op1aQualifier(MARKER_BIT)
    }
}

impl Op1aQualifier {
    /// Bit 1 (`0x02`): true if essence is stored external to the MXF file.
    #[must_use]
    pub fn external_essence(self) -> bool {
        self.0 & EXTERNAL_ESSENCE_BIT != 0
    }

    /// Bit 2 (`0x04`): true if the file is not streamable (requires random
    /// access to play).
    #[must_use]
    pub fn non_streamable(self) -> bool {
        self.0 & NON_STREAMABLE_BIT != 0
    }

    /// Bit 3 (`0x08`): true if the Essence Container has more than one
    /// essence track.
    #[must_use]
    pub fn multi_track(self) -> bool {
        self.0 & MULTI_TRACK_BIT != 0
    }

    /// Set the external-essence bit (bit 1).
    #[must_use]
    pub fn with_external_essence(mut self) -> Self {
        self.0 |= EXTERNAL_ESSENCE_BIT;
        self
    }

    /// Set the non-streamable bit (bit 2).
    #[must_use]
    pub fn with_non_streamable(mut self) -> Self {
        self.0 |= NON_STREAMABLE_BIT;
        self
    }

    /// Set the multi-track bit (bit 3).
    #[must_use]
    pub fn with_multi_track(mut self) -> Self {
        self.0 |= MULTI_TRACK_BIT;
        self
    }

    /// The raw qualifier byte value. Always carries the marker bit
    /// (`0x01`), which every real encoder sets (§6.4) — set unconditionally
    /// here rather than left to the caller.
    #[must_use]
    pub fn to_byte(self) -> u8 {
        self.0 | MARKER_BIT
    }

    /// Build from a raw qualifier byte (as found on the wire — the marker
    /// bit is whatever the byte actually carries, not forced).
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
        // marker (0x01) + external (0x02) + multi-track (0x08).
        assert_eq!(q.to_byte(), 0x0B);
    }

    #[test]
    fn is_op1a_false_for_other_ops() {
        let mut ul = op1a_ul(Op1aQualifier::default());
        ul[13] = 0x02; // change to something else
        assert!(!is_op1a(&ul));
    }

    #[test]
    fn round_trip_qualifier() {
        // `to_byte()`/`op1a_ul` always force the marker bit (0x01) on, so a
        // fresh build's byte 15 is the input OR'd with it, regardless of
        // whether the input already carried it.
        for byte in 0..=0x0F {
            let q = Op1aQualifier::from_byte(byte);
            let ul = op1a_ul(q);
            assert!(is_op1a(&ul));
            assert_eq!(ul[14], byte | MARKER_BIT);
        }
    }

    /// Oracle case: the real fixture's own qualifier byte (`0x09` — marker +
    /// multi-track only) must decode to internal/streamable/multi-track and
    /// round-trip byte-identically through `from_byte`/`to_byte` (issue
    /// #1048).
    #[test]
    fn real_fixture_qualifier_byte_decodes_correctly() {
        let q = Op1aQualifier::from_byte(0x09);
        assert!(!q.external_essence());
        assert!(!q.non_streamable());
        assert!(q.multi_track());
        assert_eq!(q.to_byte(), 0x09);
    }
}
