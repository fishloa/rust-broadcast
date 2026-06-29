//! MPEG_carriage_flags — ISO/IEC 13818-1 §2.6.59, Table 2-88.
//!
//! 2-bit field in the metadata_pointer_descriptor that indicates where
//! the referenced metadata service is carried.

/// MPEG_carriage_flags values (Table 2-88).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum MpegCarriageFlags {
    /// 0 — Same transport stream.
    SameTs,
    /// 1 — Different transport stream.
    DifferentTs,
    /// 2 — Program stream.
    ProgramStream,
    /// 3 — None of the above.
    None,
}

impl MpegCarriageFlags {
    /// Construct from a raw 2-bit value.
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v & 0x03 {
            0 => Self::SameTs,
            1 => Self::DifferentTs,
            2 => Self::ProgramStream,
            _ => Self::None,
        }
    }

    /// Return the raw 2-bit value.
    #[must_use]
    pub const fn to_u8(self) -> u8 {
        match self {
            Self::SameTs => 0,
            Self::DifferentTs => 1,
            Self::ProgramStream => 2,
            Self::None => 3,
        }
    }

    /// Returns a human-readable spec name for this value.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::SameTs => "same TS",
            Self::DifferentTs => "different TS",
            Self::ProgramStream => "program stream",
            Self::None => "none",
        }
    }
}
broadcast_common::impl_spec_display!(MpegCarriageFlags);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_values() {
        for v in 0u8..=3 {
            let cf = MpegCarriageFlags::from_u8(v);
            assert_eq!(cf.to_u8(), v, "value {v} round-trip mismatch");
        }
    }
}
