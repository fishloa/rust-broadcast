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
//! Early drafts of the AVC file format standard (and some still-circulating transcriptions)
//! list this condition as `profile_idc∈{100,110,122,144}`. `144` was the
//! placeholder profile number assigned to the not-yet-finalized "High 4:4:4"
//! profile before ITU-T H.264 Amendment 3 (2005) finalized it as **profile_idc
//! 244**, "High 4:4:4 Predictive Profile" (H.264 Table A-1); no encoder ever
//! emits SPS profile_idc 144. Real muxers (verified against an `ffmpeg -c copy`
//! remux of `fixtures/ts/h264/high444.ts`, profile_idc 244) write the
//! chroma_format/bit_depth extension for profile_idc 244, not 144.

use crate::error::{Error, Result};
use crate::nalu_types::{AvcPps, AvcSps, AvcSpsExt, VALID_LENGTH_SIZES};
use alloc::vec::Vec;
use broadcast_core::error::{BufferTooShort, InvalidValue};
use broadcast_core::len::need_out;
use broadcast_core::wire::{read_u8, read_u16, take};
use broadcast_core::{Parse, Serialize};
use core::fmt;

/// Largest permitted `bit_depth_luma_minus8` / `bit_depth_chroma_minus8`
/// (ISO/IEC 14496-15:2017 §5.3.3.1.3: range 0 to 4, inclusive).
pub(crate) const MAX_BIT_DEPTH_MINUS8: u8 = 4;

/// `AVCProfileIndication` High profile (ISO/IEC 14496-15:2017 §5.3.3.1.2).
const AVC_PROFILE_HIGH: u8 = 100;
/// `AVCProfileIndication` High 10 profile.
const AVC_PROFILE_HIGH_10: u8 = 110;
/// `AVCProfileIndication` High 4:2:2 profile.
const AVC_PROFILE_HIGH_422: u8 = 122;
/// `AVCProfileIndication` High 4:4:4 Predictive profile.
const AVC_PROFILE_HIGH_444_PREDICTIVE: u8 = 244;

/// Bytes before the parameter-set arrays: configurationVersion,
/// AVCProfileIndication, profile_compatibility, AVCLevelIndication,
/// lengthSizeMinusOne and numOfSequenceParameterSets.
const RECORD_FIXED_HEADER_LEN: usize = 6;
/// `nalUnitLength` is a `u16` (§5.3.3.1.1).
const NAL_LENGTH_PREFIX_LEN: usize = 2;
/// Bytes of the high-profile trailer before its SPS extension array:
/// chroma_format, bit_depth_luma_minus8, bit_depth_chroma_minus8 and
/// numOfSequenceParameterSetExt.
const HIGH_PROFILE_TRAILER_LEN: usize = 4;

/// Bit width of `numOfSequenceParameterSets`.
const NUM_SPS_BITS: u32 = 5;
/// Shifts and masks of the `lengthSizeMinusOne` byte.
const LENGTH_SIZE_SHIFT: u32 = 2;
const LENGTH_SIZE_RESERVED_MASK: u8 = 0x3F;
const LENGTH_SIZE_RESERVED_BITS: u8 = 0xFC;
const LENGTH_SIZE_MASK: u8 = 0x03;
/// Shifts and masks of the `numOfSequenceParameterSets` byte.
const NUM_SPS_SHIFT: u32 = 5;
const NUM_SPS_RESERVED_MASK: u8 = 0x07;
const NUM_SPS_RESERVED_BITS: u8 = 0xE0;
const NUM_SPS_MASK: u8 = 0x1F;
/// Shifts and masks of the `chroma_format` byte.
const CHROMA_FORMAT_SHIFT: u32 = 2;
const CHROMA_FORMAT_RESERVED_MASK: u8 = 0x3F;
const CHROMA_FORMAT_RESERVED_BITS: u8 = 0xFC;
const CHROMA_FORMAT_MASK: u8 = 0x03;
/// Shifts and masks of the `bit_depth_*_minus8` bytes.
const BIT_DEPTH_SHIFT: u32 = 3;
const BIT_DEPTH_RESERVED_MASK: u8 = 0x1F;
const BIT_DEPTH_RESERVED_BITS: u8 = 0xF8;
const BIT_DEPTH_MASK: u8 = 0x07;

/// One `avcC` parameter-set entry: a `u16` `nalUnitLength` followed by that
/// many bytes. The only user of this shape is this record's SPS/PPS arrays.
fn read_nalu_16(bytes: &[u8], cursor: &mut usize, what: &'static str) -> Result<Vec<u8>> {
    let len = usize::from(read_u16(bytes, cursor, what)?);
    Ok(take(bytes, cursor, len, what)?.to_vec())
}

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
    /// (chroma_format, bit depths) — ISO/IEC 14496-15:2017 §5.3.3.1.2. The
    /// *record's* condition is `AVCProfileIndication ∈ {100, 110, 122, 244}`
    /// (144 was the draft's placeholder for High 4:4:4 Predictive, finalized as
    /// 244; #563), so this is deliberately narrower than
    /// `broadcast_bitstream::sps::is_high_profile`, which is the wider H.264 Table A-1 list
    /// that governs *parsing* an SPS's syntax.
    ///
    /// The one place both an `avcC`-building caller and the SPS decoder can
    /// reach, so the record and the wire cannot disagree about the trailer's
    /// presence. Used by `transmux::rtp_sdp`'s `avcC` builder as well as this
    /// module's own constructors.
    pub fn has_high_profile_ext(profile: u8) -> bool {
        matches!(
            profile,
            AVC_PROFILE_HIGH
                | AVC_PROFILE_HIGH_10
                | AVC_PROFILE_HIGH_422
                | AVC_PROFILE_HIGH_444_PREDICTIVE
        )
    }

    /// Fail unless the record carries at least one SPS.
    ///
    /// `parse` accepts a record with `numOfSequenceParameterSets == 0`
    /// (ISO/IEC 14496-15:2017 §5.3.3.1.1: a parameter set stream's records, and
    /// an in-band-parameter-set stream's, may carry none). A caller that must
    /// derive geometry or a codec string from the record itself, such as the
    /// FLV demuxers, calls this to refuse a record that gives it nothing.
    pub fn require_sps(&self) -> Result<()> {
        if self.sps.is_empty() {
            return Err(Error::InvalidValue(InvalidValue {
                field: "numOfSequenceParameterSets",
                value: 0,
                reason: "this container needs an SPS in the avcC record",
            }));
        }
        Ok(())
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
        let _reserved_6 = (b4 >> LENGTH_SIZE_SHIFT) & LENGTH_SIZE_RESERVED_MASK;
        let length_size_minus_one = b4 & LENGTH_SIZE_MASK;
        // Reject value 2 as invalid (§5.3.3 p19).
        if !VALID_LENGTH_SIZES.contains(&length_size_minus_one) {
            return Err(Error::InvalidValue(InvalidValue {
                field: "lengthSizeMinusOne",
                value: length_size_minus_one as u64,
                reason: "must be 0, 1, or 3 (value 2 is invalid per ISO/IEC 14496-15:2017 §5.3.3.1.3)",
            }));
        }

        // reserved(3) + numOfSequenceParameterSets(5)
        let b5 = read_u8(bytes, &mut cursor, "numOfSequenceParameterSets byte")?;
        let _reserved_3 = (b5 >> NUM_SPS_SHIFT) & NUM_SPS_RESERVED_MASK;
        let num_sps = (b5 & NUM_SPS_MASK) as usize;
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

        // High-profile extensions (optional, for profile 100/110/122/244).
        // ISO/IEC 14496-15:2017 §5.3.3.1.2 places this trailer inside an
        // `if (profile_idc IN {...})` guard with no length field of its own —
        // real High-profile encoders are free to omit it entirely once the
        // PPS array ends (issue #952: a genuine DASH-IF `livesim2` capture's
        // `avcC` is exactly 41 bytes, profile 100, SPS+PPS consuming every
        // byte, no trailer at all). So presence requires BOTH the profile
        // gate AND at least one byte actually remaining; when bytes are
        // absent we leave every field `None` rather than inventing a default
        // (absent is not 4:2:0/8-bit, it is unknown).
        let mut chroma_format = None;
        let mut bit_depth_luma_minus8 = None;
        let mut bit_depth_chroma_minus8 = None;
        let mut sps_ext = Vec::new();

        if Self::has_high_profile_ext(profile_indication) && cursor < bytes.len() {
            // byte: reserved(6) + chroma_format(2)
            let b_cf = read_u8(bytes, &mut cursor, "chroma_format byte")?;
            let _reserved_cf = (b_cf >> CHROMA_FORMAT_SHIFT) & CHROMA_FORMAT_RESERVED_MASK;
            let cf = b_cf & CHROMA_FORMAT_MASK;
            chroma_format = Some(cf);

            // byte: reserved(5) + bit_depth_luma_minus8(3)
            let b_bdl = read_u8(bytes, &mut cursor, "bit_depth_luma_minus8 byte")?;
            let _reserved_bdl = (b_bdl >> BIT_DEPTH_SHIFT) & BIT_DEPTH_RESERVED_MASK;
            let bdl = check_bit_depth("bit_depth_luma_minus8", b_bdl & BIT_DEPTH_MASK)?;
            bit_depth_luma_minus8 = Some(bdl);

            // byte: reserved(5) + bit_depth_chroma_minus8(3)
            let b_bdc = read_u8(bytes, &mut cursor, "bit_depth_chroma_minus8 byte")?;
            let _reserved_bdc = (b_bdc >> BIT_DEPTH_SHIFT) & BIT_DEPTH_RESERVED_MASK;
            let bdc = check_bit_depth("bit_depth_chroma_minus8", b_bdc & BIT_DEPTH_MASK)?;
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
        let mut len = RECORD_FIXED_HEADER_LEN;
        // SPS: each has 2-byte length prefix + data
        for sps in &self.sps {
            len += NAL_LENGTH_PREFIX_LEN + sps.0.len();
        }
        len += 1; // numPPS
        for pps in &self.pps {
            len += NAL_LENGTH_PREFIX_LEN + pps.0.len();
        }
        // High-profile ext: emit it iff it was actually present (`chroma_format
        // .is_some()`), not merely because the profile allows it — a
        // High-profile `avcC` that omitted the trailer on the wire (#952) must
        // round-trip back to the same trailer-less bytes, not grow one back
        // with invented zero defaults.
        if self.chroma_format.is_some() {
            len += HIGH_PROFILE_TRAILER_LEN;
            for sps_ext in &self.sps_ext {
                len += NAL_LENGTH_PREFIX_LEN + sps_ext.0.len();
            }
        }
        len
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;

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
        buf[cursor] = LENGTH_SIZE_RESERVED_BITS | (self.length_size_minus_one & LENGTH_SIZE_MASK);
        cursor += 1;

        // reserved(3) + numOfSequenceParameterSets(5): reserved bits = all-ones
        let num_sps = broadcast_core::len::fit_bits(
            self.sps.len() as u64,
            NUM_SPS_BITS,
            "numOfSequenceParameterSets",
        )?;
        buf[cursor] = NUM_SPS_RESERVED_BITS | (num_sps as u8);
        cursor += 1;

        // SPS array
        for sps in &self.sps {
            let len = broadcast_core::len::fit_u16(sps.0.len(), "sps NALU length")?;
            buf[cursor..cursor + NAL_LENGTH_PREFIX_LEN].copy_from_slice(&len.to_be_bytes());
            cursor += NAL_LENGTH_PREFIX_LEN;
            buf[cursor..cursor + sps.0.len()].copy_from_slice(&sps.0);
            cursor += sps.0.len();
        }

        // numOfPictureParameterSets
        let num_pps = broadcast_core::len::fit_u8(self.pps.len(), "numOfPictureParameterSets")?;
        buf[cursor] = num_pps;
        cursor += 1;

        // PPS array
        for pps in &self.pps {
            let len = broadcast_core::len::fit_u16(pps.0.len(), "pps NALU length")?;
            buf[cursor..cursor + NAL_LENGTH_PREFIX_LEN].copy_from_slice(&len.to_be_bytes());
            cursor += NAL_LENGTH_PREFIX_LEN;
            buf[cursor..cursor + pps.0.len()].copy_from_slice(&pps.0);
            cursor += pps.0.len();
        }

        // High-profile ext — mirrors `serialized_len`'s presence gate above:
        // written iff it was actually present on parse, never invented.
        if let Some(cf) = self.chroma_format {
            buf[cursor] = CHROMA_FORMAT_RESERVED_BITS | (cf & CHROMA_FORMAT_MASK);
            cursor += 1;

            let bdl = check_bit_depth(
                "bit_depth_luma_minus8",
                self.bit_depth_luma_minus8.unwrap_or(0),
            )?;
            buf[cursor] = BIT_DEPTH_RESERVED_BITS | bdl;
            cursor += 1;

            let bdc = check_bit_depth(
                "bit_depth_chroma_minus8",
                self.bit_depth_chroma_minus8.unwrap_or(0),
            )?;
            buf[cursor] = BIT_DEPTH_RESERVED_BITS | bdc;
            cursor += 1;

            // numOfSequenceParameterSetExt
            let num_sps_ext =
                broadcast_core::len::fit_u8(self.sps_ext.len(), "numOfSequenceParameterSetExt")?;
            buf[cursor] = num_sps_ext;
            cursor += 1;

            // SPSExt array
            for sps_ext in &self.sps_ext {
                let len = broadcast_core::len::fit_u16(sps_ext.0.len(), "sps_ext NALU length")?;
                buf[cursor..cursor + NAL_LENGTH_PREFIX_LEN].copy_from_slice(&len.to_be_bytes());
                cursor += NAL_LENGTH_PREFIX_LEN;
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
        crate::box_types::box_len(
            crate::box_types::BOX_HEADER_MIN_SIZE + self.config.serialized_len(),
        )
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut cursor = 0usize;
        cursor += crate::box_types::write_box_header(buf, b"avcC", need)?;
        cursor += self.config.serialize_into(&mut buf[cursor..])?;
        Ok(cursor)
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn check_bit_depth(field: &'static str, value: u8) -> Result<u8> {
    if value > MAX_BIT_DEPTH_MINUS8 {
        return Err(Error::InvalidValue(InvalidValue {
            field,
            value: u64::from(value),
            reason: "must be in 0..=4 (ISO/IEC 14496-15:2017 §5.3.3.1.3)",
        }));
    }
    Ok(value)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use broadcast_core::error::InvalidValue;

    use super::*;
    use alloc::vec;
    use broadcast_core::{Parse, Serialize};

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
        let serialized = record.try_to_bytes().unwrap();
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

        let serialized = record.try_to_bytes().unwrap();
        assert_eq!(serialized, body, "high-profile avcC round-trip");
    }

    #[test]
    fn test_avc_config_high_profile_no_trailer_952() {
        // Real DASH-IF `livesim2` `avcC` (`fixtures/scte35/dash/v300/init.mp4`,
        // issue #952): profile 100 (High), 41 bytes total, SPS+PPS consuming
        // every byte — the optional §5.3.3.1.2 high-profile trailer
        // (chroma_format/bit_depth_*/sps_ext) is simply absent, which a
        // conformant High-profile encoder is free to do. Must parse (not
        // error), leave the trailer fields `None` (not an invented default),
        // and round-trip byte-identically (no trailer grown back).
        let body = test_fixtures::hex_bytes(
            "0164001effe100196764001eacd940a02ff9610000030001000003003c8f162d9601000568ebecb22c",
        );
        assert_eq!(body.len(), 41, "fixture avcC body is 41 bytes per #952");

        let record = AVCDecoderConfigurationRecord::parse(&body)
            .expect("a High-profile avcC with no trailer bytes remaining must parse, not error");
        assert_eq!(record.configuration_version, 1);
        assert_eq!(record.profile_indication, 100, "High profile");
        assert_eq!(record.level_indication, 0x1E);
        assert_eq!(record.sps.len(), 1);
        assert_eq!(record.pps.len(), 1);
        assert_eq!(
            record.chroma_format, None,
            "absent trailer must stay None, not default to 4:2:0"
        );
        assert_eq!(record.bit_depth_luma_minus8, None);
        assert_eq!(record.bit_depth_chroma_minus8, None);
        assert!(record.sps_ext.is_empty());

        let serialized = record.try_to_bytes().unwrap();
        assert_eq!(
            serialized, body,
            "a trailer-less High-profile avcC must round-trip without growing one back"
        );
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
            Error::InvalidValue(InvalidValue {
                field: "lengthSizeMinusOne",
                ..
            })
        ));
    }

    #[test]
    fn test_avc_config_field_mutation() {
        let body = make_minimal_avcc_body();
        let mut record = AVCDecoderConfigurationRecord::parse(&body).unwrap();
        record.level_indication = 50;
        let serialized = record.try_to_bytes().unwrap();
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
            0xE0 | 0x01, // numSPS=1 (#738: parse now rejects 0)
        ];
        let sps = vec![0x67, 0xF4, 0x00, 0x1E];
        body.extend_from_slice(&(sps.len() as u16).to_be_bytes());
        body.extend_from_slice(&sps);
        body.push(0x00); // numPPS=0
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
        let mut body = vec![
            0x01,
            0x90,
            0x00,
            0x1E,        // configVersion=1, profile=144, compat=0, level=30
            0xFC | 0x03, // lenSize=3
            0xE0 | 0x01, // numSPS=1 (#738: parse now rejects 0)
        ];
        let sps = vec![0x67, 0x90, 0x00, 0x1E];
        body.extend_from_slice(&(sps.len() as u16).to_be_bytes());
        body.extend_from_slice(&sps);
        body.push(0x00); // numPPS=0
        let record = AVCDecoderConfigurationRecord::parse(&body).unwrap();
        assert_eq!(record.profile_indication, 144);
        assert_eq!(record.chroma_format, None);
        assert_eq!(record.bit_depth_luma_minus8, None);
        assert_eq!(record.bit_depth_chroma_minus8, None);
    }

    #[test]
    fn zero_sps_and_pps_record_round_trips() {
        let body = vec![0x01, 0x42, 0x00, 0x1F, 0xFC | 0x03, 0xE0, 0x00];
        let record = AVCDecoderConfigurationRecord::parse(&body)
            .expect("5.3.3.1.1: a parameter-set-stream record carries no SPS/PPS");
        assert!(record.sps.is_empty());
        assert!(record.pps.is_empty());
        assert_eq!(record.try_to_bytes().unwrap(), body);
    }

    #[test]
    fn zero_sps_with_pps_and_high_profile_trailer_round_trips() {
        let mut body = vec![0x01, 0x64, 0x00, 0x1E, 0xFF, 0xE0, 0x01];
        let pps = [0x68u8, 0xEE, 0x3C];
        body.extend_from_slice(&(pps.len() as u16).to_be_bytes());
        body.extend_from_slice(&pps);
        body.extend_from_slice(&[0xFD, 0xF8, 0xF8, 0x00]);
        let record = AVCDecoderConfigurationRecord::parse(&body).unwrap();
        assert!(record.sps.is_empty());
        assert_eq!(record.pps.len(), 1);
        assert_eq!(record.chroma_format, Some(1));
        assert_eq!(record.try_to_bytes().unwrap(), body);
    }

    #[test]
    fn require_sps_refuses_an_empty_record_only() {
        let empty =
            AVCDecoderConfigurationRecord::parse(&[0x01, 0x42, 0x00, 0x1F, 0xFF, 0xE0, 0x00])
                .unwrap();
        assert!(matches!(
            empty.require_sps().unwrap_err(),
            Error::InvalidValue(InvalidValue {
                field: "numOfSequenceParameterSets",
                ..
            })
        ));
        let with_sps = AVCDecoderConfigurationRecord::parse(&make_minimal_avcc_body()).unwrap();
        with_sps.require_sps().unwrap();
    }

    fn high_profile_body(bit_depth_luma: u8, bit_depth_chroma: u8) -> Vec<u8> {
        let mut body = vec![0x01, 0x64, 0x00, 0x1E, 0xFF, 0xE1];
        let sps = [0x67u8, 0x64, 0x00, 0x1E];
        body.extend_from_slice(&(sps.len() as u16).to_be_bytes());
        body.extend_from_slice(&sps);
        body.push(0x00);
        body.push(0xFD);
        body.push(0xF8 | bit_depth_luma);
        body.push(0xF8 | bit_depth_chroma);
        body.push(0x00);
        body
    }

    #[test]
    fn bit_depth_range_is_enforced_on_parse() {
        for depth in 0..=MAX_BIT_DEPTH_MINUS8 {
            let record = AVCDecoderConfigurationRecord::parse(&high_profile_body(depth, depth))
                .expect("0..=4 is the permitted range");
            assert_eq!(record.bit_depth_luma_minus8, Some(depth));
        }
        for depth in 5..=7u8 {
            for (luma, chroma, field) in [
                (depth, 0, "bit_depth_luma_minus8"),
                (0, depth, "bit_depth_chroma_minus8"),
            ] {
                let err = AVCDecoderConfigurationRecord::parse(&high_profile_body(luma, chroma))
                    .unwrap_err();
                assert!(
                    matches!(
                        err,
                        Error::InvalidValue(InvalidValue { field: f, value, .. })
                            if f == field && value == u64::from(depth)
                    ),
                    "depth {depth} must be rejected for {field}: {err:?}"
                );
            }
        }
    }

    #[test]
    fn bit_depth_range_is_enforced_on_serialize() {
        let mut record = AVCDecoderConfigurationRecord::parse(&high_profile_body(2, 4)).unwrap();
        assert_eq!(record.try_to_bytes().unwrap(), high_profile_body(2, 4));
        record.bit_depth_luma_minus8 = Some(5);
        assert!(matches!(
            record.try_to_bytes().unwrap_err(),
            Error::InvalidValue(InvalidValue {
                field: "bit_depth_luma_minus8",
                value: 5,
                ..
            })
        ));
        record.bit_depth_luma_minus8 = Some(0);
        record.bit_depth_chroma_minus8 = Some(9);
        assert!(matches!(
            record.try_to_bytes().unwrap_err(),
            Error::InvalidValue(InvalidValue {
                field: "bit_depth_chroma_minus8",
                value: 9,
                ..
            })
        ));
    }

    fn base_record(sps: Vec<AvcSps>) -> AVCDecoderConfigurationRecord {
        AVCDecoderConfigurationRecord {
            configuration_version: 1,
            profile_indication: 0x42,
            profile_compatibility: 0,
            level_indication: 0x1F,
            length_size_minus_one: 3,
            sps,
            pps: alloc::vec![],
            chroma_format: None,
            bit_depth_luma_minus8: None,
            bit_depth_chroma_minus8: None,
            sps_ext: alloc::vec![],
        }
    }

    /// An SPS NALU of 65 536 bytes cannot fit the 16-bit `len` prefix
    /// (#1129): on the unfixed code this wrapped to 0 with `Ok` (see
    /// `prefix_test_oversized_sps_wraps_silently_unfixed`, observed pre-fix:
    /// `len_prefix == 0`, no error). Fixed code must reject it instead.
    #[test]
    fn oversized_sps_nalu_length_errors() {
        let rec = base_record(alloc::vec![AvcSps(alloc::vec![0xAAu8; 65536])]);
        let err = rec.try_to_bytes().unwrap_err();
        assert!(
            matches!(
                err,
                Error::FieldOverflow(broadcast_core::len::FieldOverflow {
                    field: "sps NALU length",
                    ..
                })
            ),
            "expected FieldOverflow for sps NALU length, got {err:?}"
        );
    }

    /// The boundary: exactly 65 535 bytes (u16::MAX) still serializes and
    /// round-trips.
    #[test]
    fn max_sps_nalu_length_round_trips() {
        let rec = base_record(alloc::vec![AvcSps(alloc::vec![0xAAu8; 65535])]);
        let bytes = rec.try_to_bytes().unwrap();
        let len_prefix = u16::from_be_bytes([bytes[6], bytes[7]]);
        assert_eq!(len_prefix, 65535);
        let parsed = AVCDecoderConfigurationRecord::parse(&bytes).unwrap();
        assert_eq!(parsed.sps[0].0.len(), 65535);
    }

    /// A 32-entry SPS array cannot fit the 5-bit `numOfSequenceParameterSets`
    /// field (max 31, #1129): the unfixed code masked with `& 0x1F`, so 32
    /// wrapped to a written count of 0 while all 32 entries were still
    /// emitted (misframed output with `Ok`).
    #[test]
    fn too_many_sps_entries_errors() {
        let sps: Vec<AvcSps> = (0..32).map(|_| AvcSps(alloc::vec![0x67])).collect();
        let rec = base_record(sps);
        let err = rec.try_to_bytes().unwrap_err();
        assert!(
            matches!(
                err,
                Error::FieldOverflow(broadcast_core::len::FieldOverflow {
                    field: "numOfSequenceParameterSets",
                    ..
                })
            ),
            "expected FieldOverflow for numOfSequenceParameterSets, got {err:?}"
        );
    }

    /// The boundary: exactly 31 SPS entries (max for a 5-bit field) still
    /// serializes and round-trips.
    #[test]
    fn max_sps_entries_round_trips() {
        let sps: Vec<AvcSps> = (0..31).map(|_| AvcSps(alloc::vec![0x67])).collect();
        let rec = base_record(sps);
        let bytes = rec.try_to_bytes().unwrap();
        let parsed = AVCDecoderConfigurationRecord::parse(&bytes).unwrap();
        assert_eq!(parsed.sps.len(), 31);
    }

    /// `read_nalu_16` reads the `u16` length, returns exactly that many bytes
    /// and leaves the cursor after them; a length the buffer cannot hold is a
    /// `BufferTooShort` carrying the byte count needed.
    #[test]
    fn read_nalu_16_takes_the_prefixed_run() {
        let bytes = [0x00, 0x03, 0x67, 0x42, 0x00, 0xFF];
        let mut cursor = 0usize;
        let nalu = read_nalu_16(&bytes, &mut cursor, "SPS").unwrap();
        assert_eq!(nalu, alloc::vec![0x67, 0x42, 0x00]);
        assert_eq!(cursor, 5);

        let mut cursor = 0usize;
        let err = read_nalu_16(&[0x00, 0x09, 0x67], &mut cursor, "SPS").unwrap_err();
        assert!(
            matches!(err, Error::BufferTooShort(_)),
            "expected BufferTooShort, got {err:?}"
        );

        let mut cursor = 0usize;
        let err = read_nalu_16(&[0x00], &mut cursor, "SPS").unwrap_err();
        assert!(
            matches!(err, Error::BufferTooShort(_)),
            "expected BufferTooShort for a truncated length, got {err:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// TranscodingInfoBox — tran (ISO/IEC 14496-15:2010 C.2.10)
// ---------------------------------------------------------------------------

/// Four-CC of the SVC lightweight transcoding box (`tran`).
pub const TRAN: [u8; 4] = *b"tran";
/// `tran` fixed part: `reserved(4)` + `conversion_idc(2)` + two flags.
const TRAN_FIXED_LEN: usize = 1;
/// `tran` optional entropy-coding part: `profile_level_idc(24)` +
/// `max_bitrate(32)` + `avg_bitrate(32)`.
const TRAN_ENTROPY_LEN: usize = 3 + 2 * core::mem::size_of::<u32>();
/// `conversion_idc` shift inside the first byte.
const TRAN_CONVERSION_SHIFT: u8 = 2;
/// `conversion_idc` mask.
const TRAN_CONVERSION_MASK: u8 = 0b11;
/// `cavlc_info_present_flag` mask.
const TRAN_CAVLC_FLAG: u8 = 0b10;
/// `cabac_info_present_flag` mask.
const TRAN_CABAC_FLAG: u8 = 0b01;
/// Reserved-bit mask inside the first byte.
const TRAN_RESERVED_MASK: u8 = 0b1111_0000;
/// Reserved-bit shift inside the first byte.
const TRAN_RESERVED_SHIFT: u8 = 4;
/// Offset of `max_bitrate` inside one entropy-coding part.
const TRAN_MAX_BITRATE_OFFSET: usize = 3;
/// Offset of `avg_bitrate` inside one entropy-coding part.
const TRAN_AVG_BITRATE_OFFSET: usize = TRAN_MAX_BITRATE_OFFSET + core::mem::size_of::<u32>();

/// One entropy coder's transcoding information
/// (ISO/IEC 14496-15:2010 C.2.10.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct AvcTranscodingInfo {
    profile_level_idc: u32,
    max_bitrate: u32,
    avg_bitrate: u32,
}

impl AvcTranscodingInfo {
    /// Build one from the three fields the clause defines.
    #[must_use]
    pub const fn new(profile_level_idc: u32, max_bitrate: u32, avg_bitrate: u32) -> Self {
        Self {
            profile_level_idc,
            max_bitrate,
            avg_bitrate,
        }
    }

    /// The 24-bit `profile_level_idc`.
    #[must_use]
    pub const fn profile_level_idc(&self) -> u32 {
        self.profile_level_idc
    }

    /// The maximum bitrate in units of 1000 bits/second.
    #[must_use]
    pub const fn max_bitrate(&self) -> u32 {
        self.max_bitrate
    }

    /// The average bitrate in units of 1000 bits/second.
    #[must_use]
    pub const fn avg_bitrate(&self) -> u32 {
        self.avg_bitrate
    }

    fn parse(bytes: &[u8], what: &'static str) -> Result<Self> {
        if bytes.len() < TRAN_ENTROPY_LEN {
            return Err(Error::BufferTooShort(BufferTooShort {
                need: TRAN_ENTROPY_LEN,
                have: bytes.len(),
                what,
            }));
        }
        Ok(Self {
            profile_level_idc: u32::from_be_bytes([
                0,
                bytes[0],
                bytes[1],
                bytes[core::mem::size_of::<u16>()],
            ]),
            max_bitrate: u32::from_be_bytes(
                bytes[TRAN_MAX_BITRATE_OFFSET
                    ..TRAN_MAX_BITRATE_OFFSET + core::mem::size_of::<u32>()]
                    .try_into()
                    .map_err(|_| Error::InvalidInput("tran max_bitrate"))?,
            ),
            avg_bitrate: u32::from_be_bytes(
                bytes[TRAN_AVG_BITRATE_OFFSET
                    ..TRAN_AVG_BITRATE_OFFSET + core::mem::size_of::<u32>()]
                    .try_into()
                    .map_err(|_| Error::InvalidInput("tran avg_bitrate"))?,
            ),
        })
    }

    fn encode(&self, buf: &mut [u8]) -> Result<usize> {
        need_out(buf, TRAN_ENTROPY_LEN)?;
        let idc = self.profile_level_idc.to_be_bytes();
        buf[..TRAN_MAX_BITRATE_OFFSET].copy_from_slice(&idc[1..]);
        buf[TRAN_MAX_BITRATE_OFFSET..TRAN_MAX_BITRATE_OFFSET + core::mem::size_of::<u32>()]
            .copy_from_slice(&self.max_bitrate.to_be_bytes());
        buf[TRAN_AVG_BITRATE_OFFSET..TRAN_AVG_BITRATE_OFFSET + core::mem::size_of::<u32>()]
            .copy_from_slice(&self.avg_bitrate.to_be_bytes());
        Ok(TRAN_ENTROPY_LEN)
    }
}

/// SVC lightweight transcoding box (`tran`) — ISO/IEC 14496-15:2010 C.2.10.
///
/// Carried in an SVC scalable group entry. It states the AVC profile, level
/// and bitrates a tier can be transcoded to, for either entropy coder or both.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct TranscodingInfoBox {
    reserved: u8,
    conversion_idc: u8,
    cavlc: Option<AvcTranscodingInfo>,
    cabac: Option<AvcTranscodingInfo>,
    trailing: Vec<u8>,
}

impl TranscodingInfoBox {
    /// Build one with no entropy-coder information.
    #[must_use]
    pub fn new(conversion_idc: u8) -> Self {
        Self {
            reserved: 0,
            conversion_idc,
            cavlc: None,
            cabac: None,
            trailing: Vec::new(),
        }
    }

    /// Add the CAVLC transcoding information.
    #[must_use]
    pub fn with_cavlc(mut self, info: AvcTranscodingInfo) -> Self {
        self.cavlc = Some(info);
        self
    }

    /// Add the CABAC transcoding information.
    #[must_use]
    pub fn with_cabac(mut self, info: AvcTranscodingInfo) -> Self {
        self.cabac = Some(info);
        self
    }

    /// The four reserved bits, carried verbatim.
    #[must_use]
    pub const fn reserved(&self) -> u8 {
        self.reserved
    }

    /// The `conversion_idc` (0, 1 or 2).
    #[must_use]
    pub const fn conversion_idc(&self) -> u8 {
        self.conversion_idc
    }

    /// The CAVLC transcoding information, when signalled.
    #[must_use]
    pub const fn cavlc(&self) -> Option<AvcTranscodingInfo> {
        self.cavlc
    }

    /// The CABAC transcoding information, when signalled.
    #[must_use]
    pub const fn cabac(&self) -> Option<AvcTranscodingInfo> {
        self.cabac
    }

    /// Bytes after the signalled parts, carried verbatim.
    #[must_use]
    pub fn trailing(&self) -> &[u8] {
        &self.trailing
    }

    /// Parse the payload after the 8-byte box header.
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        if body.len() < TRAN_FIXED_LEN {
            return Err(Error::BufferTooShort(BufferTooShort {
                need: TRAN_FIXED_LEN,
                have: body.len(),
                what: "tran body",
            }));
        }
        let first = body[0];
        let reserved = (first >> TRAN_RESERVED_SHIFT) & (TRAN_RESERVED_MASK >> TRAN_RESERVED_SHIFT);
        let conversion_idc = (first >> TRAN_CONVERSION_SHIFT) & TRAN_CONVERSION_MASK;
        let mut offset = TRAN_FIXED_LEN;
        let cavlc = if first & TRAN_CAVLC_FLAG != 0 {
            let info = AvcTranscodingInfo::parse(
                body.get(offset..).unwrap_or_default(),
                "tran cavlc_info",
            )?;
            offset += TRAN_ENTROPY_LEN;
            Some(info)
        } else {
            None
        };
        let cabac = if first & TRAN_CABAC_FLAG != 0 {
            let info = AvcTranscodingInfo::parse(
                body.get(offset..).unwrap_or_default(),
                "tran cabac_info",
            )?;
            offset += TRAN_ENTROPY_LEN;
            Some(info)
        } else {
            None
        };
        Ok(Self {
            reserved,
            conversion_idc,
            cavlc,
            cabac,
            trailing: body.get(offset..).unwrap_or_default().to_vec(),
        })
    }
}

impl<'a> Parse<'a> for TranscodingInfoBox {
    type Error = Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < crate::box_types::BOX_HEADER_MIN_SIZE {
            return Err(Error::BufferTooShort(BufferTooShort {
                need: crate::box_types::BOX_HEADER_MIN_SIZE,
                have: bytes.len(),
                what: "tran box",
            }));
        }
        if bytes[4..crate::box_types::BOX_HEADER_MIN_SIZE] != TRAN {
            return Err(Error::InvalidInput("expected tran"));
        }
        Self::parse_body(
            bytes
                .get(crate::box_types::BOX_HEADER_MIN_SIZE..)
                .unwrap_or_default(),
        )
    }
}

impl Serialize for TranscodingInfoBox {
    type Error = Error;
    fn serialized_len(&self) -> usize {
        let mut len = crate::box_types::BOX_HEADER_MIN_SIZE + TRAN_FIXED_LEN;
        if self.cavlc.is_some() {
            len += TRAN_ENTROPY_LEN;
        }
        if self.cabac.is_some() {
            len += TRAN_ENTROPY_LEN;
        }
        crate::box_types::box_len(len + self.trailing.len())
    }
    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut c = crate::box_types::write_box_header(buf, &TRAN, need)?;
        let mut first = (self.reserved << TRAN_RESERVED_SHIFT) & TRAN_RESERVED_MASK;
        first |= (self.conversion_idc & TRAN_CONVERSION_MASK) << TRAN_CONVERSION_SHIFT;
        if self.cavlc.is_some() {
            first |= TRAN_CAVLC_FLAG;
        }
        if self.cabac.is_some() {
            first |= TRAN_CABAC_FLAG;
        }
        buf[c] = first;
        c += TRAN_FIXED_LEN;
        if let Some(info) = &self.cavlc {
            c += info.encode(&mut buf[c..])?;
        }
        if let Some(info) = &self.cabac {
            c += info.encode(&mut buf[c..])?;
        }
        buf[c..c + self.trailing.len()].copy_from_slice(&self.trailing);
        c += self.trailing.len();
        Ok(c)
    }
}
