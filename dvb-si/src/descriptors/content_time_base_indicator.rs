//! Content_time_base_indicator — ISO/IEC 13818-1 §2.6.57, Table 2-85.
//!
//! 4-bit field in the content_labeling_descriptor that selects the
//! time-base semantics for the associated content.

/// Content_time_base_indicator values (Table 2-85).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum ContentTimeBaseIndicator {
    /// 0 — No content time base defined in this descriptor.
    None,
    /// 1 — STC (System Time Clock).
    Stc,
    /// 2 — NPT (Normal Play Time).
    Npt,
    /// 3–7 — Reserved.
    Reserved(u8),
    /// 8–15 — Privately defined.
    Private(u8),
}

impl ContentTimeBaseIndicator {
    /// Construct from a raw 4-bit nibble.
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v {
            0 => Self::None,
            1 => Self::Stc,
            2 => Self::Npt,
            3..=7 => Self::Reserved(v),
            v => Self::Private(v),
        }
    }

    /// Return the raw 4-bit value.
    #[must_use]
    pub const fn to_u8(self) -> u8 {
        match self {
            Self::None => 0,
            Self::Stc => 1,
            Self::Npt => 2,
            Self::Reserved(v) | Self::Private(v) => v,
        }
    }

    /// Returns a human-readable spec name for this value.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Stc => "STC",
            Self::Npt => "NPT",
            Self::Reserved(_) => "reserved",
            Self::Private(_) => "private",
        }
    }
}
broadcast_common::impl_spec_display!(ContentTimeBaseIndicator, Reserved, Private);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_values() {
        for v in 0u8..=15 {
            let indicator = ContentTimeBaseIndicator::from_u8(v);
            assert_eq!(indicator.to_u8(), v, "value {v} round-trip mismatch");
        }
    }
}
