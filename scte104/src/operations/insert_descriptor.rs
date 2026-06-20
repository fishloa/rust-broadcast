//! insert_descriptor_request_data() — ANSI/SCTE 104 2023 §9.8.5, Table 9-27 (opID 0x0108).
//!
//! Supplemental usage. Copies raw SCTE 35 descriptor images into the
//! descriptor loop of the resulting splice_info_section.

use alloc::vec::Vec;

use crate::error::{Error, Result};
use crate::traits::OperationDef;
use dvb_common::{Parse, Serialize};

/// `opID` for insert_descriptor_request (§8.3, Table 8-4).
pub const OP_ID: u16 = 0x0108;

/// insert_descriptor_request_data() — §9.8.5, Table 9-27.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct InsertDescriptor<'a> {
    /// `descriptor_count` — 1 byte.
    pub descriptor_count: u8,
    /// Raw descriptor images (each follows MPEG-2 descriptor format:
    /// tag(1) + length(1) + data(length)).
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub descriptor_images: Vec<&'a [u8]>,
}

impl<'a> Parse<'a> for InsertDescriptor<'a> {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.is_empty() {
            return Err(Error::BufferTooShort {
                need: 1,
                have: 0,
                what: "insert_descriptor descriptor_count",
            });
        }
        let count = bytes[0] as usize;
        let mut pos = 1;
        let mut images = Vec::with_capacity(count);
        for _ in 0..count {
            if bytes.len() < pos + 2 {
                return Err(Error::BufferTooShort {
                    need: pos + 2,
                    have: bytes.len(),
                    what: "insert_descriptor tag+length",
                });
            }
            let desc_len = bytes[pos + 1] as usize;
            let total = 2 + desc_len;
            if bytes.len() < pos + total {
                return Err(Error::BufferTooShort {
                    need: pos + total,
                    have: bytes.len(),
                    what: "insert_descriptor image",
                });
            }
            images.push(&bytes[pos..pos + total]);
            pos += total;
        }
        Ok(Self {
            descriptor_count: count as u8,
            descriptor_images: images,
        })
    }
}

impl Serialize for InsertDescriptor<'_> {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        1 + self
            .descriptor_images
            .iter()
            .map(|img| img.len())
            .sum::<usize>()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        buf[0] = self.descriptor_count;
        let mut pos = 1;
        for img in &self.descriptor_images {
            buf[pos..pos + img.len()].copy_from_slice(img);
            pos += img.len();
        }
        Ok(need)
    }
}

impl<'a> OperationDef<'a> for InsertDescriptor<'a> {
    const OP_ID: u16 = OP_ID;
    const NAME: &'static str = "INSERT_DESCRIPTOR";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let op = InsertDescriptor {
            descriptor_count: 2,
            descriptor_images: alloc::vec![
                &[0xAB, 0x04, 0x01, 0x02, 0x03, 0x04][..],
                &[0xCD, 0x02, 0xAA, 0xBB][..],
            ],
        };
        let bytes = op.to_bytes();
        let back = InsertDescriptor::parse(&bytes).unwrap();
        assert_eq!(op, back);
    }

    #[test]
    fn mutate_field_changes_output() {
        let op = InsertDescriptor {
            descriptor_count: 1,
            descriptor_images: alloc::vec![&[0xAB, 0x04, 0x01, 0x02, 0x03, 0x04][..]],
        };
        let bytes = op.to_bytes();
        let mut op2 = op.clone();
        op2.descriptor_count = 2;
        assert_ne!(op2.to_bytes(), bytes);
    }
}
