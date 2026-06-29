//! IOD Descriptor — ISO/IEC 13818-1 §2.6.40, Table 2-75 (tag 0x1D).
//!
//! Carries an InitialObjectDescriptor() from ISO/IEC 14496-1 §8.6.3.1 as
//! opaque bytes after the Scope_of_IOD_label and IOD_label fields.

use super::descriptor_body;
use crate::error::{Error, Result};
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for IOD_descriptor.
pub const TAG: u8 = 0x1D;
const HEADER_LEN: usize = 2;
const FIXED_LEN: usize = 2;

/// IOD Descriptor.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct IodDescriptor<'a> {
    /// Scope of the IOD label.
    pub scope_of_iod_label: u8,
    /// IOD label.
    pub iod_label: u8,
    /// InitialObjectDescriptor() carried as opaque bytes.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub initial_object_descriptor: &'a [u8],
}

impl<'a> Parse<'a> for IodDescriptor<'a> {
    type Error = crate::error::Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "IodDescriptor",
            "unexpected tag for IOD_descriptor",
        )?;
        if body.len() < FIXED_LEN {
            return Err(Error::InvalidDescriptor {
                tag: TAG,
                reason: "IOD_descriptor length too short (need >= 2)",
            });
        }
        Ok(Self {
            scope_of_iod_label: body[0],
            iod_label: body[1],
            initial_object_descriptor: &body[FIXED_LEN..],
        })
    }
}

impl Serialize for IodDescriptor<'_> {
    type Error = crate::error::Error;

    fn serialized_len(&self) -> usize {
        HEADER_LEN + FIXED_LEN + self.initial_object_descriptor.len()
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
        buf[HEADER_LEN] = self.scope_of_iod_label;
        buf[HEADER_LEN + 1] = self.iod_label;
        buf[HEADER_LEN + FIXED_LEN..len].copy_from_slice(self.initial_object_descriptor);
        Ok(len)
    }
}
impl<'a> crate::traits::DescriptorDef<'a> for IodDescriptor<'a> {
    const TAG: u8 = TAG;
    const NAME: &'static str = "IOD";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_minimal() {
        let bytes = [TAG, 2, 0x01, 0x02];
        let d = IodDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.scope_of_iod_label, 0x01);
        assert_eq!(d.iod_label, 0x02);
        assert!(d.initial_object_descriptor.is_empty());
    }

    #[test]
    fn parse_with_opaque() {
        let bytes = [TAG, 5, 0x0A, 0x0B, 0xCC, 0xDD, 0xEE];
        let d = IodDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.scope_of_iod_label, 0x0A);
        assert_eq!(d.iod_label, 0x0B);
        assert_eq!(d.initial_object_descriptor, &[0xCC, 0xDD, 0xEE]);
    }

    #[test]
    fn serialize_round_trip() {
        let d = IodDescriptor {
            scope_of_iod_label: 0x03,
            iod_label: 0x04,
            initial_object_descriptor: &[0xAA, 0xBB],
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let reparsed = IodDescriptor::parse(&buf).unwrap();
        assert_eq!(d, reparsed);
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let err = IodDescriptor::parse(&[0x02, 2, 0, 0]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: 0x02, .. }));
    }

    #[test]
    fn parse_rejects_too_short() {
        let err = IodDescriptor::parse(&[TAG, 1, 0]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { tag: TAG, .. }));
    }

    #[test]
    fn serialize_rejects_small_buffer() {
        let d = IodDescriptor {
            scope_of_iod_label: 0,
            iod_label: 0,
            initial_object_descriptor: &[],
        };
        let mut tiny = vec![0u8; 3];
        let err = d.serialize_into(&mut tiny).unwrap_err();
        assert!(matches!(err, Error::OutputBufferTooSmall { .. }));
    }
}
