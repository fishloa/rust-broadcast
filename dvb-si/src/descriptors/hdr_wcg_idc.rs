//! HDR_WCG_idc — ISO/IEC 13818-1 §2.6.96, Table 2-114.
//!
//! Indicates the presence and type of HDR/WCG signalling in the stream.

/// HDR_WCG_idc field values (Table 2-114).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum HdrWcgIdc {
    /// 0 — SDR: BT.1886 reference EOTF, colour gamut within BT.709-6.
    Sdr,
    /// 1 — WCG only: colour gamut in a BT.2020 container that exceeds BT.709-6.
    WcgOnly,
    /// 2 — Both HDR and WCG indicated in the stream.
    HdrAndWcg,
    /// 3 — No indication regarding HDR/WCG or SDR characteristics.
    NoIndication,
}

impl HdrWcgIdc {
    /// Construct from a raw 2-bit value (0..=3).
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v & 0x03 {
            0 => Self::Sdr,
            1 => Self::WcgOnly,
            2 => Self::HdrAndWcg,
            _ => Self::NoIndication,
        }
    }

    /// Return the raw 2-bit value.
    #[must_use]
    pub const fn to_u8(self) -> u8 {
        match self {
            Self::Sdr => 0,
            Self::WcgOnly => 1,
            Self::HdrAndWcg => 2,
            Self::NoIndication => 3,
        }
    }

    /// Returns a human-readable spec name for this value.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Sdr => "SDR",
            Self::WcgOnly => "WCG only",
            Self::HdrAndWcg => "HDR and WCG",
            Self::NoIndication => "no indication",
        }
    }
}
dvb_common::impl_spec_display!(HdrWcgIdc);
