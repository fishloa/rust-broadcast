//! QuickTime sound sample descriptions, versions 0, 1 and 2 — read only.
//!
//! ISO/IEC 14496-12:2026 §12.2.3 describes a sound sample entry as a 28-byte
//! fixed part followed by children. QuickTime's versions 1 and 2 extend that
//! fixed part; ffmpeg's QuickTime writer also emits version 1 without the
//! extension, so the children offset is read from the bytes instead of assumed.
//!
//! Source: `private/specs-verified/apple/isobmff/quicktime-file-format-online.md`,
//! "Sound sample description version 1 ('stsd')" and "Sound sample description
//! version 2 ('stsd')".

use alloc::vec::Vec;
use broadcast_common::Parse;
use broadcast_common::len::need_in;
use broadcast_common::wire::{i16_at, u16_at, u32_at, u64_at};

use crate::box_support::OpaqueBox;
use crate::box_types::{BOX_HEADER_MIN_SIZE, BOX_TYPE_SIZE, BoxIter};
use crate::error::{Error, Result};
use crate::mp4esds::EsdsBox;
use crate::quicktime::wave::WaveAtom;

/// Bytes of an ISO `AudioSampleEntry` before its children (§12.2.3).
pub const V0_FIXED_LEN: usize = 28;
/// Bytes of a QuickTime version 1 sound description before its children.
pub const V1_FIXED_LEN: usize = 44;
/// Bytes of a QuickTime version 2 sound description before its children.
pub const V2_FIXED_LEN: usize = 64;

/// QuickTime's `revision_level` + `vendor` width.
const QT_RESERVED_LEN: usize = 6;
/// `compression_ID` + `packet_size` width.
const COMPRESSION_LEN: usize = BOX_TYPE_SIZE;

/// Offset of the version field inside a sound entry's body.
const VERSION_OFFSET: usize = 8;
/// Offset of the `revision_level`/`vendor` block inside the body.
const RESERVED_OFFSET: usize = 10;
/// Offset of `data_reference_index` inside the body.
const DATA_REFERENCE_INDEX_OFFSET: usize = 6;
/// Offset of `channelcount` inside the body.
const CHANNELCOUNT_OFFSET: usize = 16;
/// Offset of `samplesize` inside the body.
const SAMPLESIZE_OFFSET: usize = 18;
/// Offset of `compression_ID`/`packet_size` inside the body.
const COMPRESSION_OFFSET: usize = 20;
/// Offset of the 16.16 `samplerate` field inside the body.
const SAMPLERATE_OFFSET: usize = 24;
/// Offset of `samples_per_packet` in a version 1 body.
const V1_SAMPLES_PER_PACKET_OFFSET: usize = 28;
/// Offset of `bytes_per_packet` in a version 1 body.
const V1_BYTES_PER_PACKET_OFFSET: usize = 32;
/// Offset of `bytes_per_frame` in a version 1 body.
const V1_BYTES_PER_FRAME_OFFSET: usize = 36;
/// Offset of `bytes_per_sample` in a version 1 body.
const V1_BYTES_PER_SAMPLE_OFFSET: usize = 40;
/// Offset of `always3` in a version 2 body.
const V2_ALWAYS3_OFFSET: usize = 16;
/// Offset of `always16` in a version 2 body.
const V2_ALWAYS16_OFFSET: usize = 18;
/// Offset of `alwaysMinus2` in a version 2 body.
const V2_ALWAYS_MINUS2_OFFSET: usize = 20;
/// Offset of `always0` in a version 2 body.
const V2_ALWAYS0_OFFSET: usize = 22;
/// Offset of `always65536` in a version 2 body.
const V2_ALWAYS65536_OFFSET: usize = 24;
/// Offset of `sizeOfStructOnly` in a version 2 body.
const V2_SIZE_OF_STRUCT_ONLY_OFFSET: usize = 28;
/// Offset of the 64-bit `audioSampleRate` in a version 2 body.
const V2_AUDIO_SAMPLE_RATE_OFFSET: usize = 32;
/// Offset of `numAudioChannels` in a version 2 body.
const V2_NUM_AUDIO_CHANNELS_OFFSET: usize = 40;
/// Offset of `always7F000000` in a version 2 body.
const V2_ALWAYS_7F000000_OFFSET: usize = 44;
/// Offset of `constBitsPerChannel` in a version 2 body.
const V2_CONST_BITS_PER_CHANNEL_OFFSET: usize = 48;
/// Offset of `formatSpecificFlags` in a version 2 body.
const V2_FORMAT_SPECIFIC_FLAGS_OFFSET: usize = 52;
/// Offset of `constBytesPerAudioPacket` in a version 2 body.
const V2_CONST_BYTES_PER_AUDIO_PACKET_OFFSET: usize = 56;
/// Offset of `constLPCMFramesPerAudioPacket` in a version 2 body.
const V2_CONST_LPCM_FRAMES_PER_AUDIO_PACKET_OFFSET: usize = 60;

/// The four 32-bit fields a QuickTime version 1 sound description adds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SoundDescriptionV1 {
    /// Samples in one packet.
    pub samples_per_packet: u32,
    /// Bytes in one packet.
    pub bytes_per_packet: u32,
    /// Bytes in one frame.
    pub bytes_per_frame: u32,
    /// Bytes in one sample.
    pub bytes_per_sample: u32,
}

/// The fields a QuickTime version 2 sound description adds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SoundDescriptionV2 {
    /// `const` field that is always 3.
    pub always3: u16,
    /// `const` field that is always 16.
    pub always16: u16,
    /// `const` field that is always -2.
    pub always_minus2: i16,
    /// `const` field that is always 0.
    pub always0: u16,
    /// `const` field that is always 65536.
    pub always65536: u32,
    /// Offset to the sound description's extensions.
    pub size_of_struct_only: u32,
    /// The 64-bit IEEE 754 sample rate, raw bits so the entry stays `Eq`.
    pub audio_sample_rate_bits: u64,
    /// Number of audio channels.
    pub num_audio_channels: u32,
    /// `const` field that is always `0x7F000000`.
    pub always_7f000000: u32,
    /// Bits per channel when constant.
    pub const_bits_per_channel: u32,
    /// LPCM format-specific flags.
    pub format_specific_flags: u32,
    /// Bytes per audio packet when constant.
    pub const_bytes_per_audio_packet: u32,
    /// LPCM frames per audio packet when constant.
    pub const_lpcm_frames_per_audio_packet: u32,
}

impl SoundDescriptionV2 {
    /// The `audioSampleRate` field as the 64-bit float the wire carries.
    #[must_use]
    pub fn audio_sample_rate(&self) -> f64 {
        f64::from_bits(self.audio_sample_rate_bits)
    }
}

/// The QuickTime extension a sound sample entry carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum QuickTimeSound {
    /// A QuickTime version 1 sound description's four extra fields.
    V1(SoundDescriptionV1),
    /// A QuickTime version 2 sound description.
    V2(SoundDescriptionV2),
}

impl QuickTimeSound {
    /// The sound description version this extension belongs to.
    #[must_use]
    pub fn version(&self) -> u16 {
        match self {
            Self::V1(_) => 1,
            Self::V2(_) => 2,
        }
    }
}

/// A QuickTime sound sample entry this crate has no ISO typed form for
/// (`lpcm`, `in24`, `sowt`, …). Read only.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct SoundEntry {
    /// The entry's four-CC (`lpcm`, `in24`, …).
    pub codec_type: [u8; BOX_TYPE_SIZE],
    /// `entry_version` verbatim.
    pub entry_version: u16,
    /// QuickTime's `revision_level` + `vendor` (ISO's `reserved[3]`).
    pub reserved_1: [u8; QT_RESERVED_LEN],
    /// `data_reference_index`.
    pub data_reference_index: u16,
    /// `channelcount` as the fixed field carries it.
    pub channelcount: u16,
    /// `samplesize` as the fixed field carries it.
    pub samplesize: u16,
    /// `compression_ID` + `packet_size`.
    pub compression_id_and_packet_size: [u8; COMPRESSION_LEN],
    /// The 16.16 `samplerate` field as written.
    pub samplerate: u32,
    /// The version 1/2 extension, when the entry carries one.
    pub quicktime: Option<QuickTimeSound>,
    /// Every child after the fixed part, verbatim.
    pub config_boxes: Vec<OpaqueBox>,
}

/// The `wave` atom among a sound entry's children, when present.
#[must_use]
pub fn wave_of(config_boxes: &[OpaqueBox]) -> Option<WaveAtom> {
    let child = config_boxes
        .iter()
        .find(|b| b.box_type == crate::quicktime::wave::WAVE)?;
    WaveAtom::parse_body(&child.data).ok()
}

/// The elementary stream descriptor of a sound entry, whether it sits beside
/// the fixed fields or inside a `wave` atom.
#[must_use]
pub fn esds_of(config_boxes: &[OpaqueBox]) -> Option<EsdsBox> {
    if let Some(child) = config_boxes
        .iter()
        .find(|b| b.box_type == crate::quicktime::wave::ESDS)
        && let Ok(esds) = EsdsBox::parse_body(&child.data)
    {
        return Some(esds);
    }
    wave_of(config_boxes).and_then(|wave| wave.esds)
}

impl SoundEntry {
    /// The real sample rate: the version 2 64-bit float when present, otherwise
    /// the integer part of the 16.16 field.
    #[must_use]
    pub fn sample_rate_hz(&self) -> f64 {
        match self.quicktime {
            Some(QuickTimeSound::V2(v2)) => v2.audio_sample_rate(),
            _ => f64::from(self.samplerate >> SAMPLERATE_INTEGER_SHIFT),
        }
    }

    /// The `wave` atom among this entry's children, when present.
    #[must_use]
    pub fn wave(&self) -> Option<WaveAtom> {
        wave_of(&self.config_boxes)
    }

    /// The entry's elementary stream descriptor, whether it sits beside the
    /// fixed fields or inside a `wave` atom.
    #[must_use]
    pub fn esds(&self) -> Option<EsdsBox> {
        esds_of(&self.config_boxes)
    }
}

impl<'a> Parse<'a> for SoundEntry {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        need_in(
            bytes,
            BOX_HEADER_MIN_SIZE + V0_FIXED_LEN,
            "QuickTime sound entry",
        )?;
        let mut codec_type = [0u8; BOX_TYPE_SIZE];
        codec_type.copy_from_slice(&bytes[crate::box_types::BOX_TYPE_OFFSET..BOX_HEADER_MIN_SIZE]);
        let body = crate::box_types::box_body(bytes, V0_FIXED_LEN, "QuickTime sound entry")?;
        let entry_version = u16_at(body, VERSION_OFFSET);
        let mut reserved_1 = [0u8; QT_RESERVED_LEN];
        reserved_1.copy_from_slice(&body[RESERVED_OFFSET..RESERVED_OFFSET + QT_RESERVED_LEN]);
        let mut compression = [0u8; COMPRESSION_LEN];
        compression
            .copy_from_slice(&body[COMPRESSION_OFFSET..COMPRESSION_OFFSET + COMPRESSION_LEN]);
        let (quicktime, children_at) = match entry_version {
            QUICKTIME_VERSION_1 => v1_extension(body)?,
            QUICKTIME_VERSION_2 => {
                need_in(
                    bytes,
                    BOX_HEADER_MIN_SIZE + V2_FIXED_LEN,
                    "QuickTime v2 sound entry",
                )?;
                (Some(parse_v2(body)), V2_FIXED_LEN)
            }
            _ => (None, V0_FIXED_LEN),
        };
        let mut config_boxes = Vec::new();
        for item in BoxIter::clamped(body.get(children_at..).unwrap_or(&[])) {
            let Ok(item) = item else {
                break;
            };
            if item.to_end() || item.has_largesize() {
                break;
            }
            config_boxes.push(OpaqueBox::new(item.four_cc(), item.payload().to_vec()));
        }
        Ok(Self {
            codec_type,
            entry_version,
            reserved_1,
            data_reference_index: u16_at(body, DATA_REFERENCE_INDEX_OFFSET),
            channelcount: u16_at(body, CHANNELCOUNT_OFFSET),
            samplesize: u16_at(body, SAMPLESIZE_OFFSET),
            compression_id_and_packet_size: compression,
            samplerate: u32_at(body, SAMPLERATE_OFFSET),
            quicktime,
            config_boxes,
        })
    }
}

/// Parse the version 1/2 extension of an audio sample entry and return it with
/// the body offset its children start at.
pub(crate) fn parse_extension(
    _bytes: &[u8],
    body: &[u8],
) -> Result<(Option<QuickTimeSound>, usize)> {
    match u16_at(body, VERSION_OFFSET) {
        QUICKTIME_VERSION_1 => v1_extension(body),
        QUICKTIME_VERSION_2 => {
            if body.len() < V2_FIXED_LEN {
                // A synthetic or legacy `entry_version = 2` with no version 2
                // fields: keep the ISO layout rather than rejecting the file.
                return Ok((None, V0_FIXED_LEN));
            }
            Ok((Some(parse_v2(body)), V2_FIXED_LEN))
        }
        _ => Ok((None, V0_FIXED_LEN)),
    }
}

/// The version 1 extension and the body offset its children start at.
///
/// ffmpeg writes `entry_version = 1` without the four version 1 fields and puts
/// the first child immediately after the 28-byte fixed part; a QuickTime-written
/// entry puts four 32-bit fields there instead. A child box whose type is a
/// printable four-CC and whose size fits the entry is the discriminator.
fn v1_extension(body: &[u8]) -> Result<(Option<QuickTimeSound>, usize)> {
    if plausible_child_at(body, V0_FIXED_LEN) {
        return Ok((None, V0_FIXED_LEN));
    }
    if body.len() < V1_FIXED_LEN {
        return Ok((None, V0_FIXED_LEN));
    }
    Ok((
        Some(QuickTimeSound::V1(SoundDescriptionV1 {
            samples_per_packet: u32_at(body, V1_SAMPLES_PER_PACKET_OFFSET),
            bytes_per_packet: u32_at(body, V1_BYTES_PER_PACKET_OFFSET),
            bytes_per_frame: u32_at(body, V1_BYTES_PER_FRAME_OFFSET),
            bytes_per_sample: u32_at(body, V1_BYTES_PER_SAMPLE_OFFSET),
        })),
        V1_FIXED_LEN,
    ))
}

fn parse_v2(body: &[u8]) -> QuickTimeSound {
    QuickTimeSound::V2(SoundDescriptionV2 {
        always3: u16_at(body, V2_ALWAYS3_OFFSET),
        always16: u16_at(body, V2_ALWAYS16_OFFSET),
        always_minus2: i16_at(body, V2_ALWAYS_MINUS2_OFFSET),
        always0: u16_at(body, V2_ALWAYS0_OFFSET),
        always65536: u32_at(body, V2_ALWAYS65536_OFFSET),
        size_of_struct_only: u32_at(body, V2_SIZE_OF_STRUCT_ONLY_OFFSET),
        audio_sample_rate_bits: u64_at(body, V2_AUDIO_SAMPLE_RATE_OFFSET),
        num_audio_channels: u32_at(body, V2_NUM_AUDIO_CHANNELS_OFFSET),
        always_7f000000: u32_at(body, V2_ALWAYS_7F000000_OFFSET),
        const_bits_per_channel: u32_at(body, V2_CONST_BITS_PER_CHANNEL_OFFSET),
        format_specific_flags: u32_at(body, V2_FORMAT_SPECIFIC_FLAGS_OFFSET),
        const_bytes_per_audio_packet: u32_at(body, V2_CONST_BYTES_PER_AUDIO_PACKET_OFFSET),
        const_lpcm_frames_per_audio_packet: u32_at(
            body,
            V2_CONST_LPCM_FRAMES_PER_AUDIO_PACKET_OFFSET,
        ),
    })
}

/// Whether a box header at `offset` has a printable type and a size that fits.
fn plausible_child_at(body: &[u8], offset: usize) -> bool {
    let Some(header) = body.get(offset..offset + BOX_HEADER_MIN_SIZE) else {
        return false;
    };
    let size = u32_at(header, 0);
    let ty = &header[crate::box_types::BOX_TYPE_OFFSET..BOX_HEADER_MIN_SIZE];
    if !ty.iter().all(|b| is_graphic(*b)) || ty == [0u8; BOX_TYPE_SIZE] {
        return false;
    }
    match size {
        0 => true,
        1 => false,
        size => {
            let size = usize::try_from(size).unwrap_or(usize::MAX);
            size >= BOX_HEADER_MIN_SIZE
                && offset
                    .checked_add(size)
                    .is_some_and(|end| end <= body.len())
        }
    }
}

fn is_graphic(byte: u8) -> bool {
    byte.is_ascii_graphic() || byte == SPACE || byte == COPYRIGHT_SIGN
}

/// The four-CC `©` byte used by QuickTime user-data atoms.
const COPYRIGHT_SIGN: u8 = 0xA9;
/// The printable space.
const SPACE: u8 = 0x20;
/// Bits the integer part of a 16.16 fixed-point value is shifted by.
const SAMPLERATE_INTEGER_SHIFT: u32 = 16;
/// QuickTime sound description version 1.
const QUICKTIME_VERSION_1: u16 = 1;
/// QuickTime sound description version 2.
const QUICKTIME_VERSION_2: u16 = 2;
