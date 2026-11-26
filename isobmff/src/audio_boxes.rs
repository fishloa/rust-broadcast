//! Audio signalling boxes — ISO/IEC 14496-12:2026 §12.2.4, §12.2.5, §12.2.7,
//! §8.10.5.
//!
//! | Box | Type | Clause | Container |
//! |-----|------|--------|-----------|
//! | [`ChannelLayoutBox`] | `chnl` | §12.2.4 | audio sample entry |
//! | [`DownMixInstructionsBox`] | `dmix` | §12.2.5 | audio sample entry |
//! | [`LoudnessBox`] | `ludt` | §12.2.7 | track `udta` |
//! | [`TrackLoudnessInfo`] | `tlou` | §12.2.7 | `ludt` |
//! | [`AlbumLoudnessInfo`] | `alou` | §12.2.7 | `ludt` |
//! | [`LabelBox`] | `labl` | §8.10.5 | track `udta` |
//!
//! Every type implements the symmetric [`Parse`]/[`Serialize`] pair. The
//! bit-packed bodies are read and written with
//! [`broadcast_core::bits`] so a reserved field's width
//! is never an arithmetic guess.

use alloc::vec::Vec;

use broadcast_core::bits::{BitBuf, BitReader};
use broadcast_core::len::{fit_u8, need_in, need_out};
use broadcast_core::wire::{be_u16, be_u32, u24_at};
use broadcast_core::{Parse, Serialize};

use crate::box_support::{bounded_entry_count, take_c_string, write_c_string};
use crate::box_types::{
    BOX_HEADER_MIN_SIZE, BOX_TYPE_OFFSET, BOX_TYPE_SIZE, FULLBOX_EXTRA_SIZE, FULLBOX_FLAGS_MASK,
    box_len, write_full_box_header,
};
use crate::error::{Error, Result};

/// Four-CC of the Channel Layout Box (`chnl`, ISO/IEC 14496-12:2026 §12.2.4).
pub const CHNL: [u8; BOX_TYPE_SIZE] = *b"chnl";
/// Four-CC of the DownMix Instructions Box (`dmix`, §12.2.5).
pub const DMIX: [u8; BOX_TYPE_SIZE] = *b"dmix";
/// Four-CC of the Audio Stream Loudness Box (`ludt`, §12.2.7).
pub const LUDT: [u8; BOX_TYPE_SIZE] = *b"ludt";
/// Four-CC of the Track Loudness Info Box (`tlou`, §12.2.7).
pub const TLOU: [u8; BOX_TYPE_SIZE] = *b"tlou";
/// Four-CC of the Album Loudness Info Box (`alou`, §12.2.7).
pub const ALOU: [u8; BOX_TYPE_SIZE] = *b"alou";
/// Four-CC of the Label Box (`labl`, §8.10.5).
pub const LABL: [u8; BOX_TYPE_SIZE] = *b"labl";

/// `channelStructured` bit of `stream_structure` (§12.2.4.3).
pub const STREAM_STRUCTURE_CHANNEL: u8 = 0x01;
/// `objectStructured` bit of `stream_structure` (§12.2.4.3).
pub const STREAM_STRUCTURE_OBJECT: u8 = 0x02;
/// Version of the channel layout whose `stream_structure` is one byte.
const CHNL_VERSION_BYTE: u8 = 0;
/// `speaker_position` value that carries an explicit azimuth/elevation.
const SPEAKER_POSITION_EXPLICIT: u8 = 126;
/// Width of `omittedChannelsMap` in bytes (`unsigned int(64)`).
const OMITTED_CHANNELS_LEN: usize = 8;
/// Bits in one version-1 `stream_structure` field.
const V1_STREAM_STRUCTURE_BITS: u32 = 4;
/// Bits in one version-1 `reserved` field before `channel_order_definition`.
const V1_ORDER_RESERVED_BITS: u32 = 4;
/// Bits in `channel_order_definition`.
const V1_ORDER_DEFINITION_BITS: u32 = 3;

/// Bit width of an ISO/IEC 23091-3 `OutputChannelPosition`.
const SPEAKER_POSITION_BITS: u32 = 8;
/// Bit width of an explicit `azimuth`.
const AZIMUTH_BITS: u32 = 16;
/// Bit width of an explicit `elevation`.
const ELEVATION_BITS: u32 = 8;
/// Bits of the 64-bit `omittedChannelsMap`.
const OMITTED_CHANNELS_MAP_BITS: u32 = 64;
/// Bit width of a version-0 `stream_structure`.
const CHNL_V0_STREAM_STRUCTURE_BITS: u32 = 8;
/// Bit width of `targetLayout` in a downmix instruction.
const DMIX_TARGET_LAYOUT_BITS: u32 = 8;
/// Bit width of `mae_group_ID`.
const LOUDNESS_MAE_GROUP_ID_BITS: u32 = 7;
/// Bit width of `mae_group_preset_ID`.
const LOUDNESS_MAE_PRESET_BITS: u32 = 5;
/// Bit width of `EQ_set_ID`.
const LOUDNESS_EQ_SET_BITS: u32 = 6;
/// Bit width of `downmix_ID`.
const LOUDNESS_DOWNMIX_ID_BITS: u32 = 7;
/// Bit width of `DRC_set_ID`.
const LOUDNESS_DRC_SET_BITS: u32 = 6;
/// Bit width of a true-peak measurement system or a measurement system.
const LOUDNESS_MEASUREMENT_SYSTEM_BITS: u32 = 4;
/// Bit width of a true-peak reliability or a measurement reliability.
const LOUDNESS_RELIABILITY_BITS: u32 = 4;
/// Bit width of `measurement_count`.
const LOUDNESS_MEASUREMENT_COUNT_BITS: u32 = 8;
/// Bit width of `method_definition`.
const LOUDNESS_METHOD_DEFINITION_BITS: u32 = 8;
/// Bit width of `method_value`.
const LOUDNESS_METHOD_VALUE_BITS: u32 = 8;
/// Bits of the reserved field before `downmix_ID` in a loudness entry.
const LOUDNESS_ENTRY_RESERVED_BITS: u32 = 3;
/// Bits of the reserved field before `EQ_set_ID`.
const LOUDNESS_EQ_RESERVED_BITS: u32 = 2;
/// Bits of the reserved field before an MAE group id.
const LOUDNESS_MAE_RESERVED_BITS: u32 = 1;
/// Bits of the reserved field before `mae_group_preset_ID`.
const LOUDNESS_MAE_PRESET_RESERVED_BITS: u32 = 3;
/// Bits of the reserved field in a version-1 loudness header.
const LOUDNESS_LEGACY_RESERVED_BITS: u32 = 2;
/// Width of `label_id` in a `labl` box.
const LABL_LABEL_ID_LEN: usize = core::mem::size_of::<u16>();

/// One speaker position entry of a channel layout (§12.2.4.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct SpeakerPosition {
    speaker_position: u8,
    azimuth: Option<i16>,
    elevation: Option<i8>,
}

impl SpeakerPosition {
    /// A position named by its ISO/IEC 23091-3 `OutputChannelPosition`.
    #[must_use]
    pub fn named(speaker_position: u8) -> Self {
        Self {
            speaker_position,
            azimuth: None,
            elevation: None,
        }
    }

    /// An explicit position (`speaker_position == 126`) with its angles.
    #[must_use]
    pub fn explicit(azimuth: i16, elevation: i8) -> Self {
        Self {
            speaker_position: SPEAKER_POSITION_EXPLICIT,
            azimuth: Some(azimuth),
            elevation: Some(elevation),
        }
    }

    /// The ISO/IEC 23091-3 `OutputChannelPosition`, or `126` for explicit.
    #[must_use]
    pub const fn speaker_position(&self) -> u8 {
        self.speaker_position
    }

    /// The explicit azimuth in degrees, when this is an explicit position.
    #[must_use]
    pub const fn azimuth(&self) -> Option<i16> {
        self.azimuth
    }

    /// The explicit elevation in degrees, when this is an explicit position.
    #[must_use]
    pub const fn elevation(&self) -> Option<i8> {
        self.elevation
    }

    fn parse(reader: &mut BitReader<'_>) -> Result<Self> {
        let speaker_position =
            wire_bits(reader, SPEAKER_POSITION_BITS, "chnl speaker_position")? as u8;
        let (azimuth, elevation) = if speaker_position == SPEAKER_POSITION_EXPLICIT {
            let azimuth = wire_bits(reader, AZIMUTH_BITS, "chnl azimuth")? as u16 as i16;
            let elevation = wire_bits(reader, ELEVATION_BITS, "chnl elevation")? as u8 as i8;
            (Some(azimuth), Some(elevation))
        } else {
            (None, None)
        };
        Ok(Self {
            speaker_position,
            azimuth,
            elevation,
        })
    }

    fn serialize(&self, writer: &mut BitBuf) -> Result<()> {
        write_bits(
            writer,
            u64::from(self.speaker_position),
            SPEAKER_POSITION_BITS,
            "chnl speaker_position",
        )?;
        if self.speaker_position == SPEAKER_POSITION_EXPLICIT {
            let Some(azimuth) = self.azimuth else {
                return Err(invalid("chnl explicit position without azimuth"));
            };
            let Some(elevation) = self.elevation else {
                return Err(invalid("chnl explicit position without elevation"));
            };
            write_bits(
                writer,
                u64::from(azimuth as u16),
                AZIMUTH_BITS,
                "chnl azimuth",
            )?;
            write_bits(
                writer,
                u64::from(elevation as u8),
                ELEVATION_BITS,
                "chnl elevation",
            )?;
        }
        Ok(())
    }
}

/// The order of the formats in a version-1 channel stream (§12.2.4.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum FormatOrdering {
    /// The order is not stated.
    Unknown,
    /// Channels, possibly followed by objects.
    ChannelsThenObjects,
    /// Objects, possibly followed by channels.
    ObjectsThenChannels,
    /// A value the standard leaves reserved.
    Reserved(u8),
}

impl FormatOrdering {
    /// Spec token for this ordering (`"reserved"` for an unassigned code).
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::ChannelsThenObjects => "channels-then-objects",
            Self::ObjectsThenChannels => "objects-then-channels",
            Self::Reserved(_) => "reserved",
        }
    }

    /// The wire value.
    #[must_use]
    pub const fn code(&self) -> u8 {
        match self {
            Self::Unknown => 0,
            Self::ChannelsThenObjects => 1,
            Self::ObjectsThenChannels => 2,
            Self::Reserved(code) => *code,
        }
    }

    fn from_code(code: u8) -> Self {
        match code {
            0 => Self::Unknown,
            1 => Self::ChannelsThenObjects,
            2 => Self::ObjectsThenChannels,
            other => Self::Reserved(other),
        }
    }
}

broadcast_core::impl_spec_display!(FormatOrdering, Reserved);

/// Channel Layout Box (`chnl`) — ISO/IEC 14496-12:2026 §12.2.4.
///
/// Versions 0 and 1 share the [`Self::stream_structure`] flags;
/// [`Self::format_ordering`] and [`Self::base_channel_count`] exist only in
/// version 1.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct ChannelLayoutBox {
    version: u8,
    flags: u32,
    stream_structure: u8,
    format_ordering: Option<FormatOrdering>,
    base_channel_count: Option<u8>,
    defined_layout: Option<u8>,
    channel_order_definition: Option<u8>,
    omitted_channels_map: Option<u64>,
    layout_channel_count: Option<u8>,
    speaker_positions: Vec<SpeakerPosition>,
    object_count: Option<u8>,
}

impl ChannelLayoutBox {
    /// Build a version-0 box from its structure flags.
    #[must_use]
    pub fn new(stream_structure: u8) -> Self {
        Self {
            version: CHNL_VERSION_BYTE,
            flags: 0,
            stream_structure,
            format_ordering: None,
            base_channel_count: None,
            defined_layout: None,
            channel_order_definition: None,
            omitted_channels_map: None,
            layout_channel_count: None,
            speaker_positions: Vec::new(),
            object_count: None,
        }
    }

    /// Set the `definedLayout` (an ISO/IEC 23091-3 `ChannelConfiguration`).
    #[must_use]
    pub fn with_defined_layout(mut self, defined_layout: u8) -> Self {
        self.defined_layout = Some(defined_layout);
        self
    }

    /// Set the `omittedChannelsMap`.
    #[must_use]
    pub fn with_omitted_channels_map(mut self, map: u64) -> Self {
        self.omitted_channels_map = Some(map);
        self
    }

    /// Replace the explicit speaker-position list.
    #[must_use]
    pub fn with_speaker_positions(mut self, positions: Vec<SpeakerPosition>) -> Self {
        self.speaker_positions = positions;
        self
    }

    /// Set the `object_count` (version 0).
    #[must_use]
    pub fn with_object_count(mut self, object_count: u8) -> Self {
        self.object_count = Some(object_count);
        self
    }

    /// The FullBox version (0 or 1).
    #[must_use]
    pub const fn version(&self) -> u8 {
        self.version
    }

    /// The FullBox flags.
    #[must_use]
    pub const fn flags(&self) -> u32 {
        self.flags
    }

    /// The `stream_structure` flags (§12.2.4.3).
    #[must_use]
    pub const fn stream_structure(&self) -> u8 {
        self.stream_structure
    }

    /// Whether the stream carries channels (`channelStructured`).
    #[must_use]
    pub const fn is_channel_structured(&self) -> bool {
        self.stream_structure & STREAM_STRUCTURE_CHANNEL != 0
    }

    /// Whether the stream carries objects (`objectStructured`).
    #[must_use]
    pub const fn is_object_structured(&self) -> bool {
        self.stream_structure & STREAM_STRUCTURE_OBJECT != 0
    }

    /// The version-1 `format_ordering`, when the box is version 1.
    #[must_use]
    pub const fn format_ordering(&self) -> Option<FormatOrdering> {
        self.format_ordering
    }

    /// The version-1 `baseChannelCount`.
    #[must_use]
    pub const fn base_channel_count(&self) -> Option<u8> {
        self.base_channel_count
    }

    /// The `definedLayout` (ISO/IEC 23091-3 `ChannelConfiguration`), when one is
    /// signalled.
    #[must_use]
    pub const fn defined_layout(&self) -> Option<u8> {
        self.defined_layout
    }

    /// The version-1 `channel_order_definition`.
    #[must_use]
    pub const fn channel_order_definition(&self) -> Option<u8> {
        self.channel_order_definition
    }

    /// The `omittedChannelsMap`, when present.
    #[must_use]
    pub const fn omitted_channels_map(&self) -> Option<u64> {
        self.omitted_channels_map
    }

    /// The version-1 `layout_channel_count`, when carried.
    #[must_use]
    pub const fn layout_channel_count(&self) -> Option<u8> {
        self.layout_channel_count
    }

    /// The explicit speaker-position list, in channel order.
    #[must_use]
    pub fn speaker_positions(&self) -> &[SpeakerPosition] {
        &self.speaker_positions
    }

    /// The version-0 `object_count`.
    #[must_use]
    pub const fn object_count(&self) -> Option<u8> {
        self.object_count
    }

    /// Parse the payload after the 8-byte box header.
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        need_in(body, FULLBOX_EXTRA_SIZE, "chnl body")?;
        let version = body[0];
        let flags = u24_at(body, 1) & FULLBOX_FLAGS_MASK;
        let mut reader = BitReader::new(&body[FULLBOX_EXTRA_SIZE..]);
        let mut layout = Self {
            version,
            flags,
            stream_structure: 0,
            format_ordering: None,
            base_channel_count: None,
            defined_layout: None,
            channel_order_definition: None,
            omitted_channels_map: None,
            layout_channel_count: None,
            speaker_positions: Vec::new(),
            object_count: None,
        };
        if version == CHNL_VERSION_BYTE {
            layout.stream_structure = wire_bits(
                &mut reader,
                CHNL_V0_STREAM_STRUCTURE_BITS,
                "chnl stream_structure",
            )? as u8;
            Self::parse_structure(&mut layout, &mut reader, None)?;
        } else {
            layout.stream_structure = wire_bits(
                &mut reader,
                V1_STREAM_STRUCTURE_BITS,
                "chnl stream_structure",
            )? as u8;
            layout.format_ordering = Some(FormatOrdering::from_code(wire_bits(
                &mut reader,
                V1_STREAM_STRUCTURE_BITS,
                "chnl format_ordering",
            )? as u8));
            layout.base_channel_count =
                Some(wire_bits(&mut reader, u8::BITS, "chnl baseChannelCount")? as u8);
            Self::parse_structure(&mut layout, &mut reader, Some(version))?;
        }
        Ok(layout)
    }

    fn parse_structure(
        layout: &mut Self,
        reader: &mut BitReader<'_>,
        version: Option<u8>,
    ) -> Result<()> {
        if layout.stream_structure & STREAM_STRUCTURE_CHANNEL != 0 {
            let defined_layout = wire_bits(reader, u8::BITS, "chnl definedLayout")? as u8;
            layout.defined_layout = Some(defined_layout);
            if defined_layout == 0 {
                if version.is_some() {
                    let count = wire_bits(reader, u8::BITS, "chnl layout_channel_count")? as usize;
                    layout.layout_channel_count = Some(count as u8);
                    layout.speaker_positions = read_positions(reader, count)?;
                } else {
                    let mut positions = Vec::new();
                    while reader.bits_remaining() >= u8::BITS as usize {
                        positions.push(SpeakerPosition::parse(reader)?);
                    }
                    layout.speaker_positions = positions;
                }
            } else if version.is_some() {
                wire_bits(reader, V1_ORDER_RESERVED_BITS, "chnl reserved")?;
                let order =
                    wire_bits(reader, V1_ORDER_DEFINITION_BITS, "chnl channel_order")? as u8;
                layout.channel_order_definition = Some(order);
                let present = wire_bits(reader, 1, "chnl omitted_channels_present")? != 0;
                if present {
                    layout.omitted_channels_map =
                        Some(wire_bits(reader, OMITTED_CHANNELS_MAP_BITS, "chnl map")?);
                }
            } else {
                layout.omitted_channels_map =
                    Some(wire_bits(reader, OMITTED_CHANNELS_MAP_BITS, "chnl map")?);
            }
        }
        if layout.stream_structure & STREAM_STRUCTURE_OBJECT != 0 && version.is_none() {
            layout.object_count = Some(wire_bits(reader, u8::BITS, "chnl object_count")? as u8);
        }
        Ok(())
    }
}

fn read_positions(reader: &mut BitReader<'_>, count: usize) -> Result<Vec<SpeakerPosition>> {
    let mut positions = Vec::with_capacity(count);
    for _ in 0..count {
        positions.push(SpeakerPosition::parse(reader)?);
    }
    Ok(positions)
}

impl<'a> Parse<'a> for ChannelLayoutBox {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if be_u32(bytes, BOX_TYPE_OFFSET, "chnl box")? != u32::from_be_bytes(CHNL) {
            return Err(invalid("expected chnl"));
        }
        Self::parse_body(crate::box_types::box_body(
            bytes,
            FULLBOX_EXTRA_SIZE,
            "chnl box",
        )?)
    }
}

impl Serialize for ChannelLayoutBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + self.body_len())
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut c = write_full_box_header(buf, &CHNL, need, self.version, self.flags, false)?;
        c += self.write_body(&mut buf[c..])?;
        Ok(c)
    }
}

impl ChannelLayoutBox {
    fn body_len(&self) -> usize {
        let mut len = if self.version == CHNL_VERSION_BYTE {
            1
        } else {
            core::mem::size_of::<u16>()
        };
        if self.is_channel_structured() {
            len += 1;
            if self.defined_layout == Some(0) {
                if self.version != CHNL_VERSION_BYTE {
                    len += 1;
                }
                len += position_len(&self.speaker_positions);
            } else if self.version != CHNL_VERSION_BYTE {
                len += 1;
                if self.omitted_channels_map.is_some() {
                    len += OMITTED_CHANNELS_LEN;
                }
            } else {
                len += OMITTED_CHANNELS_LEN;
            }
        }
        if self.is_object_structured() && self.version == CHNL_VERSION_BYTE {
            len += 1;
        }
        len
    }

    fn write_body(&self, buf: &mut [u8]) -> Result<usize> {
        let mut writer = BitBuf::new();
        if self.version == CHNL_VERSION_BYTE {
            write_bits(
                &mut writer,
                u64::from(self.stream_structure),
                CHNL_V0_STREAM_STRUCTURE_BITS,
                "chnl stream_structure",
            )?;
            self.write_structure(&mut writer)?;
        } else {
            write_bits(
                &mut writer,
                u64::from(self.stream_structure),
                V1_STREAM_STRUCTURE_BITS,
                "chnl stream_structure",
            )?;
            let ordering = self
                .format_ordering
                .ok_or_else(|| invalid("chnl missing format_ordering"))?;
            write_bits(
                &mut writer,
                u64::from(ordering.code()),
                V1_STREAM_STRUCTURE_BITS,
                "chnl format_ordering",
            )?;
            let base = self
                .base_channel_count
                .ok_or_else(|| invalid("chnl missing baseChannelCount"))?;
            write_bits(
                &mut writer,
                u64::from(base),
                u8::BITS,
                "chnl baseChannelCount",
            )?;
            self.write_structure(&mut writer)?;
        }
        writer.align_to_byte();
        let bytes = writer.as_bytes();
        buf[..bytes.len()].copy_from_slice(bytes);
        Ok(bytes.len())
    }

    fn write_structure(&self, writer: &mut BitBuf) -> Result<()> {
        if self.is_channel_structured() {
            let defined_layout = self
                .defined_layout
                .ok_or_else(|| invalid("chnl missing definedLayout"))?;
            write_bits(
                writer,
                u64::from(defined_layout),
                u8::BITS,
                "chnl definedLayout",
            )?;
            if defined_layout == 0 {
                if self.version != CHNL_VERSION_BYTE {
                    let count = self
                        .layout_channel_count
                        .ok_or_else(|| invalid("chnl missing layout_channel_count"))?;
                    write_bits(
                        writer,
                        u64::from(count),
                        u8::BITS,
                        "chnl layout_channel_count",
                    )?;
                }
                for position in &self.speaker_positions {
                    position.serialize(writer)?;
                }
            } else {
                if self.version != CHNL_VERSION_BYTE {
                    write_bits(writer, 0, V1_ORDER_RESERVED_BITS, "chnl reserved")?;
                    write_bits(
                        writer,
                        u64::from(self.channel_order_definition.unwrap_or(0)),
                        V1_ORDER_DEFINITION_BITS,
                        "chnl channel_order",
                    )?;
                    write_bits(
                        writer,
                        u64::from(self.omitted_channels_map.is_some()),
                        1,
                        "chnl omitted_channels_present",
                    )?;
                }
                if let Some(map) = self.omitted_channels_map {
                    write_bits(writer, map, OMITTED_CHANNELS_MAP_BITS, "chnl map")?;
                } else if self.version == CHNL_VERSION_BYTE {
                    return Err(invalid("chnl definedLayout needs omittedChannelsMap"));
                }
            }
        }
        if self.is_object_structured() && self.version == CHNL_VERSION_BYTE {
            write_bits(
                writer,
                u64::from(self.object_count.unwrap_or(0)),
                u8::BITS,
                "chnl object_count",
            )?;
        }
        Ok(())
    }
}

fn position_len(positions: &[SpeakerPosition]) -> usize {
    positions
        .iter()
        .map(|p| {
            if p.speaker_position == SPEAKER_POSITION_EXPLICIT {
                1 + core::mem::size_of::<u16>() + 1
            } else {
                1
            }
        })
        .sum()
}

fn wire_bits(reader: &mut BitReader<'_>, n: u32, what: &'static str) -> Result<u64> {
    reader.read_bits(n).map_err(|_| Error::InvalidInput(what))
}

fn write_bits(writer: &mut BitBuf, value: u64, n: u32, what: &'static str) -> Result<()> {
    writer
        .write_bits(value, n)
        .map_err(|_| Error::InvalidInput(what))
}

fn invalid(reason: &'static str) -> Error {
    Error::InvalidInput(reason)
}

// ---------------------------------------------------------------------------
// DownMixInstructionsBox — dmix (§12.2.5)
// ---------------------------------------------------------------------------

/// Bits in `downmix_instructions_count` in version 1.
const DMIX_COUNT_BITS: u32 = 7;
/// Bits in `targetChannelCount`.
const DMIX_TARGET_CHANNEL_BITS: u32 = 7;
/// Bits in `downmix_ID`.
const DMIX_ID_BITS: u32 = 7;
/// Bits in a version-0 downmix coefficient.
const DMIX_COEFF_V0_BITS: u32 = 4;
/// Bits in a version-1 downmix coefficient.
const DMIX_COEFF_V1_BITS: u32 = 5;
/// Bits in `bs_downmix_offset` (version 1).
const DMIX_OFFSET_BITS: u32 = 4;
/// Bits in one byte, for the padding calculation.
const BITS_PER_BYTE: usize = 8;

/// One downmix instruction of a [`DownMixInstructionsBox`] (§12.2.5.2).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct DownMixInstruction {
    target_layout: u8,
    target_channel_count: u8,
    in_stream: bool,
    downmix_id: u8,
    offset: Option<u8>,
    coefficients: Vec<u8>,
}

impl DownMixInstruction {
    /// Build one instruction. `coefficients` is the row-major
    /// `targetChannelCount * baseChannelCount` code array, in wire order.
    #[must_use]
    pub fn new(
        target_layout: u8,
        target_channel_count: u8,
        in_stream: bool,
        downmix_id: u8,
    ) -> Self {
        Self {
            target_layout,
            target_channel_count,
            in_stream,
            downmix_id,
            offset: None,
            coefficients: Vec::new(),
        }
    }

    /// Set the version-1 `bs_downmix_offset`.
    #[must_use]
    pub fn with_offset(mut self, offset: u8) -> Self {
        self.offset = Some(offset);
        self
    }

    /// Set the out-of-stream coefficients (row-major codes).
    #[must_use]
    pub fn with_coefficients(mut self, coefficients: Vec<u8>) -> Self {
        self.coefficients = coefficients;
        self
    }

    /// The target ISO/IEC 23091-3 `ChannelConfiguration`.
    #[must_use]
    pub const fn target_layout(&self) -> u8 {
        self.target_layout
    }

    /// The number of channels in the result.
    #[must_use]
    pub const fn target_channel_count(&self) -> u8 {
        self.target_channel_count
    }

    /// Whether the coefficients are in the stream (`true`) or in this box.
    #[must_use]
    pub const fn in_stream(&self) -> bool {
        self.in_stream
    }

    /// The downmix identifier.
    #[must_use]
    pub const fn downmix_id(&self) -> u8 {
        self.downmix_id
    }

    /// The version-1 `bs_downmix_offset`.
    #[must_use]
    pub const fn offset(&self) -> Option<u8> {
        self.offset
    }

    /// The out-of-stream coefficient codes, row-major.
    #[must_use]
    pub fn coefficients(&self) -> &[u8] {
        &self.coefficients
    }

    fn coefficient_bits(&self, version: u8) -> u32 {
        let per = if version == 0 {
            DMIX_COEFF_V0_BITS
        } else {
            DMIX_COEFF_V1_BITS
        };
        per * u32::from(self.target_channel_count)
    }
}

/// DownMix Instructions Box (`dmix`) — ISO/IEC 14496-12:2026 §12.2.5.
///
/// `baseChannelCount` is not carried in the box (it comes from the audio
/// sample entry / `chnl` layout), so parsing a box whose coefficients are
/// out of stream requires it: use
/// [`DownMixInstructionsBox::parse_body_with_base_channel_count`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct DownMixInstructionsBox {
    version: u8,
    flags: u32,
    instructions: Vec<DownMixInstruction>,
}

impl DownMixInstructionsBox {
    /// Build a version-0 box carrying exactly one instruction.
    #[must_use]
    pub fn new(instruction: DownMixInstruction) -> Self {
        Self {
            version: 0,
            flags: 0,
            instructions: alloc::vec![instruction],
        }
    }

    /// Build a box of any version from its instructions.
    #[must_use]
    pub fn with_version(version: u8, instructions: Vec<DownMixInstruction>) -> Self {
        Self {
            version,
            flags: 0,
            instructions,
        }
    }

    /// The FullBox version.
    #[must_use]
    pub const fn version(&self) -> u8 {
        self.version
    }

    /// The FullBox flags.
    #[must_use]
    pub const fn flags(&self) -> u32 {
        self.flags
    }

    /// The downmix instructions, in wire order.
    #[must_use]
    pub fn instructions(&self) -> &[DownMixInstruction] {
        &self.instructions
    }

    /// Parse the payload after the 8-byte box header, given the sample entry's
    /// `baseChannelCount`.
    pub fn parse_body_with_base_channel_count(body: &[u8], base: u8) -> Result<Self> {
        need_in(body, FULLBOX_EXTRA_SIZE, "dmix body")?;
        let version = body[0];
        let flags = u24_at(body, 1) & FULLBOX_FLAGS_MASK;
        let mut reader = BitReader::new(&body[FULLBOX_EXTRA_SIZE..]);
        let count = if version == 0 {
            1
        } else {
            wire_bits(&mut reader, 1, "dmix reserved")?;
            wire_bits(&mut reader, DMIX_COUNT_BITS, "dmix count")? as usize
        };
        let mut instructions =
            Vec::with_capacity(bounded_entry_count(body.len(), FULLBOX_EXTRA_SIZE, count));
        for _ in 0..count {
            let target_layout =
                wire_bits(&mut reader, DMIX_TARGET_LAYOUT_BITS, "dmix targetLayout")? as u8;
            wire_bits(&mut reader, 1, "dmix reserved")?;
            let target_channel_count = wire_bits(
                &mut reader,
                DMIX_TARGET_CHANNEL_BITS,
                "dmix targetChannelCount",
            )? as u8;
            let in_stream = wire_bits(&mut reader, 1, "dmix in_stream")? != 0;
            let downmix_id = wire_bits(&mut reader, DMIX_ID_BITS, "dmix downmix_ID")? as u8;
            let mut instruction = DownMixInstruction {
                target_layout,
                target_channel_count,
                in_stream,
                downmix_id,
                offset: None,
                coefficients: Vec::new(),
            };
            if !in_stream {
                if version == 0 {
                    let total = usize::from(target_channel_count) * usize::from(base);
                    for _ in 0..total {
                        instruction.coefficients.push(wire_bits(
                            &mut reader,
                            DMIX_COEFF_V0_BITS,
                            "dmix coeff",
                        )? as u8);
                    }
                } else {
                    instruction.offset =
                        Some(wire_bits(&mut reader, DMIX_OFFSET_BITS, "dmix offset")? as u8);
                    let total = usize::from(target_channel_count) * usize::from(base);
                    for _ in 0..total {
                        instruction.coefficients.push(wire_bits(
                            &mut reader,
                            DMIX_COEFF_V1_BITS,
                            "dmix coeff",
                        )? as u8);
                    }
                    reader.align_to_byte();
                }
            }
            instructions.push(instruction);
        }
        Ok(Self {
            version,
            flags,
            instructions,
        })
    }
}

impl<'a> Parse<'a> for DownMixInstructionsBox {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if be_u32(bytes, BOX_TYPE_OFFSET, "dmix box")? != u32::from_be_bytes(DMIX) {
            return Err(invalid("expected dmix"));
        }
        let body = crate::box_types::box_body(bytes, FULLBOX_EXTRA_SIZE, "dmix box")?;
        Self::parse_body_with_base_channel_count(body, 0)
    }
}

impl Serialize for DownMixInstructionsBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        let mut bits = if self.version == 0 {
            0
        } else {
            (1 + DMIX_COUNT_BITS) as usize
        };
        for instruction in &self.instructions {
            bits += u8::BITS as usize
                + 1
                + DMIX_TARGET_CHANNEL_BITS as usize
                + 1
                + DMIX_ID_BITS as usize;
            if self.version != 0 {
                if !instruction.in_stream {
                    bits += DMIX_OFFSET_BITS as usize
                        + instruction.coefficient_bits(self.version) as usize;
                    bits = round_up(bits, BITS_PER_BYTE);
                }
            } else if !instruction.in_stream {
                bits += instruction.coefficient_bits(self.version) as usize;
            }
        }
        box_len(
            BOX_HEADER_MIN_SIZE
                + FULLBOX_EXTRA_SIZE
                + round_up(bits, BITS_PER_BYTE) / BITS_PER_BYTE,
        )
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut c = write_full_box_header(buf, &DMIX, need, self.version, self.flags, false)?;
        let mut writer = BitBuf::new();
        if self.version != 0 {
            write_bits(&mut writer, 0, 1, "dmix reserved")?;
            let count = fit_u8(self.instructions.len(), "downmix_instructions_count")?;
            write_bits(&mut writer, u64::from(count), DMIX_COUNT_BITS, "dmix count")?;
        }
        for instruction in &self.instructions {
            write_bits(
                &mut writer,
                u64::from(instruction.target_layout),
                DMIX_TARGET_LAYOUT_BITS,
                "dmix targetLayout",
            )?;
            write_bits(&mut writer, 0, 1, "dmix reserved")?;
            write_bits(
                &mut writer,
                u64::from(instruction.target_channel_count),
                DMIX_TARGET_CHANNEL_BITS,
                "dmix targetChannelCount",
            )?;
            write_bits(
                &mut writer,
                u64::from(instruction.in_stream),
                1,
                "dmix in_stream",
            )?;
            write_bits(
                &mut writer,
                u64::from(instruction.downmix_id),
                DMIX_ID_BITS,
                "dmix downmix_ID",
            )?;
            if !instruction.in_stream {
                if self.version != 0 {
                    write_bits(
                        &mut writer,
                        u64::from(instruction.offset.unwrap_or(0)),
                        DMIX_OFFSET_BITS,
                        "dmix offset",
                    )?;
                }
                let per = if self.version == 0 {
                    DMIX_COEFF_V0_BITS
                } else {
                    DMIX_COEFF_V1_BITS
                };
                for &coefficient in &instruction.coefficients {
                    write_bits(&mut writer, u64::from(coefficient), per, "dmix coeff")?;
                }
                if self.version != 0 {
                    writer.align_to_byte();
                }
            }
        }
        writer.align_to_byte();
        let bytes = writer.as_bytes();
        let body_len = bytes.len();
        buf[c..c + body_len].copy_from_slice(bytes);
        c += body_len;
        Ok(c)
    }
}

fn round_up(value: usize, to: usize) -> usize {
    value.div_ceil(to) * to
}

// ---------------------------------------------------------------------------
// LoudnessBox — ludt, with TrackLoudnessInfo (tlou) / AlbumLoudnessInfo (alou)
// ---------------------------------------------------------------------------

/// First loudness version with per-entry `EQ_set_ID` and a counted base list.
const LOUDNESS_FIRST_VERSION: u8 = 1;
/// First loudness version with `loudness_info_type` and the MAE group id.
const LOUDNESS_INFO_VERSION: u8 = 2;
/// The 12-bit signed peak fields' bit width.
const LOUDNESS_PEAK_BITS: u32 = 12;
/// The 12-bit signed peak fields' modulus.
const LOUDNESS_PEAK_MODULUS: i32 = 1 << LOUDNESS_PEAK_BITS;
/// Bits of `loudness_base_count`.
const LOUDNESS_COUNT_BITS: u32 = 6;
/// Bits of `loudness_info_type`.
const LOUDNESS_INFO_TYPE_BITS: u32 = 2;
/// `loudness_info_type` that carries an `mae_group_ID`.
const LOUDNESS_INFO_MAE_GROUP: u8 = 1;
/// Second `loudness_info_type` that carries an `mae_group_ID`.
const LOUDNESS_INFO_MAE_GROUP_ALT: u8 = 2;
/// `loudness_info_type` that carries an `mae_group_preset_ID`.
const LOUDNESS_INFO_MAE_PRESET: u8 = 3;

/// Which loudness box a [`LoudnessBaseBox`] is (§12.2.7.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum LoudnessKind {
    /// `tlou` — the loudness of the track itself.
    Track,
    /// `alou` — the loudness of the album that contains the track.
    Album,
}

impl LoudnessKind {
    /// The four-CC of this box.
    #[must_use]
    pub const fn fourcc(&self) -> [u8; BOX_TYPE_SIZE] {
        match self {
            Self::Track => TLOU,
            Self::Album => ALOU,
        }
    }

    /// Spec token for this box (`"tlou"` or `"alou"`).
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Track => "tlou",
            Self::Album => "alou",
        }
    }
}

broadcast_core::impl_spec_display!(LoudnessKind);

/// One `method_definition`/`method_value` measurement (§12.2.7.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct LoudnessMeasurement {
    method_definition: u8,
    method_value: u8,
    measurement_system: u8,
    reliability: u8,
}

impl LoudnessMeasurement {
    /// Build one measurement.
    #[must_use]
    pub const fn new(
        method_definition: u8,
        method_value: u8,
        measurement_system: u8,
        reliability: u8,
    ) -> Self {
        Self {
            method_definition,
            method_value,
            measurement_system,
            reliability,
        }
    }

    /// The ISO/IEC 23003-4 method definition index.
    #[must_use]
    pub const fn method_definition(&self) -> u8 {
        self.method_definition
    }

    /// The measured value.
    #[must_use]
    pub const fn method_value(&self) -> u8 {
        self.method_value
    }

    /// The ISO/IEC 23003-4 measurement system index.
    #[must_use]
    pub const fn measurement_system(&self) -> u8 {
        self.measurement_system
    }

    /// The reliability code (§12.2.7.3).
    #[must_use]
    pub const fn reliability(&self) -> u8 {
        self.reliability
    }
}

/// One loudness base entry of a [`LoudnessBaseBox`] (§12.2.7.2).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct LoudnessEntry {
    eq_set_id: Option<u8>,
    downmix_id: u8,
    drc_set_id: u8,
    sample_peak_level: i16,
    true_peak_level: i16,
    measurement_system_for_tp: u8,
    reliability_for_tp: u8,
    measurements: Vec<LoudnessMeasurement>,
}

impl LoudnessEntry {
    /// Build an entry with no measurements.
    #[must_use]
    pub const fn new(
        downmix_id: u8,
        drc_set_id: u8,
        sample_peak_level: i16,
        true_peak_level: i16,
        measurement_system_for_tp: u8,
        reliability_for_tp: u8,
    ) -> Self {
        Self {
            eq_set_id: None,
            downmix_id,
            drc_set_id,
            sample_peak_level,
            true_peak_level,
            measurement_system_for_tp,
            reliability_for_tp,
            measurements: Vec::new(),
        }
    }

    /// Set the version-1 `EQ_set_ID`.
    #[must_use]
    pub fn with_eq_set_id(mut self, eq_set_id: u8) -> Self {
        self.eq_set_id = Some(eq_set_id);
        self
    }

    /// Replace the measurement list.
    #[must_use]
    pub fn with_measurements(mut self, measurements: Vec<LoudnessMeasurement>) -> Self {
        self.measurements = measurements;
        self
    }

    /// The version-1 `EQ_set_ID`.
    #[must_use]
    pub const fn eq_set_id(&self) -> Option<u8> {
        self.eq_set_id
    }

    /// The matching downmix identifier (0 = no downmix).
    #[must_use]
    pub const fn downmix_id(&self) -> u8 {
        self.downmix_id
    }

    /// The matching DRC set identifier (0 = no DRC).
    #[must_use]
    pub const fn drc_set_id(&self) -> u8 {
        self.drc_set_id
    }

    /// The sample peak level (`bs_sample_peak_level`), in ISO/IEC 23003-4 units.
    #[must_use]
    pub const fn sample_peak_level(&self) -> i16 {
        self.sample_peak_level
    }

    /// The true peak level (`bs_true_peak_level`).
    #[must_use]
    pub const fn true_peak_level(&self) -> i16 {
        self.true_peak_level
    }

    /// The true-peak measurement system index.
    #[must_use]
    pub const fn measurement_system_for_tp(&self) -> u8 {
        self.measurement_system_for_tp
    }

    /// The true-peak reliability code.
    #[must_use]
    pub const fn reliability_for_tp(&self) -> u8 {
        self.reliability_for_tp
    }

    /// The measurement list.
    #[must_use]
    pub fn measurements(&self) -> &[LoudnessMeasurement] {
        &self.measurements
    }
}

/// Shared body of `tlou`/`alou` (a `LoudnessBaseBox`, §12.2.7.2).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct LoudnessBaseBox {
    kind: LoudnessKind,
    version: u8,
    flags: u32,
    loudness_info_type: Option<u8>,
    mae_group_id: Option<u8>,
    mae_group_preset_id: Option<u8>,
    entries: Vec<LoudnessEntry>,
}

impl LoudnessBaseBox {
    /// Build an empty version-0 box.
    #[must_use]
    pub fn new(kind: LoudnessKind) -> Self {
        Self {
            kind,
            version: 0,
            flags: 0,
            loudness_info_type: None,
            mae_group_id: None,
            mae_group_preset_id: None,
            entries: Vec::new(),
        }
    }

    /// Set the box version.
    #[must_use]
    pub fn with_version(mut self, version: u8) -> Self {
        self.version = version;
        self
    }

    /// Set the version-2 `loudness_info_type`.
    #[must_use]
    pub fn with_loudness_info_type(mut self, info_type: u8) -> Self {
        self.loudness_info_type = Some(info_type);
        self
    }

    /// Set the version-2 MAE group identifiers.
    #[must_use]
    pub fn with_mae_ids(mut self, group_id: Option<u8>, preset_id: Option<u8>) -> Self {
        self.mae_group_id = group_id;
        self.mae_group_preset_id = preset_id;
        self
    }

    /// Replace the base entry list.
    #[must_use]
    pub fn with_entries(mut self, entries: Vec<LoudnessEntry>) -> Self {
        self.entries = entries;
        self
    }

    /// Which box this is.
    #[must_use]
    pub const fn kind(&self) -> LoudnessKind {
        self.kind
    }

    /// The box version.
    #[must_use]
    pub const fn version(&self) -> u8 {
        self.version
    }

    /// The FullBox flags.
    #[must_use]
    pub const fn flags(&self) -> u32 {
        self.flags
    }

    /// The version-2 `loudness_info_type`.
    #[must_use]
    pub const fn loudness_info_type(&self) -> Option<u8> {
        self.loudness_info_type
    }

    /// The version-2 `mae_group_ID`.
    #[must_use]
    pub const fn mae_group_id(&self) -> Option<u8> {
        self.mae_group_id
    }

    /// The version-2 `mae_group_preset_ID`.
    #[must_use]
    pub const fn mae_group_preset_id(&self) -> Option<u8> {
        self.mae_group_preset_id
    }

    /// The loudness base entries.
    #[must_use]
    pub fn entries(&self) -> &[LoudnessEntry] {
        &self.entries
    }

    /// Parse the payload after the 8-byte box header.
    pub fn parse_body(kind: LoudnessKind, body: &[u8]) -> Result<Self> {
        need_in(body, FULLBOX_EXTRA_SIZE, "loudness body")?;
        let version = body[0];
        let flags = u24_at(body, 1) & FULLBOX_FLAGS_MASK;
        let mut reader = BitReader::new(&body[FULLBOX_EXTRA_SIZE..]);
        let mut box_ = Self {
            kind,
            version,
            flags,
            loudness_info_type: None,
            mae_group_id: None,
            mae_group_preset_id: None,
            entries: Vec::new(),
        };
        let count = if version >= LOUDNESS_INFO_VERSION {
            let info_type =
                wire_bits(&mut reader, LOUDNESS_INFO_TYPE_BITS, "loudness info_type")? as u8;
            box_.loudness_info_type = Some(info_type);
            let count = wire_bits(&mut reader, LOUDNESS_COUNT_BITS, "loudness count")? as usize;
            if info_type == LOUDNESS_INFO_MAE_GROUP || info_type == LOUDNESS_INFO_MAE_GROUP_ALT {
                wire_bits(&mut reader, 1, "loudness reserved")?;
                box_.mae_group_id = Some(wire_bits(
                    &mut reader,
                    LOUDNESS_MAE_GROUP_ID_BITS,
                    "loudness mae_group_ID",
                )? as u8);
            } else if info_type == LOUDNESS_INFO_MAE_PRESET {
                wire_bits(
                    &mut reader,
                    LOUDNESS_MAE_PRESET_RESERVED_BITS,
                    "loudness reserved",
                )?;
                box_.mae_group_preset_id = Some(wire_bits(
                    &mut reader,
                    LOUDNESS_MAE_PRESET_BITS,
                    "loudness mae_group_preset_ID",
                )? as u8);
            }
            count
        } else if version == LOUDNESS_FIRST_VERSION {
            wire_bits(
                &mut reader,
                LOUDNESS_LEGACY_RESERVED_BITS,
                "loudness reserved",
            )?;
            wire_bits(&mut reader, LOUDNESS_COUNT_BITS, "loudness count")? as usize
        } else {
            1
        };
        let mut entries = Vec::with_capacity(bounded_entry_count(body.len(), 1, count));
        for _ in 0..count {
            entries.push(parse_loudness_entry(version, &mut reader)?);
        }
        box_.entries = entries;
        Ok(box_)
    }

    fn body_bits(&self) -> usize {
        let mut bits = 0usize;
        if self.version >= LOUDNESS_INFO_VERSION {
            bits += (LOUDNESS_INFO_TYPE_BITS + LOUDNESS_COUNT_BITS) as usize;
            match self.loudness_info_type.unwrap_or(0) {
                LOUDNESS_INFO_MAE_GROUP | LOUDNESS_INFO_MAE_GROUP_ALT => {
                    bits +=
                        LOUDNESS_MAE_RESERVED_BITS as usize + LOUDNESS_MAE_GROUP_ID_BITS as usize
                }
                LOUDNESS_INFO_MAE_PRESET => {
                    bits += LOUDNESS_MAE_PRESET_RESERVED_BITS as usize
                        + LOUDNESS_MAE_PRESET_BITS as usize
                }
                _ => {}
            }
        } else if self.version == LOUDNESS_FIRST_VERSION {
            bits += LOUDNESS_LEGACY_RESERVED_BITS as usize + LOUDNESS_COUNT_BITS as usize;
        }
        for entry in &self.entries {
            if self.version != 0 {
                bits += LOUDNESS_EQ_RESERVED_BITS as usize + LOUDNESS_EQ_SET_BITS as usize;
            }
            bits += LOUDNESS_ENTRY_RESERVED_BITS as usize
                + LOUDNESS_DOWNMIX_ID_BITS as usize
                + LOUDNESS_DRC_SET_BITS as usize
                + LOUDNESS_PEAK_BITS as usize
                + LOUDNESS_PEAK_BITS as usize
                + LOUDNESS_MEASUREMENT_SYSTEM_BITS as usize
                + LOUDNESS_RELIABILITY_BITS as usize
                + LOUDNESS_MEASUREMENT_COUNT_BITS as usize;
            bits += entry.measurements.len()
                * (LOUDNESS_METHOD_DEFINITION_BITS as usize
                    + LOUDNESS_METHOD_VALUE_BITS as usize
                    + LOUDNESS_MEASUREMENT_SYSTEM_BITS as usize
                    + LOUDNESS_RELIABILITY_BITS as usize);
        }
        bits
    }

    fn write_body(&self, buf: &mut [u8]) -> Result<usize> {
        let mut writer = BitBuf::new();
        if self.version >= LOUDNESS_INFO_VERSION {
            let info_type = self.loudness_info_type.unwrap_or(0);
            write_bits(
                &mut writer,
                u64::from(info_type),
                LOUDNESS_INFO_TYPE_BITS,
                "loudness info_type",
            )?;
            write_count(&mut writer, self.entries.len())?;
            if info_type == LOUDNESS_INFO_MAE_GROUP || info_type == LOUDNESS_INFO_MAE_GROUP_ALT {
                write_bits(&mut writer, 0, 1, "loudness reserved")?;
                write_bits(
                    &mut writer,
                    u64::from(self.mae_group_id.unwrap_or(0)),
                    LOUDNESS_MAE_GROUP_ID_BITS,
                    "loudness mae_group_ID",
                )?;
            } else if info_type == LOUDNESS_INFO_MAE_PRESET {
                write_bits(
                    &mut writer,
                    0,
                    LOUDNESS_MAE_PRESET_RESERVED_BITS,
                    "loudness reserved",
                )?;
                write_bits(
                    &mut writer,
                    u64::from(self.mae_group_preset_id.unwrap_or(0)),
                    LOUDNESS_MAE_PRESET_BITS,
                    "loudness mae_group_preset_ID",
                )?;
            }
        } else if self.version == LOUDNESS_FIRST_VERSION {
            write_bits(
                &mut writer,
                0,
                LOUDNESS_LEGACY_RESERVED_BITS,
                "loudness reserved",
            )?;
            write_count(&mut writer, self.entries.len())?;
        }
        for entry in &self.entries {
            write_loudness_entry(self.version, entry, &mut writer)?;
        }
        writer.align_to_byte();
        let bytes = writer.as_bytes();
        buf[..bytes.len()].copy_from_slice(bytes);
        Ok(bytes.len())
    }
}

fn write_count(writer: &mut BitBuf, count: usize) -> Result<()> {
    let count = fit_u8(count, "loudness_base_count")?;
    write_bits(
        writer,
        u64::from(count),
        LOUDNESS_COUNT_BITS,
        "loudness count",
    )
}

fn parse_loudness_entry(version: u8, reader: &mut BitReader<'_>) -> Result<LoudnessEntry> {
    let eq_set_id = if version != 0 {
        wire_bits(reader, LOUDNESS_EQ_RESERVED_BITS, "loudness reserved")?;
        Some(wire_bits(reader, LOUDNESS_EQ_SET_BITS, "loudness EQ_set_ID")? as u8)
    } else {
        None
    };
    wire_bits(reader, LOUDNESS_ENTRY_RESERVED_BITS, "loudness reserved")?;
    let downmix_id = wire_bits(reader, LOUDNESS_DOWNMIX_ID_BITS, "loudness downmix_ID")? as u8;
    let drc_set_id = wire_bits(reader, LOUDNESS_DRC_SET_BITS, "loudness DRC_set_ID")? as u8;
    let sample_peak_level = sext(
        wire_bits(reader, LOUDNESS_PEAK_BITS, "loudness sample_peak")?,
        LOUDNESS_PEAK_BITS,
    );
    let true_peak_level = sext(
        wire_bits(reader, LOUDNESS_PEAK_BITS, "loudness true_peak")?,
        LOUDNESS_PEAK_BITS,
    );
    let measurement_system_for_tp = wire_bits(
        reader,
        LOUDNESS_MEASUREMENT_SYSTEM_BITS,
        "loudness TP system",
    )? as u8;
    let reliability_for_tp =
        wire_bits(reader, LOUDNESS_RELIABILITY_BITS, "loudness TP reliability")? as u8;
    let count = wire_bits(
        reader,
        LOUDNESS_MEASUREMENT_COUNT_BITS,
        "loudness measurement_count",
    )? as usize;
    let mut measurements = Vec::with_capacity(count);
    for _ in 0..count {
        measurements.push(LoudnessMeasurement {
            method_definition: wire_bits(
                reader,
                LOUDNESS_METHOD_DEFINITION_BITS,
                "loudness method_definition",
            )? as u8,
            method_value: wire_bits(reader, LOUDNESS_METHOD_VALUE_BITS, "loudness method_value")?
                as u8,
            measurement_system: wire_bits(
                reader,
                LOUDNESS_MEASUREMENT_SYSTEM_BITS,
                "loudness measurement_system",
            )? as u8,
            reliability: wire_bits(reader, LOUDNESS_RELIABILITY_BITS, "loudness reliability")?
                as u8,
        });
    }
    Ok(LoudnessEntry {
        eq_set_id,
        downmix_id,
        drc_set_id,
        sample_peak_level,
        true_peak_level,
        measurement_system_for_tp,
        reliability_for_tp,
        measurements,
    })
}

fn write_loudness_entry(version: u8, entry: &LoudnessEntry, writer: &mut BitBuf) -> Result<()> {
    if version != 0 {
        write_bits(writer, 0, LOUDNESS_EQ_RESERVED_BITS, "loudness reserved")?;
        write_bits(
            writer,
            u64::from(entry.eq_set_id.unwrap_or(0)),
            LOUDNESS_EQ_SET_BITS,
            "loudness EQ_set_ID",
        )?;
    }
    write_bits(writer, 0, LOUDNESS_ENTRY_RESERVED_BITS, "loudness reserved")?;
    write_bits(
        writer,
        u64::from(entry.downmix_id),
        LOUDNESS_DOWNMIX_ID_BITS,
        "loudness downmix_ID",
    )?;
    write_bits(
        writer,
        u64::from(entry.drc_set_id),
        LOUDNESS_DRC_SET_BITS,
        "loudness DRC_set_ID",
    )?;
    write_peak(writer, entry.sample_peak_level, "loudness sample_peak")?;
    write_peak(writer, entry.true_peak_level, "loudness true_peak")?;
    write_bits(
        writer,
        u64::from(entry.measurement_system_for_tp),
        LOUDNESS_MEASUREMENT_SYSTEM_BITS,
        "loudness TP system",
    )?;
    write_bits(
        writer,
        u64::from(entry.reliability_for_tp),
        LOUDNESS_RELIABILITY_BITS,
        "loudness TP reliability",
    )?;
    let count = fit_u8(entry.measurements.len(), "loudness measurement_count")?;
    write_bits(
        writer,
        u64::from(count),
        LOUDNESS_MEASUREMENT_COUNT_BITS,
        "loudness measurement_count",
    )?;
    for measurement in &entry.measurements {
        write_bits(
            writer,
            u64::from(measurement.method_definition),
            LOUDNESS_METHOD_DEFINITION_BITS,
            "loudness method_definition",
        )?;
        write_bits(
            writer,
            u64::from(measurement.method_value),
            LOUDNESS_METHOD_VALUE_BITS,
            "loudness method_value",
        )?;
        write_bits(
            writer,
            u64::from(measurement.measurement_system),
            LOUDNESS_MEASUREMENT_SYSTEM_BITS,
            "loudness measurement_system",
        )?;
        write_bits(
            writer,
            u64::from(measurement.reliability),
            LOUDNESS_RELIABILITY_BITS,
            "loudness reliability",
        )?;
    }
    Ok(())
}

fn write_peak(writer: &mut BitBuf, value: i16, what: &'static str) -> Result<()> {
    let wire = i32::from(value).rem_euclid(LOUDNESS_PEAK_MODULUS) as u64;
    write_bits(writer, wire, LOUDNESS_PEAK_BITS, what)
}

fn sext(value: u64, bits: u32) -> i16 {
    let shift = u64::BITS - bits;
    ((value << shift) as i64 >> shift) as i16
}

macro_rules! loudness_wrapper {
    ($name:ident, $kind:expr, $label:literal) => {
        #[doc = concat!("The ", $label, " box — ISO/IEC 14496-12:2026 §12.2.7.")]
        #[derive(Debug, Clone, PartialEq, Eq)]
        #[cfg_attr(feature = "serde", derive(serde::Serialize))]
        #[non_exhaustive]
        pub struct $name {
            inner: LoudnessBaseBox,
        }

        impl $name {
            /// Build an empty version-0 box.
            #[must_use]
            pub fn new() -> Self {
                Self {
                    inner: LoudnessBaseBox::new($kind),
                }
            }

            /// Wrap a parsed base box of this kind.
            #[must_use]
            pub fn from_base(inner: LoudnessBaseBox) -> Self {
                Self { inner }
            }

            /// The shared base view.
            #[must_use]
            pub const fn base(&self) -> &LoudnessBaseBox {
                &self.inner
            }

            /// The mutable shared base view, for setting the version and
            /// entries.
            pub fn base_mut(&mut self) -> &mut LoudnessBaseBox {
                &mut self.inner
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl<'a> Parse<'a> for $name {
            type Error = Error;

            fn parse(bytes: &'a [u8]) -> Result<Self> {
                if be_u32(bytes, BOX_TYPE_OFFSET, concat!($label, " box"))?
                    != u32::from_be_bytes($kind.fourcc())
                {
                    return Err(invalid(concat!("expected ", $label)));
                }
                Ok(Self::from_base(LoudnessBaseBox::parse_body(
                    $kind,
                    crate::box_types::box_body(bytes, FULLBOX_EXTRA_SIZE, concat!($label, " box"))?,
                )?))
            }
        }

        impl Serialize for $name {
            type Error = Error;

            fn serialized_len(&self) -> usize {
                let need = box_len(
                    BOX_HEADER_MIN_SIZE
                        + FULLBOX_EXTRA_SIZE
                        + self.inner.body_bits().div_ceil(BITS_PER_BYTE),
                );
                need
            }

            fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
                let need = self.serialized_len();
                need_out(buf, need)?;
                let mut c = write_full_box_header(
                    buf,
                    &$kind.fourcc(),
                    need,
                    self.inner.version,
                    self.inner.flags,
                    false,
                )?;
                c += self.inner.write_body(&mut buf[c..])?;
                Ok(c)
            }
        }
    };
}

loudness_wrapper!(TrackLoudnessInfo, LoudnessKind::Track, "tlou");
loudness_wrapper!(AlbumLoudnessInfo, LoudnessKind::Album, "alou");

/// Audio Stream Loudness Box (`ludt`) — ISO/IEC 14496-12:2026 §12.2.7.
///
/// A plain container of [`TrackLoudnessInfo`] and [`AlbumLoudnessInfo`] boxes.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct LoudnessBox {
    track: Vec<TrackLoudnessInfo>,
    album: Vec<AlbumLoudnessInfo>,
}

impl LoudnessBox {
    /// Build an empty `ludt`.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Append one `tlou`.
    #[must_use]
    pub fn with_track(mut self, track: TrackLoudnessInfo) -> Self {
        self.track.push(track);
        self
    }

    /// Append one `alou`.
    #[must_use]
    pub fn with_album(mut self, album: AlbumLoudnessInfo) -> Self {
        self.album.push(album);
        self
    }

    /// The `tlou` boxes.
    #[must_use]
    pub fn track(&self) -> &[TrackLoudnessInfo] {
        &self.track
    }

    /// The `alou` boxes.
    #[must_use]
    pub fn album(&self) -> &[AlbumLoudnessInfo] {
        &self.album
    }

    /// Parse the payload after the 8-byte box header.
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        let mut out = Self::default();
        for child in crate::box_types::BoxIter::new(body) {
            let child = child?;
            match &child.four_cc() {
                b"tlou" => out.track.push(TrackLoudnessInfo::parse(child.bytes)?),
                b"alou" => out.album.push(AlbumLoudnessInfo::parse(child.bytes)?),
                other => {
                    let _ = other;
                    return Err(invalid("ludt child is not tlou or alou"));
                }
            }
        }
        Ok(out)
    }
}

impl<'a> Parse<'a> for LoudnessBox {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if be_u32(bytes, BOX_TYPE_OFFSET, "ludt box")? != u32::from_be_bytes(LUDT) {
            return Err(invalid("expected ludt"));
        }
        Self::parse_body(crate::box_types::box_body(bytes, 0, "ludt box")?)
    }
}

impl Serialize for LoudnessBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        let mut n = BOX_HEADER_MIN_SIZE;
        for track in &self.track {
            n += track.serialized_len();
        }
        for album in &self.album {
            n += album.serialized_len();
        }
        box_len(n)
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut c = crate::box_types::write_box_header(buf, &LUDT, need)?;
        for track in &self.track {
            c += track.serialize_into(&mut buf[c..])?;
        }
        for album in &self.album {
            c += album.serialize_into(&mut buf[c..])?;
        }
        Ok(c)
    }
}

// ---------------------------------------------------------------------------
// LabelBox — labl (§8.10.5)
// ---------------------------------------------------------------------------

/// Label Box (`labl`) — ISO/IEC 14496-12:2026 §8.10.5.
///
/// `label_id` groups labels; `language` is a BCP 47 tag and `label` the text.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct LabelBox {
    version: u8,
    flags: u32,
    label_id: u16,
    language: Vec<u8>,
    label: Vec<u8>,
}

impl LabelBox {
    /// Build a label with no group (`label_id == 0`).
    #[must_use]
    pub fn new(language: &str, label: &str) -> Self {
        Self {
            version: 0,
            flags: 0,
            label_id: 0,
            language: language.as_bytes().to_vec(),
            label: label.as_bytes().to_vec(),
        }
    }

    /// Set the group identifier.
    #[must_use]
    pub fn with_label_id(mut self, label_id: u16) -> Self {
        self.label_id = label_id;
        self
    }

    /// Set the `is_group_label` flag (§8.10.5.3).
    #[must_use]
    pub fn with_group_label(mut self, group: bool) -> Self {
        if group {
            self.flags |= LABEL_FLAG_GROUP;
        } else {
            self.flags &= !LABEL_FLAG_GROUP;
        }
        self
    }

    /// The FullBox flags.
    #[must_use]
    pub const fn flags(&self) -> u32 {
        self.flags
    }

    /// The group identifier (`0` when the label belongs to no group).
    #[must_use]
    pub const fn label_id(&self) -> u16 {
        self.label_id
    }

    /// Whether this label is a group summary (`is_group_label`).
    #[must_use]
    pub const fn is_group_label(&self) -> bool {
        self.flags & LABEL_FLAG_GROUP != 0
    }

    /// The BCP 47 language tag bytes.
    #[must_use]
    pub fn language(&self) -> &[u8] {
        &self.language
    }

    /// The label text bytes.
    #[must_use]
    pub fn label(&self) -> &[u8] {
        &self.label
    }

    /// The language tag as UTF-8, when it is.
    #[must_use]
    pub fn language_str(&self) -> Option<&str> {
        core::str::from_utf8(&self.language).ok()
    }

    /// The label text as UTF-8, when it is.
    #[must_use]
    pub fn label_str(&self) -> Option<&str> {
        core::str::from_utf8(&self.label).ok()
    }

    /// Parse the payload after the 8-byte box header.
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        need_in(body, FULLBOX_EXTRA_SIZE + 2, "labl body")?;
        let version = body[0];
        let flags = u24_at(body, 1) & FULLBOX_FLAGS_MASK;
        let label_id = be_u16(body, FULLBOX_EXTRA_SIZE, "labl label_id")?;
        let mut cursor = FULLBOX_EXTRA_SIZE + LABL_LABEL_ID_LEN;
        let language = take_c_string(body, &mut cursor, "labl language")?;
        let label = take_c_string(body, &mut cursor, "labl label")?;
        Ok(Self {
            version,
            flags,
            label_id,
            language,
            label,
        })
    }
}

/// `is_group_label` flag of a `labl` FullBox (§8.10.5.3).
pub const LABEL_FLAG_GROUP: u32 = 0x000001;

impl<'a> Parse<'a> for LabelBox {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if be_u32(bytes, BOX_TYPE_OFFSET, "labl box")? != u32::from_be_bytes(LABL) {
            return Err(invalid("expected labl"));
        }
        Self::parse_body(crate::box_types::box_body(
            bytes,
            FULLBOX_EXTRA_SIZE + LABL_LABEL_ID_LEN,
            "labl box",
        )?)
    }
}

impl Serialize for LabelBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        box_len(
            BOX_HEADER_MIN_SIZE
                + FULLBOX_EXTRA_SIZE
                + LABL_LABEL_ID_LEN
                + self.language.len()
                + 1
                + self.label.len()
                + 1,
        )
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut c = write_full_box_header(buf, &LABL, need, self.version, self.flags, false)?;
        buf[c..c + LABL_LABEL_ID_LEN].copy_from_slice(&self.label_id.to_be_bytes());
        c += LABL_LABEL_ID_LEN;
        c += write_c_string(buf, c, &self.language)?;
        c += write_c_string(buf, c, &self.label)?;
        Ok(c)
    }
}
