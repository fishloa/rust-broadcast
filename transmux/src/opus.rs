//! Opus in ISOBMFF — `Opus` AudioSampleEntry + `dOps` config box.
//!
//! "Encapsulation of Opus in ISO Base Media File Format"
//! (<https://opus-codec.org/docs/opus_in_isobmff.html>, §4.3.2 OpusSpecificBox).
//!
//! Unlike the Ogg `OpusHead` (RFC 7845), `dOps` fields are **big-endian** and
//! carry **no** magic signature.

use crate::error::{Error, Result};
use alloc::vec::Vec;
use broadcast_common::{Parse, Serialize};

/// FourCC of the Opus config box.
pub const DOPS_FOURCC: [u8; 4] = *b"dOps";
/// FourCC of the Opus sample entry.
pub const OPUS_FOURCC: [u8; 4] = *b"Opus";
/// Fixed length of the `dOps` body up to (and excluding) the channel-mapping table.
const DOPS_FIXED_LEN: usize = 11;

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
        if bytes.len() < DOPS_FIXED_LEN {
            return Err(Error::BufferTooShort {
                need: DOPS_FIXED_LEN,
                have: bytes.len(),
                what: "dOps body",
            });
        }
        let version = bytes[0];
        let output_channel_count = bytes[1];
        let pre_skip = u16::from_be_bytes([bytes[2], bytes[3]]);
        let input_sample_rate = u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
        let output_gain = i16::from_be_bytes([bytes[8], bytes[9]]);
        let channel_mapping_family = bytes[10];
        let channel_mapping = if channel_mapping_family != 0 {
            if bytes.len() < DOPS_FIXED_LEN + 2 {
                return Err(Error::BufferTooShort {
                    need: DOPS_FIXED_LEN + 2,
                    have: bytes.len(),
                    what: "dOps channel mapping",
                });
            }
            let stream_count = bytes[11];
            let coupled_count = bytes[12];
            let map_start = DOPS_FIXED_LEN + 2;
            let map_end = (map_start + output_channel_count as usize).min(bytes.len());
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
            n += 2 + m.channel_mapping.len();
        }
        n
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        buf[0] = self.version;
        buf[1] = self.output_channel_count;
        buf[2..4].copy_from_slice(&self.pre_skip.to_be_bytes());
        buf[4..8].copy_from_slice(&self.input_sample_rate.to_be_bytes());
        buf[8..10].copy_from_slice(&self.output_gain.to_be_bytes());
        buf[10] = self.channel_mapping_family;
        if let Some(ref m) = self.channel_mapping {
            buf[11] = m.stream_count;
            buf[12] = m.coupled_count;
            buf[13..13 + m.channel_mapping.len()].copy_from_slice(&m.channel_mapping);
        }
        Ok(need)
    }
}
