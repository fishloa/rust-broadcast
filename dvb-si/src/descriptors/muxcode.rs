//! Muxcode Descriptor — ISO/IEC 13818-1 §2.6.48, Table 2-79 (tag 0x21).
//!
//! Carries MuxCodeTableEntry() structures from ISO/IEC 14496-1 §11.2.4.3
//! as opaque bytes.

use super::descriptor_body;
use crate::error::{Error, Result};
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for Muxcode_descriptor.
pub const TAG: u8 = 0x21;
const HEADER_LEN: usize = 2;

/// Muxcode Descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct MuxcodeDescriptor<'a> {
    /// MuxCodeTableEntry() entries carried as opaque bytes.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub mux_code_table_entries: &'a [u8],
}

impl<'a> Parse<'a> for MuxcodeDescriptor<'a> {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "MuxcodeDescriptor",
            "unexpected tag for Muxcode_descriptor",
        )?;
        Ok(Self {
            mux_code_table_entries: body,
        })
    }
}

impl Serialize for MuxcodeDescriptor<'_> {
    type Error = crate::error::Error;

    fn serialized_len(&self) -> usize {
        HEADER_LEN + self.mux_code_table_entries.len()
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
        buf[1] = self.mux_code_table_entries.len() as u8;
        buf[HEADER_LEN..len].copy_from_slice(self.mux_code_table_entries);
        Ok(len)
    }
}
impl<'a> crate::traits::DescriptorDef<'a> for MuxcodeDescriptor<'a> {
    const TAG: u8 = TAG;
    const NAME: &'static str = "MUXCODE";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_empty() {
        let d = MuxcodeDescriptor::parse(&[TAG, 0]).unwrap();
        assert!(d.mux_code_table_entries.is_empty());
    }

    #[test]
    fn parse_with_data() {
        let bytes = [TAG, 3, 0xAA, 0xBB, 0xCC];
        let d = MuxcodeDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.mux_code_table_entries, &[0xAA, 0xBB, 0xCC]);
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let err = MuxcodeDescriptor::parse(&[0x02, 0]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: 0x02, .. }));
    }

    #[test]
    fn serialize_round_trip() {
        let d = MuxcodeDescriptor {
            mux_code_table_entries: &[0x11, 0x22, 0x33],
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let reparsed = MuxcodeDescriptor::parse(&buf).unwrap();
        assert_eq!(d, reparsed);
    }

    #[test]
    fn serialize_rejects_small_buffer() {
        let d = MuxcodeDescriptor {
            mux_code_table_entries: &[1, 2],
        };
        let mut tiny = vec![0u8; 2];
        let err = d.serialize_into(&mut tiny).unwrap_err();
        assert!(matches!(err, Error::OutputBufferTooSmall { .. }));
    }
}
