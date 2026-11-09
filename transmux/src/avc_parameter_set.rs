//! AVC parameter set stream — ISO/IEC 14496-15:2017 §5.4.3.
//!
//! A parameter set stream carries the SPS/PPS of a video track in a track of
//! its own: the `avcp` sample entry (§5.4.3.1), whose `avcC` record holds no
//! parameter sets, the parameter set sample (§5.4.3.2), and the `avcp` track
//! reference from the video track (§5.4.3.3, see [`crate::track_reference`]).
//!
//! The IR has no track kind for a parameter set stream, so demuxing a file
//! that contains one still reports [`Error::UnsupportedSampleEntry`] for that
//! track; these types parse and serialize its wire structures losslessly.
//!
//! Spec input: `private/specs-verified/mpeg/isobmff/iso-iec-14496-15-2017-avc-hevc-config.md`.

use crate::annexb::iter_length_prefixed_nals_with;
use crate::error::{Error, Result};
use crate::nalu_types::{AvcNalUnit, VALID_LENGTH_SIZES};
use crate::sample_entries::AVCSampleEntry;
use alloc::vec::Vec;
use broadcast_common::Serialize;
use broadcast_common::error::InvalidValue;
use broadcast_common::len::{fit_bits, need_out};

/// Sample entry four-CC of an AVC parameter set stream (§5.4.3.1).
pub const AVCP_SAMPLE_ENTRY: [u8; 4] = *b"avcp";

const BITS_PER_BYTE: u32 = 8;

/// `AVCParameterSampleEntry` (`avcp`) — ISO/IEC 14496-15:2017 §5.4.3.1: a
/// `VisualSampleEntry` and an `AVCConfigurationBox` whose record shall declare
/// no SPS and no PPS.
///
/// Parsing accepts a record that does carry parameter sets, like every other
/// config record here; [`Self::declares_no_parameter_sets`] reports whether the
/// rule holds.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct AVCParameterSampleEntry {
    /// The sample entry: `codec_type` is [`AVCP_SAMPLE_ENTRY`].
    pub entry: AVCSampleEntry,
}

impl AVCParameterSampleEntry {
    /// Parse an `avcp` sample entry from its whole box bytes (header included).
    pub fn bare_parse(bytes: &[u8]) -> Result<Self> {
        let entry = AVCSampleEntry::bare_parse(bytes)?;
        if entry.codec_type != AVCP_SAMPLE_ENTRY {
            return Err(Error::InvalidValue(InvalidValue {
                field: "sample entry type",
                value: u64::from(u32::from_be_bytes(entry.codec_type)),
                reason: "an AVCParameterSampleEntry is an 'avcp' entry (ISO/IEC 14496-15:2017 §5.4.3.1)",
            }));
        }
        Ok(Self { entry })
    }

    /// Whether the record holds no SPS and no PPS, as §5.4.3.1 requires.
    pub fn declares_no_parameter_sets(&self) -> bool {
        let record = &self.entry.config.config;
        record.sps.is_empty() && record.pps.is_empty()
    }
}

impl Serialize for AVCParameterSampleEntry {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        self.entry.serialized_len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        self.entry.serialize_into(buf)
    }
}

/// `AVCParameterSample` — ISO/IEC 14496-15:2017 §5.4.3.2: the NAL units of one
/// parameter set sample, each behind a `NALUnitLength` field of
/// `lengthSizeMinusOne + 1` bytes.
///
/// The length size comes from the track's `AVCDecoderConfigurationRecord`, so
/// parsing takes it as an argument ([`Self::parse_with`]); serializing uses the
/// value the sample was parsed with.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct AVCParameterSample {
    /// `AVCDecoderConfigurationRecord.lengthSizeMinusOne` (0, 1 or 3).
    pub length_size_minus_one: u8,
    /// The NAL units (parameter sets, or declarative SEI), in sample order.
    pub nal_units: Vec<AvcNalUnit>,
}

impl AVCParameterSample {
    /// Parse a sample of `sample_size` bytes (the whole slice) whose NAL
    /// length fields are `length_size_minus_one + 1` bytes wide.
    pub fn parse_with(bytes: &[u8], length_size_minus_one: u8) -> Result<Self> {
        if !VALID_LENGTH_SIZES.contains(&length_size_minus_one) {
            return Err(Error::InvalidValue(InvalidValue {
                field: "lengthSizeMinusOne",
                value: u64::from(length_size_minus_one),
                reason: "must be 0, 1, or 3 (ISO/IEC 14496-15:2017 §5.3.3.1.3)",
            }));
        }
        let nal_units =
            iter_length_prefixed_nals_with(bytes, usize::from(length_size_minus_one) + 1)?
                .into_iter()
                .map(|nal| AvcNalUnit(nal.to_vec()))
                .collect();
        Ok(Self {
            length_size_minus_one,
            nal_units,
        })
    }

    fn length_field_len(&self) -> usize {
        usize::from(self.length_size_minus_one) + 1
    }
}

impl Serialize for AVCParameterSample {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        self.nal_units
            .iter()
            .map(|n| self.length_field_len() + n.0.len())
            .sum()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        if !VALID_LENGTH_SIZES.contains(&self.length_size_minus_one) {
            return Err(Error::InvalidValue(InvalidValue {
                field: "lengthSizeMinusOne",
                value: u64::from(self.length_size_minus_one),
                reason: "must be 0, 1, or 3 (ISO/IEC 14496-15:2017 §5.3.3.1.3)",
            }));
        }
        let need = self.serialized_len();
        need_out(buf, need)?;
        let width = self.length_field_len();
        let bits = BITS_PER_BYTE * (u32::from(self.length_size_minus_one) + 1);
        let mut cursor = 0usize;
        for nal in &self.nal_units {
            let len = fit_bits(nal.0.len() as u64, bits, "NALUnitLength")?;
            buf[cursor..cursor + width].copy_from_slice(&len.to_be_bytes()[8 - width..]);
            cursor += width;
            buf[cursor..cursor + nal.0.len()].copy_from_slice(&nal.0);
            cursor += nal.0.len();
        }
        Ok(cursor)
    }
}
