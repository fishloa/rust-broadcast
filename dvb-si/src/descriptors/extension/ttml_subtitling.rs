//! TTML Subtitling Descriptor — ETSI EN 303 560 §5.2.1.1 (tag_extension 0x20).
//!
//! The `reserved_tail` field holds trailing `reserved_zero_future_use` bytes
//! verbatim; future spec growth is surfaced via additive typed accessors.
use super::*;
use alloc::vec::Vec;

impl<'a> ExtensionBodyDef<'a> for TtmlSubtitling<'a> {
    const TAG_EXTENSION: u8 = 0x20;
    const NAME: &'static str = "TTML_SUBTITLING";
}

/// Spec default of every `reserved_zero_future_use` field of the
/// TTML_subtitling_descriptor (the 2-bit one in byte 4 and the 1-bit one in
/// each font entry): 0 (EN 303 560 V1.1.1 Table 1;
/// `dvb-si/docs/reserved-bits-edition-audit.md` §6).
pub const TTML_RESERVED_ZERO_FUTURE_USE_DEFAULT: u8 = 0x00;
const RESERVED_ZERO_2_MASK: u8 = 0x03;
const FONT_ID_MASK: u8 = 0x7F;
const FONT_RESERVED_MASK: u8 = 0x01;

/// One `font_count` loop entry: `reserved_zero_future_use`(1) + `font_id`(7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct TtmlFont {
    /// `reserved_zero_future_use` (1 bit, right-aligned) — bit 7 of the entry
    /// byte. Preserved on parse; [`TtmlFont::new`] uses
    /// [`TTML_RESERVED_ZERO_FUTURE_USE_DEFAULT`].
    pub reserved_zero_future_use: u8,
    /// `font_id` (7 bits).
    pub font_id: u8,
}

impl TtmlFont {
    /// Build an entry with the spec-default (zero) reserved bit.
    #[must_use]
    pub const fn new(font_id: u8) -> Self {
        Self {
            reserved_zero_future_use: TTML_RESERVED_ZERO_FUTURE_USE_DEFAULT,
            font_id,
        }
    }
}

/// TTML_subtitling body (EN 303 560 Table 1, §5.2.1.1).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
#[non_exhaustive]
pub struct TtmlSubtitling<'a> {
    /// ISO_639_language_code(24).
    pub iso_639_language_code: LangCode,
    /// subtitle_purpose(6) — Table 2.
    pub subtitle_purpose: u8,
    /// TTS_suitability(2) — Table 3.
    pub tts_suitability: u8,
    /// `reserved_zero_future_use` (2 bits, right-aligned) in byte 4, between
    /// `qualifier_present_flag` and `dvb_ttml_profile_count`. Preserved on
    /// parse; defaults to [`TTML_RESERVED_ZERO_FUTURE_USE_DEFAULT`].
    pub reserved_zero_future_use: u8,
    /// dvb_ttml_profile codes (one `Vec` entry per profile byte, EN 303 560).
    pub dvb_ttml_profiles: Vec<u8>,
    /// qualifier(32), present iff qualifier_present_flag.
    pub qualifier: Option<u32>,
    /// font entries (reserved bit + font_id(7) each), present iff
    /// essential_font_usage_flag.
    pub font_ids: Option<Vec<TtmlFont>>,
    /// text_char run (DVB Annex-A text label).
    pub text: DvbText<'a>,
    /// Trailing reserved_zero_future_use bytes (opaque), preserved verbatim.
    pub reserved_tail: &'a [u8],
}

impl<'a> TtmlSubtitling<'a> {
    /// Build a body with the spec-default (zero) reserved bits and an empty
    /// `reserved_tail`.
    #[must_use]
    pub const fn new(
        iso_639_language_code: LangCode,
        subtitle_purpose: u8,
        tts_suitability: u8,
        dvb_ttml_profiles: Vec<u8>,
        qualifier: Option<u32>,
        font_ids: Option<Vec<TtmlFont>>,
        text: DvbText<'a>,
    ) -> Self {
        Self {
            iso_639_language_code,
            subtitle_purpose,
            tts_suitability,
            reserved_zero_future_use: TTML_RESERVED_ZERO_FUTURE_USE_DEFAULT,
            dvb_ttml_profiles,
            qualifier,
            font_ids,
            text,
            reserved_tail: &[],
        }
    }
}

impl<'a> Parse<'a> for TtmlSubtitling<'a> {
    type Error = crate::error::Error;
    fn parse(sel: &'a [u8]) -> Result<Self> {
        if sel.len() < TTML_FIXED_LEN {
            return Err(Error::BufferTooShort {
                need: TTML_FIXED_LEN,
                have: sel.len(),
                what: "TTML_subtitling body",
            });
        }
        let iso_639_language_code = LangCode([sel[0], sel[1], sel[2]]);
        let b3 = sel[ISO_639_LEN];
        let subtitle_purpose = b3 >> 2;
        let tts_suitability = b3 & 0x03;
        let b4 = sel[ISO_639_LEN + 1];
        let essential_font_usage_flag = (b4 >> 7) & 1;
        let qualifier_present_flag = (b4 >> 6) & 1;
        let reserved_zero_future_use = (b4 >> 4) & RESERVED_ZERO_2_MASK;
        let dvb_ttml_profile_count = (b4 & 0x0F) as usize;

        let mut pos = TTML_FIXED_LEN;

        // dvb_ttml_profile loop
        if sel.len() < pos + dvb_ttml_profile_count {
            return Err(Error::BufferTooShort {
                need: pos + dvb_ttml_profile_count,
                have: sel.len(),
                what: "TTML_subtitling body",
            });
        }
        let dvb_ttml_profiles = sel[pos..pos + dvb_ttml_profile_count].to_vec();
        pos += dvb_ttml_profile_count;

        // conditional qualifier
        let qualifier = if qualifier_present_flag != 0 {
            let (b, _) = sel
                .get(pos..)
                .and_then(|s| s.split_first_chunk::<4>())
                .ok_or(Error::BufferTooShort {
                    need: pos + 4,
                    have: sel.len(),
                    what: "TTML_subtitling body",
                })?;
            let q = u32::from_be_bytes(*b);
            pos += 4;
            Some(q)
        } else {
            None
        };

        // conditional font-id loop
        let font_ids = if essential_font_usage_flag != 0 {
            if sel.len() <= pos {
                return Err(Error::BufferTooShort {
                    need: pos + 1,
                    have: sel.len(),
                    what: "TTML_subtitling body",
                });
            }
            let font_count = sel[pos] as usize;
            pos += 1;
            if sel.len() < pos + font_count {
                return Err(Error::BufferTooShort {
                    need: pos + font_count,
                    have: sel.len(),
                    what: "TTML_subtitling body",
                });
            }
            let ids: Vec<TtmlFont> = sel[pos..pos + font_count]
                .iter()
                .map(|b| TtmlFont {
                    reserved_zero_future_use: (b >> 7) & FONT_RESERVED_MASK,
                    font_id: b & FONT_ID_MASK,
                })
                .collect();
            pos += font_count;
            Some(ids)
        } else {
            None
        };

        // text run
        if sel.len() <= pos {
            return Err(Error::BufferTooShort {
                need: pos + 1,
                have: sel.len(),
                what: "TTML_subtitling body",
            });
        }
        let text_length = sel[pos] as usize;
        pos += 1;
        if sel.len() < pos + text_length {
            return Err(Error::BufferTooShort {
                need: pos + text_length,
                have: sel.len(),
                what: "TTML_subtitling body",
            });
        }
        let text = DvbText::new(&sel[pos..pos + text_length]);
        pos += text_length;

        // opaque trailing reserved bytes
        let reserved_tail = &sel[pos..];

        Ok(TtmlSubtitling {
            iso_639_language_code,
            subtitle_purpose,
            tts_suitability,
            reserved_zero_future_use,
            dvb_ttml_profiles,
            qualifier,
            font_ids,
            text,
            reserved_tail,
        })
    }
}

impl Serialize for TtmlSubtitling<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        TTML_FIXED_LEN
            + self.dvb_ttml_profiles.len()
            + self.qualifier.map_or(0, |_| 4)
            + self.font_ids.as_ref().map_or(0, |ids| 1 + ids.len())
            + 1
            + self.text.len()
            + self.reserved_tail.len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        buf[..ISO_639_LEN].copy_from_slice(&self.iso_639_language_code.0);
        // byte 3: subtitle_purpose(6) | TTS_suitability(2)
        buf[ISO_639_LEN] = ((self.subtitle_purpose & 0x3F) << 2) | (self.tts_suitability & 0x03);
        // byte 4: essential_font_usage_flag(1) | qualifier_present_flag(1)
        //         | reserved_zero_future_use(2) (preserved) | dvb_ttml_profile_count(4)
        let dvb_ttml_profile_count = broadcast_common::len::fit_bits(
            self.dvb_ttml_profiles.len() as u64,
            4,
            "dvb_ttml_profile_count",
        )? as u8;
        buf[ISO_639_LEN + 1] = (if self.font_ids.is_some() { 0x80 } else { 0 })
            | (if self.qualifier.is_some() { 0x40 } else { 0 })
            | ((self.reserved_zero_future_use & RESERVED_ZERO_2_MASK) << 4)
            | dvb_ttml_profile_count;

        let mut pos = TTML_FIXED_LEN;

        // dvb_ttml_profile loop
        buf[pos..pos + self.dvb_ttml_profiles.len()].copy_from_slice(&self.dvb_ttml_profiles);
        pos += self.dvb_ttml_profiles.len();

        // conditional qualifier
        if let Some(q) = self.qualifier {
            buf[pos..pos + 4].copy_from_slice(&q.to_be_bytes());
            pos += 4;
        }

        // conditional font-id loop
        if let Some(ids) = &self.font_ids {
            buf[pos] = broadcast_common::len::fit_u8(ids.len(), "font_id_count")?;
            pos += 1;
            for id in ids {
                buf[pos] = ((id.reserved_zero_future_use & FONT_RESERVED_MASK) << 7)
                    | (id.font_id & FONT_ID_MASK);
                pos += 1;
            }
        }

        // text run
        let raw = self.text.raw();
        buf[pos] = broadcast_common::len::fit_u8(raw.len(), "text_length")?;
        pos += 1;
        buf[pos..pos + raw.len()].copy_from_slice(raw);
        pos += raw.len();

        // opaque trailing reserved bytes
        buf[pos..pos + self.reserved_tail.len()].copy_from_slice(self.reserved_tail);

        Ok(len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::descriptors::extension::test_support::*;
    use crate::descriptors::extension::{ExtensionBody, ExtensionDescriptor};
    use crate::text::LangCode;

    /// Full-featured parse: 2 profiles, qualifier present, 3 font_ids,
    /// short text, and a 2-byte reserved_tail.
    #[test]
    fn parse_full_ttml_subtitling() {
        // iso_639 = "eng"
        // subtitle_purpose = 0x10, tts_suitability = 0x01
        let b3 = (0x10 << 2) | 0x01; // = 0x41
        // font_flag=1, qualifier_flag=1, reserved_zero=0, profile_count=2
        let b4 = 0x80 | 0x40 | 0x02; // = 0xC2
        let sel = [
            b'e', b'n', b'g', // iso_639
            b3,   // packed: subtitle_purpose, tts_suitability
            b4,   // packed: font_flag, qual_flag, count
            0x00, 0x02, // 2 profile bytes
            0xDE, 0xAD, 0xBE, 0xEF, // qualifier u32 be
            3,    // font_count
            0x10, 0x20, 0x30, // 3 font_ids (high bit already zero)
            5,    // text_length
            b'h', b'e', b'l', b'l', b'o', // text
            0xFF, 0xFE, // reserved_tail
        ];
        let bytes = wrap(0x20, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::TtmlSubtitling(b) => {
                assert_eq!(b.iso_639_language_code, LangCode(*b"eng"));
                assert_eq!(b.subtitle_purpose, 0x10);
                assert_eq!(b.tts_suitability, 0x01);
                assert_eq!(b.dvb_ttml_profiles, &[0x00, 0x02]);
                assert_eq!(b.qualifier, Some(0xDEAD_BEEF));
                assert_eq!(
                    b.font_ids.as_deref(),
                    Some(
                        &[
                            TtmlFont::new(0x10),
                            TtmlFont::new(0x20),
                            TtmlFont::new(0x30)
                        ][..]
                    )
                );
                assert_eq!(b.text.decode(), "hello");
                assert_eq!(b.text.raw(), b"hello");
                assert_eq!(b.reserved_tail, &[0xFF, 0xFE]);
            }
            other => panic!("expected TtmlSubtitling, got {other:?}"),
        }
        round_trip(&d);
    }

    /// Minimal variant: no qualifier, no fonts, empty profiles.
    #[test]
    fn parse_minimal_ttml_subtitling() {
        let sel = [
            b'f', b'r', b'e', // iso_639
            0x00, // subtitle_purpose=0, tts_suitability=0
            0x00, // font_flag=0, qual_flag=0, profile_count=0
            3,    // text_length
            b'f', b'o', b'o', // text
        ];
        let bytes = wrap(0x20, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::TtmlSubtitling(b) => {
                assert_eq!(b.iso_639_language_code, LangCode(*b"fre"));
                assert_eq!(b.subtitle_purpose, 0);
                assert_eq!(b.tts_suitability, 0);
                assert!(b.dvb_ttml_profiles.is_empty());
                assert_eq!(b.qualifier, None);
                assert_eq!(b.font_ids, None);
                assert_eq!(b.text.decode(), "foo");
                assert_eq!(b.text.raw(), b"foo");
                assert!(b.reserved_tail.is_empty());
            }
            other => panic!("expected TtmlSubtitling, got {other:?}"),
        }
        round_trip(&d);
    }

    /// Every value of the 2-bit byte-4 region and the 1-bit per-font region
    /// re-serializes byte-identically (a V1.1.1 sender may write ones).
    #[test]
    fn ttml_reserved_preserved() {
        for r2 in 0u8..=RESERVED_ZERO_2_MASK {
            for r1 in 0u8..=1 {
                let sel = [
                    b'e',
                    b'n',
                    b'g',
                    0x00,
                    0x80 | (r2 << 4) | 1, // fonts present, 1 profile
                    0x05,                 // profile
                    2,                    // font_count
                    (r1 << 7) | 0x11,
                    ((1 - r1) << 7) | 0x22,
                    1,
                    b'x',
                ];
                let bytes = wrap(0x20, &sel);
                let d = ExtensionDescriptor::parse(&bytes).unwrap();
                let mut buf = vec![0u8; d.serialized_len()];
                d.serialize_into(&mut buf).unwrap();
                assert_eq!(buf, bytes, "r2={r2} r1={r1}");
            }
        }
    }

    #[test]
    fn ttml_default_is_spec_value() {
        let d = TtmlSubtitling::new(
            LangCode(*b"eng"),
            0,
            0,
            vec![],
            None,
            Some(vec![TtmlFont::new(0x11)]),
            DvbText::new(b""),
        );
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        assert_eq!(buf, [b'e', b'n', b'g', 0x00, 0x80, 1, 0x11, 0]);
    }

    /// Reject: font_count declares 3 entries but not enough bytes follow.
    #[test]
    fn parse_rejects_truncated_font_loop() {
        let sel = [
            b'e', b'n', b'g', // iso_639
            0x00, // subtitle_purpose=0, tts_suitability=0
            0x80, // font_flag=1, qual_flag=0, profile_count=0
            3,    // font_count = 3
            0x10, // only 1 byte — need 3
        ];
        let bytes = wrap(0x20, &sel);
        let result = ExtensionDescriptor::parse(&bytes);
        assert!(
            matches!(result, Err(crate::error::Error::BufferTooShort { .. })),
            "expected BufferTooShort, got {result:?}"
        );
    }

    /// Reject: body truncated before text_length byte.
    #[test]
    fn parse_rejects_truncated_before_text_length() {
        // font_flag=0, qual_flag=0, profile_count=0 → prefix only, no room for text_length
        let sel = [b'e', b'n', b'g', 0x00, 0x00];
        let bytes = wrap(0x20, &sel);
        let result = ExtensionDescriptor::parse(&bytes);
        assert!(
            matches!(result, Err(crate::error::Error::BufferTooShort { .. })),
            "expected BufferTooShort, got {result:?}"
        );
    }

    /// Reject: header too short (fewer than 5 prefix bytes).
    #[test]
    fn parse_rejects_header_too_short() {
        let sel = *b"en"; // only 2 bytes
        let bytes = wrap(0x20, &sel);
        let result = ExtensionDescriptor::parse(&bytes);
        assert!(
            matches!(result, Err(crate::error::Error::BufferTooShort { .. })),
            "expected BufferTooShort, got {result:?}"
        );
    }
}
