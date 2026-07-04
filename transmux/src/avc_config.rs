//! AVC decoder configuration record (`avcC`) — ISO/IEC 14496-15:2017 §5.3.3.
//!
//! The `AVCDecoderConfigurationRecord` carries the SPS and PPS NAL units for
//! H.264/AVC video, plus optional high-profile extensions for chroma and bit depth.
//!
//! # Wire layout (§5.3.3 p17–19)
//!
//! ```text
//! aligned(8) class AVCDecoderConfigurationRecord {
//!   unsigned int(8)  configurationVersion = 1;
//!   unsigned int(8)  AVCProfileIndication;
//!   unsigned int(8)  profile_compatibility;
//!   unsigned int(8)  AVCLevelIndication;
//!   bit(6) reserved = '111111'b;
//!   unsigned int(2)  lengthSizeMinusOne;
//!   bit(3) reserved = '111'b;
//!   unsigned int(5)  numOfSequenceParameterSets;
//!   for (..SPS..) { unsigned int(16) len; bit(8*len) nalu; }
//!   unsigned int(8)  numOfPictureParameterSets;
//!   for (..PPS..) { unsigned int(16) len; bit(8*len) nalu; }
//!   if (profile∈{100,110,122,244}) {
//!     bit(6) reserved; unsigned int(2) chroma_format;
//!     bit(5) reserved; unsigned int(3) bit_depth_luma_minus8;
//!     bit(5) reserved; unsigned int(3) bit_depth_chroma_minus8;
//!     unsigned int(8) numOfSequenceParameterSetExt;
//!     for (..SPSExt..) { unsigned int(16) len; bit(8*len) nalu; }
//!   }
//! }
//! ```
//!
//! # profile_idc 244, not 144 (#563)
//!
//! Early drafts of ISO/IEC 14496-15 (and some still-circulating transcriptions)
//! list this condition as `profile_idc∈{100,110,122,144}`. `144` was the
//! placeholder profile number assigned to the not-yet-finalized "High 4:4:4"
//! profile before ITU-T H.264 Amendment 3 (2005) finalized it as **profile_idc
//! 244**, "High 4:4:4 Predictive Profile" (H.264 Table A-1); no encoder ever
//! emits SPS profile_idc 144. Real muxers (verified against an `ffmpeg -c copy`
//! remux of `fixtures/ts/h264/high444.ts`, profile_idc 244) write the
//! chroma_format/bit_depth extension for profile_idc 244, not 144.

use crate::error::{Error, Result};
use crate::nalu_types::{AvcPps, AvcSps, AvcSpsExt};
use alloc::vec::Vec;
use broadcast_common::{Parse, Serialize};
use core::fmt;

/// `lengthSizeMinusOne` must be 0, 1, or 3 (value 2 is invalid).
const VALID_LENGTH_SIZES: [u8; 3] = [0, 1, 3];

// ---------------------------------------------------------------------------
// AVCDecoderConfigurationRecord
// ---------------------------------------------------------------------------

/// `AVCDecoderConfigurationRecord` — H.264/AVC decoder config (ISO/IEC 14496-15:2017 §5.3.3).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct AVCDecoderConfigurationRecord {
    /// `configurationVersion` — shall be 1.
    pub configuration_version: u8,
    /// `AVCProfileIndication` — profile_idc per ISO/IEC 14496-10.
    pub profile_indication: u8,
    /// `profile_compatibility` — the constraint-flags byte from SPS.
    pub profile_compatibility: u8,
    /// `AVCLevelIndication` — level_idc.
    pub level_indication: u8,
    /// `lengthSizeMinusOne` — NAL length field width minus 1 (∈{0,1,3}).
    pub length_size_minus_one: u8,
    /// Sequence parameter sets (SPS).
    pub sps: Vec<AvcSps>,
    /// Picture parameter sets (PPS).
    pub pps: Vec<AvcPps>,
    /// High-profile extension: chroma_format_idc (only present for profile 100/110/122/244).
    pub chroma_format: Option<u8>,
    /// High-profile extension: bit_depth_luma_minus8.
    pub bit_depth_luma_minus8: Option<u8>,
    /// High-profile extension: bit_depth_chroma_minus8.
    pub bit_depth_chroma_minus8: Option<u8>,
    /// High-profile extension: SPS ext (only present for profile 100/110/122/244).
    pub sps_ext: Vec<AvcSpsExt>,
}

impl AVCDecoderConfigurationRecord {
    /// True when the profile requires the high-profile extension fields
    /// (chroma_format, bit depths) — ISO/IEC 14496-15 §5.3.3.1.2. Shared with
    /// the TS demuxer's avcC populator via `sps::is_high_profile` so serialize
    /// and demux agree on the exact set (incl. 244 = High 4:4:4 Predictive; #563).
    fn has_high_profile_ext(profile: u8) -> bool {
        crate::sps::is_high_profile(profile)
    }
}

impl<'a> Parse<'a> for AVCDecoderConfigurationRecord {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let mut cursor = 0usize;

        // configurationVersion (8)
        let configuration_version = read_u8(bytes, &mut cursor, "configurationVersion")?;

        // AVCProfileIndication (8)
        let profile_indication = read_u8(bytes, &mut cursor, "AVCProfileIndication")?;

        // profile_compatibility (8)
        let profile_compatibility = read_u8(bytes, &mut cursor, "profile_compatibility")?;

        // AVCLevelIndication (8)
        let level_indication = read_u8(bytes, &mut cursor, "AVCLevelIndication")?;

        // reserved(6) + lengthSizeMinusOne(2)
        let b4 = read_u8(bytes, &mut cursor, "lengthSizeMinusOne byte")?;
        let _reserved_6 = (b4 >> 2) & 0x3F;
        let length_size_minus_one = b4 & 0x03;
        // Reject value 2 as invalid (§5.3.3 p19).
        if !VALID_LENGTH_SIZES.contains(&length_size_minus_one) {
            return Err(Error::InvalidValue {
                field: "lengthSizeMinusOne",
                value: length_size_minus_one as u64,
                reason: "must be 0, 1, or 3 (value 2 is invalid per ISO/IEC 14496-15:2017 §5.3.3)",
            });
        }

        // reserved(3) + numOfSequenceParameterSets(5)
        let b5 = read_u8(bytes, &mut cursor, "numOfSequenceParameterSets byte")?;
        let _reserved_3 = (b5 >> 5) & 0x07;
        let num_sps = (b5 & 0x1F) as usize;

        // SPS array
        let mut sps = Vec::with_capacity(num_sps);
        for _ in 0..num_sps {
            let nalu = read_nalu_16(bytes, &mut cursor, "SPS")?;
            sps.push(AvcSps(nalu));
        }

        // numOfPictureParameterSets (8)
        let num_pps = read_u8(bytes, &mut cursor, "numOfPictureParameterSets")? as usize;

        // PPS array
        let mut pps = Vec::with_capacity(num_pps);
        for _ in 0..num_pps {
            let nalu = read_nalu_16(bytes, &mut cursor, "PPS")?;
            pps.push(AvcPps(nalu));
        }

        // High-profile extensions (optional, for profile 100/110/122/244)
        let mut chroma_format = None;
        let mut bit_depth_luma_minus8 = None;
        let mut bit_depth_chroma_minus8 = None;
        let mut sps_ext = Vec::new();

        if Self::has_high_profile_ext(profile_indication) {
            // byte: reserved(6) + chroma_format(2)
            let b_cf = read_u8(bytes, &mut cursor, "chroma_format byte")?;
            let _reserved_cf = (b_cf >> 2) & 0x3F;
            let cf = b_cf & 0x03;
            chroma_format = Some(cf);

            // byte: reserved(5) + bit_depth_luma_minus8(3)
            let b_bdl = read_u8(bytes, &mut cursor, "bit_depth_luma_minus8 byte")?;
            let _reserved_bdl = (b_bdl >> 3) & 0x1F;
            let bdl = b_bdl & 0x07;
            bit_depth_luma_minus8 = Some(bdl);

            // byte: reserved(5) + bit_depth_chroma_minus8(3)
            let b_bdc = read_u8(bytes, &mut cursor, "bit_depth_chroma_minus8 byte")?;
            let _reserved_bdc = (b_bdc >> 3) & 0x1F;
            let bdc = b_bdc & 0x07;
            bit_depth_chroma_minus8 = Some(bdc);

            // numOfSequenceParameterSetExt (8)
            let num_sps_ext = read_u8(bytes, &mut cursor, "numOfSequenceParameterSetExt")? as usize;

            // SPSExt array
            sps_ext = Vec::with_capacity(num_sps_ext);
            for _ in 0..num_sps_ext {
                let nalu = read_nalu_16(bytes, &mut cursor, "SPSExt")?;
                sps_ext.push(AvcSpsExt(nalu));
            }
        }

        Ok(Self {
            configuration_version,
            profile_indication,
            profile_compatibility,
            level_indication,
            length_size_minus_one,
            sps,
            pps,
            chroma_format,
            bit_depth_luma_minus8,
            bit_depth_chroma_minus8,
            sps_ext,
        })
    }
}

impl Serialize for AVCDecoderConfigurationRecord {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        let mut len = 6usize; // configVersion + profile + compat + level + lengthSize + numSPS
        // SPS: each has 2-byte length prefix + data
        for sps in &self.sps {
            len += 2 + sps.0.len();
        }
        len += 1; // numPPS
        for pps in &self.pps {
            len += 2 + pps.0.len();
        }
        // High-profile ext
        if Self::has_high_profile_ext(self.profile_indication) {
            len += 4; // chroma_fmt + bit_depth_luma + bit_depth_chroma + numSPSExt
            for sps_ext in &self.sps_ext {
                len += 2 + sps_ext.0.len();
            }
        }
        len
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

        // AVCProfileIndication
        buf[cursor] = self.profile_indication;
        cursor += 1;

        // profile_compatibility
        buf[cursor] = self.profile_compatibility;
        cursor += 1;

        // AVCLevelIndication
        buf[cursor] = self.level_indication;
        cursor += 1;

        // reserved(6) + lengthSizeMinusOne(2): reserved bits = all-ones
        buf[cursor] = 0xFC | (self.length_size_minus_one & 0x03);
        cursor += 1;

        // reserved(3) + numOfSequenceParameterSets(5): reserved bits = all-ones
        let num_sps = self.sps.len();
        buf[cursor] = 0xE0 | ((num_sps as u8) & 0x1F);
        cursor += 1;

        // SPS array
        for sps in &self.sps {
            let len = sps.0.len() as u16;
            buf[cursor..cursor + 2].copy_from_slice(&len.to_be_bytes());
            cursor += 2;
            buf[cursor..cursor + sps.0.len()].copy_from_slice(&sps.0);
            cursor += sps.0.len();
        }

        // numOfPictureParameterSets
        let num_pps = self.pps.len();
        buf[cursor] = num_pps as u8;
        cursor += 1;

        // PPS array
        for pps in &self.pps {
            let len = pps.0.len() as u16;
            buf[cursor..cursor + 2].copy_from_slice(&len.to_be_bytes());
            cursor += 2;
            buf[cursor..cursor + pps.0.len()].copy_from_slice(&pps.0);
            cursor += pps.0.len();
        }

        // High-profile ext
        if Self::has_high_profile_ext(self.profile_indication) {
            let cf = self.chroma_format.unwrap_or(0);
            buf[cursor] = 0xFC | (cf & 0x03);
            cursor += 1;

            let bdl = self.bit_depth_luma_minus8.unwrap_or(0);
            buf[cursor] = 0xF8 | (bdl & 0x07);
            cursor += 1;

            let bdc = self.bit_depth_chroma_minus8.unwrap_or(0);
            buf[cursor] = 0xF8 | (bdc & 0x07);
            cursor += 1;

            // numOfSequenceParameterSetExt
            let num_sps_ext = self.sps_ext.len();
            buf[cursor] = num_sps_ext as u8;
            cursor += 1;

            // SPSExt array
            for sps_ext in &self.sps_ext {
                let len = sps_ext.0.len() as u16;
                buf[cursor..cursor + 2].copy_from_slice(&len.to_be_bytes());
                cursor += 2;
                buf[cursor..cursor + sps_ext.0.len()].copy_from_slice(&sps_ext.0);
                cursor += sps_ext.0.len();
            }
        }

        Ok(cursor)
    }
}

impl fmt::Display for AVCDecoderConfigurationRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "AVC(profile={}, compat=0x{:02x}, level={}, lenSize={}, sps={}, pps={})",
            self.profile_indication,
            self.profile_compatibility,
            self.level_indication,
            self.length_size_minus_one + 1,
            self.sps.len(),
            self.pps.len(),
        )
    }
}

// ---------------------------------------------------------------------------
// AVCConfigurationBox — wraps the record inside a Box('avcC')
// ---------------------------------------------------------------------------

/// Configuration box for AVC — wraps `AVCDecoderConfigurationRecord` in a `Box('avcC')`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct AVCConfigurationBox {
    /// The decoder configuration record.
    pub config: AVCDecoderConfigurationRecord,
}

impl AVCConfigurationBox {
    /// Parse an avcC box from its body bytes (after the box header).
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        let config = AVCDecoderConfigurationRecord::parse(body)?;
        Ok(Self { config })
    }

    /// Create from a pre-parsed record.
    pub fn new(config: AVCDecoderConfigurationRecord) -> Self {
        Self { config }
    }
}

impl Serialize for AVCConfigurationBox {
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
        buf[cursor..cursor + 4].copy_from_slice(b"avcC");
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

fn read_nalu_16(bytes: &[u8], cursor: &mut usize, what: &'static str) -> Result<Vec<u8>> {
    if *cursor + 2 > bytes.len() {
        return Err(Error::BufferTooShort {
            need: *cursor + 2,
            have: bytes.len(),
            what,
        });
    }
    let len = u16::from_be_bytes([bytes[*cursor], bytes[*cursor + 1]]) as usize;
    *cursor += 2;

    if *cursor + len > bytes.len() {
        return Err(Error::BufferTooShort {
            need: *cursor + len,
            have: bytes.len(),
            what,
        });
    }
    let data = bytes[*cursor..*cursor + len].to_vec();
    *cursor += len;
    Ok(data)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use broadcast_common::{Parse, Serialize};

    /// Minimal avcC: baseline profile at level 3.0, 1 SPS, 1 PPS, no high-profile ext.
    fn make_minimal_avcc_body() -> Vec<u8> {
        // ConfigVersion | Profile=66(baseline) | compat | level=30 | resv6+lenSize(3) | resv3+numSPS(1)
        let mut body = vec![
            0x01,        // configurationVersion
            0x42,        // AVCProfileIndication = 66 (baseline)
            0x00,        // profile_compatibility
            0x1E,        // AVCLevelIndication = 30
            0xFC | 0x03, // reserved(6)=1s, lengthSizeMinusOne=3 (4-byte NAL length)
            0xE0 | 0x01, // reserved(3)=1s, numSPS=1
        ];
        // SPS length + SPS data
        let sps = vec![0x67, 0x42, 0x00, 0x1E, 0xAB, 0x40];
        body.extend_from_slice(&(sps.len() as u16).to_be_bytes());
        body.extend_from_slice(&sps);

        // numPPS = 1
        body.push(0x01);
        let pps = vec![0x68, 0xCE, 0x3C, 0x80];
        body.extend_from_slice(&(pps.len() as u16).to_be_bytes());
        body.extend_from_slice(&pps);

        body
    }

    #[test]
    fn test_avc_config_minimal() {
        let body = make_minimal_avcc_body();
        let record = AVCDecoderConfigurationRecord::parse(&body).unwrap();

        assert_eq!(record.configuration_version, 1);
        assert_eq!(record.profile_indication, 66);
        assert_eq!(record.profile_compatibility, 0);
        assert_eq!(record.level_indication, 0x1E);
        assert_eq!(record.length_size_minus_one, 3);
        assert_eq!(record.sps.len(), 1);
        assert_eq!(record.sps[0].0, [0x67, 0x42, 0x00, 0x1E, 0xAB, 0x40]);
        assert_eq!(record.pps.len(), 1);
        assert_eq!(record.pps[0].0, [0x68, 0xCE, 0x3C, 0x80]);
        assert!(record.chroma_format.is_none());
        assert!(record.bit_depth_luma_minus8.is_none());
        assert!(record.bit_depth_chroma_minus8.is_none());
        assert!(record.sps_ext.is_empty());
    }

    #[test]
    fn test_avc_config_round_trip_minimal() {
        let body = make_minimal_avcc_body();
        let record = AVCDecoderConfigurationRecord::parse(&body).unwrap();
        let serialized = record.to_bytes();
        assert_eq!(serialized, body, "avcC round-trip must be byte-identical");
    }

    #[test]
    fn test_avc_config_high_profile_ext() {
        // High profile (100) with chroma_format=1 (420), bit_depth=8 (0), 1 SPSExt
        let mut body = vec![
            0x01,        // configurationVersion
            0x64,        // AVCProfileIndication = 100 (High)
            0x00,        // profile_compatibility
            0x1E,        // AVCLevelIndication = 30
            0xFC | 0x03, // lengthSizeMinusOne = 3
            0xE0 | 0x01, // numSPS = 1
        ];
        let sps = vec![0x67, 0x64, 0x00, 0x1E];
        body.extend_from_slice(&(sps.len() as u16).to_be_bytes());
        body.extend_from_slice(&sps);

        // numPPS = 1
        body.push(0x01);
        let pps = vec![0x68, 0xEE, 0x3C];
        body.extend_from_slice(&(pps.len() as u16).to_be_bytes());
        body.extend_from_slice(&pps);

        // High-profile ext
        body.push(0xFC | 0x01); // resv(6)=1s + chroma_format=1
        body.push(0xF8); // resv(5)=1s + bit_depth_luma_minus8=0
        body.push(0xF8); // resv(5)=1s + bit_depth_chroma_minus8=0
        body.push(0x01); // numSPSExt = 1
        let sps_ext = vec![0xF0, 0x00, 0x10];
        body.extend_from_slice(&(sps_ext.len() as u16).to_be_bytes());
        body.extend_from_slice(&sps_ext);

        let record = AVCDecoderConfigurationRecord::parse(&body).unwrap();
        assert_eq!(record.profile_indication, 100);
        assert_eq!(record.chroma_format, Some(1));
        assert_eq!(record.bit_depth_luma_minus8, Some(0));
        assert_eq!(record.bit_depth_chroma_minus8, Some(0));
        assert_eq!(record.sps_ext.len(), 1);
        assert_eq!(record.sps_ext[0].0, [0xF0, 0x00, 0x10]);

        let serialized = record.to_bytes();
        assert_eq!(serialized, body, "high-profile avcC round-trip");
    }

    #[test]
    fn test_avc_config_invalid_length_size() {
        let body = make_minimal_avcc_body();
        let mut bad = body.clone();
        // Change lengthSizeMinusOne to 2 (invalid)
        bad[4] = 0xFE;
        let err = AVCDecoderConfigurationRecord::parse(&bad).unwrap_err();
        assert!(matches!(
            err,
            Error::InvalidValue {
                field: "lengthSizeMinusOne",
                ..
            }
        ));
    }

    #[test]
    fn test_avc_config_field_mutation() {
        let body = make_minimal_avcc_body();
        let mut record = AVCDecoderConfigurationRecord::parse(&body).unwrap();
        record.level_indication = 50;
        let serialized = record.to_bytes();
        // Level is at offset 3
        assert_eq!(
            serialized[3], 50,
            "mutated level must appear in serialized bytes"
        );
        // The unmutated fields before it should still match
        assert_eq!(serialized[0], 1);
        assert_eq!(serialized[1], 66);
    }

    #[test]
    fn test_avc_config_profile_244_gets_ext() {
        // profile=244 (High 4:4:4 Predictive, ITU-T H.264 Table A-1) must get the
        // high-profile ext. `144` (the pre-Amendment-3 placeholder number, never
        // emitted by real encoders) must NOT — see the module doc (#563).
        let mut body = vec![
            0x01,
            0xF4,
            0x00,
            0x1E,        // configVersion=1, profile=244, compat=0, level=30
            0xFC | 0x03, // lenSize=3
            0xE0,        // numSPS=0
            0x00,        // numPPS=0
        ];
        // High-profile ext (all zeros with reserved bits set)
        body.push(0xFC | 0x01); // chroma_format=1
        body.push(0xF8); // bit_depth_luma=0
        body.push(0xF8); // bit_depth_chroma=0
        body.push(0x00); // numSPSExt=0

        let record = AVCDecoderConfigurationRecord::parse(&body).unwrap();
        assert_eq!(record.profile_indication, 244);
        assert_eq!(record.chroma_format, Some(1));
        assert_eq!(record.bit_depth_luma_minus8, Some(0));
        assert_eq!(record.bit_depth_chroma_minus8, Some(0));
    }

    #[test]
    fn test_avc_config_profile_144_no_ext() {
        // profile=144 is NOT a real H.264 profile_idc (see module doc, #563) —
        // it must NOT be treated as requiring the high-profile extension.
        let body = vec![
            0x01,
            0x90,
            0x00,
            0x1E,        // configVersion=1, profile=144, compat=0, level=30
            0xFC | 0x03, // lenSize=3
            0xE0,        // numSPS=0
            0x00,        // numPPS=0
        ];
        let record = AVCDecoderConfigurationRecord::parse(&body).unwrap();
        assert_eq!(record.profile_indication, 144);
        assert_eq!(record.chroma_format, None);
        assert_eq!(record.bit_depth_luma_minus8, None);
        assert_eq!(record.bit_depth_chroma_minus8, None);
    }
}
