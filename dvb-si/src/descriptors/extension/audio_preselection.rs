//! Audio Preselection Descriptor — ETSI EN 300 468 §6.4.1 (tag_extension 0x19).
//!
//! The Chinese multi-audio descriptor of GY/T 422-2025 clause 6 (Table 1) is a
//! wire-compatible subset of this descriptor (tag 0x7F, tag_extension 0x19): its
//! per-preselection flag byte carries `reserved_zero_future_use` (4 bits) where
//! DVB has `audio_description`/`spoken_subtitles`/`dialogue_enhancement`/
//! `interactivity_enabled`, and `reserved_zero_future_use` (1 bit) where DVB has
//! `text_label_present`; `language_code_present`, `multi_stream_info_present`
//! and `future_extension` and the optional fields are identical. The same parser
//! therefore reads it; [`Preselection::is_gy_t_422_profile`] tells whether an
//! entry stays within the Chinese subset. GY/T 422 Table 2
//! (`audio_rendering_indication`) matches EN 300 468 Table 111.
use super::*;
use alloc::vec::Vec;

impl Preselection<'_> {
    /// Whether this entry stays inside the GY/T 422-2025 Table 1 subset: none of
    /// the DVB-only flags (`audio_description`, `spoken_subtitles`,
    /// `dialogue_enhancement`, `interactivity_enabled`) and no `message_id`
    /// (`text_label_present`), which GY/T 422 defines as reserved zero.
    #[must_use]
    pub const fn is_gy_t_422_profile(&self) -> bool {
        !self.audio_description
            && !self.spoken_subtitles
            && !self.dialogue_enhancement
            && !self.interactivity_enabled
            && self.message_id.is_none()
    }
}

impl<'a> ExtensionBodyDef<'a> for AudioPreselection<'a> {
    const TAG_EXTENSION: u8 = 0x19;
    const NAME: &'static str = "AUDIO_PRESELECTION";
}
/// Spec default of every `reserved_zero_future_use` field of the
/// audio_preselection_descriptor (3-bit header, 5-bit after the aux
/// component tags count, 3-bit before `future_extension_length`): 0 since the
/// descriptor's introduction in EN 300 468 V1.16.1
/// (`dvb-si/docs/reserved-bits-edition-audit.md` §6).
pub const AUDIO_PRESELECTION_RESERVED_ZERO_FUTURE_USE_DEFAULT: u8 = 0x00;
const RESERVED_ZERO_3_MASK: u8 = 0x07;
const RESERVED_ZERO_5_MASK: u8 = 0x1F;

/// `audio_rendering_indication` (3 bits) — EN 300 468 Table 111 /
/// GY/T 422-2025 Table 2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum AudioRenderingIndication {
    /// 0 — no preference given for the reproduction channel layout.
    #[default]
    NoPreference,
    /// 1 — preferred layout is stereo.
    Stereo,
    /// 2 — preferred layout is two-dimensional (e.g. 5.1 multi-channel).
    TwoDimensional,
    /// 3 — preferred layout is three-dimensional (GY/T 422: 5.1.4).
    ThreeDimensional,
    /// 4 — content is pre-rendered for headphones.
    Headphones,
    /// 5..=7 — reserved; carries the value.
    Reserved(u8),
}

impl AudioRenderingIndication {
    /// Decode from the 3-bit wire value (lossless).
    #[must_use]
    pub const fn from_u8(v: u8) -> Self {
        match v & 0x07 {
            0 => Self::NoPreference,
            1 => Self::Stereo,
            2 => Self::TwoDimensional,
            3 => Self::ThreeDimensional,
            4 => Self::Headphones,
            other => Self::Reserved(other),
        }
    }

    /// The 3-bit wire value.
    #[must_use]
    pub const fn to_u8(self) -> u8 {
        match self {
            Self::NoPreference => 0,
            Self::Stereo => 1,
            Self::TwoDimensional => 2,
            Self::ThreeDimensional => 3,
            Self::Headphones => 4,
            Self::Reserved(v) => v,
        }
    }

    /// Spec label (Table 111).
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::NoPreference => "no preference",
            Self::Stereo => "stereo",
            Self::TwoDimensional => "two-dimensional",
            Self::ThreeDimensional => "three-dimensional",
            Self::Headphones => "headphones",
            Self::Reserved(_) => "reserved",
        }
    }
}
broadcast_common::impl_spec_display!(AudioRenderingIndication, Reserved);

/// audio_preselection body (Table 110, §6.4.1). The preselection loop is unfolded.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
#[non_exhaustive]
pub struct AudioPreselection<'a> {
    /// `reserved_zero_future_use` (3 bits, right-aligned) after
    /// `num_preselections`. Preserved on parse; defaults to
    /// [`AUDIO_PRESELECTION_RESERVED_ZERO_FUTURE_USE_DEFAULT`].
    pub reserved_zero_future_use_3: u8,
    /// Preselection entries (num_preselections is their count).
    pub preselections: Vec<Preselection<'a>>,
}

/// A single preselection entry in the audio_preselection_descriptor loop
/// (Table 110, §6.4.1).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
#[non_exhaustive]
pub struct Preselection<'a> {
    /// preselection_id(5).
    pub preselection_id: u8,
    /// audio_rendering_indication(3) — Table 111.
    pub audio_rendering_indication: AudioRenderingIndication,
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
    /// `reserved_zero_future_use` (5 bits, right-aligned) next to
    /// `num_aux_components`; `Some` iff `aux_component_tags` is. Preserved on
    /// parse; a hand-built `None` serializes as
    /// [`AUDIO_PRESELECTION_RESERVED_ZERO_FUTURE_USE_DEFAULT`].
    pub reserved_zero_future_use_5: Option<u8>,
    /// future_extension_byte run, present iff future_extension.
    pub future_extension: Option<&'a [u8]>,
    /// `reserved_zero_future_use` (3 bits, right-aligned) before
    /// `future_extension_length`; `Some` iff `future_extension` is. Preserved on
    /// parse; a hand-built `None` serializes as
    /// [`AUDIO_PRESELECTION_RESERVED_ZERO_FUTURE_USE_DEFAULT`].
    pub reserved_zero_future_use_3: Option<u8>,
}

impl<'a> Parse<'a> for AudioPreselection<'a> {
    type Error = crate::error::Error;
    fn parse(sel: &'a [u8]) -> Result<Self> {
        if sel.is_empty() {
            return Err(Error::BufferTooShort {
                need: 1,
                have: sel.len(),
                what: "audio_preselection body",
            });
        }
        let num_preselections = sel[0] >> 3;
        let header_reserved = sel[0] & RESERVED_ZERO_3_MASK;
        let mut pos = 1;
        let mut preselections = Vec::with_capacity(num_preselections as usize);
        for _ in 0..num_preselections {
            if pos + 2 > sel.len() {
                return Err(Error::BufferTooShort {
                    need: pos + 2,
                    have: sel.len(),
                    what: "audio_preselection body",
                });
            }
            let byte_a = sel[pos];
            let byte_b = sel[pos + 1];
            pos += 2;
            let preselection_id = byte_a >> 3;
            let audio_rendering_indication = AudioRenderingIndication::from_u8(byte_a & 0x07);
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
                    return Err(Error::BufferTooShort {
                        need: pos + ISO_639_LEN,
                        have: sel.len(),
                        what: "audio_preselection body",
                    });
                }
                let lc = LangCode([sel[pos], sel[pos + 1], sel[pos + 2]]);
                pos += ISO_639_LEN;
                Some(lc)
            } else {
                None
            };

            let message_id = if text_label_present {
                if pos >= sel.len() {
                    return Err(Error::BufferTooShort {
                        need: pos + 1,
                        have: sel.len(),
                        what: "audio_preselection body",
                    });
                }
                let id = sel[pos];
                pos += 1;
                Some(id)
            } else {
                None
            };

            let mut reserved_zero_future_use_5 = None;
            let mut reserved_zero_future_use_3 = None;
            let aux_component_tags = if multi_stream_info_present {
                if pos >= sel.len() {
                    return Err(Error::BufferTooShort {
                        need: pos + 1,
                        have: sel.len(),
                        what: "audio_preselection body",
                    });
                }
                let num_aux = sel[pos] >> 5;
                reserved_zero_future_use_5 = Some(sel[pos] & RESERVED_ZERO_5_MASK);
                pos += 1;
                if pos + num_aux as usize > sel.len() {
                    return Err(Error::BufferTooShort {
                        need: pos + num_aux as usize,
                        have: sel.len(),
                        what: "audio_preselection body",
                    });
                }
                let tags = &sel[pos..pos + num_aux as usize];
                pos += num_aux as usize;
                Some(tags)
            } else {
                None
            };

            let future_ext = if future_extension {
                if pos >= sel.len() {
                    return Err(Error::BufferTooShort {
                        need: pos + 1,
                        have: sel.len(),
                        what: "audio_preselection body",
                    });
                }
                let ext_len = (sel[pos] & 0x1F) as usize;
                reserved_zero_future_use_3 = Some(sel[pos] >> 5);
                pos += 1;
                if pos + ext_len > sel.len() {
                    return Err(Error::BufferTooShort {
                        need: pos + ext_len,
                        have: sel.len(),
                        what: "audio_preselection body",
                    });
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
                reserved_zero_future_use_5,
                future_extension: future_ext,
                reserved_zero_future_use_3,
            });
        }
        Ok(AudioPreselection {
            reserved_zero_future_use_3: header_reserved,
            preselections,
        })
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
        let num_preselections = broadcast_common::len::fit_bits(
            self.preselections.len() as u64,
            5,
            "num_preselections",
        )? as u8;
        buf[0] = (num_preselections << 3)
            | broadcast_common::len::fit_mask(
                self.reserved_zero_future_use_3,
                RESERVED_ZERO_3_MASK,
                "reserved_zero_future_use_3",
            )?;
        let mut p = 1;
        for s in &self.preselections {
            buf[p] = (broadcast_common::len::fit_mask(s.preselection_id, 0x1F, "preselection_id")?
                << 3)
                | broadcast_common::len::fit_mask(
                    s.audio_rendering_indication.to_u8(),
                    0x07,
                    "audio_rendering_indication",
                )?;
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
                let num_aux_component_tags = broadcast_common::len::fit_bits(
                    tags.len() as u64,
                    3,
                    "num_aux_component_tags",
                )? as u8;
                buf[p] = (num_aux_component_tags << 5)
                    | broadcast_common::len::fit_mask(
                        s.reserved_zero_future_use_5
                            .unwrap_or(AUDIO_PRESELECTION_RESERVED_ZERO_FUTURE_USE_DEFAULT),
                        RESERVED_ZERO_5_MASK,
                        "reserved_zero_future_use_5",
                    )?;
                p += 1;
                buf[p..p + tags.len()].copy_from_slice(tags);
                p += tags.len();
            }
            if let Some(ext) = s.future_extension {
                buf[p] = (broadcast_common::len::fit_bits(
                    ext.len() as u64,
                    5,
                    "future_extension_length",
                )? as u8)
                    | (broadcast_common::len::fit_mask(
                        s.reserved_zero_future_use_3
                            .unwrap_or(AUDIO_PRESELECTION_RESERVED_ZERO_FUTURE_USE_DEFAULT),
                        RESERVED_ZERO_3_MASK,
                        "reserved_zero_future_use_3",
                    )? << 5);
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
    use alloc::string::ToString;

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
                assert_eq!(
                    p0.audio_rendering_indication,
                    AudioRenderingIndication::TwoDimensional
                );
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
                assert_eq!(
                    p1.audio_rendering_indication,
                    AudioRenderingIndication::ThreeDimensional
                );
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
                assert_eq!(
                    p0.audio_rendering_indication,
                    AudioRenderingIndication::TwoDimensional
                );
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
                assert_eq!(
                    p1.audio_rendering_indication,
                    AudioRenderingIndication::ThreeDimensional
                );
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

    fn minimal_preselection() -> Preselection<'static> {
        Preselection::default()
    }

    /// Every value of each of the three reserved_zero_future_use regions
    /// (3-bit header, 5-bit aux, 3-bit future-extension) re-serializes
    /// byte-identically.
    #[test]
    fn audio_preselection_reserved_preserved() {
        for r in 0u8..=RESERVED_ZERO_3_MASK {
            let bytes = wrap(0x19, &[(1 << 3) | r, 0x08, 0x00]);
            let d = ExtensionDescriptor::parse(&bytes).unwrap();
            let mut buf = vec![0u8; d.serialized_len()];
            d.serialize_into(&mut buf).unwrap();
            assert_eq!(buf, bytes, "header r={r:#x}");
        }
        for r in 0u8..=RESERVED_ZERO_5_MASK {
            // multi_stream_info_present, 1 aux tag
            let bytes = wrap(0x19, &[1 << 3, 0x08, 0x02, (1 << 5) | r, 0x77]);
            let d = ExtensionDescriptor::parse(&bytes).unwrap();
            let mut buf = vec![0u8; d.serialized_len()];
            d.serialize_into(&mut buf).unwrap();
            assert_eq!(buf, bytes, "aux r={r:#x}");
        }
        for r in 0u8..=RESERVED_ZERO_3_MASK {
            // future_extension present, 2 bytes
            let bytes = wrap(0x19, &[1 << 3, 0x08, 0x01, (r << 5) | 2, 0xAA, 0xBB]);
            let d = ExtensionDescriptor::parse(&bytes).unwrap();
            let mut buf = vec![0u8; d.serialized_len()];
            d.serialize_into(&mut buf).unwrap();
            assert_eq!(buf, bytes, "future r={r:#x}");
        }
    }

    #[test]
    fn audio_preselection_default_is_spec_value() {
        let p = Preselection {
            aux_component_tags: Some(&[0x01]),
            future_extension: Some(&[0xAA]),
            ..Default::default()
        };
        let d = AudioPreselection {
            preselections: vec![p],
            ..Default::default()
        };
        let mut buf = vec![0u8; d.serialized_len()];
        d.serialize_into(&mut buf).unwrap();
        // header 1<<3 | 0; flags multi(0x02)|future(0x01); aux 1<<5; ext len 1.
        assert_eq!(buf, [1 << 3, 0, 0x03, 1 << 5, 0x01, 0x01, 0xAA]);
    }

    /// `num_preselections` is a 5-bit field (max 31, #1129): 31 entries still
    /// serialize; 32 must be rejected instead of silently wrapping the count.
    #[test]
    fn num_preselections_boundary() {
        let ok = AudioPreselection {
            preselections: (0..31).map(|_| minimal_preselection()).collect(),
            ..Default::default()
        };
        let mut buf = vec![0u8; ok.serialized_len()];
        ok.serialize_into(&mut buf).unwrap();
        assert_eq!(buf[0] >> 3, 31);

        let over = AudioPreselection {
            preselections: (0..32).map(|_| minimal_preselection()).collect(),
            ..Default::default()
        };
        let mut buf = vec![0u8; over.serialized_len()];
        let err = over.serialize_into(&mut buf).unwrap_err();
        assert!(
            matches!(err, Error::FieldOverflow(_)),
            "expected FieldOverflow, got {err:?}"
        );
    }

    /// GY/T 422-2025 Table 1 (clause 6) worked through the Chinese syntax: one
    /// preselection, id 3, `audio_rendering_indication` 2 (surround 5.1, Table 2),
    /// the 4 reserved flag bits and the reserved bit zero, `language_code_present`
    /// = 1, `multi_stream_info_present` = 1 with two aux `component_tag`s,
    /// `future_extension` = 0.
    #[test]
    fn gy_t_422_table_1_vector_round_trips() {
        // num_preselections(5)=1, reserved(3)=0
        let num = 1u8 << 3;
        let a = (3u8 << 3) | 2; // preselection_id=3, rendering=2
        // reserved(4)=0, language_code_present=1, reserved(1)=0, multi=1, future_ext=0
        let b = 0b0000_1010u8;
        let mut sel = vec![num, a, b];
        sel.extend_from_slice(b"chi"); // ISO_639_language_code
        sel.push(2u8 << 5); // num_aux_components=2, reserved(5)=0
        sel.extend_from_slice(&[0x11, 0x12]); // component_tag x2
        let bytes = wrap(0x19, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::AudioPreselection(body) => {
                assert_eq!(body.preselections.len(), 1);
                let p = &body.preselections[0];
                assert_eq!(p.preselection_id, 3);
                assert_eq!(
                    p.audio_rendering_indication,
                    AudioRenderingIndication::TwoDimensional
                );
                assert_eq!(p.language_code, Some(LangCode(*b"chi")));
                assert_eq!(p.aux_component_tags, Some(&[0x11u8, 0x12][..]));
                assert_eq!(p.future_extension, None);
                assert!(p.is_gy_t_422_profile());
            }
            other => panic!("expected AudioPreselection, got {other:?}"),
        }
        let mut out = vec![0u8; d.serialized_len()];
        let n = d.serialize_into(&mut out).unwrap();
        assert_eq!(&out[..n], &bytes[..], "byte-exact round trip");
    }

    /// GY/T 422 Table 2 values 0..=4 and reserved 5..=7, shared with Table 111.
    #[test]
    fn audio_rendering_indication_is_lossless_and_labelled() {
        for v in 0u8..8 {
            assert_eq!(AudioRenderingIndication::from_u8(v).to_u8(), v);
        }
        assert_eq!(AudioRenderingIndication::Headphones.name(), "headphones");
        assert_eq!(
            AudioRenderingIndication::from_u8(6).to_string(),
            "reserved(0x06)"
        );
    }

    /// An entry that uses a DVB-only flag is outside the GY/T 422 subset.
    #[test]
    fn dvb_only_flags_leave_the_gy_t_422_profile() {
        let p = Preselection {
            audio_description: true,
            ..Preselection::default()
        };
        assert!(!p.is_gy_t_422_profile());
        assert!(Preselection::default().is_gy_t_422_profile());
    }
}
