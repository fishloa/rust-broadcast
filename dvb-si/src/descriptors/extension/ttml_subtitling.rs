use super::*;

impl ExtensionBodyDef for TtmlSubtitling<'_> {
    const TAG_EXTENSION: u8 = 0x20;
    const NAME: &'static str = "TTML_SUBTITLING";
}

// ===========================================================================
//  Section 0x20 — TTML_subtitling_descriptor (EN 303 560 Table 1, §5.2.1.1)
// ---------------------------------------------------------------------------
//  Fixed lead-in, a profile array, optional qualifier(32), optional font list,
//  a length-delimited text field, then trailing reserved bytes. The profile
//  list, optional qualifier, font list, text and trailing reserved bytes are
//  kept raw (`tail`); the fixed lead-in is typed.
// ===========================================================================
/// TTML_subtitling body (EN 303 560 Table 1); `tail` is the raw remainder.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct TtmlSubtitling<'a> {
    /// ISO_639_language_code(24).
    pub iso_639_language_code: LangCode,
    /// subtitle_purpose(6) — Table 2.
    pub subtitle_purpose: u8,
    /// TTS_suitability(2) — Table 3.
    pub tts_suitability: u8,
    /// essential_font_usage_flag(1).
    pub essential_font_usage_flag: bool,
    /// qualifier_present_flag(1).
    pub qualifier_present_flag: bool,
    /// dvb_ttml_profile_count(4).
    pub dvb_ttml_profile_count: u8,
    /// Raw remainder: profile list + optional qualifier + font list + text + reserved.
    pub tail: &'a [u8],
}

impl<'a> Parse<'a> for TtmlSubtitling<'a> {
    type Error = crate::error::Error;
    fn parse(sel: &'a [u8]) -> Result<Self> {
        if sel.len() < TTML_FIXED_LEN {
            return Err(invalid("TTML_subtitling: header truncated"));
        }
        let b3 = sel[ISO_639_LEN];
        let b4 = sel[ISO_639_LEN + 1];
        Ok(TtmlSubtitling {
            iso_639_language_code: LangCode([sel[0], sel[1], sel[2]]),
            subtitle_purpose: b3 >> 2,
            tts_suitability: b3 & 0x03,
            essential_font_usage_flag: (b4 & 0x80) != 0,
            qualifier_present_flag: (b4 & 0x40) != 0,
            dvb_ttml_profile_count: b4 & 0x0F,
            tail: &sel[TTML_FIXED_LEN..],
        })
    }
}

impl Serialize for TtmlSubtitling<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        TTML_FIXED_LEN + self.tail.len()
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
        buf[ISO_639_LEN] = (self.subtitle_purpose << 2) | (self.tts_suitability & 0x03);
        buf[ISO_639_LEN + 1] = (u8::from(self.essential_font_usage_flag) << 7)
            | (u8::from(self.qualifier_present_flag) << 6)
            | (self.dvb_ttml_profile_count & 0x0F);
        buf[TTML_FIXED_LEN..len].copy_from_slice(self.tail);
        Ok(len)
    }
}
