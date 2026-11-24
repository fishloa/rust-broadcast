//! DTS-in-ISOBMFF decoder configuration — ETSI TS 102 114 §E.2.2.3.
//!
//! [`DtsSpecificBox`] (`ddts`) and the DTS sample-entry FourCCs. The core
//! substream frame-header parser, frame splitter and the `ddts` constructor
//! live in [`crate::dts_core`].

use crate::box_types::BOX_TYPE_SIZE;
use crate::error::{Error, Result};
use crate::wire_cursor::{u16_at, u32_at};
use broadcast_common::len::{need_in, need_out};
use broadcast_common::{Parse, Serialize};

/// Byte offset of `DTSSamplingFrequency` in the `ddts` body.
const DDTS_SAMPLING_FREQUENCY_OFFSET: usize = 0;
/// Byte offset of `maxBitrate` in the `ddts` body.
const DDTS_MAX_BITRATE_OFFSET: usize = 4;
/// Byte offset of `avgBitrate` in the `ddts` body.
const DDTS_AVG_BITRATE_OFFSET: usize = 8;
/// Byte offset of `pcmSampleDepth` in the `ddts` body.
const DDTS_PCM_SAMPLE_DEPTH_OFFSET: usize = 12;
/// Byte offset of the packed `ddts` bit fields.
const DDTS_PACKED_OFFSET: usize = 13;
/// Byte offset of `ChannelLayout` in the `ddts` body.
const DDTS_CHANNEL_LAYOUT_OFFSET: usize = 17;
/// Byte offset of the `ddts` flags byte.
const DDTS_FLAGS_OFFSET: usize = 19;

/// Mask of `FrameDuration` in the packed `ddts` word.
const DDTS_FRAME_DURATION_MASK: u32 = 0x03;
/// Shift of `FrameDuration` in the packed `ddts` word.
const DDTS_FRAME_DURATION_SHIFT: u32 = 30;
/// Mask of `StreamConstruction` in the packed `ddts` word.
const DDTS_STREAM_CONSTRUCTION_MASK: u32 = 0x1F;
/// Shift of `StreamConstruction` in the packed `ddts` word.
const DDTS_STREAM_CONSTRUCTION_SHIFT: u32 = 25;
/// Shift of `CoreLFEPresent` in the packed `ddts` word.
const DDTS_CORE_LFE_PRESENT_SHIFT: u32 = 24;
/// Mask of `CoreLayout` in the packed `ddts` word.
const DDTS_CORE_LAYOUT_MASK: u32 = 0x3F;
/// Shift of `CoreLayout` in the packed `ddts` word.
const DDTS_CORE_LAYOUT_SHIFT: u32 = 18;
/// Mask of `CoreSize` in the packed `ddts` word.
const DDTS_CORE_SIZE_MASK: u32 = 0x3FFF;
/// Shift of `CoreSize` in the packed `ddts` word.
const DDTS_CORE_SIZE_SHIFT: u32 = 4;
/// Shift of `StereoDownmix` in the packed `ddts` word.
const DDTS_STEREO_DOWNMIX_SHIFT: u32 = 3;
/// Mask of `RepresentationType` in the packed `ddts` word.
const DDTS_REPRESENTATION_TYPE_MASK: u32 = 0x07;

/// Mask of `MultiAssetFlag` in the `ddts` flags byte.
const DDTS_MULTI_ASSET_FLAG_MASK: u8 = 0x80;
/// Mask of `LBRDurationMod` in the `ddts` flags byte.
const DDTS_LBR_DURATION_MOD_MASK: u8 = 0x40;
/// Mask of `ReservedBoxPresent` in the `ddts` flags byte.
const DDTS_RESERVED_BOX_PRESENT_MASK: u8 = 0x20;
/// FourCC of the DTS config box.
pub const DDTS_FOURCC: [u8; 4] = *b"ddts";

/// FourCC of the DTS core-only sample entry (`dtsc` — ETSI TS 102 114 Table E-1).
pub const DTSC_FOURCC: [u8; 4] = *b"dtsc";
/// FourCC of the DTS core+extension multi-asset sample entry (`dtsh`).
pub const DTSH_FOURCC: [u8; 4] = *b"dtsh";
/// FourCC of the DTS LBR-only sample entry (`dtsl`).
pub const DTSL_FOURCC: [u8; 4] = *b"dtsl";
/// FourCC of the DTS extension-substream-only sample entry (`dtse`).
pub const DTSE_FOURCC: [u8; 4] = *b"dtse";

/// Fixed serialized length of the `ddts` box body (20 bytes).
///
/// Layout (ETSI TS 102 114 §E.2.2.3.1):
/// 4 (`DTSSamplingFrequency`) + 4 (`maxBitrate`) + 4 (`avgBitrate`) +
/// 1 (`pcmSampleDepth`) + 4 (packed bits: `FrameDuration`/`StreamConstruction`/
/// `CoreLFEPresent`/`CoreLayout`/`CoreSize`/`StereoDownmix`/`RepresentationType`) +
/// 2 (`ChannelLayout`) + 1 (flags byte: `MultiAssetFlag`/`LBRDurationMod`/
/// `ReservedBoxPresent`/`Reserved`).
pub const DDTS_BODY_LEN: usize = 20;

/// `DTSSpecificBox` (`ddts` box body) — ETSI TS 102 114 §E.2.2.3.
///
/// All fields map directly to the spec syntax. The packed bit fields occupy
/// two multi-byte regions: a 32-bit word and a trailing byte.
///
/// Packed 32-bit word layout (most-significant bit first):
/// - `[31:30]` `FrameDuration` (2 bits)
/// - `[29:25]` `StreamConstruction` (5 bits)
/// - `[24]`    `CoreLFEPresent` (1 bit)
/// - `[23:18]` `CoreLayout` (6 bits)
/// - `[17:4]`  `CoreSize` (14 bits)
/// - `[3]`     `StereoDownmix` (1 bit)
/// - `[2:0]`   `RepresentationType` (3 bits)
///
/// Trailing flags byte layout:
/// - `[7]`     `MultiAssetFlag` (1 bit)
/// - `[6]`     `LBRDurationMod` (1 bit)
/// - `[5]`     `ReservedBoxPresent` (1 bit)
/// - `[4:0]`   `Reserved` (5 bits, always 0)
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct DtsSpecificBox {
    /// Nominal sampling frequency in Hz — ETSI TS 102 114 §E.2.2.3.1.
    pub dts_sampling_frequency: u32,
    /// Maximum bitrate in bits per second.
    pub max_bitrate: u32,
    /// Average bitrate in bits per second.
    pub avg_bitrate: u32,
    /// PCM sample depth: 16 or 24 bits.
    pub pcm_sample_depth: u8,
    /// `[31:30]` Frame duration code: 0 = 512, 1 = 1024, 2 = 2048, 3 = 4096 samples.
    pub frame_duration: u8,
    /// `[29:25]` Stream construction code (ETSI TS 102 114 Table E-2).
    pub stream_construction: u8,
    /// `[24]` Core LFE channel present: 0 = none, 1 = LFE exists.
    pub core_lfe_present: bool,
    /// `[23:18]` Core channel layout code (ETSI TS 102 114 Table E-3).
    pub core_layout: u8,
    /// `[17:4]` Core substream size in bytes.
    pub core_size: u16,
    /// `[3]` Stereo downmix embedded: 0 = none, 1 = present.
    pub stereo_downmix: bool,
    /// `[2:0]` Representation type (ETSI TS 102 114 Table E-4).
    pub representation_type: u8,
    /// Channel layout bitmask (ETSI TS 102 114 Table E-5).
    pub channel_layout: u16,
    /// `[7]` Multi-asset flag: 0 = single asset, 1 = multiple assets.
    pub multi_asset_flag: bool,
    /// `[6]` LBR duration modifier: 0 = ignore, 1 = special LBR modifier.
    pub lbr_duration_mod: bool,
    /// `[5]` Reserved box present: 0 = no reserved box, 1 = reserved box present.
    pub reserved_box_present: bool,
}

impl DtsSpecificBox {
    /// RFC 6381 codec string derived from the DTS sample-entry FourCC.
    ///
    /// Returns the fourcc as a UTF-8 string. For the four defined DTS FourCCs
    /// (`dtsc`, `dtsh`, `dtsl`, `dtse`) the result is the codec identifier used
    /// in `Content-Type` and HLS `CODECS=` attributes.
    pub fn rfc6381(codec_fourcc: &[u8; BOX_TYPE_SIZE]) -> &'static str {
        match codec_fourcc {
            b"dtsc" => "dtsc",
            b"dtsh" => "dtsh",
            b"dtsl" => "dtsl",
            b"dtse" => "dtse",
            _ => "dtsc",
        }
    }

    /// Encode the two packed multi-bit regions into a single u32 (big-endian word).
    ///
    /// Bit layout (MSB first):
    /// `[31:30]` `frame_duration`, `[29:25]` `stream_construction`,
    /// `[24]` `core_lfe_present`, `[23:18]` `core_layout`,
    /// `[17:4]` `core_size`, `[3]` `stereo_downmix`,
    /// `[2:0]` `representation_type`.
    fn encode_packed_word(&self) -> u32 {
        let mut w: u32 = 0;
        w |= (u32::from(self.frame_duration) & DDTS_FRAME_DURATION_MASK)
            << DDTS_FRAME_DURATION_SHIFT;
        w |= (u32::from(self.stream_construction) & DDTS_STREAM_CONSTRUCTION_MASK)
            << DDTS_STREAM_CONSTRUCTION_SHIFT;
        w |= u32::from(self.core_lfe_present) << DDTS_CORE_LFE_PRESENT_SHIFT;
        w |= (u32::from(self.core_layout) & DDTS_CORE_LAYOUT_MASK) << DDTS_CORE_LAYOUT_SHIFT;
        w |= (u32::from(self.core_size) & DDTS_CORE_SIZE_MASK) << DDTS_CORE_SIZE_SHIFT;
        w |= u32::from(self.stereo_downmix) << DDTS_STEREO_DOWNMIX_SHIFT;
        w |= u32::from(self.representation_type) & DDTS_REPRESENTATION_TYPE_MASK;
        w
    }

    /// Decode the packed 32-bit word produced by [`encode_packed_word`].
    fn decode_packed_word(w: u32) -> (u8, u8, bool, u8, u16, bool, u8) {
        let frame_duration = ((w >> DDTS_FRAME_DURATION_SHIFT) & DDTS_FRAME_DURATION_MASK) as u8;
        let stream_construction =
            ((w >> DDTS_STREAM_CONSTRUCTION_SHIFT) & DDTS_STREAM_CONSTRUCTION_MASK) as u8;
        let core_lfe_present = ((w >> DDTS_CORE_LFE_PRESENT_SHIFT) & 1) != 0;
        let core_layout = ((w >> DDTS_CORE_LAYOUT_SHIFT) & DDTS_CORE_LAYOUT_MASK) as u8;
        let core_size = ((w >> DDTS_CORE_SIZE_SHIFT) & DDTS_CORE_SIZE_MASK) as u16;
        let stereo_downmix = ((w >> DDTS_STEREO_DOWNMIX_SHIFT) & 1) != 0;
        let representation_type = (w & DDTS_REPRESENTATION_TYPE_MASK) as u8;
        (
            frame_duration,
            stream_construction,
            core_lfe_present,
            core_layout,
            core_size,
            stereo_downmix,
            representation_type,
        )
    }

    /// Encode the trailing flags byte.
    ///
    /// Bit layout: `[7]` `multi_asset_flag`, `[6]` `lbr_duration_mod`,
    /// `[5]` `reserved_box_present`, `[4:0]` Reserved (always 0).
    fn encode_flags_byte(&self) -> u8 {
        let mut b: u8 = 0;
        if self.multi_asset_flag {
            b |= DDTS_MULTI_ASSET_FLAG_MASK;
        }
        if self.lbr_duration_mod {
            b |= DDTS_LBR_DURATION_MOD_MASK;
        }
        if self.reserved_box_present {
            b |= DDTS_RESERVED_BOX_PRESENT_MASK;
        }
        b
    }
}

impl<'a> Parse<'a> for DtsSpecificBox {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        need_in(bytes, DDTS_BODY_LEN, "ddts")?;
        let dts_sampling_frequency = u32_at(bytes, DDTS_SAMPLING_FREQUENCY_OFFSET);
        let max_bitrate = u32_at(bytes, DDTS_MAX_BITRATE_OFFSET);
        let avg_bitrate = u32_at(bytes, DDTS_AVG_BITRATE_OFFSET);
        let pcm_sample_depth = bytes[DDTS_PCM_SAMPLE_DEPTH_OFFSET];
        let packed = u32_at(bytes, DDTS_PACKED_OFFSET);
        let (
            frame_duration,
            stream_construction,
            core_lfe_present,
            core_layout,
            core_size,
            stereo_downmix,
            representation_type,
        ) = Self::decode_packed_word(packed);
        let channel_layout = u16_at(bytes, DDTS_CHANNEL_LAYOUT_OFFSET);
        let flags_byte = bytes[DDTS_FLAGS_OFFSET];
        let multi_asset_flag = (flags_byte & DDTS_MULTI_ASSET_FLAG_MASK) != 0;
        let lbr_duration_mod = (flags_byte & DDTS_LBR_DURATION_MOD_MASK) != 0;
        let reserved_box_present = (flags_byte & DDTS_RESERVED_BOX_PRESENT_MASK) != 0;

        Ok(Self {
            dts_sampling_frequency,
            max_bitrate,
            avg_bitrate,
            pcm_sample_depth,
            frame_duration,
            stream_construction,
            core_lfe_present,
            core_layout,
            core_size,
            stereo_downmix,
            representation_type,
            channel_layout,
            multi_asset_flag,
            lbr_duration_mod,
            reserved_box_present,
        })
    }
}

impl Serialize for DtsSpecificBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        DDTS_BODY_LEN
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = DDTS_BODY_LEN;
        need_out(buf, need)?;
        buf[DDTS_SAMPLING_FREQUENCY_OFFSET..DDTS_MAX_BITRATE_OFFSET]
            .copy_from_slice(&self.dts_sampling_frequency.to_be_bytes());
        buf[DDTS_MAX_BITRATE_OFFSET..DDTS_AVG_BITRATE_OFFSET]
            .copy_from_slice(&self.max_bitrate.to_be_bytes());
        buf[DDTS_AVG_BITRATE_OFFSET..DDTS_PCM_SAMPLE_DEPTH_OFFSET]
            .copy_from_slice(&self.avg_bitrate.to_be_bytes());
        buf[DDTS_PCM_SAMPLE_DEPTH_OFFSET] = self.pcm_sample_depth;
        buf[DDTS_PACKED_OFFSET..DDTS_CHANNEL_LAYOUT_OFFSET]
            .copy_from_slice(&self.encode_packed_word().to_be_bytes());
        buf[DDTS_CHANNEL_LAYOUT_OFFSET..DDTS_FLAGS_OFFSET]
            .copy_from_slice(&self.channel_layout.to_be_bytes());
        buf[DDTS_FLAGS_OFFSET] = self.encode_flags_byte();
        Ok(need)
    }
}
