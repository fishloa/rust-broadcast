//! Scrambling Descriptor — ETSI EN 300 468 §6.2.32 (tag 0x65).
//!
//! A single byte identifying the scrambling mode in use (Table 86 syntax /
//! Table 87 coding, PDF pp. 98-99): 0x01 = DVB-CSA1, 0x02 = DVB-CSA2,
//! 0x03 = DVB-CSA3, 0x10 = DVB-CISSA v1, etc.

use super::descriptor_body;
use crate::error::{Error, Result};
use dvb_common::{Parse, Serialize};

/// Descriptor tag for scrambling_descriptor.
pub const TAG: u8 = 0x65;
const HEADER_LEN: usize = 2;
/// Fixed payload length: a single scrambling_mode byte (EN 300 468 Table 86).
const BODY_LEN: u8 = 1;

/// Scrambling Descriptor (tag 0x65).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ScramblingDescriptor {
    /// 8-bit scrambling_mode (ETSI Table 87, PDF p. 99).
    pub scrambling_mode: u8,
}

impl<'a> Parse<'a> for ScramblingDescriptor {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "ScramblingDescriptor",
            "unexpected tag for scrambling_descriptor",
        )?;
        if body.len() != BODY_LEN as usize {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "scrambling_descriptor length must equal 1",
            });
        }
        Ok(Self {
            scrambling_mode: body[0],
        })
    }
}

impl Serialize for ScramblingDescriptor {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        HEADER_LEN + BODY_LEN as usize
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
        buf[1] = BODY_LEN;
        buf[HEADER_LEN] = self.scrambling_mode;
        Ok(len)
    }
}
impl<'a> crate::traits::DescriptorDef<'a> for ScramblingDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "SCRAMBLING";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_extracts_scrambling_mode() {
        let bytes = [TAG, 1, 0x02];
        let d = ScramblingDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.scrambling_mode, 0x02);
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let err = ScramblingDescriptor::parse(&[0x66, 1, 0x02]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: 0x66, .. }));
    }

    #[test]
    fn parse_rejects_short_buffer() {
        let err = ScramblingDescriptor::parse(&[TAG]).unwrap_err();
        assert!(matches!(err, Error::BufferTooShort { .. }));
    }

    #[test]
    fn parse_rejects_truncated_body() {
        // length=1 but no payload byte present.
        let err = ScramblingDescriptor::parse(&[TAG, 1]).unwrap_err();
        assert!(matches!(err, Error::BufferTooShort { .. }));
    }

    #[test]
    fn parse_rejects_wrong_length() {
        let err = ScramblingDescriptor::parse(&[TAG, 2, 0x02, 0x03]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { .. }));
    }

    #[test]
    fn serialize_round_trip() {
        let d = ScramblingDescriptor {
            scrambling_mode: 0x10,
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let re = ScramblingDescriptor::parse(&buf).unwrap();
        assert_eq!(d, re);
    }

    #[test]
    fn serialize_rejects_too_small_buffer() {
        let d = ScramblingDescriptor {
            scrambling_mode: 0x03,
        };
        let mut tiny = [0u8; 1];
        let err = d.serialize_into(&mut tiny).unwrap_err();
        assert!(matches!(err, Error::OutputBufferTooSmall { .. }));
    }

    #[test]
    fn descriptor_length_matches_payload() {
        let d = ScramblingDescriptor {
            scrambling_mode: 0x01,
        };
        assert_eq!(d.serialized_len() - 2, 1);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_round_trip() {
        let d = ScramblingDescriptor {
            scrambling_mode: 0x02,
        };
        let json = serde_json::to_string(&d).unwrap();
        // Serialize-only: assert the emitted JSON re-parses (serialize-stable).
        let _v: serde_json::Value = serde_json::from_str(&json).unwrap();
    }
}
