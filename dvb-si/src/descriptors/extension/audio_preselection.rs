use super::*;

impl ExtensionBodyDef for AudioPreselection<'_> {
    const TAG_EXTENSION: u8 = 0x19;
    const NAME: &'static str = "AUDIO_PRESELECTION";
}

// ===========================================================================
//  Section 0x19 — audio_preselection_descriptor (Table 110, §6.4.1)
// ---------------------------------------------------------------------------
//  num_preselections(5) | reserved_zero_future_use(3), then a preselection
//  loop whose entries carry conditional language / message / aux-component /
//  future-extension fields. The loop is fully unfolded into typed entries.
// ===========================================================================
/// audio_preselection body (Table 110, §6.4.1). The preselection loop is unfolded.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct AudioPreselection<'a> {
    /// Preselection entries (num_preselections is their count).
    pub preselections: Vec<Preselection<'a>>,
}

/// A single preselection entry in the audio_preselection_descriptor loop
/// (Table 110, §6.4.1).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct Preselection<'a> {
    /// preselection_id(5).
    pub preselection_id: u8,
    /// audio_rendering_indication(3) — Table 111.
    pub audio_rendering_indication: u8,
    /// audio_description(1).
    pub audio_description: bool,
    /// spoken_subtitles(1).
    pub spoken_subtitles: bool,
    /// dialogue_enhancement(1).
    pub dialogue_enhancement: bool,
    /// interactivity_enabled(1).
    pub interactivity_enabled: bool,
    /// ISO_639_language_code(24), present iff language_code_present.
    pub language_code: Option<LangCode>,
    /// message_id(8), present iff text_label_present.
    pub message_id: Option<u8>,
    /// component_tag bytes (num_aux_components of them), present iff multi_stream_info_present.
    pub aux_component_tags: Option<&'a [u8]>,
    /// future_extension_byte run, present iff future_extension.
    pub future_extension: Option<&'a [u8]>,
}

impl<'a> Parse<'a> for AudioPreselection<'a> {
    type Error = crate::error::Error;
    fn parse(sel: &'a [u8]) -> Result<Self> {
        if sel.is_empty() {
            return Err(invalid("audio_preselection: count byte missing"));
        }
        let num_preselections = sel[0] >> 3;
        let mut pos = 1;
        let mut preselections = Vec::with_capacity(num_preselections as usize);
        for _ in 0..num_preselections {
            if pos + 2 > sel.len() {
                return Err(invalid("audio_preselection: preselection overruns body"));
            }
            let byte_a = sel[pos];
            let byte_b = sel[pos + 1];
            pos += 2;
            let preselection_id = byte_a >> 3;
            let audio_rendering_indication = byte_a & 0x07;
            let audio_description = (byte_b >> 7) & 1 != 0;
            let spoken_subtitles = (byte_b >> 6) & 1 != 0;
            let dialogue_enhancement = (byte_b >> 5) & 1 != 0;
            let interactivity_enabled = (byte_b >> 4) & 1 != 0;
            let language_code_present = (byte_b >> 3) & 1 != 0;
            let text_label_present = (byte_b >> 2) & 1 != 0;
            let multi_stream_info_present = (byte_b >> 1) & 1 != 0;
            let future_extension = byte_b & 1 != 0;

            let language_code = if language_code_present {
                if pos + ISO_639_LEN > sel.len() {
                    return Err(invalid("audio_preselection: preselection overruns body"));
                }
                let lc = LangCode([sel[pos], sel[pos + 1], sel[pos + 2]]);
                pos += ISO_639_LEN;
                Some(lc)
            } else {
                None
            };

            let message_id = if text_label_present {
                if pos >= sel.len() {
                    return Err(invalid("audio_preselection: preselection overruns body"));
                }
                let id = sel[pos];
                pos += 1;
                Some(id)
            } else {
                None
            };

            let aux_component_tags = if multi_stream_info_present {
                if pos >= sel.len() {
                    return Err(invalid("audio_preselection: preselection overruns body"));
                }
                let num_aux = sel[pos] >> 5;
                pos += 1;
                if pos + num_aux as usize > sel.len() {
                    return Err(invalid("audio_preselection: preselection overruns body"));
                }
                let tags = &sel[pos..pos + num_aux as usize];
                pos += num_aux as usize;
                Some(tags)
            } else {
                None
            };

            let future_ext = if future_extension {
                if pos >= sel.len() {
                    return Err(invalid("audio_preselection: preselection overruns body"));
                }
                let ext_len = (sel[pos] & 0x1F) as usize;
                pos += 1;
                if pos + ext_len > sel.len() {
                    return Err(invalid("audio_preselection: preselection overruns body"));
                }
                let ext = &sel[pos..pos + ext_len];
                pos += ext_len;
                Some(ext)
            } else {
                None
            };

            preselections.push(Preselection {
                preselection_id,
                audio_rendering_indication,
                audio_description,
                spoken_subtitles,
                dialogue_enhancement,
                interactivity_enabled,
                language_code,
                message_id,
                aux_component_tags,
                future_extension: future_ext,
            });
        }
        Ok(AudioPreselection { preselections })
    }
}

impl Serialize for AudioPreselection<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        let body: usize = self
            .preselections
            .iter()
            .map(|p| {
                2 + p.language_code.map_or(0, |_| ISO_639_LEN)
                    + p.message_id.map_or(0, |_| 1)
                    + p.aux_component_tags.map_or(0, |t| 1 + t.len())
                    + p.future_extension.map_or(0, |e| 1 + e.len())
            })
            .sum();
        1 + body
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        buf[0] = ((self.preselections.len() as u8) & 0x1F) << 3;
        let mut p = 1;
        for s in &self.preselections {
            buf[p] = ((s.preselection_id & 0x1F) << 3) | (s.audio_rendering_indication & 0x07);
            buf[p + 1] = (u8::from(s.audio_description) << 7)
                | (u8::from(s.spoken_subtitles) << 6)
                | (u8::from(s.dialogue_enhancement) << 5)
                | (u8::from(s.interactivity_enabled) << 4)
                | (u8::from(s.language_code.is_some()) << 3)
                | (u8::from(s.message_id.is_some()) << 2)
                | (u8::from(s.aux_component_tags.is_some()) << 1)
                | u8::from(s.future_extension.is_some());
            p += 2;
            if let Some(ref lc) = s.language_code {
                buf[p..p + ISO_639_LEN].copy_from_slice(&lc.0);
                p += ISO_639_LEN;
            }
            if let Some(id) = s.message_id {
                buf[p] = id;
                p += 1;
            }
            if let Some(tags) = s.aux_component_tags {
                buf[p] = ((tags.len() as u8) & 0x07) << 5;
                p += 1;
                buf[p..p + tags.len()].copy_from_slice(tags);
                p += tags.len();
            }
            if let Some(ext) = s.future_extension {
                buf[p] = ext.len() as u8 & 0x1F;
                p += 1;
                buf[p..p + ext.len()].copy_from_slice(ext);
                p += ext.len();
            }
        }
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
    fn parse_structured_round_trip_two_preselections() {
        let num_pre = 0x02 << 3;
        let pre0_a = (1 << 3) | 2; // id=1, rendering=2
        let pre0_b = (1 << 6) | (1 << 4); // spoken_subtitles=1, interactivity_enabled=1
        let pre1_a = (2 << 3) | 3; // id=2, rendering=3
                                   // all flags: ad=1, ss=0, de=1, ie=0, lang=1, text=1, multi=1, futext=1
        let pre1_b = (1 << 7) | (1 << 5) | (1 << 3) | (1 << 2) | (1 << 1) | 1;
        let lang = b"foo";
        let msg_id = 0xABu8;
        let aux_tags: &[u8] = &[0x01, 0x02];
        let aux_byte = ((aux_tags.len() as u8) & 0x07) << 5;
        let fut_ext: &[u8] = &[0x25, 0x89, 0x63, 0x21, 0x47];
        let fut_byte = fut_ext.len() as u8 & 0x1F;

        let mut sel = vec![num_pre];
        sel.push(pre0_a);
        sel.push(pre0_b);
        sel.push(pre1_a);
        sel.push(pre1_b);
        sel.extend_from_slice(lang);
        sel.push(msg_id);
        sel.push(aux_byte);
        sel.extend_from_slice(aux_tags);
        sel.push(fut_byte);
        sel.extend_from_slice(fut_ext);

        let bytes = wrap(0x19, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::AudioPreselection(b) => {
                assert_eq!(b.preselections.len(), 2);

                let p0 = &b.preselections[0];
                assert_eq!(p0.preselection_id, 1);
                assert_eq!(p0.audio_rendering_indication, 2);
                assert!(!p0.audio_description);
                assert!(p0.spoken_subtitles);
                assert!(!p0.dialogue_enhancement);
                assert!(p0.interactivity_enabled);
                assert_eq!(p0.language_code, None);
                assert_eq!(p0.message_id, None);
                assert_eq!(p0.aux_component_tags, None);
                assert_eq!(p0.future_extension, None);

                let p1 = &b.preselections[1];
                assert_eq!(p1.preselection_id, 2);
                assert_eq!(p1.audio_rendering_indication, 3);
                assert!(p1.audio_description);
                assert!(!p1.spoken_subtitles);
                assert!(p1.dialogue_enhancement);
                assert!(!p1.interactivity_enabled);
                assert_eq!(p1.language_code, Some(LangCode(*b"foo")));
                assert_eq!(p1.message_id, Some(0xAB));
                assert_eq!(p1.aux_component_tags, Some(aux_tags as &[u8]));
                assert_eq!(p1.future_extension, Some(fut_ext as &[u8]));
            }
            other => panic!("expected AudioPreselection, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn tsduck_byte_exact_test_015() {
        // TSDuck test-015 reference: 2 preselections — one minimal, one with all
        // optional fields present (lang "foo", message_id 0xAB, 2 aux tags, 5
        // future_extension bytes).
        let hex = "7f1319100a5013af666f6fab400102052589632147";
        let bytes = from_hex(hex);
        let d = ExtensionDescriptor::parse(&bytes).unwrap_or_else(|e| panic!("parse {hex}: {e:?}"));
        assert_eq!(
            d.kind(),
            Some(super::super::ExtensionTag::AudioPreselection)
        );
        match &d.body {
            ExtensionBody::AudioPreselection(b) => {
                assert_eq!(b.preselections.len(), 2);

                let p0 = &b.preselections[0];
                assert_eq!(p0.preselection_id, 1);
                assert_eq!(p0.audio_rendering_indication, 2);
                assert!(!p0.audio_description);
                assert!(p0.spoken_subtitles);
                assert!(!p0.dialogue_enhancement);
                assert!(p0.interactivity_enabled);
                assert_eq!(p0.language_code, None);
                assert_eq!(p0.message_id, None);
                assert_eq!(p0.aux_component_tags, None);
                assert_eq!(p0.future_extension, None);

                let p1 = &b.preselections[1];
                assert_eq!(p1.preselection_id, 2);
                assert_eq!(p1.audio_rendering_indication, 3);
                assert!(p1.audio_description);
                assert!(!p1.spoken_subtitles);
                assert!(p1.dialogue_enhancement);
                assert!(!p1.interactivity_enabled);
                assert_eq!(p1.language_code, Some(LangCode(*b"foo")));
                assert_eq!(p1.message_id, Some(0xAB));
                assert_eq!(p1.aux_component_tags, Some(&[0x01u8, 0x02u8][..]));
                assert_eq!(
                    p1.future_extension,
                    Some(&[0x25u8, 0x89, 0x63, 0x21, 0x47][..])
                );
            }
            other => panic!("expected AudioPreselection, got {other:?}"),
        }
        round_trip(&d);
    }
}
