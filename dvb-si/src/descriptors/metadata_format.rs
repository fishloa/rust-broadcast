//! Metadata_format — ISO/IEC 13818-1 §2.6.59, Table 2-87.
//!
//! 8-bit field that identifies the metadata encoding format.

/// Metadata_format values (Table 2-87).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum MetadataFormat {
    /// 0x00–0x0F — Reserved.
    Reserved0(u8),
    /// 0x10 — ISO/IEC 15938-1 TeM.
    TeM,
    /// 0x11 — ISO/IEC 15938-1 BiM.
    BiM,
    /// 0x12–0x3E — Reserved.
    Reserved1(u8),
    /// 0x3F — Defined by metadata application format.
    AppFormat,
    /// 0x40–0xFE — Private use.
    Private(u8),
    /// 0xFF — Defined by metadata_format_identifier field.
    Identifier,
}

impl MetadataFormat {
    /// Construct from a raw byte.
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v {
            0x00..=0x0F => Self::Reserved0(v),
            0x10 => Self::TeM,
            0x11 => Self::BiM,
            0x12..=0x3E => Self::Reserved1(v),
            0x3F => Self::AppFormat,
            0x40..=0xFE => Self::Private(v),
            0xFF => Self::Identifier,
        }
    }

    /// Return the raw byte value.
    #[must_use]
    pub fn to_u8(self) -> u8 {
        match self {
            Self::Reserved0(v) => v,
            Self::TeM => 0x10,
            Self::BiM => 0x11,
            Self::Reserved1(v) => v,
            Self::AppFormat => 0x3F,
            Self::Private(v) => v,
            Self::Identifier => 0xFF,
        }
    }

    /// Returns a human-readable spec name for this value.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Reserved0(_) => "reserved",
            Self::TeM => "TeM",
            Self::BiM => "BiM",
            Self::Reserved1(_) => "reserved",
            Self::AppFormat => "app format",
            Self::Private(_) => "private",
            Self::Identifier => "identifier",
        }
    }
}
dvb_common::impl_spec_display!(MetadataFormat, Reserved0, Reserved1, Private);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_values() {
        let values = [0x00u8, 0x05, 0x10, 0x11, 0x20, 0x3F, 0x80, 0xFE, 0xFF];
        for v in values {
            let mf = MetadataFormat::from_u8(v);
            assert_eq!(mf.to_u8(), v, "value 0x{v:02X} round-trip mismatch");
        }
    }
}
