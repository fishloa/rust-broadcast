//! HDR10 static metadata: the H.265 mastering display colour volume and
//! content light level SEI messages.
//!
//! - SEI syntax: ITU-T H.265 D.2.28 (payloadType 137) and D.2.35 (payloadType
//!   144), semantics D.3.28 / D.3.35; transcribed in
//!   `private/specs-verified/mpeg/hdr/h265-d2-28-mastering-display-syntax.md`,
//!   `h265-d2-35-content-light-level-syntax.md` and
//!   `h265-sei-payload-dispatch.md`.
//! - Box layout: the `mdcv` / `clli` body types live in [`crate::hdr_boxes`];
//!   the SEI payload and the box body are byte-identical, so one type serves
//!   both and the box header is added by the sample-entry writer.

use crate::error::Result;
use crate::hdr_boxes::{
    BOX_TYPE_LEN, CLLI_FOURCC, ContentLightLevel, MDCV_FOURCC, MasteringDisplayColourVolume,
};
use crate::hevc_config::HEVCDecoderConfigurationRecord;
use crate::nal::HEVC_NAL_HEADER_LEN;
use alloc::vec::Vec;
use broadcast_common::{Parse, Serialize};

pub const HEVC_PREFIX_SEI_NUT: u8 = 39;
/// SEI `payloadType` of `mastering_display_colour_volume` (H.265 D.2.28).
pub const SEI_PAYLOAD_MASTERING_DISPLAY_COLOUR_VOLUME: u32 = 137;
/// SEI `payloadType` of `content_light_level_info` (H.265 D.2.35).
pub const SEI_PAYLOAD_CONTENT_LIGHT_LEVEL_INFO: u32 = 144;

/// The static HDR10 metadata a video track carries: whichever of the two SEI
/// messages were seen (the first occurrence of each is kept).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HdrStaticMetadata {
    /// `mastering_display_colour_volume`, when signalled.
    pub mastering_display: Option<MasteringDisplayColourVolume>,
    /// `content_light_level_info`, when signalled.
    pub content_light_level: Option<ContentLightLevel>,
}

impl HdrStaticMetadata {
    /// No metadata present.
    pub fn is_empty(&self) -> bool {
        self.mastering_display.is_none() && self.content_light_level.is_none()
    }

    /// Fold the messages of one HEVC prefix SEI NAL unit (2-byte NAL header
    /// included) into `self`, keeping any value already present.
    pub fn absorb_hevc_sei_nal(&mut self, nal: &[u8]) {
        if nal.len() <= HEVC_NAL_HEADER_LEN
            || crate::nal::nal_unit_type(crate::nal::NalCodec::Hevc, nal)
                != Some(HEVC_PREFIX_SEI_NUT)
        {
            return;
        }
        for (payload_type, payload) in crate::nal::sei_messages(&nal[HEVC_NAL_HEADER_LEN..]) {
            match payload_type {
                SEI_PAYLOAD_MASTERING_DISPLAY_COLOUR_VOLUME if self.mastering_display.is_none() => {
                    self.mastering_display = MasteringDisplayColourVolume::parse(&payload).ok();
                }
                SEI_PAYLOAD_CONTENT_LIGHT_LEVEL_INFO if self.content_light_level.is_none() => {
                    self.content_light_level = ContentLightLevel::parse(&payload).ok();
                }
                _ => {}
            }
        }
    }

    /// A prefix SEI NAL unit holding only the declarative `mdcv` / `clli`
    /// messages (ISO/IEC 14496-15:2017 §8.3.3.1.1 restricts the `hvcC` SEI array to
    /// declarative messages); `None` when empty.
    pub fn to_hevc_declarative_sei_nal(&self) -> Result<Option<Vec<u8>>> {
        let mut payloads: Vec<(u32, Vec<u8>)> = Vec::new();
        if let Some(m) = &self.mastering_display {
            payloads.push((
                SEI_PAYLOAD_MASTERING_DISPLAY_COLOUR_VOLUME,
                m.try_to_bytes()?,
            ));
        }
        if let Some(c) = &self.content_light_level {
            payloads.push((SEI_PAYLOAD_CONTENT_LIGHT_LEVEL_INFO, c.try_to_bytes()?));
        }
        if payloads.is_empty() {
            return Ok(None);
        }
        let messages: Vec<(u32, &[u8])> =
            payloads.iter().map(|(t, p)| (*t, p.as_slice())).collect();
        Ok(Some(crate::nal::hevc_prefix_sei_nal(&messages)))
    }

    /// Whether a prefix SEI NAL unit carries either HDR static message.
    #[cfg(test)]
    pub(crate) fn hevc_sei_nal_carries_hdr(nal: &[u8]) -> bool {
        let mut m = Self::default();
        m.absorb_hevc_sei_nal(nal);
        !m.is_empty()
    }

    /// Metadata found in the prefix SEI NAL units of an `hvcC` record; `None`
    /// when there are none.
    pub fn from_hevc_record(record: &HEVCDecoderConfigurationRecord) -> Option<Self> {
        let mut m = Self::default();
        for array in &record.arrays {
            if array.nal_unit_type != HEVC_PREFIX_SEI_NUT {
                continue;
            }
            for nal in &array.nalus {
                m.absorb_hevc_sei_nal(&nal.0);
            }
        }
        (!m.is_empty()).then_some(m)
    }

    /// The `mdcv` / `clli` sample-entry child boxes as `(fourcc, body)`, in
    /// the order `mdcv` then `clli`.
    pub fn sample_entry_boxes(&self) -> Result<Vec<([u8; BOX_TYPE_LEN], Vec<u8>)>> {
        let mut out = Vec::new();
        if let Some(m) = &self.mastering_display {
            out.push((MDCV_FOURCC, m.try_to_bytes()?));
        }
        if let Some(c) = &self.content_light_level {
            out.push((CLLI_FOURCC, c.try_to_bytes()?));
        }
        Ok(out)
    }

    /// Read `mdcv` / `clli` bodies found among sample-entry child boxes
    /// (`(fourcc, body)`); the first of each wins. `None` when neither parses.
    pub fn from_sample_entry_boxes<'a>(
        boxes: impl IntoIterator<Item = (&'a [u8; BOX_TYPE_LEN], &'a [u8])>,
    ) -> Option<Self> {
        let mut m = Self::default();
        for (fourcc, body) in boxes {
            if *fourcc == MDCV_FOURCC && m.mastering_display.is_none() {
                m.mastering_display = MasteringDisplayColourVolume::parse(body).ok();
            } else if *fourcc == CLLI_FOURCC && m.content_light_level.is_none() {
                m.content_light_level = ContentLightLevel::parse(body).ok();
            }
        }
        (!m.is_empty()).then_some(m)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hdr_boxes::{CLLI_PAYLOAD_LEN, ChromaticityXy, MDCV_PAYLOAD_LEN};

    const MDCV_BODY: [u8; 24] = [
        0x33, 0xc2, 0x86, 0xc4, 0x1d, 0x4c, 0x0b, 0xb8, 0x84, 0xd0, 0x3e, 0x80, 0x3d, 0x13, 0x40,
        0x42, 0x00, 0x98, 0x96, 0x80, 0x00, 0x00, 0x00, 0x01,
    ];
    const CLLI_BODY: [u8; 4] = [0x03, 0xe8, 0x01, 0x90];

    #[test]
    fn mdcv_round_trips_and_decodes() {
        let m = MasteringDisplayColourVolume::parse(&MDCV_BODY).unwrap();
        assert_eq!(
            m.display_primaries[0],
            ChromaticityXy { x: 13250, y: 34500 }
        );
        assert_eq!(m.white_point, ChromaticityXy { x: 15635, y: 16450 });
        assert_eq!(m.max_display_mastering_luminance, 10_000_000);
        assert_eq!(m.min_display_mastering_luminance, 1);
        assert_eq!(m.try_to_bytes().unwrap(), MDCV_BODY);
        let mut m2 = m;
        m2.white_point.x += 1;
        assert_ne!(m2.try_to_bytes().unwrap(), MDCV_BODY);
    }

    #[test]
    fn clli_round_trips_and_decodes() {
        let c = ContentLightLevel::parse(&CLLI_BODY).unwrap();
        assert_eq!(c.max_content_light_level, 1000);
        assert_eq!(c.max_pic_average_light_level, 400);
        assert_eq!(c.try_to_bytes().unwrap(), CLLI_BODY);
    }

    #[test]
    fn wrong_lengths_are_rejected() {
        assert!(matches!(
            MasteringDisplayColourVolume::parse(&MDCV_BODY[..23]),
            Err(crate::error::Error::BufferTooShort(..))
        ));
        let mut long = MDCV_BODY.to_vec();
        long.push(0);
        assert!(matches!(
            MasteringDisplayColourVolume::parse(&long),
            Err(crate::error::Error::InvalidValue(..))
        ));
        assert!(matches!(
            ContentLightLevel::parse(&CLLI_BODY[..3]),
            Err(crate::error::Error::BufferTooShort(..))
        ));
    }

    #[test]
    fn serializing_into_a_short_buffer_is_an_error() {
        let m = MasteringDisplayColourVolume::parse(&MDCV_BODY).unwrap();
        let mut short = [0u8; MDCV_PAYLOAD_LEN - 1];
        assert!(matches!(
            m.serialize_into(&mut short),
            Err(crate::error::Error::OutputBufferTooSmall(..))
        ));
        let c = ContentLightLevel::parse(&CLLI_BODY).unwrap();
        let mut short = [0u8; CLLI_PAYLOAD_LEN - 1];
        assert!(matches!(
            c.serialize_into(&mut short),
            Err(crate::error::Error::OutputBufferTooSmall(..))
        ));
    }

    #[test]
    fn sei_nal_with_both_messages_is_absorbed() {
        let mut nal = alloc::vec![HEVC_PREFIX_SEI_NUT << 1, 1];
        nal.push(137);
        nal.push(24);
        nal.extend_from_slice(&MDCV_BODY);
        nal.push(144);
        nal.push(4);
        nal.extend_from_slice(&CLLI_BODY);
        nal.push(0x80);
        let mut m = HdrStaticMetadata::default();
        m.absorb_hevc_sei_nal(&nal);
        assert_eq!(
            m.mastering_display.unwrap().try_to_bytes().unwrap(),
            MDCV_BODY
        );
        assert_eq!(
            m.content_light_level.unwrap().try_to_bytes().unwrap(),
            CLLI_BODY
        );
        assert!(HdrStaticMetadata::hevc_sei_nal_carries_hdr(&nal));
    }

    #[test]
    fn unrelated_sei_is_ignored() {
        let nal = [HEVC_PREFIX_SEI_NUT << 1, 1, 5, 1, 0xAA, 0x80];
        assert!(!HdrStaticMetadata::hevc_sei_nal_carries_hdr(&nal));
    }

    #[test]
    fn declarative_sei_nal_drops_other_messages() {
        let mdcv = [0x11u8; MDCV_PAYLOAD_LEN];
        let clli = [0x03, 0xE8, 0x01, 0x90];
        let other = [0xA5u8; 6];
        let bundled = crate::nal::hevc_prefix_sei_nal(&[
            (SEI_PAYLOAD_MASTERING_DISPLAY_COLOUR_VOLUME, &mdcv),
            (4, &other),
            (SEI_PAYLOAD_CONTENT_LIGHT_LEVEL_INFO, &clli),
        ]);
        let mut m = HdrStaticMetadata::default();
        m.absorb_hevc_sei_nal(&bundled);
        assert!(m.mastering_display.is_some() && m.content_light_level.is_some());
        let rebuilt = m.to_hevc_declarative_sei_nal().unwrap().unwrap();
        let types: Vec<u32> = crate::nal::sei_messages(&rebuilt[HEVC_NAL_HEADER_LEN..])
            .into_iter()
            .map(|(t, _)| t)
            .collect();
        assert_eq!(
            types,
            [
                SEI_PAYLOAD_MASTERING_DISPLAY_COLOUR_VOLUME,
                SEI_PAYLOAD_CONTENT_LIGHT_LEVEL_INFO
            ]
        );
        assert!(
            HdrStaticMetadata::default()
                .to_hevc_declarative_sei_nal()
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn one_message_is_not_empty() {
        let mdcv_only = HdrStaticMetadata {
            mastering_display: Some(MasteringDisplayColourVolume::parse(&MDCV_BODY).unwrap()),
            content_light_level: None,
        };
        assert!(!mdcv_only.is_empty(), "only mastering_display present");
        let clli_only = HdrStaticMetadata {
            mastering_display: None,
            content_light_level: Some(ContentLightLevel::parse(&CLLI_BODY).unwrap()),
        };
        assert!(!clli_only.is_empty(), "only content_light_level present");
        assert!(HdrStaticMetadata::default().is_empty());
    }

    #[test]
    fn a_single_sample_entry_box_is_read() {
        let m = HdrStaticMetadata::from_sample_entry_boxes([(&MDCV_FOURCC, &MDCV_BODY[..])])
            .expect("a single mdcv box");
        assert_eq!(
            m.mastering_display.unwrap().try_to_bytes().unwrap(),
            MDCV_BODY
        );
        assert!(m.content_light_level.is_none());
        let c = HdrStaticMetadata::from_sample_entry_boxes([(&CLLI_FOURCC, &CLLI_BODY[..])])
            .expect("a single clli box");
        assert!(c.mastering_display.is_none());
        assert_eq!(
            c.content_light_level.unwrap().try_to_bytes().unwrap(),
            CLLI_BODY
        );
    }

    #[test]
    fn a_suffix_or_short_sei_nal_is_ignored() {
        // H.265 Table 7-1: `SUFFIX_SEI_NUT`.
        const HEVC_SUFFIX_SEI_NUT: u8 = 40;
        let mdcv = [0x11u8; MDCV_PAYLOAD_LEN];
        let mut suffix = crate::nal::hevc_prefix_sei_nal(&[(
            SEI_PAYLOAD_MASTERING_DISPLAY_COLOUR_VOLUME,
            &mdcv,
        )]);
        suffix[0] = HEVC_SUFFIX_SEI_NUT << 1;
        let mut m = HdrStaticMetadata::default();
        m.absorb_hevc_sei_nal(&suffix);
        assert!(m.is_empty(), "a suffix SEI NAL carries no prefix message");

        let mut m = HdrStaticMetadata::default();
        m.absorb_hevc_sei_nal(&[HEVC_PREFIX_SEI_NUT << 1, 1]);
        assert!(m.is_empty(), "a bare 2-byte NAL has no SEI body");
    }

    #[test]
    fn the_first_message_of_each_kind_is_kept() {
        const MDCV_OTHER: [u8; 24] = [0x22; 24];
        const CLLI_OTHER: [u8; 4] = [0x00, 0x64, 0x00, 0x32];
        let first = crate::nal::hevc_prefix_sei_nal(&[
            (SEI_PAYLOAD_MASTERING_DISPLAY_COLOUR_VOLUME, &MDCV_BODY),
            (SEI_PAYLOAD_CONTENT_LIGHT_LEVEL_INFO, &CLLI_BODY),
        ]);
        let second = crate::nal::hevc_prefix_sei_nal(&[
            (SEI_PAYLOAD_MASTERING_DISPLAY_COLOUR_VOLUME, &MDCV_OTHER),
            (SEI_PAYLOAD_CONTENT_LIGHT_LEVEL_INFO, &CLLI_OTHER),
        ]);
        let mut m = HdrStaticMetadata::default();
        m.absorb_hevc_sei_nal(&first);
        m.absorb_hevc_sei_nal(&second);
        assert_eq!(
            m.mastering_display.unwrap().try_to_bytes().unwrap(),
            MDCV_BODY,
            "the first mdcv message wins"
        );
        assert_eq!(
            m.content_light_level.unwrap().try_to_bytes().unwrap(),
            CLLI_BODY,
            "the first clli message wins"
        );

        let from_boxes = HdrStaticMetadata::from_sample_entry_boxes([
            (&MDCV_FOURCC, &MDCV_BODY[..]),
            (&MDCV_FOURCC, &MDCV_OTHER[..]),
            (&CLLI_FOURCC, &CLLI_BODY[..]),
            (&CLLI_FOURCC, &CLLI_OTHER[..]),
        ])
        .expect("four boxes");
        assert_eq!(
            from_boxes
                .mastering_display
                .unwrap()
                .try_to_bytes()
                .unwrap(),
            MDCV_BODY,
            "the first mdcv box wins"
        );
        assert_eq!(
            from_boxes
                .content_light_level
                .unwrap()
                .try_to_bytes()
                .unwrap(),
            CLLI_BODY,
            "the first clli box wins"
        );
    }
}
