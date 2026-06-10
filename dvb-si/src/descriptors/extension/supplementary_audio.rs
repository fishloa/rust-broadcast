//! Supplementary Audio Descriptor — ETSI EN 300 468 §6.4.11 (tag_extension 0x06).
use super::*;

impl super::sealed::Sealed for SupplementaryAudio<'_> {}
impl ExtensionBodyDef for SupplementaryAudio<'_> {
    const TAG_EXTENSION: u8 = 0x06;
    const NAME: &'static str = "SUPPLEMENTARY_AUDIO";
}
/// supplementary_audio body (Table 153).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct SupplementaryAudio<'a> {
    /// mix_type(1) — Table 154.
    pub mix_type: bool,
    /// editorial_classification(5) — Table 155.
    pub editorial_classification: u8,
    /// language_code_present(1).
    pub language_code_present: bool,
    /// ISO_639_language_code(24), present iff `language_code_present`.
    pub iso_639_language_code: Option<LangCode>,
    /// Trailing private_data_byte run.
    pub private_data: &'a [u8],
}

impl<'a> Parse<'a> for SupplementaryAudio<'a> {
    type Error = crate::error::Error;
    fn parse(sel: &'a [u8]) -> Result<Self> {
        if sel.is_empty() {
            return Err(invalid("supplementary_audio: flags byte missing"));
        }
        let flags = sel[0];
        let mix_type = (flags & 0x80) != 0;
        let editorial_classification = (flags >> 2) & 0x1F;
        let language_code_present = (flags & 0x01) != 0;
        let mut pos = 1;
        let iso_639_language_code = if language_code_present {
            if sel.len() < pos + ISO_639_LEN {
                return Err(invalid("supplementary_audio: language code truncated"));
            }
            let lc = &sel[pos..pos + ISO_639_LEN];
            pos += ISO_639_LEN;
            Some(LangCode([lc[0], lc[1], lc[2]]))
        } else {
            None
        };
        Ok(SupplementaryAudio {
            mix_type,
            editorial_classification,
            language_code_present,
            iso_639_language_code,
            private_data: &sel[pos..],
        })
    }
}

impl Serialize for SupplementaryAudio<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        1 + self.iso_639_language_code.map_or(0, |_| ISO_639_LEN) + self.private_data.len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        // Table 153 bit 1 is plain reserved_future_use → emitted as 1.
        buf[0] = (u8::from(self.mix_type) << 7)
            | ((self.editorial_classification & 0x1F) << 2)
            | 0x02
            | u8::from(self.language_code_present);
        let mut p = 1;
        if let Some(lc) = self.iso_639_language_code {
            buf[p..p + ISO_639_LEN].copy_from_slice(&lc.0);
            p += ISO_639_LEN;
        }
        buf[p..p + self.private_data.len()].copy_from_slice(self.private_data);
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
    fn parse_supplementary_audio_with_language() {
        // mix_type=1, editorial=0x17, reserved=1, language_code_present=1,
        // then "fre", private 0xAA
        let flags = 0x80 | (0x17 << 2) | 0x02 | 0x01;
        let sel = [flags, b'f', b'r', b'e', 0xAA];
        let bytes = wrap(0x06, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::SupplementaryAudio(b) => {
                assert!(b.mix_type);
                assert_eq!(b.editorial_classification, 0x17);
                assert!(b.language_code_present);
                assert_eq!(b.iso_639_language_code, Some(LangCode(*b"fre")));
                assert_eq!(b.private_data, &[0xAA]);
            }
            other => panic!("expected SupplementaryAudio, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn parse_supplementary_audio_no_language() {
        let flags = ((0x01 << 2) & 0x7C) | 0x02; // mix=0, editorial=1, reserved=1, lang=0
        let sel = [flags];
        let bytes = wrap(0x06, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::SupplementaryAudio(b) => {
                assert!(!b.language_code_present);
                assert_eq!(b.iso_639_language_code, None);
                assert!(b.private_data.is_empty());
            }
            other => panic!("expected SupplementaryAudio, got {other:?}"),
        }
        round_trip(&d);
    }
}
