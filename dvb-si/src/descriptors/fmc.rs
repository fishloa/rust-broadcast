//! FMC Descriptor — ISO/IEC 13818-1 §2.6.44, Table 2-77 (tag 0x1F).
//!
//! A list of (ES_ID, FlexMuxChannel) pairs — 3 bytes each, consuming the
//! entire descriptor body.

use super::descriptor_body;
use crate::error::{Error, Result};
use alloc::vec::Vec;
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for FMC_descriptor.
pub const TAG: u8 = 0x1F;
const HEADER_LEN: usize = 2;

/// FMC Descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct FmcDescriptor {
    /// List of (ES_ID, FlexMuxChannel) pairs.
    pub entries: Vec<(u16, u8)>,
}

impl<'a> Parse<'a> for FmcDescriptor {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "FmcDescriptor",
            "unexpected tag for FMC_descriptor",
        )?;
        if body.len() % 3 != 0 {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "FMC_descriptor body length must be a multiple of 3",
            });
        }
        let entries = body
            .chunks_exact(3)
            .map(|chunk| {
                let es_id = u16::from_be_bytes([chunk[0], chunk[1]]);
                (es_id, chunk[2])
            })
            .collect();
        Ok(Self { entries })
    }
}

impl Serialize for FmcDescriptor {
    type Error = crate::error::Error;

    fn serialized_len(&self) -> usize {
        HEADER_LEN + self.entries.len() * 3
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
        buf[1] = (len - HEADER_LEN) as u8;
        for (i, &(es_id, fmc)) in self.entries.iter().enumerate() {
            let off = HEADER_LEN + i * 3;
            buf[off..off + 2].copy_from_slice(&es_id.to_be_bytes());
            buf[off + 2] = fmc;
        }
        Ok(len)
    }
}
impl crate::traits::DescriptorDef<'_> for FmcDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "FMC";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_empty() {
        let d = FmcDescriptor::parse(&[TAG, 0]).unwrap();
        assert!(d.entries.is_empty());
    }

    #[test]
    fn parse_single_entry() {
        let bytes = [TAG, 3, 0x00, 0x01, 0x42];
        let d = FmcDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.entries, vec![(0x0001, 0x42)]);
    }

    #[test]
    fn parse_multiple_entries() {
        let bytes = [TAG, 6, 0x00, 0x0A, 0x01, 0x00, 0x0B, 0x02];
        let d = FmcDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.entries, vec![(0x000A, 0x01), (0x000B, 0x02)]);
    }

    #[test]
    fn parse_rejects_unaligned_length() {
        let err = FmcDescriptor::parse(&[TAG, 1, 0x00]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { .. }));
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let err = FmcDescriptor::parse(&[0x02, 0, 0]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: 0x02, .. }));
    }

    #[test]
    fn serialize_round_trip() {
        let d = FmcDescriptor {
            entries: vec![(0xDEAD, 0x42), (0xBEEF, 0x99)],
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let reparsed = FmcDescriptor::parse(&buf).unwrap();
        assert_eq!(d, reparsed);
    }

    #[test]
    fn serialize_rejects_small_buffer() {
        let d = FmcDescriptor {
            entries: vec![(0, 0)],
        };
        let mut tiny = vec![0u8; 3];
        let err = d.serialize_into(&mut tiny).unwrap_err();
        assert!(matches!(err, Error::OutputBufferTooSmall { .. }));
    }
}
