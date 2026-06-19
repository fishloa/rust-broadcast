//! Protection Message Descriptor — ETSI TS 102 809 §9.3.3, Table 40 (tag_extension 0x18).
use super::*;

impl<'a> ExtensionBodyDef<'a> for ProtectionMessage<'a> {
    const TAG_EXTENSION: u8 = 0x18;
    const NAME: &'static str = "PROTECTION_MESSAGE";
}

/// Largest `component_count` the 4-bit field can carry.
const MAX_COMPONENT_COUNT: usize = 0x0F;

/// protection_message body (Table 40, §9.3.3): the list of protected
/// `component_tag`s. `component_count` is a 4-bit field, so at most 15 tags.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct ProtectionMessage<'a> {
    /// reserved(4) — preserved verbatim for byte-exact round-trip.
    pub reserved: u8,
    /// The `component_tag` list (one `u8` per protected component;
    /// `component_count` is its length).
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub component_tags: &'a [u8],
}

impl<'a> Parse<'a> for ProtectionMessage<'a> {
    type Error = crate::error::Error;
    fn parse(sel: &'a [u8]) -> Result<Self> {
        let first = *sel.first().ok_or(Error::BufferTooShort {
            need: 1,
            have: 0,
            what: "protection_message body",
        })?;
        let reserved = first >> 4;
        let component_count = usize::from(first & 0x0F);
        let component_tags = sel
            .get(1..1 + component_count)
            .ok_or(Error::BufferTooShort {
                need: 1 + component_count,
                have: sel.len(),
                what: "protection_message component_tags",
            })?;
        Ok(ProtectionMessage {
            reserved,
            component_tags,
        })
    }
}

impl Serialize for ProtectionMessage<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        1 + self.component_tags.len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        if self.component_tags.len() > MAX_COMPONENT_COUNT {
            return Err(Error::ValueOutOfRange {
                field: "protection_message component_count",
                reason: "more than 15 component_tags (4-bit field)",
            });
        }
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        buf[0] = ((self.reserved & 0x0F) << 4) | (self.component_tags.len() as u8 & 0x0F);
        buf[1..len].copy_from_slice(self.component_tags);
        Ok(len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::descriptors::extension::test_support::*;
    use crate::descriptors::extension::{ExtensionBody, ExtensionDescriptor};

    #[test]
    fn parse_protection_message_structured() {
        // reserved=0xF, component_count=3, tags 0x10/0x20/0x30
        let sel = [0xF3, 0x10, 0x20, 0x30];
        let bytes = wrap(0x18, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::ProtectionMessage(b) => {
                assert_eq!(b.reserved, 0x0F);
                assert_eq!(b.component_tags, &[0x10, 0x20, 0x30]);
            }
            other => panic!("expected ProtectionMessage, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn parse_protection_message_empty() {
        let sel = [0xF0]; // reserved=0xF, component_count=0
        let bytes = wrap(0x18, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::ProtectionMessage(b) => assert!(b.component_tags.is_empty()),
            other => panic!("expected ProtectionMessage, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn serialize_rejects_oversized_component_list() {
        let tags = [0u8; 16];
        let pm = ProtectionMessage {
            reserved: 0x0F,
            component_tags: &tags,
        };
        let mut buf = [0u8; 32];
        assert!(matches!(
            pm.serialize_into(&mut buf),
            Err(Error::ValueOutOfRange { .. })
        ));
    }
}
