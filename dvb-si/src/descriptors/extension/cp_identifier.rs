//! CP Identifier Descriptor — ETSI EN 300 468 §6.4.6.1, Table 114 (tag_extension 0x03).
//!
//! Lists the CP_system_ids of the content protection systems available for the
//! associated service/bouquet — a loop of 16-bit identifiers.
use super::*;
use alloc::vec::Vec;

impl ExtensionBodyDef<'_> for CpIdentifier {
    const TAG_EXTENSION: u8 = 0x03;
    const NAME: &'static str = "CP_IDENTIFIER";
}

/// CP_identifier descriptor body (Table 114, §6.4.6.1).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct CpIdentifier {
    /// The CP_system_id loop.
    pub cp_system_ids: Vec<u16>,
}

impl<'a> Parse<'a> for CpIdentifier {
    type Error = crate::error::Error;
    fn parse(sel: &'a [u8]) -> Result<Self> {
        if sel.len() % 2 != 0 {
            return Err(Error::InvalidDescriptor {
                tag: crate::descriptor_tag::DescriptorTag::Extension as u8,
                reason: "CP_identifier body length must be a multiple of 2",
            });
        }
        let cp_system_ids = sel
            .chunks_exact(2)
            .map(|c| u16::from_be_bytes([c[0], c[1]]))
            .collect();
        Ok(CpIdentifier { cp_system_ids })
    }
}

impl Serialize for CpIdentifier {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        self.cp_system_ids.len() * 2
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        for (i, id) in self.cp_system_ids.iter().enumerate() {
            buf[i * 2..i * 2 + 2].copy_from_slice(&id.to_be_bytes());
        }
        Ok(len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::descriptors::extension::test_support::*;
    use crate::descriptors::extension::{ExtensionBody, ExtensionDescriptor, ExtensionTag};

    #[test]
    fn parse_cp_identifier() {
        let sel = [0x12, 0x34, 0x56, 0x78];
        let bytes = wrap(0x03, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.kind(), Some(ExtensionTag::CpIdentifier));
        match &d.body {
            ExtensionBody::CpIdentifier(b) => {
                assert_eq!(b.cp_system_ids, alloc::vec![0x1234, 0x5678]);
            }
            other => panic!("expected CpIdentifier, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn parse_cp_identifier_rejects_odd_length() {
        let err = CpIdentifier::parse(&[0x00, 0x00, 0x00]).unwrap_err();
        assert!(matches!(err, Error::InvalidDescriptor { .. }));
    }
}
