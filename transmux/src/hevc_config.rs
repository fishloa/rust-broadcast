//! HEVC decoder configuration record (`hvcC`) — ISO/IEC 14496-15:2017 §8.3.3.
//!
//! The `HEVCDecoderConfigurationRecord` carries the VPS/SPS/PPS/SEI NAL unit arrays
//! for H.265/HEVC video.
//!
//! # Wire layout (§8.3.3 p70–72)
//!
//! ```text
//! aligned(8) class HEVCDecoderConfigurationRecord {
//!   unsigned int(8)  configurationVersion = 1;
//!   unsigned int(2)  general_profile_space;
//!   unsigned int(1)  general_tier_flag;
//!   unsigned int(5)  general_profile_idc;
//!   unsigned int(32) general_profile_compatibility_flags;
//!   unsigned int(48) general_constraint_indicator_flags;
//!   unsigned int(8)  general_level_idc;
//!   bit(4) reserved; unsigned int(12) min_spatial_segmentation_idc;
//!   bit(6) reserved; unsigned int(2)  parallelismType;
//!   bit(6) reserved; unsigned int(2)  chroma_format_idc;
//!   bit(5) reserved; unsigned int(3)  bit_depth_luma_minus8;
//!   bit(5) reserved; unsigned int(3)  bit_depth_chroma_minus8;
//!   unsigned int(16) avgFrameRate;
//!   unsigned int(2)  constantFrameRate;
//!   unsigned int(3)  numTemporalLayers;
//!   unsigned int(1)  temporalIdNested;
//!   unsigned int(2)  lengthSizeMinusOne;
//!   unsigned int(8)  numOfArrays;
//!   for (j=0; j<numOfArrays; j++) {
//!     unsigned int(1)  array_completeness;
//!     bit(1) reserved=0;
//!     unsigned int(6)  NAL_unit_type;
//!     unsigned int(16) numNalus;
//!     for (i=0; i<numNalus; i++) { unsigned int(16) nalUnitLength; bit(8*len) nalUnit; }
//!   }
//! }
//! ```

use crate::error::{Error, Result};
use crate::nalu_types::{HevcNalArray, HevcNalUnit};
use alloc::vec::Vec;
use broadcast_common::{Parse, Serialize};
use core::fmt;

/// `lengthSizeMinusOne` must be 0, 1, or 3 (value 2 is invalid).
const VALID_LENGTH_SIZES: [u8; 3] = [0, 1, 3];

// ---------------------------------------------------------------------------
// HEVCDecoderConfigurationRecord
// ---------------------------------------------------------------------------

/// `HEVCDecoderConfigurationRecord` — H.265/HEVC decoder config (ISO/IEC 14496-15:2017 §8.3.3).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct HEVCDecoderConfigurationRecord {
    /// `configurationVersion` — shall be 1.
    pub configuration_version: u8,
    /// `general_profile_space` (2 bits).
    pub general_profile_space: u8,
    /// `general_tier_flag` (1 bit).
    pub general_tier_flag: bool,
    /// `general_profile_idc` (5 bits).
    pub general_profile_idc: u8,
    /// `general_profile_compatibility_flags` (32 bits).
    pub general_profile_compatibility_flags: u32,
    /// `general_constraint_indicator_flags` (48 bits).
    pub general_constraint_indicator_flags: u64,
    /// `general_level_idc` (8 bits).
    pub general_level_idc: u8,
    /// `min_spatial_segmentation_idc` (12 bits).
    pub min_spatial_segmentation_idc: u16,
    /// `parallelismType` (2 bits): 0=mixed/unknown, 1=slice, 2=tile, 3=entropy-sync.
    pub parallelism_type: u8,
    /// `chroma_format_idc` (2 bits).
    pub chroma_format_idc: u8,
    /// `bit_depth_luma_minus8` (3 bits).
    pub bit_depth_luma_minus8: u8,
    /// `bit_depth_chroma_minus8` (3 bits).
    pub bit_depth_chroma_minus8: u8,
    /// `avgFrameRate` (16 bits), frames per 256 seconds, 0=unspecified.
    pub avg_frame_rate: u16,
    /// `constantFrameRate` (2 bits): 0=not constant, 1=constant.
    pub constant_frame_rate: u8,
    /// `numTemporalLayers` (3 bits).
    pub num_temporal_layers: u8,
    /// `temporalIdNested` (1 bit).
    pub temporal_id_nested: bool,
    /// `lengthSizeMinusOne` ∈ {0,1,3}.
    pub length_size_minus_one: u8,
    /// `numOfArrays` — NAL unit arrays (VPS, SPS, PPS, SEI).
    pub arrays: Vec<HevcNalArray>,
}

impl HEVCDecoderConfigurationRecord {
    /// Whether the record has reserved bits that must be all-ones (the fields
    /// before avgFrameRate use all-ones reserved; the array header uses a zero reserved bit).
    pub fn is_valid_length_size(v: u8) -> bool {
        (v == 0) | (v == 1) | (v == 3)
    }

    /// RFC 6381 codec string (`hvc1.A.B.C.constraints`) built from the profile/
    /// tier/level fields carried in this record — ISO/IEC 14496-15:2017 §E.3.
    ///
    /// Delegates to [`crate::sps::rfc6381_hvc1`]; the profile-space/idc,
    /// compatibility flags, tier flag, level_idc and constraint-indicator flags
    /// carried here are the same values the decoder derives from the SPS.
    pub fn rfc6381(&self) -> alloc::string::String {
        crate::sps::rfc6381_hvc1(&crate::sps::HevcSpsInfo {
            general_profile_space: self.general_profile_space,
            general_tier_flag: self.general_tier_flag,
            general_profile_idc: self.general_profile_idc,
            general_profile_compatibility_flags: self.general_profile_compatibility_flags,
            general_constraint_indicator_flags: self.general_constraint_indicator_flags,
            general_level_idc: self.general_level_idc,
            chroma_format_idc: self.chroma_format_idc,
            bit_depth_luma: self.bit_depth_luma_minus8 + 8,
            bit_depth_chroma: self.bit_depth_chroma_minus8 + 8,
            width: 0,
            height: 0,
        })
    }
}

impl<'a> Parse<'a> for HEVCDecoderConfigurationRecord {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let mut cursor = 0usize;

        // configurationVersion (8)
        let configuration_version = read_u8(bytes, &mut cursor, "HEVC configurationVersion")?;

        // general_profile_space(2) + general_tier_flag(1) + general_profile_idc(5)
        let b1 = read_u8(bytes, &mut cursor, "general_profile_space/tier/idc")?;
        let general_profile_space = (b1 >> 6) & 0x03;
        let general_tier_flag = ((b1 >> 5) & 0x01) != 0;
        let general_profile_idc = b1 & 0x1F;

        // general_profile_compatibility_flags (32)
        let general_profile_compatibility_flags =
            read_u32(bytes, &mut cursor, "general_profile_compatibility_flags")?;

        // general_constraint_indicator_flags (48)
        let general_constraint_indicator_flags =
            read_u48(bytes, &mut cursor, "general_constraint_indicator_flags")?;

        // general_level_idc (8)
        let general_level_idc = read_u8(bytes, &mut cursor, "general_level_idc")?;

        // reserved(4) + min_spatial_segmentation_idc(12)
        let msid_bytes = read_u16(bytes, &mut cursor, "min_spatial_segmentation_idc")?;
        let _reserved_msid = (msid_bytes >> 12) & 0x0F;
        let min_spatial_segmentation_idc = msid_bytes & 0x0FFF;

        // reserved(6) + parallelismType(2)
        let b_pt = read_u8(bytes, &mut cursor, "parallelismType")?;
        let _reserved_pt = (b_pt >> 2) & 0x3F;
        let parallelism_type = b_pt & 0x03;

        // reserved(6) + chroma_format_idc(2)
        let b_cf = read_u8(bytes, &mut cursor, "chroma_format_idc")?;
        let _reserved_cf = (b_cf >> 2) & 0x3F;
        let chroma_format_idc = b_cf & 0x03;

        // reserved(5) + bit_depth_luma_minus8(3)
        let b_bdl = read_u8(bytes, &mut cursor, "bit_depth_luma_minus8")?;
        let _reserved_bdl = (b_bdl >> 3) & 0x1F;
        let bit_depth_luma_minus8 = b_bdl & 0x07;

        // reserved(5) + bit_depth_chroma_minus8(3)
        let b_bdc = read_u8(bytes, &mut cursor, "bit_depth_chroma_minus8")?;
        let _reserved_bdc = (b_bdc >> 3) & 0x1F;
        let bit_depth_chroma_minus8 = b_bdc & 0x07;

        // avgFrameRate (16)
        let avg_frame_rate = read_u16(bytes, &mut cursor, "avgFrameRate")?;

        // constantFrameRate(2) + numTemporalLayers(3) + temporalIdNested(1) + lengthSizeMinusOne(2)
        let b_cfr = read_u8(bytes, &mut cursor, "constantFrameRate/temporal")?;
        let constant_frame_rate = (b_cfr >> 6) & 0x03;
        let num_temporal_layers = (b_cfr >> 3) & 0x07;
        let temporal_id_nested = ((b_cfr >> 2) & 0x01) != 0;
        let length_size_minus_one = b_cfr & 0x03;
        if !VALID_LENGTH_SIZES.contains(&length_size_minus_one) {
            return Err(Error::InvalidValue {
                field: "lengthSizeMinusOne",
                value: length_size_minus_one as u64,
                reason: "must be 0, 1, or 3 (value 2 is invalid per ISO/IEC 14496-15:2017 §8.3.3)",
            });
        }

        // numOfArrays (8)
        let num_arrays = read_u8(bytes, &mut cursor, "numOfArrays")? as usize;

        // NAL arrays
        let mut arrays = Vec::with_capacity(num_arrays);
        for _ in 0..num_arrays {
            // byte: array_completeness(1) + reserved(1)=0 + NAL_unit_type(6)
            let b_arr = read_u8(bytes, &mut cursor, "NAL array header")?;
            let array_completeness = (b_arr >> 7) != 0;
            let _reserved_arr = (b_arr >> 6) & 0x01;
            let nal_unit_type = b_arr & 0x3F;

            // numNalus (16)
            let num_nalus = read_u16(bytes, &mut cursor, "numNalus")? as usize;

            let mut nalus = Vec::with_capacity(num_nalus);
            for _ in 0..num_nalus {
                // nalUnitLength (16)
                let nal_unit_len = read_u16(bytes, &mut cursor, "nalUnitLength")? as usize;
                if cursor + nal_unit_len > bytes.len() {
                    return Err(Error::BufferTooShort {
                        need: cursor + nal_unit_len,
                        have: bytes.len(),
                        what: "HEVC nalUnit",
                    });
                }
                let nalu_data = bytes[cursor..cursor + nal_unit_len].to_vec();
                cursor += nal_unit_len;
                nalus.push(HevcNalUnit(nalu_data));
            }

            arrays.push(HevcNalArray {
                array_completeness,
                nal_unit_type,
                nalus,
            });
        }

        Ok(Self {
            configuration_version,
            general_profile_space,
            general_tier_flag,
            general_profile_idc,
            general_profile_compatibility_flags,
            general_constraint_indicator_flags,
            general_level_idc,
            min_spatial_segmentation_idc,
            parallelism_type,
            chroma_format_idc,
            bit_depth_luma_minus8,
            bit_depth_chroma_minus8,
            avg_frame_rate,
            constant_frame_rate,
            num_temporal_layers,
            temporal_id_nested,
            length_size_minus_one,
            arrays,
        })
    }
}

impl Serialize for HEVCDecoderConfigurationRecord {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        // 23 bytes fixed before numOfArrays, then the arrays
        1 + 1
            + 4
            + 6
            + 1
            + 2
            + 1
            + 1
            + 1
            + 1
            + 2
            + 1
            + 1
            + self
                .arrays
                .iter()
                .map(|a| {
                    // 1 (array_completeness/reserved/nal_unit_type) + 2 (numNalus) + per-nalu: 2 (len) + data
                    3 + a.nalus.iter().map(|n| 2 + n.0.len()).sum::<usize>()
                })
                .sum::<usize>()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }

        let mut cursor = 0usize;

        // configurationVersion
        buf[cursor] = self.configuration_version;
        cursor += 1;

        // general_profile_space(2) + general_tier_flag(1) + general_profile_idc(5)
        buf[cursor] = ((self.general_profile_space & 0x03) << 6)
            | (if self.general_tier_flag { 0x20 } else { 0 })
            | (self.general_profile_idc & 0x1F);
        cursor += 1;

        // general_profile_compatibility_flags (32)
        buf[cursor..cursor + 4]
            .copy_from_slice(&self.general_profile_compatibility_flags.to_be_bytes());
        cursor += 4;

        // general_constraint_indicator_flags (48) — u64 stored as 6 bytes big-endian
        let ciflags = self.general_constraint_indicator_flags;
        buf[cursor..cursor + 6].copy_from_slice(&[
            ((ciflags >> 40) & 0xFF) as u8,
            ((ciflags >> 32) & 0xFF) as u8,
            ((ciflags >> 24) & 0xFF) as u8,
            ((ciflags >> 16) & 0xFF) as u8,
            ((ciflags >> 8) & 0xFF) as u8,
            (ciflags & 0xFF) as u8,
        ]);
        cursor += 6;

        // general_level_idc
        buf[cursor] = self.general_level_idc;
        cursor += 1;

        // reserved(4)=1s + min_spatial_segmentation_idc(12)
        let msid = 0xF000 | (self.min_spatial_segmentation_idc & 0x0FFF);
        buf[cursor..cursor + 2].copy_from_slice(&msid.to_be_bytes());
        cursor += 2;

        // reserved(6)=1s + parallelismType(2)
        buf[cursor] = 0xFC | (self.parallelism_type & 0x03);
        cursor += 1;

        // reserved(6)=1s + chroma_format_idc(2)
        buf[cursor] = 0xFC | (self.chroma_format_idc & 0x03);
        cursor += 1;

        // reserved(5)=1s + bit_depth_luma_minus8(3)
        buf[cursor] = 0xF8 | (self.bit_depth_luma_minus8 & 0x07);
        cursor += 1;

        // reserved(5)=1s + bit_depth_chroma_minus8(3)
        buf[cursor] = 0xF8 | (self.bit_depth_chroma_minus8 & 0x07);
        cursor += 1;

        // avgFrameRate (16)
        buf[cursor..cursor + 2].copy_from_slice(&self.avg_frame_rate.to_be_bytes());
        cursor += 2;

        // constantFrameRate(2) + numTemporalLayers(3) + temporalIdNested(1) + lengthSizeMinusOne(2)
        buf[cursor] = ((self.constant_frame_rate & 0x03) << 6)
            | ((self.num_temporal_layers & 0x07) << 3)
            | (if self.temporal_id_nested { 0x04 } else { 0x00 })
            | (self.length_size_minus_one & 0x03);
        cursor += 1;

        // numOfArrays
        buf[cursor] = self.arrays.len() as u8;
        cursor += 1;

        // NAL arrays
        for arr in &self.arrays {
            let first_byte =
                (if arr.array_completeness { 0x80u8 } else { 0 }) | (arr.nal_unit_type & 0x3F);
            buf[cursor] = first_byte;
            cursor += 1;

            let num = arr.nalus.len();
            buf[cursor..cursor + 2].copy_from_slice(&(num as u16).to_be_bytes());
            cursor += 2;

            for nalu in &arr.nalus {
                let len = nalu.0.len();
                buf[cursor..cursor + 2].copy_from_slice(&(len as u16).to_be_bytes());
                cursor += 2;
                buf[cursor..cursor + len].copy_from_slice(&nalu.0);
                cursor += len;
            }
        }

        Ok(cursor)
    }
}

impl fmt::Display for HEVCDecoderConfigurationRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "HEVC(tier={}, profile_idc={}, level={}, lenSize={}, arrays={})",
            if self.general_tier_flag {
                "high"
            } else {
                "main"
            },
            self.general_profile_idc,
            self.general_level_idc,
            self.length_size_minus_one + 1,
            self.arrays.len(),
        )
    }
}

// ---------------------------------------------------------------------------
// HEVCConfigurationBox — wraps the record inside a Box('hvcC')
// ---------------------------------------------------------------------------

/// Configuration box for HEVC — wraps `HEVCDecoderConfigurationRecord` in a `Box('hvcC')`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct HEVCConfigurationBox {
    /// The decoder configuration record.
    pub config: HEVCDecoderConfigurationRecord,
}

impl HEVCConfigurationBox {
    /// Parse an hvcC box from its body bytes (after the box header).
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        let config = HEVCDecoderConfigurationRecord::parse(body)?;
        Ok(Self { config })
    }

    /// Create from a pre-parsed record.
    pub fn new(config: HEVCDecoderConfigurationRecord) -> Self {
        Self { config }
    }
}

impl Serialize for HEVCConfigurationBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        8 + self.config.serialized_len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        let mut cursor = 0usize;
        let size32 = need as u32;
        buf[cursor..cursor + 4].copy_from_slice(&size32.to_be_bytes());
        cursor += 4;
        buf[cursor..cursor + 4].copy_from_slice(b"hvcC");
        cursor += 4;
        cursor += self.config.serialize_into(&mut buf[cursor..])?;
        Ok(cursor)
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn read_u8(bytes: &[u8], cursor: &mut usize, what: &'static str) -> Result<u8> {
    if *cursor >= bytes.len() {
        return Err(Error::BufferTooShort {
            need: *cursor + 1,
            have: bytes.len(),
            what,
        });
    }
    let v = bytes[*cursor];
    *cursor += 1;
    Ok(v)
}

fn read_u16(bytes: &[u8], cursor: &mut usize, what: &'static str) -> Result<u16> {
    if *cursor + 2 > bytes.len() {
        return Err(Error::BufferTooShort {
            need: *cursor + 2,
            have: bytes.len(),
            what,
        });
    }
    let v = u16::from_be_bytes([bytes[*cursor], bytes[*cursor + 1]]);
    *cursor += 2;
    Ok(v)
}

fn read_u32(bytes: &[u8], cursor: &mut usize, what: &'static str) -> Result<u32> {
    if *cursor + 4 > bytes.len() {
        return Err(Error::BufferTooShort {
            need: *cursor + 4,
            have: bytes.len(),
            what,
        });
    }
    let v = u32::from_be_bytes([
        bytes[*cursor],
        bytes[*cursor + 1],
        bytes[*cursor + 2],
        bytes[*cursor + 3],
    ]);
    *cursor += 4;
    Ok(v)
}

fn read_u48(bytes: &[u8], cursor: &mut usize, what: &'static str) -> Result<u64> {
    if *cursor + 6 > bytes.len() {
        return Err(Error::BufferTooShort {
            need: *cursor + 6,
            have: bytes.len(),
            what,
        });
    }
    let v = (bytes[*cursor] as u64) << 40
        | (bytes[*cursor + 1] as u64) << 32
        | (bytes[*cursor + 2] as u64) << 24
        | (bytes[*cursor + 3] as u64) << 16
        | (bytes[*cursor + 4] as u64) << 8
        | bytes[*cursor + 5] as u64;
    *cursor += 6;
    Ok(v)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use broadcast_common::{Parse, Serialize};

    fn make_minimal_hvcc_body() -> Vec<u8> {
        let mut body = Vec::new();

        // configurationVersion = 1
        body.push(0x01);
        // profile_space=0, tier_flag=0(main), profile_idc=1(main)
        body.push(0x01);
        // profile_compatibility_flags = 0
        body.extend_from_slice(&[0u8; 4]);
        // constraint_indicator_flags = 0 (48 bits)
        body.extend_from_slice(&[0u8; 6]);
        // level_idc = 93 (3.1 equivalent)
        body.push(93);
        // reserved(4)=1s + min_spatial_segmentation_idc=0
        body.extend_from_slice(&[0xF0, 0x00]);
        // reserved(6)=1s + parallelism=0
        body.push(0xFC);
        // reserved(6)=1s + chroma_format=1 (420)
        body.push(0xFD);
        // reserved(5)=1s + bit_depth_luma=0 (8 bit)
        body.push(0xF8);
        // reserved(5)=1s + bit_depth_chroma=0 (8 bit)
        body.push(0xF8);
        // avgFrameRate = 0 (unspecified)
        body.extend_from_slice(&[0u8; 2]);
        // constFrate=0 + numTempLayers=1 + temporalIdNested=0 + lenSize=3
        body.push(0x03);
        // numOfArrays = 1
        body.push(0x01);
        // Array: array_completeness=1, reserved=0, type=32(VPS)
        body.push(0x80 | 32);
        // numNalus = 1
        body.extend_from_slice(&[0x00, 0x01]);
        // NAL unit: length=5, data
        let vps = b"\x40\x01\x0C\x01\xFF";
        body.extend_from_slice(&(vps.len() as u16).to_be_bytes());
        body.extend_from_slice(vps);

        body
    }

    #[test]
    fn test_hevc_config_minimal() {
        let body = make_minimal_hvcc_body();
        let record = HEVCDecoderConfigurationRecord::parse(&body).unwrap();

        assert_eq!(record.configuration_version, 1);
        assert_eq!(record.general_profile_space, 0);
        assert!(!record.general_tier_flag);
        assert_eq!(record.general_profile_idc, 1);
        assert_eq!(record.general_level_idc, 93);
        assert_eq!(record.min_spatial_segmentation_idc, 0);
        assert_eq!(record.parallelism_type, 0);
        assert_eq!(record.chroma_format_idc, 1);
        assert_eq!(record.bit_depth_luma_minus8, 0);
        assert_eq!(record.bit_depth_chroma_minus8, 0);
        assert_eq!(record.avg_frame_rate, 0);
        assert_eq!(record.constant_frame_rate, 0);
        assert_eq!(record.num_temporal_layers, 0);
        assert!(!record.temporal_id_nested);
        assert_eq!(record.length_size_minus_one, 3);
        assert_eq!(record.arrays.len(), 1);
        assert!(record.arrays[0].array_completeness);
        assert_eq!(record.arrays[0].nal_unit_type, 32);
        assert_eq!(record.arrays[0].nalus.len(), 1);
        assert_eq!(record.arrays[0].nalus[0].0, b"\x40\x01\x0C\x01\xFF");
    }

    #[test]
    fn test_hevc_config_round_trip_minimal() {
        let body = make_minimal_hvcc_body();
        let record = HEVCDecoderConfigurationRecord::parse(&body).unwrap();
        let serialized = record.to_bytes();
        assert_eq!(serialized, body, "hvcC round-trip must be byte-identical");
    }

    #[test]
    fn test_hevc_config_invalid_length_size() {
        let mut body = make_minimal_hvcc_body();
        // The lengthSizeMinusOne is in byte at offset 21 (0-indexed: header 23 bytes before arrays)
        // Actually: 1+1+4+6+1+2+1+1+1+1+2 = 21 then the CFR byte
        body[21] = 0x02 | 0x04; // set lenSize=2 while preserving temporal bits
        let err = HEVCDecoderConfigurationRecord::parse(&body).unwrap_err();
        assert!(matches!(
            err,
            Error::InvalidValue {
                field: "lengthSizeMinusOne",
                ..
            }
        ));
    }

    #[test]
    fn test_hevc_config_field_mutation() {
        let body = make_minimal_hvcc_body();
        let mut record = HEVCDecoderConfigurationRecord::parse(&body).unwrap();
        record.general_level_idc = 120;
        let serialized = record.to_bytes();
        // level_idc is at offset 12 (1+1+4+6=12)
        assert_eq!(serialized[12], 120);
    }

    #[test]
    fn test_hevc_config_multiple_arrays() {
        let mut body = make_minimal_hvcc_body();
        // We need to rebuild the body with 3 arrays (VPS, SPS, PPS)
        body.clear();
        body.push(0x01); // configVersion
        body.push(0x01); // profile_space=0, tier=0, profile_idc=1
        body.extend_from_slice(&[0u8; 4]); // compat flags
        body.extend_from_slice(&[0u8; 6]); // constraint flags
        body.push(93); // level
        body.extend_from_slice(&[0xF0, 0x00]); // reserved+spatial
        body.push(0xFC); // reserved+parallelism
        body.push(0xFC); // reserved+chroma
        body.push(0xF8); // reserved+luma depth
        body.push(0xF8); // reserved+chroma depth
        body.extend_from_slice(&[0u8; 2]); // avgFrameRate
        body.push(0x0B); // CFR=0, temporal=1, nested=0, lenSize=3
        body.push(0x03); // numOfArrays = 3

        // VPS array (type=32): 1 NAL
        body.push(0x80 | 32);
        body.extend_from_slice(&[0x00, 0x01]);
        let vps = vec![0x40; 4];
        body.extend_from_slice(&(vps.len() as u16).to_be_bytes());
        body.extend_from_slice(&vps);

        // SPS array (type=33): 2 NALs
        body.push(0x80 | 33);
        body.extend_from_slice(&[0x00, 0x02]);
        let sps1 = vec![0x42; 5];
        body.extend_from_slice(&(sps1.len() as u16).to_be_bytes());
        body.extend_from_slice(&sps1);
        let sps2 = vec![0x44; 3];
        body.extend_from_slice(&(sps2.len() as u16).to_be_bytes());
        body.extend_from_slice(&sps2);

        // PPS array (type=34): 0 NALs
        body.push(34);
        body.extend_from_slice(&[0x00, 0x00]);

        let record = HEVCDecoderConfigurationRecord::parse(&body).unwrap();
        assert_eq!(record.arrays.len(), 3);
        assert_eq!(record.arrays[0].nal_unit_type, 32);
        assert_eq!(record.arrays[0].nalus.len(), 1);
        assert_eq!(record.arrays[1].nal_unit_type, 33);
        assert_eq!(record.arrays[1].nalus.len(), 2);
        assert_eq!(record.arrays[2].nal_unit_type, 34);
        assert_eq!(record.arrays[2].nalus.len(), 0);

        let serialized = record.to_bytes();
        assert_eq!(serialized, body);
    }

    #[test]
    fn test_hevc_config_nested_metadata() {
        let body = make_minimal_hvcc_body();
        let mut record = HEVCDecoderConfigurationRecord::parse(&body).unwrap();
        // Tweak various sub-byte fields
        record.temporal_id_nested = true;
        record.num_temporal_layers = 3;
        record.constant_frame_rate = 1;
        record.parallelism_type = 2;
        record.chroma_format_idc = 2;

        let serialized = record.to_bytes();
        let re = HEVCDecoderConfigurationRecord::parse(&serialized).unwrap();
        assert!(re.temporal_id_nested);
        assert_eq!(re.num_temporal_layers, 3);
        assert_eq!(re.constant_frame_rate, 1);
        assert_eq!(re.parallelism_type, 2);
        assert_eq!(re.chroma_format_idc, 2);
    }
}
