//! FlexMuxTiming Descriptor — ISO/IEC 13818-1 §2.6.54, Table 2-82 (tag 0x2C).
//!
//! Carries flex-mux timing parameters: FCR_ES_ID, FCRResolution, FCRLength,
//! and FmxRateLength.

use super::descriptor_body;
use crate::error::{Error, Result};
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for FlexMuxTiming_descriptor.
pub const TAG: u8 = 0x2C;
const HEADER_LEN: usize = 2;
const BODY_LEN: u8 = 10;

/// FlexMuxTiming Descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct FlexMuxTimingDescriptor {
    /// ES_ID of the FlexMux clock reference stream.
    pub fcr_es_id: u16,
    /// Clock resolution in Hz.
    pub fcr_resolution: u32,
    /// FCR length in bytes.
    pub fcr_length: u8,
    /// FlexMux rate length in bits.
    pub fmx_rate_length: u8,
}

impl<'a> Parse<'a> for FlexMuxTimingDescriptor {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "FlexMuxTimingDescriptor",
            "unexpected tag for FlexMuxTiming_descriptor",
        )?;
        if body.len() != BODY_LEN as usize {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "FlexMuxTiming_descriptor length must equal 10",
            });
        }
        Ok(Self {
            fcr_es_id: u16::from_be_bytes([body[0], body[1]]),
            fcr_resolution: u32::from_be_bytes([body[2], body[3], body[4], body[5]]),
            fcr_length: body[6],
            fmx_rate_length: body[7],
        })
    }
}

impl Serialize for FlexMuxTimingDescriptor {
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
        buf[HEADER_LEN..HEADER_LEN + 2].copy_from_slice(&self.fcr_es_id.to_be_bytes());
        buf[HEADER_LEN + 2..HEADER_LEN + 6].copy_from_slice(&self.fcr_resolution.to_be_bytes());
        buf[HEADER_LEN + 6] = self.fcr_length;
        buf[HEADER_LEN + 7] = self.fmx_rate_length;
        Ok(len)
    }
}
impl crate::traits::DescriptorDef<'_> for FlexMuxTimingDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "FLEX_MUX_TIMING";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_extracts_fields() {
        let bytes = [
            TAG, 10, 0x00, 0x01, 0x12, 0x34, 0x56, 0x78, 0x05, 0x1E, 0, 0,
        ];
        let d = FlexMuxTimingDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.fcr_es_id, 0x0001);
        assert_eq!(d.fcr_resolution, 0x12345678);
        assert_eq!(d.fcr_length, 0x05);
        assert_eq!(d.fmx_rate_length, 0x1E);
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let err =
            FlexMuxTimingDescriptor::parse(&[0x02, 10, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: 0x02, .. }));
    }

    #[test]
    fn parse_rejects_wrong_length() {
        let err = FlexMuxTimingDescriptor::parse(&[TAG, 9, 0, 0, 0, 0, 0, 0, 0, 0, 0]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { .. }));
    }

    #[test]
    fn parse_rejects_short_buffer() {
        let err = FlexMuxTimingDescriptor::parse(&[TAG, 11, 0, 0]).unwrap_err();
        assert!(matches!(err, Error::BufferTooShort { .. }));
    }

    #[test]
    fn serialize_round_trip() {
        let d = FlexMuxTimingDescriptor {
            fcr_es_id: 0xBEEF,
            fcr_resolution: 0xDEADBEEF,
            fcr_length: 0x42,
            fmx_rate_length: 0x99,
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let reparsed = FlexMuxTimingDescriptor::parse(&buf).unwrap();
        assert_eq!(d, reparsed);
    }

    #[test]
    fn serialize_rejects_small_buffer() {
        let d = FlexMuxTimingDescriptor {
            fcr_es_id: 0,
            fcr_resolution: 0,
            fcr_length: 0,
            fmx_rate_length: 0,
        };
        let mut tiny = vec![0u8; 5];
        let err = d.serialize_into(&mut tiny).unwrap_err();
        assert!(matches!(err, Error::OutputBufferTooSmall { .. }));
    }
}
