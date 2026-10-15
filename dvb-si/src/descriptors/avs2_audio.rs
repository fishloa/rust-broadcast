//! AVS2_audio_descriptor — T/AI 109.7-2024 clause 8.2.3.4 (Table 10), tag
//! `0xD3` (211), carried in the PMT ES_info loop of an AVS2 audio elementary
//! stream (`stream_type` 0xD3, see [`crate::tables::pmt::StreamType::Avs2Audio`]).
//!
//! Private descriptor: T/AI 109.7 clause 8.2.3.3 uses a `registration_descriptor`
//! with `format_identifier` 'AVSA' (0x41565341); the registry also accepts a
//! private_data_specifier 'AVSA'. Enable it with
//! [`crate::descriptors::DescriptorRegistry::with_avs2_audio`].
//!
//! ```text
//! descriptor_tag(8)=0xD3 descriptor_length(8)
//! num_channels(8)
//! sample_rate_index(4) avs_version_flag(1) text_present_flag(1) language_present_flag(1) reserved(1)='0'
//! if text_present_flag:     text_length(8) text[text_length](16 each, UCS-2)
//! if language_present_flag: language(24)
//! if avs_version_flag:
//!   audio_codec_id(4) anc_data_index(1) coding_profile(3)
//!   if audio_codec_id==0: bitrate_index(4) bitstream_type(1) reserved(3) raw_frame_length(16)
//!   resolution(2) reserved(6)
//!   if anc_data_index: anc_data_block()
//! addition_info[N](8)
//! ```
//!
//! `anc_data_block()` is defined in GB/T 33475.3-2018 Annex A.2, which is not
//! vendored, so its length is unknown: the bytes following the version block
//! (the `anc_data_block()` when `anc_data_index` is set, then `addition_info`)
//! are kept undivided in [`Avs2AudioDescriptor::trailing`]. The `reserved` bits
//! are captured on parse (the one after `language_present_flag` is printed as
//! `'0'`; the others state no value and default to all ones when hand-built).

use super::{descriptor_body, write_descriptor_header};
use crate::error::{Error, Result};
use crate::text::LangCode;
use alloc::vec::Vec;
use broadcast_common::len::{fit_bits, fit_u8};
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for AVS2_audio_descriptor (T/AI 109.7 Table 9: 211).
pub const TAG: u8 = 0xD3;
const HEADER_LEN: usize = 2;
const LANG_LEN: usize = 3;
const RESERVED6_ONES: u8 = 0x3F;
const RESERVED3_ONES: u8 = 0b111;

/// `audio_codec_id == 0` body of the version block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct Avs2AudioCodec0 {
    /// `bitrate_index` (4 bits, GB/T 33475.3 Annex A.2).
    pub bitrate_index: u8,
    /// `bitstream_type` (1 bit).
    pub bitstream_type: bool,
    /// `reserved` (3 bits).
    pub reserved: u8,
    /// `raw_frame_length` (16 bits).
    pub raw_frame_length: u16,
}

impl Avs2AudioCodec0 {
    /// Build with the all-ones reserved default.
    #[must_use]
    pub const fn new(bitrate_index: u8, bitstream_type: bool, raw_frame_length: u16) -> Self {
        Self {
            bitrate_index,
            bitstream_type,
            reserved: RESERVED3_ONES,
            raw_frame_length,
        }
    }
}

/// The block present iff `avs_version_flag == 1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct Avs2AudioVersion {
    /// `audio_codec_id` (4 bits).
    pub audio_codec_id: u8,
    /// `anc_data_index` (1 bit): an `anc_data_block()` follows in `trailing`.
    pub anc_data_index: bool,
    /// `coding_profile` (3 bits).
    pub coding_profile: u8,
    /// Present iff `audio_codec_id == 0`.
    pub codec0: Option<Avs2AudioCodec0>,
    /// `resolution` (2 bits).
    pub resolution: u8,
    /// `reserved` (6 bits) after `resolution`.
    pub reserved: u8,
}

impl Avs2AudioVersion {
    /// Build with the all-ones reserved default.
    #[must_use]
    pub const fn new(
        audio_codec_id: u8,
        anc_data_index: bool,
        coding_profile: u8,
        codec0: Option<Avs2AudioCodec0>,
        resolution: u8,
    ) -> Self {
        Self {
            audio_codec_id,
            anc_data_index,
            coding_profile,
            codec0,
            resolution,
            reserved: RESERVED6_ONES,
        }
    }
}

/// AVS2_audio_descriptor (T/AI 109.7-2024 Table 10).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct Avs2AudioDescriptor {
    /// `num_channels` (8 bits).
    pub num_channels: u8,
    /// `sample_rate_index` (4 bits, GB/T 33475.3 Annex A.2).
    pub sample_rate_index: u8,
    /// `reserved` (1 bit) after `language_present_flag`; printed as `'0'`.
    pub reserved: u8,
    /// `text[]`: 16-bit Unicode units, present iff `text_present_flag`.
    pub text: Option<Vec<u16>>,
    /// `language` (24 bits, GB/T 4880.2-2000), present iff `language_present_flag`.
    pub language: Option<LangCode>,
    /// The version block, present iff `avs_version_flag`.
    pub version: Option<Avs2AudioVersion>,
    /// Bytes after the version block: `anc_data_block()` (when `anc_data_index`
    /// is set; GB/T 33475.3 Annex A.2, length unknown here) followed by the
    /// optional `addition_info[]`.
    pub trailing: Vec<u8>,
}

impl Avs2AudioDescriptor {
    /// Build a descriptor with no text, language, version block or trailer.
    #[must_use]
    pub const fn new(num_channels: u8, sample_rate_index: u8) -> Self {
        Self {
            num_channels,
            sample_rate_index,
            reserved: 0,
            text: None,
            language: None,
            version: None,
            trailing: Vec::new(),
        }
    }
}

struct Cursor<'a> {
    body: &'a [u8],
    pos: usize,
}

impl Cursor<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8]> {
        let end = self.pos + n;
        let s = self.body.get(self.pos..end).ok_or(Error::BufferTooShort {
            need: end,
            have: self.body.len(),
            what: "Avs2AudioDescriptor",
        })?;
        self.pos = end;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
}

impl<'a> Parse<'a> for Avs2AudioDescriptor {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "Avs2AudioDescriptor",
            "unexpected tag for AVS2_audio_descriptor",
        )?;
        let mut c = Cursor { body, pos: 0 };
        let num_channels = c.u8()?;
        let b1 = c.u8()?;
        let avs_version_flag = b1 & 0x08 != 0;
        let text_present = b1 & 0x04 != 0;
        let language_present = b1 & 0x02 != 0;
        let text = if text_present {
            let n = usize::from(c.u8()?);
            let raw = c.take(n * 2)?;
            Some(
                raw.chunks_exact(2)
                    .map(|p| u16::from_be_bytes([p[0], p[1]]))
                    .collect(),
            )
        } else {
            None
        };
        let language = if language_present {
            let l = c.take(LANG_LEN)?;
            Some(LangCode([l[0], l[1], l[2]]))
        } else {
            None
        };
        let version = if avs_version_flag {
            let v0 = c.u8()?;
            let audio_codec_id = v0 >> 4;
            let codec0 = if audio_codec_id == 0 {
                let x = c.u8()?;
                let hi = c.u8()?;
                let lo = c.u8()?;
                Some(Avs2AudioCodec0 {
                    bitrate_index: x >> 4,
                    bitstream_type: x & 0x08 != 0,
                    reserved: x & 0x07,
                    raw_frame_length: u16::from_be_bytes([hi, lo]),
                })
            } else {
                None
            };
            let r = c.u8()?;
            Some(Avs2AudioVersion {
                audio_codec_id,
                anc_data_index: v0 & 0x08 != 0,
                coding_profile: v0 & 0x07,
                codec0,
                resolution: r >> 6,
                reserved: r & 0x3F,
            })
        } else {
            None
        };
        Ok(Self {
            num_channels,
            sample_rate_index: b1 >> 4,
            reserved: b1 & 0x01,
            text,
            language,
            version,
            trailing: body[c.pos..].to_vec(),
        })
    }
}

impl Avs2AudioDescriptor {
    fn body_bytes(&self) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        out.push(self.num_channels);
        let sri = fit_bits(u64::from(self.sample_rate_index), 4, "sample_rate_index")? as u8;
        let rsv = fit_bits(u64::from(self.reserved), 1, "reserved")? as u8;
        out.push(
            (sri << 4)
                | (u8::from(self.version.is_some()) << 3)
                | (u8::from(self.text.is_some()) << 2)
                | (u8::from(self.language.is_some()) << 1)
                | rsv,
        );
        if let Some(text) = &self.text {
            out.push(fit_u8(text.len(), "text_length")?);
            for u in text {
                out.extend_from_slice(&u.to_be_bytes());
            }
        }
        if let Some(l) = &self.language {
            out.extend_from_slice(&l.0);
        }
        if let Some(v) = &self.version {
            let id = fit_bits(u64::from(v.audio_codec_id), 4, "audio_codec_id")? as u8;
            let prof = fit_bits(u64::from(v.coding_profile), 3, "coding_profile")? as u8;
            out.push((id << 4) | (u8::from(v.anc_data_index) << 3) | prof);
            match (id, &v.codec0) {
                (0, Some(c)) => {
                    let br = fit_bits(u64::from(c.bitrate_index), 4, "bitrate_index")? as u8;
                    let rs = fit_bits(u64::from(c.reserved), 3, "reserved")? as u8;
                    out.push((br << 4) | (u8::from(c.bitstream_type) << 3) | rs);
                    out.extend_from_slice(&c.raw_frame_length.to_be_bytes());
                }
                (0, None) | (1..=15, Some(_)) => {
                    return Err(Error::ValueOutOfRange {
                        field: "codec0",
                        reason: "present iff audio_codec_id == 0",
                    });
                }
                _ => {}
            }
            let res = fit_bits(u64::from(v.resolution), 2, "resolution")? as u8;
            let rs6 = fit_bits(u64::from(v.reserved), 6, "reserved")? as u8;
            out.push((res << 6) | rs6);
        }
        out.extend_from_slice(&self.trailing);
        Ok(out)
    }
}

impl Serialize for Avs2AudioDescriptor {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        self.body_bytes()
            .map_or(HEADER_LEN, |b| HEADER_LEN + b.len())
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let body = self.body_bytes()?;
        let len = HEADER_LEN + body.len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        write_descriptor_header(buf, TAG, body.len())?;
        buf[HEADER_LEN..len].copy_from_slice(&body);
        Ok(len)
    }
}

impl crate::traits::DescriptorDef<'_> for Avs2AudioDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "AVS2_AUDIO";
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn round_trip(bytes: &[u8]) -> Avs2AudioDescriptor {
        let d = Avs2AudioDescriptor::parse(bytes).unwrap();
        let mut out = vec![0u8; d.serialized_len()];
        let n = d.serialize_into(&mut out).unwrap();
        assert_eq!(&out[..n], bytes, "byte-exact round trip");
        d
    }

    /// Table 10, minimal form: only `num_channels`/`sample_rate_index`
    /// (2 bytes, the stated minimum descriptor_length); all flags clear.
    #[test]
    fn minimal_two_byte_form() {
        let bytes = [TAG, 2, 6, 0x20];
        let d = round_trip(&bytes);
        assert_eq!((d.num_channels, d.sample_rate_index), (6, 2));
        assert!(d.text.is_none() && d.language.is_none() && d.version.is_none());
        assert!(d.trailing.is_empty());
    }

    /// All optional parts: text "AB" (2 x UCS-2), language 'chi', version block
    /// with audio_codec_id 0 (bitrate_index 5, bitstream_type 1, raw_frame_length
    /// 0x0400, resolution 1) and addition_info.
    #[test]
    fn full_form_with_version_block() {
        // sri=3, avs_version=1, text=1, lang=1, reserved=0
        let b1 = (3 << 4) | 0b1110;
        let bytes = [
            TAG, 16, 2, b1, 2, 0x00, 0x41, 0x00, 0x42, b'c', b'h', b'i',
            0x02, // codec 0, anc 0, profile 2
            0x5F, 0x04, 0x00, 0x7F, 0xAB,
        ];
        let v = bytes.to_vec();
        let d = round_trip(&v);
        assert_eq!(d.text.as_deref(), Some(&[0x41u16, 0x42][..]));
        assert_eq!(d.language, Some(LangCode(*b"chi")));
        let ver = d.version.unwrap();
        assert_eq!((ver.audio_codec_id, ver.coding_profile), (0, 2));
        let c0 = ver.codec0.unwrap();
        assert_eq!(
            (c0.bitrate_index, c0.bitstream_type, c0.raw_frame_length),
            (5, true, 0x0400)
        );
        assert_eq!((ver.resolution, ver.reserved), (1, 0x3F));
        assert_eq!(d.trailing, [0xAB]);
    }

    #[test]
    fn nonzero_codec_id_has_no_codec0_block() {
        // sri=1, version only; audio_codec_id 1, anc 0, profile 3; resolution 2.
        let bytes = [TAG, 5, 2, 0x18, 0x13, 0xBF, 0xEE];
        let d = round_trip(&bytes);
        let ver = d.version.unwrap();
        assert_eq!(ver.audio_codec_id, 1);
        assert!(ver.codec0.is_none());
        assert_eq!(d.trailing, [0xEE]);
    }

    #[test]
    fn rejects_wrong_tag_and_truncation() {
        assert!(Avs2AudioDescriptor::parse(&[0xD2, 2, 6, 0x20]).is_err());
        assert!(Avs2AudioDescriptor::parse(&[TAG, 1, 6]).is_err());
        // text_present but text_length claims more than present
        assert!(Avs2AudioDescriptor::parse(&[TAG, 4, 2, 0x04, 5, 0]).is_err());
        // language_present but no language bytes
        assert!(Avs2AudioDescriptor::parse(&[TAG, 2, 2, 0x02]).is_err());
        // version flag but no version block
        assert!(Avs2AudioDescriptor::parse(&[TAG, 2, 2, 0x08]).is_err());
    }

    #[test]
    fn serialize_rejects_inconsistent_and_overwide_fields() {
        let mut d = Avs2AudioDescriptor::new(2, 16);
        assert!(d.serialize_into(&mut [0u8; 8]).is_err());
        d.sample_rate_index = 1;
        d.version = Some(Avs2AudioVersion::new(0, false, 1, None, 1));
        assert!(
            d.serialize_into(&mut [0u8; 16]).is_err(),
            "codec id 0 needs codec0"
        );
        d.version = Some(Avs2AudioVersion::new(
            1,
            false,
            1,
            Some(Avs2AudioCodec0::new(1, false, 1)),
            1,
        ));
        assert!(
            d.serialize_into(&mut [0u8; 16]).is_err(),
            "codec0 only for id 0"
        );
        d.version = Some(Avs2AudioVersion::new(1, false, 8, None, 1));
        assert!(d.serialize_into(&mut [0u8; 16]).is_err());
    }
}
