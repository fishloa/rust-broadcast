//! CPCM Delivery Signalling Descriptor — ETSI TS 102 825-9 §4.1.5, Table 2 (tag_extension 0x01).
use super::*;

impl<'a> ExtensionBodyDef<'a> for CpcmDeliverySignalling<'a> {
    const TAG_EXTENSION: u8 = 0x01;
    const NAME: &'static str = "CPCM_DELIVERY_SIGNALLING";
}

/// cpcm_delivery_signalling body (Table 2, §4.1.5): an encoding version plus the
/// version-dependent CPCM USI `selector_byte`s. For `cpcm_version == 1` the
/// selector is the CPCM delivery signalling (USI) of ETSI TS 102 825-4; at the
/// descriptor level it is a version-tagged opaque payload.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct CpcmDeliverySignalling<'a> {
    /// `cpcm_version` — encoding version of the USI structure in the selector bytes.
    pub cpcm_version: u8,
    /// The `selector_byte`s (version-dependent CPCM USI payload; see TS 102 825-4).
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub selector_bytes: &'a [u8],
}

impl<'a> Parse<'a> for CpcmDeliverySignalling<'a> {
    type Error = crate::error::Error;
    fn parse(sel: &'a [u8]) -> Result<Self> {
        let (cpcm_version, selector_bytes) = sel.split_first().ok_or(Error::BufferTooShort {
            need: 1,
            have: 0,
            what: "cpcm_delivery_signalling body",
        })?;
        Ok(CpcmDeliverySignalling {
            cpcm_version: *cpcm_version,
            selector_bytes,
        })
    }
}

impl Serialize for CpcmDeliverySignalling<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        1 + self.selector_bytes.len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        buf[0] = self.cpcm_version;
        buf[1..len].copy_from_slice(self.selector_bytes);
        Ok(len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::descriptors::extension::test_support::*;
    use crate::descriptors::extension::{ExtensionBody, ExtensionDescriptor};

    #[test]
    fn parse_cpcm_delivery_signalling_structured() {
        // cpcm_version=1, then 4 selector bytes
        let sel = [0x01, 0x39, 0x24, 0x45, 0x03];
        let bytes = wrap(0x01, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::CpcmDeliverySignalling(b) => {
                assert_eq!(b.cpcm_version, 1);
                assert_eq!(b.selector_bytes, &[0x39, 0x24, 0x45, 0x03]);
            }
            other => panic!("expected CpcmDeliverySignalling, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn parse_cpcm_delivery_signalling_version_only() {
        let sel = [0x01]; // cpcm_version, empty selector
        let bytes = wrap(0x01, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::CpcmDeliverySignalling(b) => {
                assert_eq!(b.cpcm_version, 1);
                assert!(b.selector_bytes.is_empty());
            }
            other => panic!("expected CpcmDeliverySignalling, got {other:?}"),
        }
        round_trip(&d);
    }
}
