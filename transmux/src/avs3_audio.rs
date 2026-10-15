//! Audio Vivid (AVS3-P3) in ISOBMFF — GY/T 420-2025 Annex A.4: the `dca3`
//! decoder configuration box (`CA3SpecificBox`) of an `av3a` audio sample entry.
//!
//! | Type | FourCC | Spec |
//! |------|--------|------|
//! | [`Ca3SpecificBox`] | `dca3` | A.4.3.1 |
//! | [`Avs3AudioGaConfig`] | (in `dca3`, `audio_codec_id` 2) | A.4.2.1 `Avs3AudioGASpecificConfig` |
//! | [`Avs3AudioLlConfig`] | (in `dca3`, `audio_codec_id` 1) | A.4.2.2 `Avs3AudioLLSpecificConfig` |
//! | [`Avs3AudioGhConfig`] | (in `dca3`, `audio_codec_id` 0) | T/AI 109.7-2024 5.1.2.2 `Avs3AudioGHSpecificConfig` |
//!
//! GY/T 420-2025 defines `audio_codec_id` 1 and 2 only; T/AI 109.7-2024 clause 5.1.3.1
//! additionally defines `audio_codec_id` 0 (general high-bit-rate) and the `a3as`
//! (AASF) sample entry, whose `avs3_as_header` is not modelled.
//!
//! The `dca3` body is `audio_codec_id(4)` immediately followed (bit-packed, not
//! byte aligned in between) by the selected configuration. Every field's
//! semantics are those of GY/T 363-2023 Annex A (see `dvb-si`'s
//! `Avs3AudioDescriptor` for the transport-stream counterpart, which shares the
//! field names). The `av3a` [`AV3A_FOURCC`] sample entry itself is a
//! plain `AudioSampleEntry` whose `ChannelCount`/`SampleSize`/`SampleRate` the
//! decoder shall ignore; wiring it into the init-segment audio machinery is not
//! done here. The `reserved` bits state no value in the standard: captured on
//! parse and re-emitted verbatim; hand-built values are 0 (ISOBMFF convention).

use crate::error::{Error, Result};
use alloc::vec::Vec;
use broadcast_common::bits::{BitReader, BitWriter};
use broadcast_common::len::{fit_bits, fit_u16};
use broadcast_common::{Parse, Serialize};

/// FourCC of the Audio Vivid decoder configuration box.
pub const DCA3_FOURCC: [u8; 4] = *b"dca3";
/// FourCC of the Audio Vivid audio sample entry.
pub const AV3A_FOURCC: [u8; 4] = *b"av3a";

/// `sampling_frequency_index` value that adds an explicit 24-bit `sampling_frequency`.
pub const SAMPLING_FREQUENCY_INDEX_EXPLICIT: u8 = 0xF;

fn short(what: &'static str) -> Error {
    Error::BufferTooShort {
        need: 0,
        have: 0,
        what,
    }
}

/// `content_type` (4 bits) and its dependent fields — A.4.2.1 / Table A.2.
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
        /// `number_objects` (7 bits) = `object_channel_number` + 1.
        number_objects: u8,
        /// `reserved` (1 bit).
        reserved: u8,
    },
    /// 2 — channel and object mix.
    ChannelAndObject {
        /// `channel_number_index` (7 bits).
        channel_number_index: u8,
        /// `reserved` (1 bit) after `channel_number_index`.
        reserved_channel: u8,
        /// `number_objects` (7 bits).
        number_objects: u8,
        /// `reserved` (1 bit) after `number_objects`.
        reserved_object: u8,
    },
    /// 3 — HOA signal.
    Hoa {
        /// `hoa_order` (4 bits) = `order` + 1 of GY/T 363 Annex A.
        hoa_order: u8,
    },
    /// 4..=15 — reserved `content_type`, no dependent fields; carries the value.
    Reserved(u8),
}

impl Avs3AudioContent {
    /// The 4-bit `content_type`.
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

    /// Spec label.
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

/// `Avs3AudioGASpecificConfig` (general-bit-rate coding, `audio_codec_id` 2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct Avs3AudioGaConfig {
    /// `sampling_frequency_index` (4 bits).
    pub sampling_frequency_index: u8,
    /// `nn_type` (3 bits).
    pub nn_type: u8,
    /// `reserved` (1 bit) between `nn_type` and `content_type`.
    pub reserved: u8,
    /// `content_type` and dependent fields.
    pub content: Avs3AudioContent,
    /// `total_bitrate` (16 bits, kbps).
    pub total_bitrate: u16,
    /// `resolution` (2 bits).
    pub resolution: u8,
    /// Trailing `reserved`: 2 bits when `content_type` is 3 (HOA), else 6 bits.
    pub reserved_tail: u8,
}

/// `Avs3AudioLLSpecificConfig` (lossless coding, `audio_codec_id` 1).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct Avs3AudioLlConfig {
    /// `sampling_frequency_index` (4 bits).
    pub sampling_frequency_index: u8,
    /// `sampling_frequency` (24 bits, Hz), present iff the index is 0xF.
    pub sampling_frequency: Option<u32>,
    /// `anc_data_index` (1 bit).
    pub anc_data_index: bool,
    /// `coding_profile` (3 bits).
    pub coding_profile: u8,
    /// `channel_number` (8 bits).
    pub channel_number: u8,
    /// `resolution` (2 bits).
    pub resolution: u8,
    /// `addition_info` bytes (`addition_info_length` is their count).
    pub addition_info: Vec<u8>,
    /// Trailing `reserved` (2 bits).
    pub reserved: u8,
}

/// `Avs3AudioGHSpecificConfig` (general high-bit-rate coding, `audio_codec_id`
/// 0, T/AI 109.7-2024 clause 5.1.2.2).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct Avs3AudioGhConfig {
    /// `sampling_frequency_index` (4 bits).
    pub sampling_frequency_index: u8,
    /// `anc_data_index` (1 bit).
    pub anc_data_index: bool,
    /// `coding_profile` (3 bits).
    pub coding_profile: u8,
    /// `bitstream_type` (1 bit).
    pub bitstream_type: bool,
    /// `channel_number_index` (7 bits).
    pub channel_number_index: u8,
    /// `bitrate_index` (4 bits).
    pub bitrate_index: u8,
    /// `raw_frame_length` (16 bits).
    pub raw_frame_length: u16,
    /// `resolution` (2 bits).
    pub resolution: u8,
    /// `addition_info` bytes (`addition_info_length` is their count).
    pub addition_info: Vec<u8>,
    /// Trailing `reserved` (6 bits).
    pub reserved: u8,
}

/// The configuration selected by `audio_codec_id`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum Ca3Config {
    /// `audio_codec_id == 2`: general-bit-rate coding.
    GeneralBitrate(Avs3AudioGaConfig),
    /// `audio_codec_id == 1`: lossless coding.
    Lossless(Avs3AudioLlConfig),
    /// `audio_codec_id == 0`: general high-bit-rate coding (T/AI 109.7-2024).
    GeneralHigh(Avs3AudioGhConfig),
    /// Any other `audio_codec_id`: no configuration is defined; the whole box
    /// body (including the 4-bit id) is preserved.
    Other {
        /// The 4-bit `audio_codec_id`.
        audio_codec_id: u8,
        /// The complete `dca3` body, verbatim.
        body: Vec<u8>,
    },
}

/// `CA3SpecificBox` — the body of the `dca3` box (A.4.3.1).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct Ca3SpecificBox {
    /// `audio_codec_id` and the selected configuration.
    pub config: Ca3Config,
}

impl Ca3Config {
    /// Spec label of the configuration kind.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::GeneralBitrate(_) => "general full-rate (audio_codec_id 2)",
            Self::Lossless(_) => "lossless (audio_codec_id 1)",
            Self::GeneralHigh(_) => "general high-rate (audio_codec_id 0)",
            Self::Other { .. } => "reserved audio_codec_id",
        }
    }

    /// The 4-bit `audio_codec_id`.
    #[must_use]
    pub const fn audio_codec_id(&self) -> u8 {
        match self {
            Self::GeneralBitrate(_) => 2,
            Self::Lossless(_) => 1,
            Self::GeneralHigh(_) => 0,
            Self::Other { audio_codec_id, .. } => *audio_codec_id,
        }
    }
}

broadcast_common::impl_spec_display!(Ca3Config);

fn parse_ga(r: &mut BitReader<'_>) -> Result<Avs3AudioGaConfig> {
    let w = "dca3 GA config";
    let rd = |r: &mut BitReader<'_>, n: u32| r.read_bits(n).map_err(|_| short(w));
    let sampling_frequency_index = rd(r, 4)? as u8;
    let nn_type = rd(r, 3)? as u8;
    let reserved = rd(r, 1)? as u8;
    let content_type = rd(r, 4)? as u8;
    let content = match content_type {
        0 => Avs3AudioContent::Channel {
            channel_number_index: rd(r, 7)? as u8,
            reserved: rd(r, 1)? as u8,
        },
        1 => Avs3AudioContent::Object {
            number_objects: rd(r, 7)? as u8,
            reserved: rd(r, 1)? as u8,
        },
        2 => Avs3AudioContent::ChannelAndObject {
            channel_number_index: rd(r, 7)? as u8,
            reserved_channel: rd(r, 1)? as u8,
            number_objects: rd(r, 7)? as u8,
            reserved_object: rd(r, 1)? as u8,
        },
        3 => Avs3AudioContent::Hoa {
            hoa_order: rd(r, 4)? as u8,
        },
        other => Avs3AudioContent::Reserved(other),
    };
    let total_bitrate = rd(r, 16)? as u16;
    let resolution = rd(r, 2)? as u8;
    let tail_bits = if content_type == 3 { 2 } else { 6 };
    let reserved_tail = rd(r, tail_bits)? as u8;
    Ok(Avs3AudioGaConfig {
        sampling_frequency_index,
        nn_type,
        reserved,
        content,
        total_bitrate,
        resolution,
        reserved_tail,
    })
}

fn parse_gh(r: &mut BitReader<'_>) -> Result<Avs3AudioGhConfig> {
    let w = "dca3 GH config";
    let rd = |r: &mut BitReader<'_>, n: u32| r.read_bits(n).map_err(|_| short(w));
    let sampling_frequency_index = rd(r, 4)? as u8;
    let anc_data_index = rd(r, 1)? != 0;
    let coding_profile = rd(r, 3)? as u8;
    let bitstream_type = rd(r, 1)? != 0;
    let channel_number_index = rd(r, 7)? as u8;
    let bitrate_index = rd(r, 4)? as u8;
    let raw_frame_length = rd(r, 16)? as u16;
    let resolution = rd(r, 2)? as u8;
    let n = rd(r, 16)? as usize;
    let mut addition_info = Vec::with_capacity(n.min(r.bits_remaining() / 8));
    for _ in 0..n {
        addition_info.push(rd(r, 8)? as u8);
    }
    let reserved = rd(r, 6)? as u8;
    Ok(Avs3AudioGhConfig {
        sampling_frequency_index,
        anc_data_index,
        coding_profile,
        bitstream_type,
        channel_number_index,
        bitrate_index,
        raw_frame_length,
        resolution,
        addition_info,
        reserved,
    })
}

fn parse_ll(r: &mut BitReader<'_>) -> Result<Avs3AudioLlConfig> {
    let w = "dca3 LL config";
    let rd = |r: &mut BitReader<'_>, n: u32| r.read_bits(n).map_err(|_| short(w));
    let sampling_frequency_index = rd(r, 4)? as u8;
    let sampling_frequency = if sampling_frequency_index == SAMPLING_FREQUENCY_INDEX_EXPLICIT {
        Some(rd(r, 24)? as u32)
    } else {
        None
    };
    let anc_data_index = rd(r, 1)? != 0;
    let coding_profile = rd(r, 3)? as u8;
    let channel_number = rd(r, 8)? as u8;
    let resolution = rd(r, 2)? as u8;
    let n = rd(r, 16)? as usize;
    let mut addition_info = Vec::with_capacity(n.min(r.bits_remaining() / 8));
    for _ in 0..n {
        addition_info.push(rd(r, 8)? as u8);
    }
    let reserved = rd(r, 2)? as u8;
    Ok(Avs3AudioLlConfig {
        sampling_frequency_index,
        sampling_frequency,
        anc_data_index,
        coding_profile,
        channel_number,
        resolution,
        addition_info,
        reserved,
    })
}

impl<'a> Parse<'a> for Ca3SpecificBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let first = *bytes.first().ok_or(Error::BufferTooShort {
            need: 1,
            have: 0,
            what: "dca3 body",
        })?;
        let id = first >> 4;
        let mut r = BitReader::new(bytes);
        r.skip_bits(4).map_err(|_| short("dca3 body"))?;
        let config = match id {
            2 => Ca3Config::GeneralBitrate(parse_ga(&mut r)?),
            1 => Ca3Config::Lossless(parse_ll(&mut r)?),
            0 => Ca3Config::GeneralHigh(parse_gh(&mut r)?),
            other => {
                return Ok(Self {
                    config: Ca3Config::Other {
                        audio_codec_id: other,
                        body: bytes.to_vec(),
                    },
                });
            }
        };
        if r.bits_remaining() >= 8 {
            return Err(Error::InvalidInput("dca3 body has trailing bytes"));
        }
        Ok(Self { config })
    }
}

fn wr(w: &mut BitWriter<'_>, v: u64, n: u32, field: &'static str) -> Result<()> {
    let v = fit_bits(v, n, field)?;
    w.write_bits(v, n)
        .map_err(|_| Error::InvalidInput("dca3 bit writer overflow"))
}

impl Ca3SpecificBox {
    fn bits(&self) -> usize {
        match &self.config {
            Ca3Config::GeneralBitrate(g) => {
                let content = match g.content {
                    Avs3AudioContent::Channel { .. } | Avs3AudioContent::Object { .. } => 8,
                    Avs3AudioContent::ChannelAndObject { .. } => 16,
                    Avs3AudioContent::Hoa { .. } => 4,
                    Avs3AudioContent::Reserved(_) => 0,
                };
                let tail = if g.content.content_type() == 3 { 2 } else { 6 };
                4 + 4 + 3 + 1 + 4 + content + 16 + 2 + tail
            }
            Ca3Config::Lossless(l) => {
                let sf = if l.sampling_frequency.is_some() {
                    24
                } else {
                    0
                };
                4 + 4 + sf + 1 + 3 + 8 + 2 + 16 + 8 * l.addition_info.len() + 2
            }
            Ca3Config::GeneralHigh(g) => {
                4 + 4 + 1 + 3 + 1 + 7 + 4 + 16 + 2 + 16 + 8 * g.addition_info.len() + 6
            }
            Ca3Config::Other { body, .. } => 8 * body.len(),
        }
    }
}

impl Serialize for Ca3SpecificBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        self.bits().div_ceil(8)
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        if let Ca3Config::Other {
            body,
            audio_codec_id,
        } = &self.config
        {
            if body.first().map(|b| b >> 4) != Some(*audio_codec_id)
                || matches!(*audio_codec_id, 0..=2)
            {
                return Err(Error::InvalidInput(
                    "dca3 Other body must start with a non-1/2 audio_codec_id",
                ));
            }
            buf[..need].copy_from_slice(body);
            return Ok(need);
        }
        buf[..need].fill(0);
        let mut w = BitWriter::new(&mut buf[..need]);
        wr(
            &mut w,
            u64::from(self.config.audio_codec_id()),
            4,
            "audio_codec_id",
        )?;
        match &self.config {
            Ca3Config::GeneralBitrate(g) => {
                wr(
                    &mut w,
                    u64::from(g.sampling_frequency_index),
                    4,
                    "sampling_frequency_index",
                )?;
                wr(&mut w, u64::from(g.nn_type), 3, "nn_type")?;
                wr(&mut w, u64::from(g.reserved), 1, "reserved")?;
                wr(
                    &mut w,
                    u64::from(g.content.content_type()),
                    4,
                    "content_type",
                )?;
                match g.content {
                    Avs3AudioContent::Channel {
                        channel_number_index,
                        reserved,
                    } => {
                        wr(
                            &mut w,
                            u64::from(channel_number_index),
                            7,
                            "channel_number_index",
                        )?;
                        wr(&mut w, u64::from(reserved), 1, "reserved")?;
                    }
                    Avs3AudioContent::Object {
                        number_objects,
                        reserved,
                    } => {
                        wr(&mut w, u64::from(number_objects), 7, "number_objects")?;
                        wr(&mut w, u64::from(reserved), 1, "reserved")?;
                    }
                    Avs3AudioContent::ChannelAndObject {
                        channel_number_index,
                        reserved_channel,
                        number_objects,
                        reserved_object,
                    } => {
                        wr(
                            &mut w,
                            u64::from(channel_number_index),
                            7,
                            "channel_number_index",
                        )?;
                        wr(&mut w, u64::from(reserved_channel), 1, "reserved")?;
                        wr(&mut w, u64::from(number_objects), 7, "number_objects")?;
                        wr(&mut w, u64::from(reserved_object), 1, "reserved")?;
                    }
                    Avs3AudioContent::Hoa { hoa_order } => {
                        wr(&mut w, u64::from(hoa_order), 4, "hoa_order")?;
                    }
                    Avs3AudioContent::Reserved(_) => {}
                }
                wr(&mut w, u64::from(g.total_bitrate), 16, "total_bitrate")?;
                wr(&mut w, u64::from(g.resolution), 2, "resolution")?;
                let tail_bits = if g.content.content_type() == 3 { 2 } else { 6 };
                wr(&mut w, u64::from(g.reserved_tail), tail_bits, "reserved")?;
            }
            Ca3Config::Lossless(l) => {
                let explicit = l.sampling_frequency_index == SAMPLING_FREQUENCY_INDEX_EXPLICIT;
                wr(
                    &mut w,
                    u64::from(l.sampling_frequency_index),
                    4,
                    "sampling_frequency_index",
                )?;
                match (explicit, l.sampling_frequency) {
                    (true, Some(f)) => wr(&mut w, u64::from(f), 24, "sampling_frequency")?,
                    (false, None) => {}
                    _ => {
                        return Err(Error::InvalidInput(
                            "sampling_frequency present iff sampling_frequency_index == 0xF",
                        ));
                    }
                }
                wr(&mut w, u64::from(l.anc_data_index), 1, "anc_data_index")?;
                wr(&mut w, u64::from(l.coding_profile), 3, "coding_profile")?;
                wr(&mut w, u64::from(l.channel_number), 8, "channel_number")?;
                wr(&mut w, u64::from(l.resolution), 2, "resolution")?;
                let n = fit_u16(l.addition_info.len(), "addition_info_length")?;
                wr(&mut w, u64::from(n), 16, "addition_info_length")?;
                for b in &l.addition_info {
                    wr(&mut w, u64::from(*b), 8, "addition_info")?;
                }
                wr(&mut w, u64::from(l.reserved), 2, "reserved")?;
            }
            Ca3Config::GeneralHigh(g) => {
                wr(
                    &mut w,
                    u64::from(g.sampling_frequency_index),
                    4,
                    "sampling_frequency_index",
                )?;
                wr(&mut w, u64::from(g.anc_data_index), 1, "anc_data_index")?;
                wr(&mut w, u64::from(g.coding_profile), 3, "coding_profile")?;
                wr(&mut w, u64::from(g.bitstream_type), 1, "bitstream_type")?;
                wr(
                    &mut w,
                    u64::from(g.channel_number_index),
                    7,
                    "channel_number_index",
                )?;
                wr(&mut w, u64::from(g.bitrate_index), 4, "bitrate_index")?;
                wr(
                    &mut w,
                    u64::from(g.raw_frame_length),
                    16,
                    "raw_frame_length",
                )?;
                wr(&mut w, u64::from(g.resolution), 2, "resolution")?;
                let n = fit_u16(g.addition_info.len(), "addition_info_length")?;
                wr(&mut w, u64::from(n), 16, "addition_info_length")?;
                for b in &g.addition_info {
                    wr(&mut w, u64::from(*b), 8, "addition_info")?;
                }
                wr(&mut w, u64::from(g.reserved), 6, "reserved")?;
            }
            Ca3Config::Other { .. } => {}
        }
        Ok(need)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn round_trip(bytes: &[u8]) -> Ca3SpecificBox {
        let b = Ca3SpecificBox::parse(bytes).unwrap();
        let mut out = vec![0u8; b.serialized_len()];
        let n = b.serialize_into(&mut out).unwrap();
        assert_eq!(&out[..n], bytes, "byte-exact round trip");
        b
    }

    /// A.4.2.1 with `content_type` 0 (channel signal): id 2, sfi 2 (48 kHz),
    /// nn_type 1, content_type 0, channel_number_index 0x02 (5.1), total_bitrate
    /// 0x0140 (320), resolution 2 (24-bit), reserved all zero.
    #[test]
    fn dca3_general_channel_signal() {
        // 0010 0010 | 001 0 0000 | 0000010 0 | 0x01 0x40 | 10 000000
        let bytes = [0x22, 0x20, 0x04, 0x01, 0x40, 0x80];
        let b = round_trip(&bytes);
        let Ca3Config::GeneralBitrate(g) = b.config else {
            panic!()
        };
        assert_eq!((g.sampling_frequency_index, g.nn_type), (2, 1));
        assert_eq!(
            g.content,
            Avs3AudioContent::Channel {
                channel_number_index: 2,
                reserved: 0
            }
        );
        assert_eq!(
            (g.total_bitrate, g.resolution, g.reserved_tail),
            (320, 2, 0)
        );
    }

    /// `content_type` 2 (channel + objects) and 3 (HOA, nibble-packed, with the
    /// 2-bit trailing reserved).
    #[test]
    fn dca3_general_mixed_and_hoa() {
        // id 2, sfi 1, nn 0, rsv 0, ct 2 | ch idx 3 rsv0 | n_obj 4 rsv0 | br 0x0100 | res 1 + 000000
        let mixed = [0x21, 0x02, 0x06, 0x08, 0x01, 0x00, 0x40];
        let b = round_trip(&mixed);
        let Ca3Config::GeneralBitrate(g) = b.config else {
            panic!()
        };
        assert_eq!(
            g.content,
            Avs3AudioContent::ChannelAndObject {
                channel_number_index: 3,
                reserved_channel: 0,
                number_objects: 4,
                reserved_object: 0
            }
        );
        // HOA: id 2, sfi 2, nn 0 rsv 0 ct 3 | hoa_order 3 (4 bits) | br 0x0200 | res 2 + rsv 00
        // bits: 0010 0010 0000 0011 0011 0000 0010 0000 0000 10 00 -> 24 bits
        let hoa = [0x22, 0x03, 0x30, 0x20, 0x08];
        let b = round_trip(&hoa);
        let Ca3Config::GeneralBitrate(g) = b.config else {
            panic!()
        };
        assert_eq!(g.content, Avs3AudioContent::Hoa { hoa_order: 3 });
        assert_eq!((g.total_bitrate, g.resolution), (0x0200, 2));
    }

    /// A.4.2.2 lossless: id 1, sfi 0xF with an explicit 24-bit frequency
    /// (96000 = 0x017700), anc 0, profile 0, channel_number 2, resolution 2,
    /// addition_info_length 2 (0xDE 0xAD), reserved 0.
    #[test]
    fn dca3_lossless_explicit_frequency() {
        let ll = Ca3SpecificBox {
            config: Ca3Config::Lossless(Avs3AudioLlConfig {
                sampling_frequency_index: 0xF,
                sampling_frequency: Some(96_000),
                anc_data_index: false,
                coding_profile: 0,
                channel_number: 2,
                resolution: 2,
                addition_info: vec![0xDE, 0xAD],
                reserved: 0,
            }),
        };
        let mut out = vec![0u8; ll.serialized_len()];
        let n = ll.serialize_into(&mut out).unwrap();
        // bit stream: 0001 1111 | 00000001 01110111 00000000 (96000) | anc 0 profile 000
        // | channel_number 00000010 | resolution 10 | length 0000000000000010
        // | 11011110 10101101 | reserved 00, packed MSB-first into 10 bytes.
        assert_eq!(
            &out[..n],
            &[0x1F, 0x01, 0x77, 0x00, 0x00, 0x28, 0x00, 0x0B, 0x7A, 0xB4][..]
        );
        let back = Ca3SpecificBox::parse(&out[..n]).unwrap();
        assert_eq!(back, ll);
    }

    /// T/AI 109.7-2024 5.1.2.2 (`audio_codec_id` 0): sfi 2, anc 0, profile 0,
    /// bitstream_type 1, channel_number_index 2, bitrate_index 3, raw_frame_length
    /// 0x0400, resolution 2, no addition_info, reserved 0.
    #[test]
    fn dca3_general_high_bitrate() {
        // bit stream: id 0000 sfi 0010 | anc 0 profile 000 bitstream_type 1
        // channel_number_index 0000010 | bitrate_index 0011 | raw_frame_length
        // 0000010000000000 | resolution 10 | addition_info_length 0 | reserved 000000
        let g = Avs3AudioGhConfig {
            sampling_frequency_index: 2,
            anc_data_index: false,
            coding_profile: 0,
            bitstream_type: true,
            channel_number_index: 2,
            bitrate_index: 3,
            raw_frame_length: 0x0400,
            resolution: 2,
            addition_info: vec![],
            reserved: 0,
        };
        let b = Ca3SpecificBox {
            config: Ca3Config::GeneralHigh(g),
        };
        let mut out = vec![0u8; b.serialized_len()];
        let n = b.serialize_into(&mut out).unwrap();
        assert_eq!(n, 8);
        assert_eq!(
            &out[..n],
            &[0x02, 0x08, 0x23, 0x04, 0x00, 0x80, 0x00, 0x00][..]
        );
        assert_eq!(round_trip(&out[..n]), b);
    }

    #[test]
    fn dca3_other_codec_id_is_preserved_and_malformed_rejected() {
        let b = round_trip(&[0x70, 0xAB, 0xCD]);
        assert_eq!(b.config.audio_codec_id(), 7);
        assert!(Ca3SpecificBox::parse(&[]).is_err());
        assert!(
            Ca3SpecificBox::parse(&[0x22, 0x20]).is_err(),
            "truncated GA config"
        );
        assert!(
            Ca3SpecificBox::parse(&[0x22, 0x20, 0x04, 0x01, 0x40, 0x80, 0xFF]).is_err(),
            "trailing"
        );
        // an Other body that claims id 2 would not round-trip as GA
        let bad = Ca3SpecificBox {
            config: Ca3Config::Other {
                audio_codec_id: 2,
                body: vec![0x20],
            },
        };
        assert!(bad.serialize_into(&mut [0u8; 4]).is_err());
    }

    #[test]
    fn dca3_serialize_rejects_overwide_and_inconsistent() {
        let mut g = match round_trip(&[0x22, 0x20, 0x04, 0x01, 0x40, 0x80]).config {
            Ca3Config::GeneralBitrate(g) => g,
            _ => unreachable!(),
        };
        g.nn_type = 8;
        let b = Ca3SpecificBox {
            config: Ca3Config::GeneralBitrate(g),
        };
        assert!(b.serialize_into(&mut [0u8; 8]).is_err());
        let b = Ca3SpecificBox {
            config: Ca3Config::Lossless(Avs3AudioLlConfig {
                sampling_frequency_index: 0xF,
                sampling_frequency: None,
                anc_data_index: false,
                coding_profile: 0,
                channel_number: 2,
                resolution: 1,
                addition_info: vec![],
                reserved: 0,
            }),
        };
        assert!(b.serialize_into(&mut [0u8; 16]).is_err());
    }
}
