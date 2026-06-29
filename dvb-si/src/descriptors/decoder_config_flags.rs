//! decoder_config_flags — ISO/IEC 13818-1 §2.6.61, Table 2-90.
//!
//! 3-bit field in the metadata_descriptor that indicates how decoder
//! configuration information is conveyed.

/// decoder_config_flags values (Table 2-90).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum DecoderConfigFlags {
    /// 0b000 — No decoder configuration needed.
    None,
    /// 0b001 — Carried in this descriptor (decoder_config_byte).
    InDescriptor,
    /// 0b010 — Carried in the same metadata service.
    SameService,
    /// 0b011 — Carried in a DSM-CC carousel (dec_config_identification_record).
    DsmccCarousel,
    /// 0b100 — Carried in another metadata service (decoder_config_metadata_service_id).
    OtherService,
    /// 0b101–0b110 — Reserved.
    Reserved(u8),
    /// 0b111 — Privately defined.
    Private,
}

impl DecoderConfigFlags {
    /// Construct from a raw 3-bit value.
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v & 0x07 {
            0 => Self::None,
            1 => Self::InDescriptor,
            2 => Self::SameService,
            3 => Self::DsmccCarousel,
            4 => Self::OtherService,
            5 | 6 => Self::Reserved(v & 0x07),
            _ => Self::Private,
        }
    }

    /// Return the raw 3-bit value.
    #[must_use]
    pub const fn to_u8(self) -> u8 {
        match self {
            Self::None => 0,
            Self::InDescriptor => 1,
            Self::SameService => 2,
            Self::DsmccCarousel => 3,
            Self::OtherService => 4,
            Self::Reserved(v) => v,
            Self::Private => 7,
        }
    }

    /// Returns a human-readable spec name for this value.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::InDescriptor => "in descriptor",
            Self::SameService => "same service",
            Self::DsmccCarousel => "DSM-CC carousel",
            Self::OtherService => "other service",
            Self::Reserved(_) => "reserved",
            Self::Private => "private",
        }
    }
}
broadcast_common::impl_spec_display!(DecoderConfigFlags, Reserved);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_values() {
        for v in 0u8..=7 {
            let cf = DecoderConfigFlags::from_u8(v);
            assert_eq!(cf.to_u8(), v & 0x07, "value {v} round-trip mismatch");
        }
    }
}
