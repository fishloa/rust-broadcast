//! CI Ancillary Data Descriptor — ETSI EN 300 468 §6.4.3, Table 112 (tag_extension 0x14).
//!
//! Carries CI Plus ancillary data as an opaque byte run; its format is defined
//! by the CI ecosystem, so it is preserved verbatim as a borrowed slice.
use super::*;

impl<'a> ExtensionBodyDef<'a> for CiAncillaryData<'a> {
    const TAG_EXTENSION: u8 = 0x14;
    const NAME: &'static str = "CI_ANCILLARY_DATA";
}

/// CI_ancillary_data descriptor body (Table 112, §6.4.3).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct CiAncillaryData<'a> {
    /// ancillary_data_byte run (the entire descriptor body).
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub ancillary_data: &'a [u8],
}

impl<'a> Parse<'a> for CiAncillaryData<'a> {
    type Error = crate::error::Error;
    fn parse(sel: &'a [u8]) -> Result<Self> {
        Ok(CiAncillaryData {
            ancillary_data: sel,
        })
    }
}

impl Serialize for CiAncillaryData<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        self.ancillary_data.len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        buf[..len].copy_from_slice(self.ancillary_data);
        Ok(len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::descriptors::extension::test_support::*;
    use crate::descriptors::extension::{ExtensionBody, ExtensionDescriptor, ExtensionTag};

    #[test]
    fn parse_ci_ancillary_data() {
        let sel = [0xDE, 0xAD, 0xBE, 0xEF];
        let bytes = wrap(0x14, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        assert_eq!(d.kind(), Some(ExtensionTag::CiAncillaryData));
        match &d.body {
            ExtensionBody::CiAncillaryData(b) => {
                assert_eq!(b.ancillary_data, &[0xDE, 0xAD, 0xBE, 0xEF]);
            }
            other => panic!("expected CiAncillaryData, got {other:?}"),
        }
        round_trip(&d);
    }
}
