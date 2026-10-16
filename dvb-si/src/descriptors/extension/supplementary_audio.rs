//! Supplementary Audio Descriptor — ETSI EN 300 468 §6.4.11 (tag_extension 0x06).
use super::*;

impl<'a> ExtensionBodyDef<'a> for SupplementaryAudio<'a> {
    const TAG_EXTENSION: u8 = 0x06;
    const NAME: &'static str = "SUPPLEMENTARY_AUDIO";
}
/// Default of the 1-bit `reserved_future_use` (Table 153 bit 1): `1` in every
/// edition (V1.13.1 `reserved` ... V1.20.1 `reserved_future_use`, audit §3.3).
/// 37 of 47 real descriptors write `0` (cause unexplained), so parse keeps it.
pub const RESERVED_FUTURE_USE_DEFAULT: u8 = 1;

/// supplementary_audio body (Table 153).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
#[non_exhaustive]
pub struct SupplementaryAudio<'a> {
    /// `reserved_future_use(1)` between `editorial_classification` and
    /// `language_code_present` (1 bit, right-aligned): captured on parse and
    /// re-emitted verbatim; [`SupplementaryAudio::new`] defaults to
    /// [`RESERVED_FUTURE_USE_DEFAULT`] (1).
    pub reserved_future_use: u8,
    /// mix_type(1) — Table 154.
    pub mix_type: bool,
    /// editorial_classification(5) — Table 155.
    pub editorial_classification: u8,
    /// language_code_present(1), as parsed from the wire.
    ///
    /// Informational only: [`Serialize`] derives the wire bit from
    /// `iso_639_language_code.is_some()`, so a stale value can never make the
    /// serializer emit the bit without the code (or the code without the
    /// bit) — r03-W5.
    pub language_code_present: bool,
    /// ISO_639_language_code(24), present iff `language_code_present`.
    pub iso_639_language_code: Option<LangCode>,
    /// Trailing private_data_byte run.
    pub private_data: &'a [u8],
}

impl<'a> SupplementaryAudio<'a> {
    /// Build a body with the spec-default reserved bit, no language code and
    /// no private data.
    #[must_use]
    pub const fn new(mix_type: bool, editorial_classification: u8) -> Self {
        Self {
            reserved_future_use: RESERVED_FUTURE_USE_DEFAULT,
            mix_type,
            editorial_classification,
            language_code_present: false,
            iso_639_language_code: None,
            private_data: &[],
        }
    }
}

impl<'a> Parse<'a> for SupplementaryAudio<'a> {
    type Error = crate::error::Error;
    fn parse(sel: &'a [u8]) -> Result<Self> {
        if sel.is_empty() {
            return Err(Error::BufferTooShort {
                need: 1,
                have: sel.len(),
                what: "supplementary_audio body",
            });
        }
        let flags = sel[0];
        let mix_type = (flags & 0x80) != 0;
        let editorial_classification = (flags >> 2) & 0x1F;
        let language_code_present = (flags & 0x01) != 0;
        let mut pos = 1;
        let iso_639_language_code = if language_code_present {
            if sel.len() < pos + ISO_639_LEN {
                return Err(Error::BufferTooShort {
                    need: pos + ISO_639_LEN,
                    have: sel.len(),
                    what: "supplementary_audio body",
                });
            }
            let lc = &sel[pos..pos + ISO_639_LEN];
            pos += ISO_639_LEN;
            Some(LangCode([lc[0], lc[1], lc[2]]))
        } else {
            None
        };
        Ok(SupplementaryAudio {
            reserved_future_use: (flags >> 1) & 1,
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
        // Table 153 bit 1 reserved_future_use: preserved, default 1.
        buf[0] = (u8::from(self.mix_type) << 7)
            | (broadcast_common::len::fit_mask(
                self.editorial_classification,
                0x1F,
                "editorial_classification",
            )? << 2)
            | (broadcast_common::len::fit_mask(self.reserved_future_use, 1, "reserved_future_use")?
                << 1)
            | u8::from(self.iso_639_language_code.is_some());
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

    /// r03-W5: `language_code_present` follows the Option in both mismatch
    /// directions, so the output always re-parses to the same framing.
    #[test]
    fn serialize_derives_language_flag_from_option() {
        let mut d = SupplementaryAudio::new(false, 1);
        d.language_code_present = true; // lies: no code
        d.private_data = &[0x55];
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let re = SupplementaryAudio::parse(&buf).unwrap();
        assert!(!re.language_code_present);
        assert_eq!(re.private_data, &[0x55]);

        d.language_code_present = false; // lies: code present
        d.iso_639_language_code = Some(LangCode(*b"fre"));
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        let re = SupplementaryAudio::parse(&buf).unwrap();
        assert!(re.language_code_present);
        assert_eq!(re.iso_639_language_code, Some(LangCode(*b"fre")));
        assert_eq!(re.private_data, &[0x55]);
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

    /// Both values of the reserved bit round-trip (France TNT / TSDuck samples
    /// write 0, the spec value is 1).
    #[test]
    fn reserved_future_use_preserved() {
        for r in 0u8..=1 {
            let flags = (0x05 << 2) | (r << 1) | 0x01;
            let sel = [flags, b'f', b'r', b'e'];
            let d = SupplementaryAudio::parse(&sel).unwrap();
            assert_eq!(d.reserved_future_use, r);
            let mut buf = vec![0u8; d.serialized_len()];
            d.serialize_into(&mut buf).unwrap();
            assert_eq!(buf, sel);
        }
    }

    #[test]
    fn default_reserved_future_use_is_one() {
        let d = SupplementaryAudio::new(false, 0);
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(buf, [0x02]);
    }
}
