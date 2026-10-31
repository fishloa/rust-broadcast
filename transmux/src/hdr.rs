//! HDR10 static metadata: the H.265 mastering display colour volume and
//! content light level SEI messages, and the ISOBMFF `mdcv` / `clli` boxes that
//! carry the same two payloads.
//!
//! - SEI syntax: ITU-T H.265 D.2.28 (payloadType 137) and D.2.35 (payloadType
//!   144), semantics D.3.28 / D.3.35; transcribed in
//!   `private/specs-verified/mpeg/hdr/h265-d2-28-mastering-display-syntax.md`,
//!   `h265-d2-35-content-light-level-syntax.md` and
//!   `h265-sei-payload-dispatch.md`.
//! - Box layout: `private/specs-verified/mpeg/hdr/isobmff-mdcv-clli-boxes.md`.
//!   ISO/IEC 14496-12 clause 12.1.5 is paywalled and not vendored; that file is
//!   reconstructed from free sources (AOM AV1-ISOBMFF binding, FFmpeg, GPAC) and
//!   checked against a GPAC-written fixture, so the box layout is high
//!   confidence but not verbatim ISO.
//!
//! The SEI payload and the box body are byte-identical (all big-endian), so one
//! type serves both; the box header is added by the sample-entry writer.

use crate::error::{Error, Result};
use crate::hevc_config::HEVCDecoderConfigurationRecord;
use alloc::vec::Vec;
use broadcast_common::{Parse, Serialize};

/// H.265 `PREFIX_SEI_NUT` (Table 7-1).
pub const HEVC_PREFIX_SEI_NUT: u8 = 39;
/// SEI `payloadType` of `mastering_display_colour_volume` (H.265 D.2.28).
pub const SEI_PAYLOAD_MASTERING_DISPLAY_COLOUR_VOLUME: u32 = 137;
/// SEI `payloadType` of `content_light_level_info` (H.265 D.2.35).
pub const SEI_PAYLOAD_CONTENT_LIGHT_LEVEL_INFO: u32 = 144;

/// FourCC of the mastering display colour volume box.
pub const MDCV_FOURCC: [u8; 4] = *b"mdcv";
/// FourCC of the content light level box.
pub const CLLI_FOURCC: [u8; 4] = *b"clli";

const HEVC_NAL_HEADER_LEN: usize = 2;
const CIE_PRIMARY_COUNT: usize = 3;
const MDCV_PAYLOAD_LEN: usize = 24;
const CLLI_PAYLOAD_LEN: usize = 4;

/// A CIE 1931 chromaticity coordinate pair in increments of 0.00002 (H.265
/// D.3.28; 50000 = 1.0).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChromaticityXy {
    /// `x`, 0..=50000.
    pub x: u16,
    /// `y`, 0..=50000.
    pub y: u16,
}

/// `mastering_display_colour_volume` (H.265 D.2.28) / ISOBMFF `mdcv` body.
///
/// `display_primaries` are in SEI order: index 0 green, 1 blue, 2 red.
/// Luminances are in units of 0.0001 cd/m2 (H.265 D.3.28).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MasteringDisplayColourVolume {
    /// `display_primaries_x[c]` / `display_primaries_y[c]` for c = 0..3.
    pub display_primaries: [ChromaticityXy; CIE_PRIMARY_COUNT],
    /// `white_point_x` / `white_point_y`.
    pub white_point: ChromaticityXy,
    /// `max_display_mastering_luminance`.
    pub max_display_mastering_luminance: u32,
    /// `min_display_mastering_luminance`.
    pub min_display_mastering_luminance: u32,
}

/// `content_light_level_info` (H.265 D.2.35) / ISOBMFF `clli` body, in cd/m2.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContentLightLevel {
    /// `max_content_light_level` (MaxCLL).
    pub max_content_light_level: u16,
    /// `max_pic_average_light_level` (MaxFALL).
    pub max_pic_average_light_level: u16,
}

fn be16(b: &[u8], at: usize) -> u16 {
    u16::from_be_bytes([b[at], b[at + 1]])
}

fn be32(b: &[u8], at: usize) -> u32 {
    u32::from_be_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

fn check_exact(bytes: &[u8], need: usize, what: &'static str) -> Result<()> {
    if bytes.len() < need {
        return Err(Error::BufferTooShort {
            need,
            have: bytes.len(),
            what,
        });
    }
    if bytes.len() > need {
        return Err(Error::InvalidValue {
            field: what,
            value: bytes.len() as u64,
            reason: "payload longer than the defined syntax",
        });
    }
    Ok(())
}

fn check_out(buf: &[u8], need: usize) -> Result<()> {
    if buf.len() < need {
        return Err(Error::OutputBufferTooSmall {
            need,
            have: buf.len(),
        });
    }
    Ok(())
}

impl<'a> Parse<'a> for MasteringDisplayColourVolume {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        check_exact(bytes, MDCV_PAYLOAD_LEN, "mdcv payload")?;
        let mut display_primaries = [ChromaticityXy { x: 0, y: 0 }; CIE_PRIMARY_COUNT];
        for (c, p) in display_primaries.iter_mut().enumerate() {
            p.x = be16(bytes, c * 4);
            p.y = be16(bytes, c * 4 + 2);
        }
        let wp_at = CIE_PRIMARY_COUNT * 4;
        Ok(Self {
            display_primaries,
            white_point: ChromaticityXy {
                x: be16(bytes, wp_at),
                y: be16(bytes, wp_at + 2),
            },
            max_display_mastering_luminance: be32(bytes, wp_at + 4),
            min_display_mastering_luminance: be32(bytes, wp_at + 8),
        })
    }
}

impl Serialize for MasteringDisplayColourVolume {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        MDCV_PAYLOAD_LEN
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        check_out(buf, MDCV_PAYLOAD_LEN)?;
        for (c, p) in self.display_primaries.iter().enumerate() {
            buf[c * 4..c * 4 + 2].copy_from_slice(&p.x.to_be_bytes());
            buf[c * 4 + 2..c * 4 + 4].copy_from_slice(&p.y.to_be_bytes());
        }
        let wp_at = CIE_PRIMARY_COUNT * 4;
        buf[wp_at..wp_at + 2].copy_from_slice(&self.white_point.x.to_be_bytes());
        buf[wp_at + 2..wp_at + 4].copy_from_slice(&self.white_point.y.to_be_bytes());
        buf[wp_at + 4..wp_at + 8]
            .copy_from_slice(&self.max_display_mastering_luminance.to_be_bytes());
        buf[wp_at + 8..wp_at + 12]
            .copy_from_slice(&self.min_display_mastering_luminance.to_be_bytes());
        Ok(MDCV_PAYLOAD_LEN)
    }
}

impl<'a> Parse<'a> for ContentLightLevel {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        check_exact(bytes, CLLI_PAYLOAD_LEN, "clli payload")?;
        Ok(Self {
            max_content_light_level: be16(bytes, 0),
            max_pic_average_light_level: be16(bytes, 2),
        })
    }
}

impl Serialize for ContentLightLevel {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        CLLI_PAYLOAD_LEN
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        check_out(buf, CLLI_PAYLOAD_LEN)?;
        buf[..2].copy_from_slice(&self.max_content_light_level.to_be_bytes());
        buf[2..4].copy_from_slice(&self.max_pic_average_light_level.to_be_bytes());
        Ok(CLLI_PAYLOAD_LEN)
    }
}

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
    /// messages (ISO/IEC 14496-15 restricts the `hvcC` SEI array to
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
    pub fn hevc_sei_nal_carries_hdr(nal: &[u8]) -> bool {
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
    pub fn sample_entry_boxes(&self) -> Result<Vec<([u8; 4], Vec<u8>)>> {
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
        boxes: impl IntoIterator<Item = (&'a [u8; 4], &'a [u8])>,
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
        assert!(MasteringDisplayColourVolume::parse(&MDCV_BODY[..23]).is_err());
        let mut long = MDCV_BODY.to_vec();
        long.push(0);
        assert!(MasteringDisplayColourVolume::parse(&long).is_err());
        assert!(ContentLightLevel::parse(&CLLI_BODY[..3]).is_err());
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
}
