//! Time Shifted Event Descriptor — ETSI EN 300 468 §6.2.44 (tag 0x4F).
//!
//! Table 103 (PDF p. 109). Carried in the EIT for an NVOD time-shifted
//! event; points at the reference service + event it shifts. Body is a 16-bit
//! `reference_service_id` followed by a 16-bit `reference_event_id`.

use crate::error::{Error, Result};
use crate::traits::Descriptor;
use dvb_common::{Parse, Serialize};

/// Descriptor tag for time_shifted_event_descriptor.
pub const TAG: u8 = 0x4F;
const HEADER_LEN: usize = 2;
const BODY_LEN: usize = 4;

/// Time Shifted Event Descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct TimeShiftedEventDescriptor {
    /// service_id of the reference (NVOD reference) service.
    pub reference_service_id: u16,
    /// event_id of the reference event within that service.
    pub reference_event_id: u16,
}

impl<'a> Parse<'a> for TimeShiftedEventDescriptor {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < HEADER_LEN {
            return Err(Error::BufferTooShort {
                need: HEADER_LEN,
                have: bytes.len(),
                what: "TimeShiftedEventDescriptor header",
            });
        }
        if bytes[0] != TAG {
            return Err(Error::InvalidDescriptor {
                tag: bytes[0],
                reason: "unexpected tag for time_shifted_event_descriptor",
            });
        }
        let length = bytes[1] as usize;
        if length != BODY_LEN {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "time_shifted_event_descriptor length must be 4",
            });
        }
        let end = HEADER_LEN + BODY_LEN;
        if bytes.len() < end {
            return Err(Error::BufferTooShort {
                need: end,
                have: bytes.len(),
                what: "TimeShiftedEventDescriptor body",
            });
        }
        Ok(Self {
            reference_service_id: u16::from_be_bytes([bytes[2], bytes[3]]),
            reference_event_id: u16::from_be_bytes([bytes[4], bytes[5]]),
        })
    }
}

impl Serialize for TimeShiftedEventDescriptor {
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
        buf[4..6].copy_from_slice(&self.reference_event_id.to_be_bytes());
        Ok(len)
    }
}

impl<'a> Descriptor<'a> for TimeShiftedEventDescriptor {
    const TAG: u8 = TAG;
    fn descriptor_length(&self) -> u8 {
        BODY_LEN as u8
    }
}

impl<'a> crate::traits::DescriptorDef<'a> for TimeShiftedEventDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "TIME_SHIFTED_EVENT";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_reference_ids() {
        let bytes = [TAG, 4, 0x12, 0x34, 0x56, 0x78];
        let d = TimeShiftedEventDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.reference_service_id, 0x1234);
        assert_eq!(d.reference_event_id, 0x5678);
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        assert!(matches!(
            TimeShiftedEventDescriptor::parse(&[0x50, 4, 0, 0, 0, 0]).unwrap_err(),
            Error::InvalidDescriptor { tag: 0x50, .. }
        ));
    }

    #[test]
    fn parse_rejects_short_buffer() {
        // declares length 4 but only 3 body bytes present
        let bytes = [TAG, 4, 0x12, 0x34, 0x56];
        assert!(matches!(
            TimeShiftedEventDescriptor::parse(&bytes).unwrap_err(),
            Error::BufferTooShort { .. }
        ));
    }

    #[test]
    fn parse_rejects_wrong_length() {
        let bytes = [TAG, 5, 0x12, 0x34, 0x56, 0x78, 0x00];
        assert!(matches!(
            TimeShiftedEventDescriptor::parse(&bytes).unwrap_err(),
            Error::InvalidDescriptor { tag: TAG, .. }
        ));
    }

    #[test]
    fn serialize_round_trip() {
        let d = TimeShiftedEventDescriptor {
            reference_service_id: 0xABCD,
            reference_event_id: 0x0102,
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(TimeShiftedEventDescriptor::parse(&buf).unwrap(), d);
    }

    #[test]
    fn serialize_rejects_small_buffer() {
        let d = TimeShiftedEventDescriptor {
            reference_service_id: 1,
            reference_event_id: 2,
        };
        let mut tiny = [0u8; 5];
        assert!(matches!(
            d.serialize_into(&mut tiny).unwrap_err(),
            Error::OutputBufferTooSmall { .. }
        ));
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_round_trip() {
        let d = TimeShiftedEventDescriptor {
            reference_service_id: 0x1234,
            reference_event_id: 0x5678,
        };
        let json = serde_json::to_string(&d).unwrap();
        // Serialize-only: assert the emitted JSON re-parses (serialize-stable).
        let _v: serde_json::Value = serde_json::from_str(&json).unwrap();
    }
}
