//! MultiplexBuffer Descriptor — ISO/IEC 13818-1 §2.6.52, Table 2-81 (tag 0x23).
//!
//! Two 24-bit uimsbf fields: MB_buffer_size and TB_leak_rate.

use super::descriptor_body;
use crate::error::{Error, Result};
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for MultiplexBuffer_descriptor.
pub const TAG: u8 = 0x23;
const HEADER_LEN: usize = 2;
const BODY_LEN: u8 = 6;

/// MultiplexBuffer Descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct MultiplexBufferDescriptor {
    /// MB_buffer_size, 24-bit uimsbf, in bytes.
    pub mb_buffer_size: u32,
    /// TB_leak_rate, 24-bit uimsbf, in units of 400 bits/s.
    pub tb_leak_rate: u32,
}

impl<'a> Parse<'a> for MultiplexBufferDescriptor {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "MultiplexBufferDescriptor",
            "unexpected tag for MultiplexBuffer_descriptor",
        )?;
        if body.len() != BODY_LEN as usize {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "MultiplexBuffer_descriptor length must equal 6",
            });
        }
        let mb_buffer_size = u32::from_be_bytes([0, body[0], body[1], body[2]]);
        let tb_leak_rate = u32::from_be_bytes([0, body[3], body[4], body[5]]);
        Ok(Self {
            mb_buffer_size,
            tb_leak_rate,
        })
    }
}

impl Serialize for MultiplexBufferDescriptor {
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
        buf[HEADER_LEN..HEADER_LEN + 3].copy_from_slice(&self.mb_buffer_size.to_be_bytes()[1..]);
        buf[HEADER_LEN + 3..HEADER_LEN + 6].copy_from_slice(&self.tb_leak_rate.to_be_bytes()[1..]);
        Ok(len)
    }
}
impl<'a> crate::traits::DescriptorDef<'a> for MultiplexBufferDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "MULTIPLEX_BUFFER";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_extracts_fields() {
        let bytes = [TAG, 6, 0x12, 0x34, 0x56, 0x78, 0x9A, 0xBC];
        let d = MultiplexBufferDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.mb_buffer_size, 0x00123456);
        assert_eq!(d.tb_leak_rate, 0x00789ABC);
    }

    #[test]
    fn parse_max_values() {
        let bytes = [TAG, 6, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF];
        let d = MultiplexBufferDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.mb_buffer_size, 0x00FF_FFFF);
        assert_eq!(d.tb_leak_rate, 0x00FF_FFFF);
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let err = MultiplexBufferDescriptor::parse(&[0x02, 6, 0, 0, 0, 0, 0, 0]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: 0x02, .. }));
    }

    #[test]
    fn parse_rejects_wrong_length() {
        let err = MultiplexBufferDescriptor::parse(&[TAG, 5, 0, 0, 0, 0, 0]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { .. }));
    }

    #[test]
    fn parse_rejects_short_buffer() {
        let err = MultiplexBufferDescriptor::parse(&[TAG, 7, 0, 0]).unwrap_err();
        assert!(matches!(err, Error::BufferTooShort { .. }));
    }

    #[test]
    fn serialize_round_trip() {
        let d = MultiplexBufferDescriptor {
            mb_buffer_size: 0x00ABCDEF,
            tb_leak_rate: 0x00123456,
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let reparsed = MultiplexBufferDescriptor::parse(&buf).unwrap();
        assert_eq!(d, reparsed);
    }

    #[test]
    fn serialize_rejects_small_buffer() {
        let d = MultiplexBufferDescriptor {
            mb_buffer_size: 0,
            tb_leak_rate: 0,
        };
        let mut tiny = vec![0u8; 5];
        let err = d.serialize_into(&mut tiny).unwrap_err();
        assert!(matches!(err, Error::OutputBufferTooSmall { .. }));
    }
}
