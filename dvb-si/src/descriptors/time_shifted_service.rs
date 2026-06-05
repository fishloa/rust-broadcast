//! Time Shifted Service Descriptor — ETSI EN 300 468 §6.2.45 (tag 0x4C).
//!
//! Table 104 (PDF p. 109). Carried in the SDT for an NVOD time-shifted
//! service; points at the reference service whose schedule it shifts. Body is
//! a single 16-bit `reference_service_id`.

use crate::error::{Error, Result};
use crate::traits::Descriptor;
use dvb_common::{Parse, Serialize};

/// Descriptor tag for time_shifted_service_descriptor.
pub const TAG: u8 = 0x4C;
const HEADER_LEN: usize = 2;
const BODY_LEN: usize = 2;

/// Time Shifted Service Descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct TimeShiftedServiceDescriptor {
    /// service_id of the reference (NVOD reference) service.
    pub reference_service_id: u16,
}

impl<'a> Parse<'a> for TimeShiftedServiceDescriptor {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < HEADER_LEN {
            return Err(Error::BufferTooShort {
                need: HEADER_LEN,
                have: bytes.len(),
                what: "TimeShiftedServiceDescriptor header",
            });
        }
        if bytes[0] != TAG {
            return Err(Error::InvalidDescriptor {
                tag: bytes[0],
                reason: "unexpected tag for time_shifted_service_descriptor",
            });
        }
        let length = bytes[1] as usize;
        if length != BODY_LEN {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "time_shifted_service_descriptor length must be 2",
            });
        }
        let end = HEADER_LEN + BODY_LEN;
        if bytes.len() < end {
            return Err(Error::BufferTooShort {
                need: end,
                have: bytes.len(),
                what: "TimeShiftedServiceDescriptor body",
            });
        }
        Ok(Self {
            reference_service_id: u16::from_be_bytes([bytes[2], bytes[3]]),
        })
    }
}

impl Serialize for TimeShiftedServiceDescriptor {
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
        buf[2..4].copy_from_slice(&self.reference_service_id.to_be_bytes());
        Ok(len)
    }
}

impl<'a> Descriptor<'a> for TimeShiftedServiceDescriptor {
    const TAG: u8 = TAG;
    fn descriptor_length(&self) -> u8 {
        BODY_LEN as u8
    }
}

impl<'a> crate::traits::DescriptorDef<'a> for TimeShiftedServiceDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "TIME_SHIFTED_SERVICE";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_reference_service_id() {
        let bytes = [TAG, 2, 0x12, 0x34];
        let d = TimeShiftedServiceDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.reference_service_id, 0x1234);
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        assert!(matches!(
            TimeShiftedServiceDescriptor::parse(&[0x4D, 2, 0x12, 0x34]).unwrap_err(),
            Error::InvalidDescriptor { tag: 0x4D, .. }
        ));
    }

    #[test]
    fn parse_rejects_short_buffer() {
        // declares length 2 but only 1 body byte present
        let bytes = [TAG, 2, 0x12];
        assert!(matches!(
            TimeShiftedServiceDescriptor::parse(&bytes).unwrap_err(),
            Error::BufferTooShort { .. }
        ));
    }

    #[test]
    fn parse_rejects_wrong_length() {
        let bytes = [TAG, 3, 0x12, 0x34, 0x00];
        assert!(matches!(
            TimeShiftedServiceDescriptor::parse(&bytes).unwrap_err(),
            Error::InvalidDescriptor { tag: TAG, .. }
        ));
    }

    #[test]
    fn serialize_round_trip() {
        let d = TimeShiftedServiceDescriptor {
            reference_service_id: 0xABCD,
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(TimeShiftedServiceDescriptor::parse(&buf).unwrap(), d);
    }

    #[test]
    fn serialize_rejects_small_buffer() {
        let d = TimeShiftedServiceDescriptor {
            reference_service_id: 1,
        };
        let mut tiny = [0u8; 3];
        assert!(matches!(
            d.serialize_into(&mut tiny).unwrap_err(),
            Error::OutputBufferTooSmall { .. }
        ));
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_round_trip() {
        let d = TimeShiftedServiceDescriptor {
            reference_service_id: 0x1234,
        };
        let json = serde_json::to_string(&d).unwrap();
        // Serialize-only: assert the emitted JSON re-parses (serialize-stable).
        let _v: serde_json::Value = serde_json::from_str(&json).unwrap();
    }
}
