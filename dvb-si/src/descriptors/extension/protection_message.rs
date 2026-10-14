//! Protection Message Descriptor — ETSI TS 102 809 §9.3.3, Table 40 (tag_extension 0x18).
use super::*;

impl<'a> ExtensionBodyDef<'a> for ProtectionMessage<'a> {
    const TAG_EXTENSION: u8 = 0x18;
    const NAME: &'static str = "PROTECTION_MESSAGE";
}

/// Spec default of the 4-bit `reserved` field: all ones (ETSI TS 102 809
/// Table 40, V1.3.1 and earlier editions;
/// `dvb-si/docs/reserved-bits-edition-audit.md` §6).
pub const PROTECTION_MESSAGE_RESERVED_DEFAULT: u8 = 0x0F;
const RESERVED_MASK: u8 = 0x0F;

/// protection_message body (Table 40, §9.3.3): the list of protected
/// `component_tag`s. `component_count` is a 4-bit field, so at most 15 tags.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
#[non_exhaustive]
pub struct ProtectionMessage<'a> {
    /// `reserved` (4 bits, right-aligned) — preserved on parse;
    /// [`ProtectionMessage::new`] uses [`PROTECTION_MESSAGE_RESERVED_DEFAULT`] (all ones).
    pub reserved: u8,
    /// The `component_tag` list (one `u8` per protected component;
    /// `component_count` is its length).
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub component_tags: &'a [u8],
    /// Bytes after the `component_tag` loop. Table 40 defines none; real
    /// streams carry slack (r03-W9) which is preserved verbatim and re-emitted
    /// (same pattern as `DepthRangeBody::Other` in `video_depth_range`).
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub trailing: &'a [u8],
}

impl<'a> ProtectionMessage<'a> {
    /// Build a body with the spec-default (all ones) `reserved` bits and no
    /// trailing bytes.
    #[must_use]
    pub const fn new(component_tags: &'a [u8]) -> Self {
        Self {
            reserved: PROTECTION_MESSAGE_RESERVED_DEFAULT,
            component_tags,
            trailing: &[],
        }
    }
}

impl<'a> Parse<'a> for ProtectionMessage<'a> {
    type Error = crate::error::Error;
    fn parse(sel: &'a [u8]) -> Result<Self> {
        let first = *sel.first().ok_or(Error::BufferTooShort {
            need: 1,
            have: 0,
            what: "protection_message body",
        })?;
        let reserved = (first >> 4) & RESERVED_MASK;
        let component_count = usize::from(first & 0x0F);
        let component_tags = sel
            .get(1..1 + component_count)
            .ok_or(Error::BufferTooShort {
                need: 1 + component_count,
                have: sel.len(),
                what: "protection_message component_tags",
            })?;
        // Table 40 ends with the component_tag loop. Bytes after it have no
        // defined meaning and are deliberately tolerated, not rejected: real
        // streams carry slack and rejection would break them (r03-W9; the
        // no-reject policy of the fixture-checked table parsers). They are
        // kept in `trailing` and re-emitted byte-identically.
        Ok(ProtectionMessage {
            reserved,
            component_tags,
            trailing: &sel[1 + component_count..],
        })
    }
}

impl Serialize for ProtectionMessage<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        1 + self.component_tags.len() + self.trailing.len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        let component_count = broadcast_common::len::fit_bits(
            self.component_tags.len() as u64,
            4,
            "component_count",
        )? as u8;
        buf[0] = (broadcast_common::len::fit_mask(self.reserved, RESERVED_MASK, "reserved")? << 4)
            | component_count;
        let tags_end = 1 + self.component_tags.len();
        buf[1..tags_end].copy_from_slice(self.component_tags);
        buf[tags_end..len].copy_from_slice(self.trailing);
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

    /// r03-W9: slack after the component_tag loop is tolerated, never an
    /// error; the tags parse and the slack is preserved and re-emitted.
    #[test]
    fn parse_tolerates_trailing_bytes() {
        let bytes = wrap(0x18, &[0xF1, 0x10, 0xEE]);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::ProtectionMessage(b) => {
                assert_eq!(b.component_tags, &[0x10]);
                assert_eq!(b.trailing, &[0xEE]);
            }
            other => panic!("expected ProtectionMessage, got {other:?}"),
        }
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(buf, bytes);
    }

    /// Every `reserved` value (spec 1111 and the non-default ones) and a
    /// trailing run re-serialize byte-identically.
    #[test]
    fn protection_message_reserved_preserved() {
        for r in 0u8..=RESERVED_MASK {
            for tail in [&[][..], &[0xAA, 0xBB][..]] {
                let mut sel = vec![(r << 4) | 2, 0x10, 0x20];
                sel.extend_from_slice(tail);
                let bytes = wrap(0x18, &sel);
                let d = ExtensionDescriptor::parse(&bytes).unwrap();
                let mut buf = vec![0u8; d.serialized_len()];
                d.serialize_into(&mut buf).unwrap();
                assert_eq!(buf, bytes, "r={r:#x}");
            }
        }
    }

    #[test]
    fn protection_message_default_is_spec_value() {
        let pm = ProtectionMessage::new(&[0x10]);
        let mut buf = vec![0u8; pm.serialized_len()];
        pm.serialize_into(&mut buf).unwrap();
        assert_eq!(buf, [0xF1, 0x10]);
    }

    #[test]
    fn serialize_rejects_oversized_component_list() {
        let tags = [0u8; 16];
        let pm = ProtectionMessage::new(&tags);
        let mut buf = [0u8; 32];
        assert!(matches!(
            pm.serialize_into(&mut buf),
            Err(Error::FieldOverflow(_))
        ));
    }
}
