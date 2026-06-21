//! DTS-HD Descriptor — ETSI EN 300 468 Annex G.3, Tables G.6–G.10 (tag_extension 0x0E).
use super::*;
use alloc::vec::Vec;

impl<'a> ExtensionBodyDef<'a> for DtsHd<'a> {
    const TAG_EXTENSION: u8 = 0x0E;
    const NAME: &'static str = "DTS_HD";
}

/// DTS-HD descriptor body (Table G.6, Annex G.3).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct DtsHd<'a> {
    /// `substream_core_flag`(1).
    pub substream_core_flag: bool,
    /// `substream_0_flag`(1).
    pub substream_0_flag: bool,
    /// `substream_1_flag`(1).
    pub substream_1_flag: bool,
    /// `substream_2_flag`(1).
    pub substream_2_flag: bool,
    /// `substream_3_flag`(1).
    pub substream_3_flag: bool,
    /// `reserved_future_use`(3) — preserved for byte-exact round-trip.
    pub reserved: u8,
    /// Substream info blocks, one per set flag (core first, then 0..3 in order).
    pub substreams: Vec<SubstreamInfo>,
    /// Optional `additional_info_byte` run.
    #[cfg_attr(feature = "serde", serde(borrow))]
    pub additional_info: &'a [u8],
}

/// `substream_info()` block (Table G.7).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SubstreamInfo {
    /// `channel_count`(5).
    pub channel_count: u8,
    /// `lfe_flag`(1).
    pub lfe_flag: bool,
    /// `sampling_frequency`(4) — coded per Table G.8.
    pub sampling_frequency: SamplingFrequency,
    /// `sample_resolution`(1) — '1' if decoded resolution > 16 bit.
    pub sample_resolution: bool,
    /// `reserved_future_use`(2) — preserved for byte-exact round-trip.
    pub reserved: u8,
    /// `asset_info()` entries (num_assets + 1).
    pub assets: Vec<AssetInfo>,
}

/// `asset_info()` block (Tables G.9, G.10).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct AssetInfo {
    /// `asset_construction`(5) — interpreted per Table G.10.
    pub asset_construction: u8,
    /// `vbr_flag`(1).
    pub vbr_flag: bool,
    /// `post_encode_br_scaling_flag`(1).
    pub post_encode_br_scaling_flag: bool,
    /// `component_type_flag`(1).
    pub component_type_flag: bool,
    /// `language_code_flag`(1).
    pub language_code_flag: bool,
    /// `bit_rate`(13) or `bit_rate_scaled`(13), selected by `post_encode_br_scaling_flag`.
    pub bit_rate_or_scaled: u16,
    /// `reserved_future_use`(2) — preserved for byte-exact round-trip.
    pub reserved: u8,
    /// `component_type`(8) — present iff `component_type_flag`.
    pub component_type: Option<u8>,
    /// `ISO_639_language_code`(24) — present iff `language_code_flag`.
    pub iso_639_language_code: Option<LangCode>,
}

/// `sampling_frequency`(4) — Table G.8.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
#[repr(u8)]
pub enum SamplingFrequency {
    /// 8 kHz.
    Khz8 = 0,
    /// 16 kHz.
    Khz16 = 1,
    /// 32 kHz.
    Khz32 = 2,
    /// 64 kHz.
    Khz64 = 3,
    /// 128 kHz (not for core).
    Khz128 = 4,
    /// 22,05 kHz.
    Khz22_05 = 5,
    /// 44,1 kHz.
    Khz44_1 = 6,
    /// 88,2 kHz.
    Khz88_2 = 7,
    /// 176,4 kHz (not for core).
    Khz176_4 = 8,
    /// 352,8 kHz (not for core).
    Khz352_8 = 9,
    /// 12 kHz.
    Khz12 = 10,
    /// 24 kHz.
    Khz24 = 11,
    /// 48 kHz.
    Khz48 = 12,
    /// 96 kHz.
    Khz96 = 13,
    /// 192 kHz (not for core).
    Khz192 = 14,
    /// 348 kHz (not for core).
    Khz348 = 15,
}

impl SamplingFrequency {
    /// Construct from a raw `u8`; total, lossless (4-bit field).
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v {
            0 => SamplingFrequency::Khz8,
            1 => SamplingFrequency::Khz16,
            2 => SamplingFrequency::Khz32,
            3 => SamplingFrequency::Khz64,
            4 => SamplingFrequency::Khz128,
            5 => SamplingFrequency::Khz22_05,
            6 => SamplingFrequency::Khz44_1,
            7 => SamplingFrequency::Khz88_2,
            8 => SamplingFrequency::Khz176_4,
            9 => SamplingFrequency::Khz352_8,
            10 => SamplingFrequency::Khz12,
            11 => SamplingFrequency::Khz24,
            12 => SamplingFrequency::Khz48,
            13 => SamplingFrequency::Khz96,
            14 => SamplingFrequency::Khz192,
            15 => SamplingFrequency::Khz348,
            _ => SamplingFrequency::Khz48, // unreachable for 4-bit
        }
    }

    /// Inverse of `from_u8`.
    #[must_use]
    pub const fn to_u8(self) -> u8 {
        self as u8
    }

    /// Human-readable spec name per Table G.8.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            SamplingFrequency::Khz8 => "8 kHz",
            SamplingFrequency::Khz16 => "16 kHz",
            SamplingFrequency::Khz32 => "32 kHz",
            SamplingFrequency::Khz64 => "64 kHz",
            SamplingFrequency::Khz128 => "128 kHz",
            SamplingFrequency::Khz22_05 => "22,05 kHz",
            SamplingFrequency::Khz44_1 => "44,1 kHz",
            SamplingFrequency::Khz88_2 => "88,2 kHz",
            SamplingFrequency::Khz176_4 => "176,4 kHz",
            SamplingFrequency::Khz352_8 => "352,8 kHz",
            SamplingFrequency::Khz12 => "12 kHz",
            SamplingFrequency::Khz24 => "24 kHz",
            SamplingFrequency::Khz48 => "48 kHz",
            SamplingFrequency::Khz96 => "96 kHz",
            SamplingFrequency::Khz192 => "192 kHz",
            SamplingFrequency::Khz348 => "348 kHz",
        }
    }
}
dvb_common::impl_spec_display!(SamplingFrequency);

/// Maximum `channel_count` (5-bit field).
const MAX_CHANNEL_COUNT: u8 = 0x1F;
/// Maximum bit-rate value (13-bit field).
const MAX_BIT_RATE: u16 = 0x1FFF;

/// Substream info header size (excluding `asset_info()` blocks):
/// `substream_length`(1) + packed(1) + packed(1) = 3 bytes.
const SUBSTREAM_HEADER_LEN: usize = 3;
/// Asset info fixed length (excluding optional `component_type` and
/// `ISO_639_language_code`): `asset_construction`+flags(1) + br(2) = 3 bytes.
const ASSET_FIXED_LEN: usize = 3;

impl Serialize for SubstreamInfo {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        let payload_len: usize = self
            .assets
            .iter()
            .map(|a| {
                ASSET_FIXED_LEN
                    + usize::from(a.component_type_flag)
                    + usize::from(a.language_code_flag) * ISO_639_LEN
            })
            .sum();
        SUBSTREAM_HEADER_LEN + payload_len
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let payload_len = self.serialized_len() - SUBSTREAM_HEADER_LEN;
        let substream_length = payload_len;
        if substream_length > 0xFF {
            return Err(Error::ValueOutOfRange {
                field: "substream_length",
                reason: "substream payload exceeds 255 bytes",
            });
        }
        if self.channel_count > MAX_CHANNEL_COUNT {
            return Err(Error::ValueOutOfRange {
                field: "channel_count",
                reason: "exceeds 5-bit field",
            });
        }
        buf[0] = substream_length as u8;
        let num_assets = self.assets.len();
        if num_assets == 0 {
            return Err(Error::ValueOutOfRange {
                field: "num_assets",
                reason: "substream must have at least 1 asset",
            });
        }
        let na = (num_assets - 1) as u8;
        buf[1] = (na << 5) | (self.channel_count & MAX_CHANNEL_COUNT);
        buf[2] = ((u8::from(self.lfe_flag)) << 7)
            | ((self.sampling_frequency.to_u8() & 0x0F) << 3)
            | ((u8::from(self.sample_resolution)) << 2)
            | (self.reserved & 0x03);
        let mut pos = SUBSTREAM_HEADER_LEN;
        for a in &self.assets {
            if a.bit_rate_or_scaled > MAX_BIT_RATE {
                return Err(Error::ValueOutOfRange {
                    field: "bit_rate_or_scaled",
                    reason: "exceeds 13-bit field",
                });
            }
            buf[pos] = (a.asset_construction << 3)
                | ((u8::from(a.vbr_flag)) << 2)
                | ((u8::from(a.post_encode_br_scaling_flag)) << 1)
                | u8::from(a.component_type_flag);
            buf[pos + 1] =
                (u8::from(a.language_code_flag) << 7) | ((a.bit_rate_or_scaled >> 6) as u8 & 0x7F);
            buf[pos + 2] = ((a.bit_rate_or_scaled as u8 & 0x3F) << 2) | (a.reserved & 0x03);
            pos += ASSET_FIXED_LEN;
            if a.component_type_flag {
                buf[pos] = a.component_type.unwrap_or(0);
                pos += 1;
            }
            if a.language_code_flag {
                buf[pos..pos + ISO_639_LEN]
                    .copy_from_slice(&a.iso_639_language_code.unwrap_or(LangCode(*b"   ")).0);
                pos += ISO_639_LEN;
            }
        }
        Ok(pos)
    }
}

impl<'a> Parse<'a> for DtsHd<'a> {
    type Error = crate::error::Error;
    fn parse(sel: &'a [u8]) -> Result<Self> {
        let first = *sel.first().ok_or(Error::BufferTooShort {
            need: 1,
            have: 0,
            what: "DTS-HD descriptor body",
        })?;
        let substream_core_flag = (first & 0x80) != 0;
        let substream_0_flag = (first & 0x40) != 0;
        let substream_1_flag = (first & 0x20) != 0;
        let substream_2_flag = (first & 0x10) != 0;
        let substream_3_flag = (first & 0x08) != 0;
        let reserved = first & 0x07;
        let flags = [
            substream_core_flag,
            substream_0_flag,
            substream_1_flag,
            substream_2_flag,
            substream_3_flag,
        ];
        let num_substreams = flags.iter().filter(|&&f| f).count();
        let mut pos = 1;
        let mut substreams = Vec::with_capacity(num_substreams);

        for &present in &flags {
            if !present {
                continue;
            }
            let si = parse_substream_info(sel, &mut pos)?;
            substreams.push(si);
        }

        Ok(DtsHd {
            substream_core_flag,
            substream_0_flag,
            substream_1_flag,
            substream_2_flag,
            substream_3_flag,
            reserved,
            substreams,
            additional_info: &sel[pos..],
        })
    }
}

fn parse_substream_info(sel: &[u8], pos: &mut usize) -> Result<SubstreamInfo> {
    let need = *pos + SUBSTREAM_HEADER_LEN;
    if sel.len() < need {
        return Err(Error::BufferTooShort {
            need,
            have: sel.len(),
            what: "substream_info header",
        });
    }
    let substream_length = sel[*pos] as usize;
    *pos += 1;
    let end = *pos + substream_length;
    if sel.len() < end {
        return Err(Error::BufferTooShort {
            need: end,
            have: sel.len(),
            what: "substream_info body",
        });
    }
    let num_assets = ((sel[*pos] >> 5) & 0x07) as usize + 1;
    let channel_count = sel[*pos] & 0x1F;
    *pos += 1;
    let lfe_flag = (sel[*pos] & 0x80) != 0;
    let sampling_frequency = SamplingFrequency::from_u8((sel[*pos] >> 3) & 0x0F);
    let sample_resolution = (sel[*pos] & 0x04) != 0;
    let s_reserved = sel[*pos] & 0x03;
    *pos += 1;

    let mut assets = Vec::with_capacity(num_assets);
    for _ in 0..num_assets {
        let a = parse_asset_info(sel, pos)?;
        assets.push(a);
    }

    Ok(SubstreamInfo {
        channel_count,
        lfe_flag,
        sampling_frequency,
        sample_resolution,
        reserved: s_reserved,
        assets,
    })
}

fn parse_asset_info(sel: &[u8], pos: &mut usize) -> Result<AssetInfo> {
    let need = *pos + ASSET_FIXED_LEN;
    if sel.len() < need {
        return Err(Error::BufferTooShort {
            need,
            have: sel.len(),
            what: "asset_info",
        });
    }
    let asset_construction = (sel[*pos] >> 3) & 0x1F;
    let vbr_flag = (sel[*pos] & 0x04) != 0;
    let post_encode_br_scaling_flag = (sel[*pos] & 0x02) != 0;
    let component_type_flag = (sel[*pos] & 0x01) != 0;
    *pos += 1;
    let language_code_flag = (sel[*pos] & 0x80) != 0;
    let bit_rate_or_scaled = ((u16::from(sel[*pos] & 0x7F)) << 6) | (u16::from(sel[*pos + 1]) >> 2);
    *pos += 1;
    let a_reserved = sel[*pos] & 0x03;
    *pos += 1;

    let component_type = if component_type_flag {
        let ct = *sel.get(*pos).ok_or(Error::BufferTooShort {
            need: *pos + 1,
            have: sel.len(),
            what: "asset_info component_type",
        })?;
        *pos += 1;
        Some(ct)
    } else {
        None
    };

    let iso_639_language_code = if language_code_flag {
        let need_lang = *pos + ISO_639_LEN;
        let lang_bytes = sel.get(*pos..need_lang).ok_or(Error::BufferTooShort {
            need: need_lang,
            have: sel.len(),
            what: "asset_info ISO_639_language_code",
        })?;
        *pos += ISO_639_LEN;
        Some(LangCode([lang_bytes[0], lang_bytes[1], lang_bytes[2]]))
    } else {
        None
    };

    Ok(AssetInfo {
        asset_construction,
        vbr_flag,
        post_encode_br_scaling_flag,
        component_type_flag,
        language_code_flag,
        bit_rate_or_scaled,
        reserved: a_reserved,
        component_type,
        iso_639_language_code,
    })
}

impl Serialize for DtsHd<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        1 + self
            .substreams
            .iter()
            .map(|s| s.serialized_len())
            .sum::<usize>()
            + self.additional_info.len()
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        buf[0] = ((u8::from(self.substream_core_flag)) << 7)
            | ((u8::from(self.substream_0_flag)) << 6)
            | ((u8::from(self.substream_1_flag)) << 5)
            | ((u8::from(self.substream_2_flag)) << 4)
            | ((u8::from(self.substream_3_flag)) << 3)
            | (self.reserved & 0x07);

        let flags = [
            self.substream_core_flag,
            self.substream_0_flag,
            self.substream_1_flag,
            self.substream_2_flag,
            self.substream_3_flag,
        ];
        let mut pos = 1;
        let mut si = 0;
        for &present in &flags {
            if !present {
                continue;
            }
            if si >= self.substreams.len() {
                return Err(Error::ValueOutOfRange {
                    field: "DTS-HD substreams",
                    reason: "fewer substreams than flags indicate",
                });
            }
            let written = self.substreams[si].serialize_into(&mut buf[pos..])?;
            pos += written;
            si += 1;
        }
        if si != self.substreams.len() {
            return Err(Error::ValueOutOfRange {
                field: "DTS-HD substreams",
                reason: "more substreams than flags indicate",
            });
        }
        buf[pos..len].copy_from_slice(self.additional_info);
        Ok(len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::descriptors::extension::test_support::*;
    use crate::descriptors::extension::{ExtensionBody, ExtensionDescriptor};

    #[test]
    fn decodes_sampling_frequency() {
        assert_eq!(SamplingFrequency::from_u8(12).name(), "48 kHz");
        assert_eq!(SamplingFrequency::from_u8(0).name(), "8 kHz");
    }

    #[test]
    fn parse_dts_hd_core_only() {
        // flags: core=1, others=0, reserved=7
        // substream: length=6, num_assets=0, channel_count=6, lfe=1, sf=12,
        //   sample_res=1, rsv=3
        // asset: ac=1, vbr=0, pe=0, ct=1, lang=1, br=755, rsv=3
        //   component_type=0x42, lang=eng
        let sel = [
            0x87, // flags
            0x06, // substream_length
            0x06, // num_assets=0, channel_count=6
            0xE7, // lfe=1, sf=12, sample_res=1, rsv=3
            0x09, 0x8B, 0xCF, // asset: ac=1, vbr=0, pe=0, ct=1, lang=1, br=755, rsv=3
            0x42, // component_type
            b'e', b'n', b'g', // lang
        ];
        let bytes = wrap(0x0E, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::DtsHd(b) => {
                assert!(b.substream_core_flag);
                assert!(!b.substream_0_flag);
                assert!(!b.substream_1_flag);
                assert!(!b.substream_2_flag);
                assert!(!b.substream_3_flag);
                assert_eq!(b.reserved, 7);
                assert_eq!(b.substreams.len(), 1);
                let s = &b.substreams[0];
                assert_eq!(s.channel_count, 6);
                assert!(s.lfe_flag);
                assert_eq!(s.sampling_frequency, SamplingFrequency::Khz48);
                assert!(s.sample_resolution);
                assert_eq!(s.reserved, 3);
                assert_eq!(s.assets.len(), 1);
                let a = &s.assets[0];
                assert_eq!(a.asset_construction, 1);
                assert!(!a.vbr_flag);
                assert!(!a.post_encode_br_scaling_flag);
                assert!(a.component_type_flag);
                assert!(a.language_code_flag);
                assert_eq!(a.bit_rate_or_scaled, 755);
                assert_eq!(a.reserved, 3);
                assert_eq!(a.component_type, Some(0x42));
                assert_eq!(a.iso_639_language_code, Some(LangCode(*b"eng")));
                assert!(b.additional_info.is_empty());
            }
            other => panic!("expected DtsHd, got {other:?}"),
        }
        round_trip(&d);
    }

    #[test]
    fn parse_dts_hd_flags_only() {
        // No substreams set — all flags zero, reserved=0
        let sel = [0x00];
        let bytes = wrap(0x0E, &sel);
        let d = ExtensionDescriptor::parse(&bytes).unwrap();
        match &d.body {
            ExtensionBody::DtsHd(b) => {
                assert!(!b.substream_core_flag);
                assert!(b.substreams.is_empty());
            }
            other => panic!("expected DtsHd, got {other:?}"),
        }
        round_trip(&d);
    }
}
