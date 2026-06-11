//! Ancillary Data Descriptor — ETSI EN 300 468 §6.2.3 (tag 0x6B, Table 15, PDF p. 55).
//!
//! Carried inside the PMT ES_info loop. Fixed 1-byte body: a bit-flag field
//! `ancillary_data_identifier` whose bits select which ancillary-data formats
//! are present (Table 16).

use super::descriptor_body;
use crate::error::{Error, Result};
use dvb_common::{Parse, Serialize};

/// Descriptor tag for ancillary_data_descriptor.
pub const TAG: u8 = 0x6B;
/// Length of the header (tag byte + length byte).
pub const HEADER_LEN: usize = 2;
/// Fixed body length: one identifier flag byte.
pub const BODY_LEN: usize = 1;

/// Table 16 bit positions (0-based from LSB): `b₁` = bit 0, `b₂` = bit 1, …
const DVD_VIDEO_AD: u8 = 1 << 0;
const EXTENDED_AD: u8 = 1 << 1;
const ANNOUNCEMENT_SWITCHING: u8 = 1 << 2;
const DAB_AD: u8 = 1 << 3;
const SCF_CRC: u8 = 1 << 4;
const MPEG4_AD: u8 = 1 << 5;
const RDS_UECP: u8 = 1 << 6;

/// Decoded ancillary data flags — ETSI EN 300 468 Table 16.
///
/// Bit numbering per the spec: `b₁` (LSB, transmitted last per §5.1.6)
/// through `b₈` (MSB).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AncillaryDataFlags {
    /// DVD Video Ancillary Data (`b₁` = bit 0).
    pub dvd_video_ad: bool,
    /// Extended Ancillary Data (`b₂` = bit 1).
    pub extended_ad: bool,
    /// Announcement Switching Data (`b₃` = bit 2).
    pub announcement_switching: bool,
    /// DAB Ancillary Data (`b₄` = bit 3).
    pub dab_ad: bool,
    /// Scale Factor Error Check (ScF-CRC) (`b₅` = bit 4).
    pub scf_crc: bool,
    /// MPEG-4 ancillary data (`b₆` = bit 5).
    pub mpeg4_ad: bool,
    /// RDS via UECP (`b₇` = bit 6).
    pub rds_uecp: bool,
}

/// Ancillary Data Descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct AncillaryDataDescriptor {
    /// 8-bit ancillary_data_identifier flag field (Table 16).
    pub ancillary_data_identifier: u8,
}

impl AncillaryDataDescriptor {
    /// Decodes the `ancillary_data_identifier` flag byte into named booleans
    /// per ETSI EN 300 468 Table 16.
    #[must_use]
    pub fn flags(&self) -> AncillaryDataFlags {
        let b = self.ancillary_data_identifier;
        AncillaryDataFlags {
            dvd_video_ad: (b & DVD_VIDEO_AD) != 0,
            extended_ad: (b & EXTENDED_AD) != 0,
            announcement_switching: (b & ANNOUNCEMENT_SWITCHING) != 0,
            dab_ad: (b & DAB_AD) != 0,
            scf_crc: (b & SCF_CRC) != 0,
            mpeg4_ad: (b & MPEG4_AD) != 0,
            rds_uecp: (b & RDS_UECP) != 0,
        }
    }
}

impl<'a> Parse<'a> for AncillaryDataDescriptor {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "AncillaryDataDescriptor",
            "unexpected tag for ancillary_data_descriptor",
        )?;
        if body.len() != BODY_LEN {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "ancillary_data_descriptor length must be exactly 1",
            });
        }
        Ok(Self {
            ancillary_data_identifier: body[0],
        })
    }
}

impl Serialize for AncillaryDataDescriptor {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + BODY_LEN
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        buf[0] = TAG;
        buf[1] = BODY_LEN as u8;
        buf[HEADER_LEN] = self.ancillary_data_identifier;
        Ok(len)
    }
}
impl<'a> crate::traits::DescriptorDef<'a> for AncillaryDataDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "ANCILLARY_DATA";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_extracts_identifier() {
        let bytes = [TAG, 1, 0x55];
        let d = AncillaryDataDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.ancillary_data_identifier, 0x55);
    }

    #[test]
    fn flags_decode_all_set() {
        // bits 0–6 set → 0b0111_1111 = 0x7F
        let d = AncillaryDataDescriptor {
            ancillary_data_identifier: 0x7F,
        };
        let f = d.flags();
        assert!(f.dvd_video_ad);
        assert!(f.extended_ad);
        assert!(f.announcement_switching);
        assert!(f.dab_ad);
        assert!(f.scf_crc);
        assert!(f.mpeg4_ad);
        assert!(f.rds_uecp);
    }

    #[test]
    fn flags_decode_none_set() {
        let d = AncillaryDataDescriptor {
            ancillary_data_identifier: 0x00,
        };
        let f = d.flags();
        assert!(!f.dvd_video_ad);
        assert!(!f.extended_ad);
        assert!(!f.announcement_switching);
        assert!(!f.dab_ad);
        assert!(!f.scf_crc);
        assert!(!f.mpeg4_ad);
        assert!(!f.rds_uecp);
    }

    #[test]
    fn flags_decode_extended_ad_only() {
        // bit 1 only → 0b0000_0010 = 0x02
        let d = AncillaryDataDescriptor {
            ancillary_data_identifier: 0x02,
        };
        let f = d.flags();
        assert!(!f.dvd_video_ad);
        assert!(f.extended_ad);
        assert!(!f.announcement_switching);
        assert!(!f.dab_ad);
        assert!(!f.scf_crc);
        assert!(!f.mpeg4_ad);
        assert!(!f.rds_uecp);
    }

    #[test]
    fn flags_decode_rds_uecp_only() {
        // bit 6 only → 0b0100_0000 = 0x40
        let d = AncillaryDataDescriptor {
            ancillary_data_identifier: 0x40,
        };
        let f = d.flags();
        assert!(!f.dvd_video_ad);
        assert!(!f.extended_ad);
        assert!(!f.announcement_switching);
        assert!(!f.dab_ad);
        assert!(!f.scf_crc);
        assert!(!f.mpeg4_ad);
        assert!(f.rds_uecp);
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        assert!(matches!(
            AncillaryDataDescriptor::parse(&[0x6C, 1, 0]).unwrap_err(),
            Error::InvalidDescriptor { tag: 0x6C, .. }
        ));
    }

    #[test]
    fn parse_rejects_wrong_length() {
        assert!(matches!(
            AncillaryDataDescriptor::parse(&[TAG, 2, 0, 0]).unwrap_err(),
            Error::InvalidDescriptor { tag: TAG, .. }
        ));
    }

    #[test]
    fn parse_rejects_short_body() {
        assert!(matches!(
            AncillaryDataDescriptor::parse(&[TAG, 1]).unwrap_err(),
            Error::BufferTooShort { .. }
        ));
    }

    #[test]
    fn serialize_round_trip() {
        let d = AncillaryDataDescriptor {
            ancillary_data_identifier: 0xA3,
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(buf, [TAG, 1, 0xA3]);
        assert_eq!(AncillaryDataDescriptor::parse(&buf).unwrap(), d);
    }

    #[test]
    fn serialize_rejects_too_small_buffer() {
        let d = AncillaryDataDescriptor {
            ancillary_data_identifier: 0,
        };
        let mut buf = vec![0u8; 2];
        assert!(matches!(
            d.serialize_into(&mut buf).unwrap_err(),
            Error::OutputBufferTooSmall { .. }
        ));
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_round_trip() {
        let d = AncillaryDataDescriptor {
            ancillary_data_identifier: 0xA3,
        };
        let json = serde_json::to_string(&d).unwrap();
        // Serialize-only: assert the emitted JSON re-parses (serialize-stable).
        let _v: serde_json::Value = serde_json::from_str(&json).unwrap();
    }
}
