//! AVS3_audio_descriptor — T/AI 109.7-2024 clause 8.1.3.4 (Table 4), tag
//! `0xD2` (210), carried in the PMT ES_info loop of an AVS3-P3 audio
//! elementary stream (`stream_type` 0xD5, see [`crate::tables::pmt::StreamType::Avs3Audio`]).
//!
//! The descriptor is **private**: tag 0xD2 lies in the user-defined range, so
//! ETSI EN 300 468 V1.20.1 Annex O and ETSI TS 101 154 V2.10.1 section 4.1.8.35
//! require it to be preceded by a `private_data_specifier_descriptor` with
//! value `0x41565341` ([`crate::descriptors::private_data_specifier::PDS_AVSA`], 'AVSA').
//! Accordingly it is **not** auto-dispatched by `parse_loop`; enable it with
//! [`crate::descriptors::DescriptorRegistry::with_avs3_audio`], which registers it
//! for that private_data_specifier only. The Chinese documents (T/AI 109.7 clause
//! 8.1.3.3, GY/T 420) scope it with a `registration_descriptor` whose
//! `format_identifier` is 'AVSA' instead; the registry accepts that form too.
//!
//! The type is owned (`addition_info` is a `Vec<u8>`) because registry-dispatched
//! private descriptors must be `'static` (see [`crate::descriptors::DescriptorRegistry`]).
//!
//! Layout (Table 4; `audio_codec_id` selects the body, field semantics come
//! from T/AI 109.3 Annex A except `content_type`, Table 5):
//!
//! ```text
//! descriptor_tag(8)=0xD2 descriptor_length(8)
//! audio_codec_id(4) sampling_frequency_index(4)
//! if audio_codec_id==0: anc_data_index(1) coding_profile(3) bitrate_index(4)
//!                       bitstream_type(1) channel_number_index(7) raw_frame_length(16)
//! if audio_codec_id==1: [sampling_frequency(24) if sampling_frequency_index==0xF]
//!                       anc_data_index(1) coding_profile(3) reserved(4) channel_number(8)
//! if audio_codec_id==2: nn_type(3) reserved(1) content_type(4)
//!                       content_type 0: channel_number_index(7) reserved(1)
//!                       content_type 1: object_channel_number(7) reserved(1)
//!                       content_type 2: channel_number_index(7) reserved(1) object_channel_number(7) reserved(1)
//!                       content_type 3: hoa_order(4) reserved(4)
//!                       total_bitrate(16)
//! resolution(2) reserved(6) addition_info[N](8)
//! ```
//!
//! The spec gives no value for its `reserved` bits: they are captured on
//! parse and re-emitted verbatim; hand-built values default to all ones (the
//! MPEG-2 systems convention for reserved bits).

use super::{descriptor_body, write_descriptor_header};
use crate::error::{Error, Result};
use alloc::vec::Vec;
use broadcast_common::len::fit_bits;
use broadcast_common::{Parse, Serialize};

/// Descriptor tag for AVS3_audio_descriptor (T/AI 109.7 Table 3: 210).
pub const TAG: u8 = 0xD2;
const HEADER_LEN: usize = 2;

/// `sampling_frequency_index` value that signals an explicit 24-bit
/// `sampling_frequency` in the `audio_codec_id == 1` body (Table 4).
pub const SAMPLING_FREQUENCY_INDEX_EXPLICIT: u8 = 0xF;

const NIBBLE: u32 = 4;
const ALL_ONES_1: u8 = 0x1;
const ALL_ONES_4: u8 = 0xF;
const ALL_ONES_6: u8 = 0x3F;

/// `audio_codec_id == 0` body: channel-based coding (Table 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct Avs3AudioCodec0 {
    /// `anc_data_index` (1 bit, T/AI 109.3 Annex A).
    pub anc_data_index: bool,
    /// `coding_profile` (3 bits).
    pub coding_profile: u8,
    /// `bitrate_index` (4 bits).
    pub bitrate_index: u8,
    /// `bitstream_type` (1 bit).
    pub bitstream_type: bool,
    /// `channel_number_index` (7 bits).
    pub channel_number_index: u8,
    /// `raw_frame_length` (16 bits).
    pub raw_frame_length: u16,
}

/// `audio_codec_id == 1` body (Table 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct Avs3AudioCodec1 {
    /// `sampling_frequency` (24 bits), present iff
    /// `sampling_frequency_index == 0xF`.
    pub sampling_frequency: Option<u32>,
    /// `anc_data_index` (1 bit).
    pub anc_data_index: bool,
    /// `coding_profile` (3 bits).
    pub coding_profile: u8,
    /// `reserved` (4 bits, right-aligned); captured on parse, default `0xF`.
    pub reserved: u8,
    /// `channel_number` (8 bits).
    pub channel_number: u8,
}

/// `content_type` and its dependent fields (Table 4 / Table 5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum Avs3AudioContent {
    /// 0 — channel signal.
    Channel {
        /// `channel_number_index` (7 bits).
        channel_number_index: u8,
        /// `reserved` (1 bit).
        reserved: u8,
    },
    /// 1 — object signal.
    Object {
        /// `object_channel_number` (7 bits).
        object_channel_number: u8,
        /// `reserved` (1 bit).
        reserved: u8,
    },
    /// 2 — mixed channel and object signal.
    ChannelAndObject {
        /// `channel_number_index` (7 bits).
        channel_number_index: u8,
        /// `reserved` (1 bit) after `channel_number_index`.
        reserved_channel: u8,
        /// `object_channel_number` (7 bits).
        object_channel_number: u8,
        /// `reserved` (1 bit) after `object_channel_number`.
        reserved_object: u8,
    },
    /// 3 — HOA signal.
    Hoa {
        /// `hoa_order` (4 bits).
        hoa_order: u8,
        /// `reserved` (4 bits).
        reserved: u8,
    },
    /// 4..=15 — reserved `content_type`, no dependent fields; carries the value.
    Reserved(u8),
}

impl Avs3AudioContent {
    /// The 4-bit `content_type` wire value.
    #[must_use]
    pub const fn content_type(&self) -> u8 {
        match self {
            Self::Channel { .. } => 0,
            Self::Object { .. } => 1,
            Self::ChannelAndObject { .. } => 2,
            Self::Hoa { .. } => 3,
            Self::Reserved(v) => *v,
        }
    }

    /// Spec label (Table 5).
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Channel { .. } => "channel signal",
            Self::Object { .. } => "object signal",
            Self::ChannelAndObject { .. } => "channel and object signal",
            Self::Hoa { .. } => "HOA signal",
            Self::Reserved(_) => "reserved",
        }
    }
}
broadcast_common::impl_spec_display!(Avs3AudioContent, Reserved);

/// `audio_codec_id == 2` body (Table 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct Avs3AudioCodec2 {
    /// `nn_type` (3 bits).
    pub nn_type: u8,
    /// `reserved` (1 bit) between `nn_type` and `content_type`.
    pub reserved: u8,
    /// `content_type` + dependent fields.
    pub content: Avs3AudioContent,
    /// `total_bitrate` (16 bits, kbps, Table 6).
    pub total_bitrate: u16,
}

/// The `audio_codec_id`-selected body. The variant fixes `audio_codec_id`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum Avs3AudioBody {
    /// `audio_codec_id == 0`.
    Codec0(Avs3AudioCodec0),
    /// `audio_codec_id == 1`.
    Codec1(Avs3AudioCodec1),
    /// `audio_codec_id == 2`.
    Codec2(Avs3AudioCodec2),
    /// Any other `audio_codec_id` (3..=15): Table 4 defines no body for it.
    Other(u8),
}

impl Avs3AudioBody {
    /// The 4-bit `audio_codec_id`.
    #[must_use]
    pub const fn audio_codec_id(&self) -> u8 {
        match self {
            Self::Codec0(_) => 0,
            Self::Codec1(_) => 1,
            Self::Codec2(_) => 2,
            Self::Other(id) => *id,
        }
    }

    /// Label for the body kind.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Codec0(_) => "audio_codec_id 0",
            Self::Codec1(_) => "audio_codec_id 1",
            Self::Codec2(_) => "audio_codec_id 2",
            Self::Other(_) => "reserved audio_codec_id",
        }
    }
}
broadcast_common::impl_spec_display!(Avs3AudioBody, Other);

/// AVS3_audio_descriptor (T/AI 109.7 Table 4).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct Avs3AudioDescriptor {
    /// `sampling_frequency_index` (4 bits).
    pub sampling_frequency_index: u8,
    /// Body selected by `audio_codec_id`.
    pub body: Avs3AudioBody,
    /// `resolution` (2 bits, T/AI 109.3 Annex A).
    pub resolution: u8,
    /// `reserved` (6 bits) after `resolution`; captured on parse, default `0x3F`.
    pub reserved: u8,
    /// `addition_info[N]`: the optional trailing additional information (opaque
    /// per Table 4: "可选字段，附加信息").
    pub addition_info: Vec<u8>,
}

fn bit(b: u8, shift: u32) -> bool {
    (b >> shift) & 1 != 0
}

struct Cursor<'a> {
    body: &'a [u8],
    pos: usize,
}

impl Cursor<'_> {
    fn u8(&mut self) -> Result<u8> {
        let b = *self.body.get(self.pos).ok_or(Error::BufferTooShort {
            need: self.pos + 1,
            have: self.body.len(),
            what: "Avs3AudioDescriptor",
        })?;
        self.pos += 1;
        Ok(b)
    }
}

impl<'a> Parse<'a> for Avs3AudioDescriptor {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let body = descriptor_body(
            bytes,
            TAG,
            "Avs3AudioDescriptor",
            "unexpected tag for AVS3_audio_descriptor",
        )?;
        let mut c = Cursor { body, pos: 0 };
        let b0 = c.u8()?;
        let audio_codec_id = b0 >> NIBBLE;
        let sampling_frequency_index = b0 & 0x0F;
        let parsed = match audio_codec_id {
            0 => {
                let b1 = c.u8()?;
                let b2 = c.u8()?;
                let hi = c.u8()?;
                let lo = c.u8()?;
                Avs3AudioBody::Codec0(Avs3AudioCodec0 {
                    anc_data_index: bit(b1, 7),
                    coding_profile: (b1 >> 4) & 0x07,
                    bitrate_index: b1 & 0x0F,
                    bitstream_type: bit(b2, 7),
                    channel_number_index: b2 & 0x7F,
                    raw_frame_length: u16::from_be_bytes([hi, lo]),
                })
            }
            1 => {
                let sampling_frequency =
                    if sampling_frequency_index == SAMPLING_FREQUENCY_INDEX_EXPLICIT {
                        let a = c.u8()?;
                        let b = c.u8()?;
                        let d = c.u8()?;
                        Some(u32::from_be_bytes([0, a, b, d]))
                    } else {
                        None
                    };
                let b1 = c.u8()?;
                let channel_number = c.u8()?;
                Avs3AudioBody::Codec1(Avs3AudioCodec1 {
                    sampling_frequency,
                    anc_data_index: bit(b1, 7),
                    coding_profile: (b1 >> 4) & 0x07,
                    reserved: b1 & 0x0F,
                    channel_number,
                })
            }
            2 => {
                let b1 = c.u8()?;
                let content_type = b1 & 0x0F;
                let content = match content_type {
                    0 => {
                        let v = c.u8()?;
                        Avs3AudioContent::Channel {
                            channel_number_index: v >> 1,
                            reserved: v & 1,
                        }
                    }
                    1 => {
                        let v = c.u8()?;
                        Avs3AudioContent::Object {
                            object_channel_number: v >> 1,
                            reserved: v & 1,
                        }
                    }
                    2 => {
                        let v = c.u8()?;
                        let w = c.u8()?;
                        Avs3AudioContent::ChannelAndObject {
                            channel_number_index: v >> 1,
                            reserved_channel: v & 1,
                            object_channel_number: w >> 1,
                            reserved_object: w & 1,
                        }
                    }
                    3 => {
                        let v = c.u8()?;
                        Avs3AudioContent::Hoa {
                            hoa_order: v >> NIBBLE,
                            reserved: v & 0x0F,
                        }
                    }
                    other => Avs3AudioContent::Reserved(other),
                };
                let hi = c.u8()?;
                let lo = c.u8()?;
                Avs3AudioBody::Codec2(Avs3AudioCodec2 {
                    nn_type: b1 >> 5,
                    reserved: (b1 >> NIBBLE) & 1,
                    content,
                    total_bitrate: u16::from_be_bytes([hi, lo]),
                })
            }
            other => Avs3AudioBody::Other(other),
        };
        let r = c.u8()?;
        Ok(Self {
            sampling_frequency_index,
            body: parsed,
            resolution: r >> 6,
            reserved: r & ALL_ONES_6,
            addition_info: body[c.pos..].to_vec(),
        })
    }
}

impl Avs3AudioDescriptor {
    /// Build a descriptor with the all-ones default for every reserved run
    /// (the spec states no value). Per-body reserved fields inside `body` are
    /// the caller's (use [`Avs3AudioCodec1::new`] etc.).
    #[must_use]
    pub const fn new(sampling_frequency_index: u8, body: Avs3AudioBody, resolution: u8) -> Self {
        Self {
            sampling_frequency_index,
            body,
            resolution,
            reserved: ALL_ONES_6,
            addition_info: Vec::new(),
        }
    }
}

impl Avs3AudioCodec0 {
    /// Build a body from its functional fields.
    #[must_use]
    pub const fn new(
        anc_data_index: bool,
        coding_profile: u8,
        bitrate_index: u8,
        bitstream_type: bool,
        channel_number_index: u8,
        raw_frame_length: u16,
    ) -> Self {
        Self {
            anc_data_index,
            coding_profile,
            bitrate_index,
            bitstream_type,
            channel_number_index,
            raw_frame_length,
        }
    }
}

impl Avs3AudioCodec1 {
    /// Build a body with the all-ones default reserved nibble.
    #[must_use]
    pub const fn new(
        sampling_frequency: Option<u32>,
        anc_data_index: bool,
        coding_profile: u8,
        channel_number: u8,
    ) -> Self {
        Self {
            sampling_frequency,
            anc_data_index,
            coding_profile,
            reserved: ALL_ONES_4,
            channel_number,
        }
    }
}

impl Avs3AudioCodec2 {
    /// Build a body with the all-ones default reserved bit.
    #[must_use]
    pub const fn new(nn_type: u8, content: Avs3AudioContent, total_bitrate: u16) -> Self {
        Self {
            nn_type,
            reserved: ALL_ONES_1,
            content,
            total_bitrate,
        }
    }
}

fn pack(hi: u8, hi_bits: u32, lo: u8, lo_bits: u32, name: &'static str) -> Result<u8> {
    let h = fit_bits(u64::from(hi), hi_bits, name)? as u8;
    let l = fit_bits(u64::from(lo), lo_bits, name)? as u8;
    Ok((h << lo_bits) | l)
}

impl Avs3AudioDescriptor {
    fn body_bytes(&self) -> Result<alloc::vec::Vec<u8>> {
        let mut out = alloc::vec::Vec::new();
        let sfi = fit_bits(
            u64::from(self.sampling_frequency_index),
            4,
            "sampling_frequency_index",
        )? as u8;
        let id = fit_bits(u64::from(self.body.audio_codec_id()), 4, "audio_codec_id")? as u8;
        out.push((id << NIBBLE) | sfi);
        match &self.body {
            Avs3AudioBody::Codec0(b) => {
                let prof = fit_bits(u64::from(b.coding_profile), 3, "coding_profile")? as u8;
                let rate = fit_bits(u64::from(b.bitrate_index), 4, "bitrate_index")? as u8;
                out.push((u8::from(b.anc_data_index) << 7) | (prof << 4) | rate);
                let ch =
                    fit_bits(u64::from(b.channel_number_index), 7, "channel_number_index")? as u8;
                out.push((u8::from(b.bitstream_type) << 7) | ch);
                out.extend_from_slice(&b.raw_frame_length.to_be_bytes());
            }
            Avs3AudioBody::Codec1(b) => {
                let explicit = sfi == SAMPLING_FREQUENCY_INDEX_EXPLICIT;
                match (explicit, b.sampling_frequency) {
                    (true, Some(f)) => {
                        let f = fit_bits(u64::from(f), 24, "sampling_frequency")? as u32;
                        out.extend_from_slice(&f.to_be_bytes()[1..]);
                    }
                    (false, None) => {}
                    _ => {
                        return Err(Error::ValueOutOfRange {
                            field: "sampling_frequency",
                            reason: "present iff sampling_frequency_index == 0xF",
                        });
                    }
                }
                let prof = fit_bits(u64::from(b.coding_profile), 3, "coding_profile")? as u8;
                let res = fit_bits(u64::from(b.reserved), 4, "reserved")? as u8;
                out.push((u8::from(b.anc_data_index) << 7) | (prof << 4) | res);
                out.push(b.channel_number);
            }
            Avs3AudioBody::Codec2(b) => {
                let nn = fit_bits(u64::from(b.nn_type), 3, "nn_type")? as u8;
                let res = fit_bits(u64::from(b.reserved), 1, "reserved")? as u8;
                let ct = fit_bits(u64::from(b.content.content_type()), 4, "content_type")? as u8;
                out.push((nn << 5) | (res << 4) | ct);
                match &b.content {
                    Avs3AudioContent::Channel {
                        channel_number_index,
                        reserved,
                    } => out.push(pack(
                        *channel_number_index,
                        7,
                        *reserved,
                        1,
                        "channel_number_index",
                    )?),
                    Avs3AudioContent::Object {
                        object_channel_number,
                        reserved,
                    } => out.push(pack(
                        *object_channel_number,
                        7,
                        *reserved,
                        1,
                        "object_channel_number",
                    )?),
                    Avs3AudioContent::ChannelAndObject {
                        channel_number_index,
                        reserved_channel,
                        object_channel_number,
                        reserved_object,
                    } => {
                        out.push(pack(
                            *channel_number_index,
                            7,
                            *reserved_channel,
                            1,
                            "channel_number_index",
                        )?);
                        out.push(pack(
                            *object_channel_number,
                            7,
                            *reserved_object,
                            1,
                            "object_channel_number",
                        )?);
                    }
                    Avs3AudioContent::Hoa {
                        hoa_order,
                        reserved,
                    } => {
                        out.push(pack(*hoa_order, 4, *reserved, 4, "hoa_order")?);
                    }
                    Avs3AudioContent::Reserved(_) => {}
                }
                out.extend_from_slice(&b.total_bitrate.to_be_bytes());
            }
            Avs3AudioBody::Other(_) => {}
        }
        out.push(pack(self.resolution, 2, self.reserved, 6, "resolution")?);
        out.extend_from_slice(&self.addition_info);
        Ok(out)
    }
}

impl Serialize for Avs3AudioDescriptor {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        // An invalid (unserializable) descriptor reports the minimum; the
        // serialize_into error is authoritative.
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

impl crate::traits::DescriptorDef<'_> for Avs3AudioDescriptor {
    const TAG: u8 = TAG;
    const NAME: &'static str = "AVS3_AUDIO";
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;
    use alloc::vec;

    fn rt(bytes: &[u8]) -> Avs3AudioDescriptor {
        let d = Avs3AudioDescriptor::parse(bytes).unwrap();
        let mut out = vec![0u8; d.serialized_len()];
        let n = d.serialize_into(&mut out).unwrap();
        assert_eq!(&out[..n], bytes, "byte-exact round trip");
        d
    }

    /// Table 4, `audio_codec_id == 0`: codec 0 / sfi 3, anc=1 profile=2 rate=0xA,
    /// bitstream_type=1 ch_idx=0x15, raw_frame_length=0x0400, resolution=1
    /// reserved=0b10_1010.
    #[test]
    fn codec0_vector() {
        let bytes = [TAG, 6, 0x03, 0xAA, 0x80 | 0x15, 0x04, 0x00, 0x6A];
        let d = rt(&bytes);
        assert_eq!(d.sampling_frequency_index, 3);
        let Avs3AudioBody::Codec0(b) = d.body else {
            panic!()
        };
        assert!(b.anc_data_index && b.bitstream_type);
        assert_eq!((b.coding_profile, b.bitrate_index), (2, 0xA));
        assert_eq!((b.channel_number_index, b.raw_frame_length), (0x15, 0x0400));
        assert_eq!((d.resolution, d.reserved), (1, 0b10_1010));
        assert!(d.addition_info.is_empty());
    }

    /// `audio_codec_id == 1` with the explicit 24-bit sampling_frequency
    /// (sfi == 0xF) and trailing addition_info.
    #[test]
    fn codec1_explicit_frequency_and_addition_info() {
        // 0x1F | 0x00BB80 (48000) | anc=0 profile=1 reserved=0xF | ch=2 | res=2 rsv=0x3F | 0xDE 0xAD
        let bytes = [TAG, 9, 0x1F, 0x00, 0xBB, 0x80, 0x1F, 0x02, 0xBF, 0xDE, 0xAD];
        let d = rt(&bytes);
        let Avs3AudioBody::Codec1(b) = d.body else {
            panic!()
        };
        assert_eq!(b.sampling_frequency, Some(48_000));
        assert_eq!(
            (b.coding_profile, b.reserved, b.channel_number),
            (1, 0xF, 2)
        );
        assert_eq!(d.addition_info, [0xDE, 0xAD]);
    }

    /// `audio_codec_id == 1` without explicit frequency (sfi != 0xF).
    #[test]
    fn codec1_indexed_frequency() {
        let bytes = [TAG, 4, 0x13, 0x8F, 0x06, 0x3F];
        let d = rt(&bytes);
        let Avs3AudioBody::Codec1(b) = d.body else {
            panic!()
        };
        assert_eq!(b.sampling_frequency, None);
        assert!(b.anc_data_index);
    }

    /// `audio_codec_id == 2`, one vector per `content_type` (Table 5).
    #[test]
    fn codec2_content_types() {
        // nn_type=5, reserved=1.
        let head = (5u8 << 5) | (1 << 4);
        let cases: [(&[u8], Avs3AudioContent); 5] = [
            (
                &[head, 0x0B],
                Avs3AudioContent::Channel {
                    channel_number_index: 5,
                    reserved: 1,
                },
            ),
            (
                &[head | 1, 0x06],
                Avs3AudioContent::Object {
                    object_channel_number: 3,
                    reserved: 0,
                },
            ),
            (
                &[head | 2, 0x0B, 0x07],
                Avs3AudioContent::ChannelAndObject {
                    channel_number_index: 5,
                    reserved_channel: 1,
                    object_channel_number: 3,
                    reserved_object: 1,
                },
            ),
            (
                &[head | 3, 0x3F],
                Avs3AudioContent::Hoa {
                    hoa_order: 3,
                    reserved: 0xF,
                },
            ),
            (&[head | 9], Avs3AudioContent::Reserved(9)),
        ];
        for (mid, content) in cases {
            let mut body = vec![0x20u8]; // codec 2 / sfi 0
            body.extend_from_slice(mid);
            body.extend_from_slice(&[0x01, 0x80]); // total_bitrate 384
            body.push(0xFF); // resolution 3, reserved all-ones
            let mut bytes = vec![TAG, body.len() as u8];
            bytes.extend_from_slice(&body);
            let d = rt(&bytes);
            let Avs3AudioBody::Codec2(b) = d.body else {
                panic!()
            };
            assert_eq!(b.nn_type, 5);
            assert_eq!(b.content, content);
            assert_eq!(b.total_bitrate, 384);
            assert_eq!(d.resolution, 3);
        }
    }

    /// An `audio_codec_id` with no Table 4 body (3..=15) is just the header
    /// byte + resolution byte, and round-trips.
    #[test]
    fn unknown_codec_id_round_trips() {
        let bytes = [TAG, 2, 0x72, 0x45];
        let d = rt(&bytes);
        assert_eq!(d.body, Avs3AudioBody::Other(7));
        assert_eq!(d.body.audio_codec_id(), 7);
        assert_eq!(d.reserved, 0b000101);
    }

    #[test]
    fn rejects_wrong_tag_and_truncation() {
        assert!(Avs3AudioDescriptor::parse(&[0xD3, 2, 0x72, 0]).is_err());
        // codec 0 needs 5 body bytes + resolution byte
        assert!(Avs3AudioDescriptor::parse(&[TAG, 3, 0x03, 0x00, 0x00]).is_err());
        assert!(Avs3AudioDescriptor::parse(&[TAG, 0]).is_err());
    }

    #[test]
    fn serialize_rejects_inconsistent_and_overwide_fields() {
        // explicit-frequency flag without a frequency
        let d = Avs3AudioDescriptor::new(
            SAMPLING_FREQUENCY_INDEX_EXPLICIT,
            Avs3AudioBody::Codec1(Avs3AudioCodec1::new(None, false, 0, 2)),
            0,
        );
        assert!(d.serialize_into(&mut [0u8; 32]).is_err());
        // coding_profile wider than 3 bits
        let d = Avs3AudioDescriptor::new(
            0,
            Avs3AudioBody::Codec1(Avs3AudioCodec1::new(None, false, 8, 2)),
            0,
        );
        assert!(d.serialize_into(&mut [0u8; 32]).is_err());
        // resolution wider than 2 bits
        let d = Avs3AudioDescriptor::new(0, Avs3AudioBody::Other(5), 4);
        assert!(d.serialize_into(&mut [0u8; 32]).is_err());
    }

    #[test]
    fn labels() {
        assert_eq!(
            Avs3AudioContent::Hoa {
                hoa_order: 1,
                reserved: 0
            }
            .name(),
            "HOA signal"
        );
        assert_eq!(Avs3AudioContent::Reserved(9).to_string(), "reserved(0x09)");
        assert_eq!(
            Avs3AudioBody::Other(7).to_string(),
            "reserved audio_codec_id(0x07)"
        );
        assert_eq!(Avs3AudioContent::Reserved(9).content_type(), 9);
    }

    #[test]
    fn built_defaults_are_all_ones() {
        let c = Avs3AudioCodec2::new(
            1,
            Avs3AudioContent::Channel {
                channel_number_index: 2,
                reserved: 1,
            },
            100,
        );
        assert_eq!(c.reserved, 1);
        let d = Avs3AudioDescriptor::new(0, Avs3AudioBody::Codec2(c), 2);
        assert_eq!(d.reserved, 0x3F);
    }
}
