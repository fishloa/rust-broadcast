//! Metadata STD Descriptor — ISO/IEC 13818-1 §2.6.62, Table 2-91 (tag 0x27).
//!
//! Three 22-bit uimsbf values (stored as u32 with top 10 bits zero),
//! each preceded by 2 reserved bits. Total body: 9 bytes.

use super::descriptor_body;
use crate::error::{Error, Result};
use dvb_common::{Parse, Serialize};

/// Descriptor tag for Metadata_STD_descriptor.
pub const TAG: u8 = 0x27;
const HEADER_LEN: usize = 2;
const BODY_LEN: u8 = 9;

/// Metadata STD Descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct MetadataStdDescriptor {
    /// metadata_input_leak_rate, 22-bit uimsbf, in units of 400 bits/s.
    pub metadata_input_leak_rate: u32,
    /// metadata_buffer_size, 22-bit uimsbf, in units of 1024 bytes.
    pub metadata_buffer_size: u32,
    /// metadata_output_leak_rate, 22-bit uimsbf, in units of 400 bits/s.
    pub metadata_output_leak_rate: u32,
}

/// Extract a 22-bit uimsbf from a 3-byte slice (top 2 bits reserved/zero).
fn read_u22(bytes: &[u8]) -> u32 {
    ((bytes[0] as u32) << 16) | ((bytes[1] as u32) << 8) | (bytes[2] as u32)
}

/// Write a 22-bit value into a 3-byte slice (top 2 bits zero).
fn write_u22(val: u32, buf: &mut [u8]) {
    let masked = val & 0x003F_FFFF;
    buf[0] = ((masked >> 16) & 0x3F) as u8;
    buf[1] = ((masked >> 8) & 0xFF) as u8;
    buf[2] = (masked & 0xFF) as u8;
}

impl<'a> Parse<'a> for MetadataStdDescriptor {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "MetadataStdDescriptor",
            "unexpected tag for Metadata_STD_descriptor",
        )?;
        if body.len() != BODY_LEN as usize {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "Metadata_STD_descriptor length must equal 9",
            });
        }
        Ok(Self {
            metadata_input_leak_rate: read_u22(&body[0..3]),
            metadata_buffer_size: read_u22(&body[3..6]),
            metadata_output_leak_rate: read_u22(&body[6..9]),
        })
    }
}

impl Serialize for MetadataStdDescriptor {
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
        write_u22(
            self.metadata_input_leak_rate,
            &mut buf[HEADER_LEN..HEADER_LEN + 3],
        );
        write_u22(
            self.metadata_buffer_size,
            &mut buf[HEADER_LEN + 3..HEADER_LEN + 6],
        );
        write_u22(
            self.metadata_output_leak_rate,
            &mut buf[HEADER_LEN + 6..HEADER_LEN + 9],
        );
        Ok(len)
    }
}
impl<'a> crate::traits::DescriptorDef<'a> for MetadataStdDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "METADATA_STD";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_extracts_fields() {
        let bytes = [TAG, 9, 0x10, 0x20, 0x30, 0x01, 0x02, 0x03, 0x3F, 0xFF, 0xFF];
        let d = MetadataStdDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.metadata_input_leak_rate, read_u22(&[0x10, 0x20, 0x30]));
        assert_eq!(d.metadata_buffer_size, read_u22(&[0x01, 0x02, 0x03]));
        assert_eq!(d.metadata_output_leak_rate, read_u22(&[0x3F, 0xFF, 0xFF]));
    }

    #[test]
    fn parse_max_values() {
        let bytes = [TAG, 9, 0x3F, 0xFF, 0xFF, 0x3F, 0xFF, 0xFF, 0x3F, 0xFF, 0xFF];
        let d = MetadataStdDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.metadata_input_leak_rate, 0x003F_FFFF);
        assert_eq!(d.metadata_buffer_size, 0x003F_FFFF);
        assert_eq!(d.metadata_output_leak_rate, 0x003F_FFFF);
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let err = MetadataStdDescriptor::parse(&[0x02, 9, 0, 0, 0, 0, 0, 0, 0, 0, 0]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: 0x02, .. }));
    }

    #[test]
    fn parse_rejects_wrong_length() {
        let err = MetadataStdDescriptor::parse(&[TAG, 8, 0, 0, 0, 0, 0, 0, 0, 0]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { .. }));
    }

    #[test]
    fn serialize_round_trip() {
        let d = MetadataStdDescriptor {
            metadata_input_leak_rate: 0x000A_BCDE,
            metadata_buffer_size: 0x003F_0000,
            metadata_output_leak_rate: 0x0000_1234,
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let reparsed = MetadataStdDescriptor::parse(&buf).unwrap();
        assert_eq!(d, reparsed);
    }

    #[test]
    fn serialize_round_trip_zero_upper_bits() {
        // Ensure top 2 bits of each 3-byte group are zero on serialization
        let d = MetadataStdDescriptor {
            metadata_input_leak_rate: 0xFFFF_FFFF, // test mask
            metadata_buffer_size: 0xFFFF_FFFF,
            metadata_output_leak_rate: 0xFFFF_FFFF,
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let reparsed = MetadataStdDescriptor::parse(&buf).unwrap();
        assert_eq!(reparsed.metadata_input_leak_rate, 0x003F_FFFF);
        assert_eq!(reparsed.metadata_buffer_size, 0x003F_FFFF);
        assert_eq!(reparsed.metadata_output_leak_rate, 0x003F_FFFF);
    }

    #[test]
    fn serialize_rejects_small_buffer() {
        let d = MetadataStdDescriptor {
            metadata_input_leak_rate: 0,
            metadata_buffer_size: 0,
            metadata_output_leak_rate: 0,
        };
        let mut tiny = vec![0u8; 8];
        let err = d.serialize_into(&mut tiny).unwrap_err();
        assert!(matches!(err, Error::OutputBufferTooSmall { .. }));
    }
}
