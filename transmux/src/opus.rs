//! Opus in ISOBMFF — `Opus` AudioSampleEntry + `dOps` config box.
//!
//! "Encapsulation of Opus in ISO Base Media File Format"
//! (<https://opus-codec.org/docs/opus_in_isobmff.html>, §4.3.2 OpusSpecificBox).
//!
//! Unlike the Ogg `OpusHead` (RFC 7845), `dOps` fields are **big-endian** and
//! carry **no** magic signature.

use crate::error::{Error, Result};
use crate::wire_cursor::{be_u16, be_u32, i16_at};
use alloc::vec::Vec;
use broadcast_common::len::{need_in, need_out};
use broadcast_common::{Parse, Serialize};

/// FourCC of the Opus config box.
pub const DOPS_FOURCC: [u8; 4] = *b"dOps";
/// FourCC of the Opus sample entry.
pub const OPUS_FOURCC: [u8; 4] = *b"Opus";
/// Fixed length of the `dOps` body up to (and excluding) the channel-mapping table.
const DOPS_FIXED_LEN: usize = 11;

/// Offset of `PreSkip` in the `dOps` body.
const PRE_SKIP_OFFSET: usize = 2;
/// Byte width of `PreSkip`.
const PRE_SKIP_LEN: usize = core::mem::size_of::<u16>();
/// Offset of `InputSampleRate` in the `dOps` body.
const INPUT_SAMPLE_RATE_OFFSET: usize = 4;
/// Byte width of `InputSampleRate`.
const INPUT_SAMPLE_RATE_LEN: usize = core::mem::size_of::<u32>();
/// Offset of `OutputGain` in the `dOps` body.
const OUTPUT_GAIN_OFFSET: usize = 8;
/// Byte width of `OutputGain`.
const OUTPUT_GAIN_LEN: usize = core::mem::size_of::<i16>();
/// Offset of `ChannelMappingFamily` in the `dOps` body.
const CHANNEL_MAPPING_FAMILY_OFFSET: usize = DOPS_FIXED_LEN - 1;
/// Byte width of the `StreamCount` + `CoupledCount` prefix of the channel-mapping
/// table.
const CHANNEL_MAPPING_TABLE_LEN: usize = core::mem::size_of::<u16>();
/// Offset of `StreamCount` in the `dOps` body.
const STREAM_COUNT_OFFSET: usize = DOPS_FIXED_LEN;
/// Offset of `CoupledCount` in the `dOps` body.
const COUPLED_COUNT_OFFSET: usize = STREAM_COUNT_OFFSET + 1;
/// Offset of `ChannelMapping[OutputChannelCount]` in the `dOps` body.
const CHANNEL_MAPPING_OFFSET: usize = DOPS_FIXED_LEN + CHANNEL_MAPPING_TABLE_LEN;

/// OpusSpecificBox (`dOps` box body) — Opus-in-ISOBMFF §4.3.2.
///
/// Layout: `Version(8)` | `OutputChannelCount(8)` | `PreSkip(16)` |
/// `InputSampleRate(32)` | `OutputGain(16, signed 8.8)` |
/// `ChannelMappingFamily(8)`; when the family is non-zero, a channel-mapping
/// table follows (`StreamCount(8)`, `CoupledCount(8)`, `ChannelMapping[Nch]`).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct OpusSpecificBox {
    /// `Version` (0).
    pub version: u8,
    /// `OutputChannelCount`.
    pub output_channel_count: u8,
    /// `PreSkip` samples at 48 kHz.
    pub pre_skip: u16,
    /// `InputSampleRate` in Hz (original rate; playback is always 48 kHz).
    pub input_sample_rate: u32,
    /// `OutputGain` (signed Q7.8 fixed-point, dB).
    pub output_gain: i16,
    /// `ChannelMappingFamily`.
    pub channel_mapping_family: u8,
    /// Channel-mapping table (`StreamCount`, `CoupledCount`, `ChannelMapping[]`),
    /// present verbatim iff `channel_mapping_family != 0`.
    pub channel_mapping: Option<ChannelMappingTable>,
}

/// Opus channel-mapping table (present when `ChannelMappingFamily != 0`).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ChannelMappingTable {
    /// `StreamCount`.
    pub stream_count: u8,
    /// `CoupledCount`.
    pub coupled_count: u8,
    /// `ChannelMapping[OutputChannelCount]`.
    pub channel_mapping: Vec<u8>,
}

impl OpusSpecificBox {
    /// RFC 6381 codec string — always the literal `"Opus"`.
    pub fn rfc6381(&self) -> &'static str {
        "Opus"
    }
}

impl<'a> Parse<'a> for OpusSpecificBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        need_in(bytes, DOPS_FIXED_LEN, "dOps body")?;
        let version = bytes[0];
        let output_channel_count = bytes[1];
        let pre_skip = be_u16(bytes, PRE_SKIP_OFFSET, "dOps body")?;
        let input_sample_rate = be_u32(bytes, INPUT_SAMPLE_RATE_OFFSET, "dOps body")?;
        let output_gain = i16_at(bytes, OUTPUT_GAIN_OFFSET);
        let channel_mapping_family = bytes[CHANNEL_MAPPING_FAMILY_OFFSET];
        let channel_mapping = if channel_mapping_family != 0 {
            need_in(
                bytes,
                DOPS_FIXED_LEN + CHANNEL_MAPPING_TABLE_LEN,
                "dOps channel mapping",
            )?;
            let stream_count = bytes[STREAM_COUNT_OFFSET];
            let coupled_count = bytes[COUPLED_COUNT_OFFSET];
            let map_start = CHANNEL_MAPPING_OFFSET;
            // `ChannelMapping[OutputChannelCount]` (Opus-in-ISOBMFF §4.3.2):
            // the table's width is derived from `OutputChannelCount`, so a short
            // tail is truncated input — accepting it would parse a shorter map
            // than the box declares and re-serialize a different box.
            let map_end = map_start + output_channel_count as usize;
            need_in(bytes, map_end, "dOps ChannelMapping")?;
            Some(ChannelMappingTable {
                stream_count,
                coupled_count,
                channel_mapping: bytes[map_start..map_end].to_vec(),
            })
        } else {
            None
        };
        Ok(Self {
            version,
            output_channel_count,
            pre_skip,
            input_sample_rate,
            output_gain,
            channel_mapping_family,
            channel_mapping,
        })
    }
}

impl Serialize for OpusSpecificBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        let mut n = DOPS_FIXED_LEN;
        if let Some(ref m) = self.channel_mapping {
            n += CHANNEL_MAPPING_TABLE_LEN + m.channel_mapping.len();
        }
        n
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        buf[0] = self.version;
        buf[1] = self.output_channel_count;
        buf[PRE_SKIP_OFFSET..PRE_SKIP_OFFSET + PRE_SKIP_LEN]
            .copy_from_slice(&self.pre_skip.to_be_bytes());
        buf[INPUT_SAMPLE_RATE_OFFSET..INPUT_SAMPLE_RATE_OFFSET + INPUT_SAMPLE_RATE_LEN]
            .copy_from_slice(&self.input_sample_rate.to_be_bytes());
        buf[OUTPUT_GAIN_OFFSET..OUTPUT_GAIN_OFFSET + OUTPUT_GAIN_LEN]
            .copy_from_slice(&self.output_gain.to_be_bytes());
        buf[CHANNEL_MAPPING_FAMILY_OFFSET] = self.channel_mapping_family;
        if let Some(ref m) = self.channel_mapping {
            buf[STREAM_COUNT_OFFSET] = m.stream_count;
            buf[COUPLED_COUNT_OFFSET] = m.coupled_count;
            buf[CHANNEL_MAPPING_OFFSET..CHANNEL_MAPPING_OFFSET + m.channel_mapping.len()]
                .copy_from_slice(&m.channel_mapping);
        }
        Ok(need)
    }
}
