//! insert_descriptor_request_data() — ANSI/SCTE 104 2023 §9.8.5, Table 9-27 (opID 0x0108).
//!
//! Supplemental usage. Copies raw SCTE 35 descriptor images into the
//! descriptor loop of the resulting splice_info_section.

use alloc::vec::Vec;

use crate::error::{Error, Result};
use crate::traits::OperationDef;
use broadcast_common::{Parse, Serialize};

/// `opID` for insert_descriptor_request (§8.3, Table 8-4).
pub const OP_ID: u16 = 0x0108;

/// insert_descriptor_request_data() — §9.8.5, Table 9-27.
///
/// `descriptor_count` is not a stored field: it is always
/// `descriptor_images.len()`, written through `broadcast_common::len::fit_u8`
/// on serialize (audit run-09 S4-W1). A separately-stored count could
/// disagree with the vec it is meant to describe — e.g.
/// `InsertDescriptor { descriptor_count: 2, descriptor_images: vec![one] }`
/// would have serialized a count of 2 with only one image present, and a
/// peer would misframe the following operation by reading 4 bytes of it as
/// a second image's tag+length.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct InsertDescriptor<'a> {
    /// Raw descriptor images (each follows MPEG-2 descriptor format:
    /// tag(1) + length(1) + data(length)).
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub descriptor_images: Vec<&'a [u8]>,
}

impl InsertDescriptor<'_> {
    /// The `descriptor_count` value that will be written on serialize:
    /// `descriptor_images.len()`.
    #[must_use]
    pub fn descriptor_count(&self) -> usize {
        self.descriptor_images.len()
    }
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
        buf[0] = broadcast_common::len::fit_u8(
            self.descriptor_images.len(),
            "insert_descriptor.descriptor_count",
        )?;
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
            descriptor_images: alloc::vec![
                &[0xAB, 0x04, 0x01, 0x02, 0x03, 0x04][..],
                &[0xCD, 0x02, 0xAA, 0xBB][..],
            ],
        };
        assert_eq!(op.descriptor_count(), 2);
        let bytes = op.to_bytes();
        assert_eq!(
            bytes[0], 2,
            "descriptor_count byte must match the vec length"
        );
        let back = InsertDescriptor::parse(&bytes).unwrap();
        assert_eq!(op, back);
    }

    #[test]
    fn mutate_field_changes_output() {
        let op = InsertDescriptor {
            descriptor_images: alloc::vec![&[0xAB, 0x04, 0x01, 0x02, 0x03, 0x04][..]],
        };
        let bytes = op.to_bytes();
        let mut op2 = op.clone();
        op2.descriptor_images.push(&[0xCD, 0x02, 0xAA, 0xBB][..]);
        assert_ne!(op2.to_bytes(), bytes);
    }

    /// S4-W1/S4-W3 (#1103): `descriptor_count` is derived from
    /// `descriptor_images.len()`, so it can never disagree with the vec,
    /// and over 255 images is rejected rather than silently wrapping the
    /// count byte.
    #[test]
    fn over_255_images_rejected_not_wrapped() {
        let img = &[0xAB, 0x00][..];
        let op = InsertDescriptor {
            descriptor_images: alloc::vec![img; 256],
        };
        assert!(op.try_to_bytes().is_err());
    }
}
