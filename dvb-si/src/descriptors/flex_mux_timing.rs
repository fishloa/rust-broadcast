//! FlexMuxTiming Descriptor — ISO/IEC 13818-1 §2.6.54, Table 2-82 (tag 0x2C).
//!
//! Carries flex-mux timing parameters: FCR_ES_ID, FCRResolution, FCRLength,
//! and FmxRateLength.

use super::descriptor_body;
use crate::error::{Error, Result};
use broadcast_core::len::need_out;
use broadcast_core::{Parse, Serialize};

/// Descriptor tag for FlexMuxTiming_descriptor.
pub const TAG: u8 = 0x2C;
const HEADER_LEN: usize = 2;
// Table 2-82: FCR_ES_ID(16) + FCRResolution(32) + FCRLength(8) + FmxRateLength(8) = 64 bits = 8 bytes.
const BODY_LEN: u8 = 8;
/// Offset of `FCR_ES_ID` in the descriptor body.
const FCR_ES_ID_OFFSET: usize = 0;
/// Offset of `FCR_resolution` in the descriptor body.
const FCR_RESOLUTION_OFFSET: usize = FCR_ES_ID_OFFSET + core::mem::size_of::<u16>();
/// Width of `FCR_resolution`, bytes.
const FCR_RESOLUTION_LEN: usize = core::mem::size_of::<u32>();
/// Offset of `FCR_length` in the descriptor body.
const FCR_LENGTH_OFFSET: usize = FCR_RESOLUTION_OFFSET + FCR_RESOLUTION_LEN;
/// Offset of `FMX_rate_length` in the descriptor body.
const FMX_RATE_LENGTH_OFFSET: usize = FCR_LENGTH_OFFSET + 1;

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
                reason: "FlexMuxTiming_descriptor length must equal 8",
            });
        }
        let mut resolution_bytes = [0u8; FCR_RESOLUTION_LEN];
        resolution_bytes.copy_from_slice(&body[FCR_RESOLUTION_OFFSET..FCR_LENGTH_OFFSET]);
        Ok(Self {
            fcr_es_id: u16::from_be_bytes([body[0], body[1]]),
            fcr_resolution: u32::from_be_bytes(resolution_bytes),
            fcr_length: body[FCR_LENGTH_OFFSET],
            fmx_rate_length: body[FMX_RATE_LENGTH_OFFSET],
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
        need_out(buf, len)?;
        buf[0] = TAG;
        buf[1] = BODY_LEN;
        buf[HEADER_LEN + FCR_ES_ID_OFFSET..HEADER_LEN + FCR_RESOLUTION_OFFSET]
            .copy_from_slice(&self.fcr_es_id.to_be_bytes());
        buf[HEADER_LEN + FCR_RESOLUTION_OFFSET..HEADER_LEN + FCR_LENGTH_OFFSET]
            .copy_from_slice(&self.fcr_resolution.to_be_bytes());
        buf[HEADER_LEN + FCR_LENGTH_OFFSET] = self.fcr_length;
        buf[HEADER_LEN + FMX_RATE_LENGTH_OFFSET] = self.fmx_rate_length;
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
    use alloc::vec;

    #[test]
    fn parse_extracts_fields() {
        let bytes = [TAG, 8, 0x00, 0x01, 0x12, 0x34, 0x56, 0x78, 0x05, 0x1E];
        let d = FlexMuxTimingDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.fcr_es_id, 0x0001);
        assert_eq!(d.fcr_resolution, 0x12345678);
        assert_eq!(d.fcr_length, 0x05);
        assert_eq!(d.fmx_rate_length, 0x1E);
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let err = FlexMuxTimingDescriptor::parse(&[0x02, 8, 0, 0, 0, 0, 0, 0, 0, 0]).unwrap_err();
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
        assert!(matches!(err, Error::BufferTooShort(..)));
    }

    /// Regression for issue #1053: the pre-fix BODY_LEN of 10 rejected every
    /// conformant (8-byte body) descriptor and, on serialize, wrote
    /// descriptor_length=10 while only filling 8 body bytes, leaking 2 stale
    /// caller bytes into the wire output.
    #[test]
    fn serialize_writes_conformant_8_byte_body() {
        let d = FlexMuxTimingDescriptor {
            fcr_es_id: 0x0001,
            fcr_resolution: 0x1234_5678,
            fcr_length: 0x05,
            fmx_rate_length: 0x1E,
        };
        assert_eq!(d.serialized_len(), 10); // HEADER_LEN(2) + BODY_LEN(8)
        let mut buf = vec![0xFFu8; d.serialized_len()];
        let written = d.serialize_into(&mut buf).unwrap();
        assert_eq!(written, 10);
        assert_eq!(buf[1], 8, "descriptor_length must be 8 per Table 2-82");
        let reparsed = FlexMuxTimingDescriptor::parse(&buf).unwrap();
        assert_eq!(reparsed, d);
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
        assert!(matches!(err, Error::OutputBufferTooSmall(..)));
    }
}
