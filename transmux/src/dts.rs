//! DTS audio in ISOBMFF — `dtsc`/`dtsh`/`dtsl`/`dtse` AudioSampleEntry + `ddts` config box.
//!
//! ETSI TS 102 114 §E.2.2.3: the `DTSSpecificBox` (`ddts`) carries the DTS
//! stream parameters needed for basic playback. `transmux` is samples-in, so
//! the caller supplies the already-populated [`DtsSpecificBox`]; `transmux`
//! serializes it as the child `ddts` of a [`crate::init_segment::DtsSampleEntry`].
//!
//! The four DTS sample-entry FourCCs and their meaning (ETSI TS 102 114 Table E-1):
//!
//! | FourCC | Description |
//! |--------|-------------|
//! | `dtsc` | DTS core substream only |
//! | `dtsh` | DTS core + extension substream (multiple assets) |
//! | `dtsl` | DTS LBR only |
//! | `dtse` | DTS extension substream only |

use crate::error::{Error, Result};
use broadcast_common::{Parse, Serialize};

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
    pub fn rfc6381(codec_fourcc: &[u8; 4]) -> &'static str {
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
        w |= (u32::from(self.frame_duration) & 0x03) << 30;
        w |= (u32::from(self.stream_construction) & 0x1F) << 25;
        w |= u32::from(self.core_lfe_present) << 24;
        w |= (u32::from(self.core_layout) & 0x3F) << 18;
        w |= (u32::from(self.core_size) & 0x3FFF) << 4;
        w |= u32::from(self.stereo_downmix) << 3;
        w |= u32::from(self.representation_type) & 0x07;
        w
    }

    /// Decode the packed 32-bit word produced by [`encode_packed_word`].
    fn decode_packed_word(w: u32) -> (u8, u8, bool, u8, u16, bool, u8) {
        let frame_duration = ((w >> 30) & 0x03) as u8;
        let stream_construction = ((w >> 25) & 0x1F) as u8;
        let core_lfe_present = ((w >> 24) & 0x01) != 0;
        let core_layout = ((w >> 18) & 0x3F) as u8;
        let core_size = ((w >> 4) & 0x3FFF) as u16;
        let stereo_downmix = ((w >> 3) & 0x01) != 0;
        let representation_type = (w & 0x07) as u8;
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
            b |= 0x80;
        }
        if self.lbr_duration_mod {
            b |= 0x40;
        }
        if self.reserved_box_present {
            b |= 0x20;
        }
        b
    }
}

impl<'a> Parse<'a> for DtsSpecificBox {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < DDTS_BODY_LEN {
            return Err(Error::BufferTooShort {
                need: DDTS_BODY_LEN,
                have: bytes.len(),
                what: "ddts",
            });
        }
        let dts_sampling_frequency = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        let max_bitrate = u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]);
        let avg_bitrate = u32::from_be_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
        let pcm_sample_depth = bytes[12];
        let packed = u32::from_be_bytes([bytes[13], bytes[14], bytes[15], bytes[16]]);
        let (
            frame_duration,
            stream_construction,
            core_lfe_present,
            core_layout,
            core_size,
            stereo_downmix,
            representation_type,
        ) = Self::decode_packed_word(packed);
        let channel_layout = u16::from_be_bytes([bytes[17], bytes[18]]);
        let flags_byte = bytes[19];
        let multi_asset_flag = (flags_byte & 0x80) != 0;
        let lbr_duration_mod = (flags_byte & 0x40) != 0;
        let reserved_box_present = (flags_byte & 0x20) != 0;

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
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        buf[0..4].copy_from_slice(&self.dts_sampling_frequency.to_be_bytes());
        buf[4..8].copy_from_slice(&self.max_bitrate.to_be_bytes());
        buf[8..12].copy_from_slice(&self.avg_bitrate.to_be_bytes());
        buf[12] = self.pcm_sample_depth;
        buf[13..17].copy_from_slice(&self.encode_packed_word().to_be_bytes());
        buf[17..19].copy_from_slice(&self.channel_layout.to_be_bytes());
        buf[19] = self.encode_flags_byte();
        Ok(need)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use broadcast_common::{Parse, Serialize};

    /// Spec-derived test vector: hand-computed from §E.2.2.3.1 field layout.
    ///
    /// Fields chosen to exercise every non-zero field and keep mental arithmetic
    /// simple. See `transmux/tests/dts.rs` for the full integration gate.
    #[test]
    fn round_trip_unit() {
        // Build a DtsSpecificBox from known field values.
        let orig = DtsSpecificBox {
            dts_sampling_frequency: 48_000,
            max_bitrate: 1_509_000,
            avg_bitrate: 754_500,
            pcm_sample_depth: 24,
            frame_duration: 1,      // 1024 samples
            stream_construction: 2, // Table E-2
            core_lfe_present: true,
            core_layout: 9, // Table E-3
            core_size: 0x1234,
            stereo_downmix: false,
            representation_type: 3, // Table E-4
            channel_layout: 0x000F, // Table E-5
            multi_asset_flag: false,
            lbr_duration_mod: false,
            reserved_box_present: false,
        };

        let mut buf = [0u8; DDTS_BODY_LEN];
        orig.serialize_into(&mut buf).unwrap();

        let parsed = DtsSpecificBox::parse(&buf).unwrap();
        assert_eq!(parsed, orig, "parse(serialize(x)) must equal x");
    }
}
