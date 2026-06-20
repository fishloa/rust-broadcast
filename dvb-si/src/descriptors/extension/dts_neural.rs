//! DTS Neural Descriptor — ETSI EN 300 468 Annex L.1, Table L.1 (tag_extension 0x0F).
use super::*;

impl<'a> ExtensionBodyDef<'a> for DtsNeural<'a> {
    const TAG_EXTENSION: u8 = 0x0F;
    const NAME: &'static str = "DTS_NEURAL";
}

/// DTS Neural descriptor body (Table L.1, Annex L.1).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct DtsNeural<'a> {
    /// `config_id` — audio channel configuration of the host stream (Tables L.2/L.3).
    pub config_id: u8,
    /// Optional `additional_info_byte` run.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub additional_info: &'a [u8],
}

impl<'a> Parse<'a> for DtsNeural<'a> {
    type Error = crate::error::Error;
    fn parse(sel: &'a [u8]) -> Result<Self> {
        let (&config_id, additional_info) = sel.split_first().ok_or(Error::BufferTooShort {
            need: 1,
            have: 0,
            what: "DTS-Neural descriptor body",
        })?;
        Ok(DtsNeural {
            config_id,
            additional_info,
        })
    }
}

impl Serialize for DtsNeural<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        1 + self.additional_info.len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        buf[0] = self.config_id;
        buf[1..len].copy_from_slice(self.additional_info);
        Ok(len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::descriptors::extension::test_support::*;
    use crate::descriptors::extension::{ExtensionBody, ExtensionDescriptor};

    #[test]
    fn parse_dts_neural_structured() {
        let sel = [0x07, 0xFF];
        let bytes = wrap(0x0F, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::DtsNeural(b) => {
                assert_eq!(b.config_id, 0x07);
                assert_eq!(b.additional_info, &[0xFF]);
            }
            other => panic!("expected DtsNeural, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn parse_dts_neural_config_only() {
        let sel = [0x07];
        let bytes = wrap(0x0F, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::DtsNeural(b) => {
                assert_eq!(b.config_id, 0x07);
                assert!(b.additional_info.is_empty());
            }
            other => panic!("expected DtsNeural, got {other:?}"),
        }
        round_trip(&d);
    }
}
