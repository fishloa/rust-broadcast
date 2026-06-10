use super::*;

impl super::sealed::Sealed for Message<'_> {}
impl ExtensionBodyDef for Message<'_> {
    const TAG_EXTENSION: u8 = 0x08;
    const NAME: &'static str = "MESSAGE";
}

// ===========================================================================
//  Section 0x08 — message_descriptor (Table 148, §6.4.9)
// ===========================================================================
/// message body (Table 148).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct Message<'a> {
    /// message_id(8).
    pub message_id: u8,
    /// ISO_639_language_code(24).
    pub iso_639_language_code: LangCode,
    /// DVB Annex-A encoded text_char run (remainder of body).
    pub text: DvbText<'a>,
}

impl<'a> Parse<'a> for Message<'a> {
    type Error = crate::error::Error;
    fn parse(sel: &'a [u8]) -> Result<Self> {
        if sel.len() < 1 + ISO_639_LEN {
            return Err(invalid("message: header truncated"));
        }
        Ok(Message {
            message_id: sel[0],
            iso_639_language_code: LangCode([sel[1], sel[2], sel[3]]),
            text: DvbText::new(&sel[1 + ISO_639_LEN..]),
        })
    }
}

impl Serialize for Message<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        1 + ISO_639_LEN + self.text.len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        buf[0] = self.message_id;
        buf[1..1 + ISO_639_LEN].copy_from_slice(&self.iso_639_language_code.0);
        buf[1 + ISO_639_LEN..len].copy_from_slice(self.text.raw());
        Ok(len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::descriptors::extension::test_support::*;
    use crate::descriptors::extension::{ExtensionBody, ExtensionDescriptor};
    use crate::text::LangCode;

    #[test]
    fn parse_message() {
        let sel = [0x07, b'e', b'n', b'g', b'H', b'i'];
        let bytes = wrap(0x08, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::Message(b) => {
                assert_eq!(b.message_id, 0x07);
                assert_eq!(b.iso_639_language_code, LangCode(*b"eng"));
                assert_eq!(b.text.raw(), b"Hi");
            }
            other => panic!("expected Message, got {other:?}"),
        }
        round_trip(&d);
    }
}
