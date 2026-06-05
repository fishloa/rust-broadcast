//! PDC Descriptor — ETSI EN 300 468 §6.2.30 (tag 0x69, Table 84, PDF p. 97).
//!
//! Programme Delivery Control. Carried inside EIT. Fixed 3-byte body:
//! `reserved_future_use` (4 bits) + `programme_identification_label` (20 bits).
//! The PIL encodes a "day month hour minute" stamp used by VCRs to trigger
//! recording independent of schedule slippage.

use crate::error::{Error, Result};
use crate::traits::Descriptor;
use dvb_common::{Parse, Serialize};

/// Descriptor tag for PDC_descriptor.
pub const TAG: u8 = 0x69;
/// Length of the header (tag byte + length byte).
pub const HEADER_LEN: usize = 2;
/// Fixed body length: 4 reserved bits + 20-bit PIL = 24 bits = 3 bytes.
pub const BODY_LEN: usize = 3;
/// The programme_identification_label occupies the low 20 bits.
pub const PIL_MASK: u32 = 0x000F_FFFF;
/// Reserved bits occupy the top 4 of the 24-bit body. Ignored on parse,
/// emitted as 1s on serialize (EN 300 468 §5.1).
pub const RESERVED_BITS: u32 = 0x00F0_0000;

/// PDC Descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct PdcDescriptor {
    /// 20-bit programme_identification_label (day/month/hour/minute packed).
    pub programme_identification_label: u32,
}

impl<'a> Parse<'a> for PdcDescriptor {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < HEADER_LEN {
            return Err(Error::BufferTooShort {
                need: HEADER_LEN,
                have: bytes.len(),
                what: "PdcDescriptor header",
            });
        }
        if bytes[0] != TAG {
            return Err(Error::InvalidDescriptor {
                tag: bytes[0],
                reason: "unexpected tag for PDC_descriptor",
            });
        }
        let length = bytes[1] as usize;
        if length != BODY_LEN {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "PDC_descriptor length must be exactly 3",
            });
        }
        let end = HEADER_LEN + length;
        if bytes.len() < end {
            return Err(Error::BufferTooShort {
                need: end,
                have: bytes.len(),
                what: "PdcDescriptor body",
            });
        }
        // 24-bit big-endian field; top 4 bits reserved (ignored on parse).
        let raw = (u32::from(bytes[HEADER_LEN]) << 16)
            | (u32::from(bytes[HEADER_LEN + 1]) << 8)
            | u32::from(bytes[HEADER_LEN + 2]);
        Ok(Self {
            programme_identification_label: raw & PIL_MASK,
        })
    }
}

impl Serialize for PdcDescriptor {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + BODY_LEN
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        if self.programme_identification_label > PIL_MASK {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "programme_identification_label exceeds 20 bits",
            });
        }
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        buf[0] = TAG;
        buf[1] = BODY_LEN as u8;
        // Reserved 4 bits emitted as 1s.
        let raw = RESERVED_BITS | (self.programme_identification_label & PIL_MASK);
        buf[HEADER_LEN] = (raw >> 16) as u8;
        buf[HEADER_LEN + 1] = (raw >> 8) as u8;
        buf[HEADER_LEN + 2] = raw as u8;
        Ok(len)
    }
}

impl<'a> Descriptor<'a> for PdcDescriptor {
    const TAG: u8 = TAG;
    fn descriptor_length(&self) -> u8 {
        BODY_LEN as u8
    }
}

impl<'a> crate::traits::DescriptorDef<'a> for PdcDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "PDC";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_extracts_pil() {
        // body 0x0A_BCDE: top nibble reserved (0), PIL = 0x0ABCDE.
        let bytes = [TAG, 3, 0x0A, 0xBC, 0xDE];
        let d = PdcDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.programme_identification_label, 0x0A_BCDE);
    }

    #[test]
    fn parse_ignores_reserved_bits() {
        // Top nibble set (reserved) must be masked off, not rejected (§5.1).
        let bytes = [TAG, 3, 0xFA, 0xBC, 0xDE];
        let d = PdcDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.programme_identification_label, 0x0A_BCDE);
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        assert!(matches!(
            PdcDescriptor::parse(&[0x6A, 3, 0, 0, 0]).unwrap_err(),
            Error::InvalidDescriptor { tag: 0x6A, .. }
        ));
    }

    #[test]
    fn parse_rejects_wrong_length() {
        assert!(matches!(
            PdcDescriptor::parse(&[TAG, 2, 0, 0]).unwrap_err(),
            Error::InvalidDescriptor { tag: TAG, .. }
        ));
    }

    #[test]
    fn parse_rejects_short_body() {
        assert!(matches!(
            PdcDescriptor::parse(&[TAG, 3, 0, 0]).unwrap_err(),
            Error::BufferTooShort { .. }
        ));
    }

    #[test]
    fn serialize_round_trip() {
        let d = PdcDescriptor {
            programme_identification_label: 0x0A_BCDE,
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        // Reserved nibble emitted as 1s.
        assert_eq!(buf, [TAG, 3, 0xFA, 0xBC, 0xDE]);
        assert_eq!(PdcDescriptor::parse(&buf).unwrap(), d);
    }

    #[test]
    fn serialize_rejects_too_small_buffer() {
        let d = PdcDescriptor {
            programme_identification_label: 0,
        };
        let mut buf = vec![0u8; 2];
        assert!(matches!(
            d.serialize_into(&mut buf).unwrap_err(),
            Error::OutputBufferTooSmall { .. }
        ));
    }

    #[test]
    fn serialize_rejects_over_range_pil() {
        let d = PdcDescriptor {
            programme_identification_label: 0x10_0000, // 21 bits
        };
        let mut buf = vec![0u8; d.serialized_len()];
        assert!(matches!(
            d.serialize_into(&mut buf).unwrap_err(),
            Error::InvalidDescriptor { tag: TAG, .. }
        ));
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_round_trip() {
        let d = PdcDescriptor {
            programme_identification_label: 0x0A_BCDE,
        };
        let json = serde_json::to_string(&d).unwrap();
        // Serialize-only: assert the emitted JSON re-parses (serialize-stable).
        let _v: serde_json::Value = serde_json::from_str(&json).unwrap();
    }
}
