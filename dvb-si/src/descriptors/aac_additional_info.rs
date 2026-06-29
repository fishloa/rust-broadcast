//! MPEG-2_AAC_additional_information — ISO/IEC 13818-1 §2.6.69, Table 2-95.
//!
//! Indicates whether and how AAC data with Bandwidth Extension is present.

/// MPEG-2_AAC_additional_information field values (Table 2-95).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum AacAdditionalInfo {
    /// 0x00 — AAC data according to ISO/IEC 13818-7:2006.
    AacData,
    /// 0x01 — AAC data with Bandwidth Extension data present.
    AacWithBandwidthExtension,
    /// 0x02..0xFF — Reserved.
    Reserved(u8),
}

impl AacAdditionalInfo {
    /// Construct from a raw byte; unknown values preserved for round-trip.
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v {
            0x00 => Self::AacData,
            0x01 => Self::AacWithBandwidthExtension,
            v => Self::Reserved(v),
        }
    }

    /// Return the raw byte value.
    #[must_use]
    pub const fn to_u8(self) -> u8 {
        match self {
            Self::AacData => 0x00,
            Self::AacWithBandwidthExtension => 0x01,
            Self::Reserved(v) => v,
        }
    }

    /// Returns a human-readable spec name for this value.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::AacData => "AAC data",
            Self::AacWithBandwidthExtension => "AAC with bandwidth extension",
            Self::Reserved(_) => "reserved",
        }
    }
}
broadcast_common::impl_spec_display!(AacAdditionalInfo, Reserved);
