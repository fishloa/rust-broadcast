//! NAL unit byte-slice newtypes and typed arrays for AVC/HEVC decoder config records.
//!
//! Each NAL unit is stored as a `Vec<u8>` of its raw bytes (header + payload).
//! Arrays carry them in wire order and compute their own serialized length.

use crate::error::{Error, Result};
use crate::sps::{
    AvcSpsInfo, HevcSpsInfo, decode_avc_sps, decode_hevc_sps, rfc6381_avc1, rfc6381_hvc1,
};
use alloc::string::String;
use alloc::vec::Vec;
use broadcast_common::Serialize;

// ---------------------------------------------------------------------------
// AVC: SPS, PPS, and SPSExt NAL units (sequenceParameterSetNALUnit)
// ---------------------------------------------------------------------------

/// A single AVC sequence parameter set (SPS) NAL unit — raw bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct AvcSps(pub Vec<u8>);

impl AvcSps {
    /// Decode the SPS fields needed for codec configuration.
    ///
    /// Returns profile, level, dimensions, chroma, bit-depth, and
    /// frame-type information.
    pub fn decode(&self) -> Result<AvcSpsInfo> {
        decode_avc_sps(&self.0)
    }

    /// Build the RFC 6381 codec string (e.g. `"avc1.4D400D"`).
    pub fn rfc6381(&self) -> Result<String> {
        let info = self.decode()?;
        Ok(rfc6381_avc1(
            info.profile_idc,
            info.constraint_flags,
            info.level_idc,
        ))
    }
}

/// A single AVC picture parameter set (PPS) NAL unit — raw bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct AvcPps(pub Vec<u8>);

/// A single AVC sequence parameter set extension NAL unit — raw bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct AvcSpsExt(pub Vec<u8>);

// ---------------------------------------------------------------------------
// HEVC: typed NAL array entry
// ---------------------------------------------------------------------------

/// A single HEVC NAL unit in the decoder config record arrays.
///
/// Equivalent to the per-array loop: `nalUnitLength + nalUnit`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct HevcNalUnit(pub Vec<u8>);

impl HevcNalUnit {
    const LENGTH_FIELD_SIZE: usize = 2;

    /// Build a new HEVC NAL unit from raw bytes.
    pub fn new(data: Vec<u8>) -> Self {
        Self(data)
    }

    /// Decode the SPS fields if this NAL unit is an SPS (NAL unit type 33).
    ///
    /// Returns `None` if the NAL unit type is not SPS.
    pub fn decode_sps(&self) -> Result<Option<HevcSpsInfo>> {
        if self.0.len() < 2 {
            return Ok(None);
        }
        let nal_type = (self.0[0] >> 1) & 0x3F;
        if nal_type != 33 {
            return Ok(None);
        }
        decode_hevc_sps(&self.0).map(Some)
    }

    /// Build the RFC 6381 `hvc1.…` codec string if this is an SPS NAL unit.
    pub fn rfc6381(&self) -> Result<Option<String>> {
        if let Some(info) = self.decode_sps()? {
            Ok(Some(rfc6381_hvc1(&info)))
        } else {
            Ok(None)
        }
    }
}

impl Serialize for HevcNalUnit {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        Self::LENGTH_FIELD_SIZE + self.0.len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        let len = self.0.len();
        buf[..2].copy_from_slice(&(len as u16).to_be_bytes());
        buf[2..need].copy_from_slice(&self.0);
        Ok(need)
    }
}

/// One HEVC NAL array in the decoder config record.
///
/// Corresponds to one iteration of the `numOfArrays` loop in
/// `HEVCDecoderConfigurationRecord` (ISO/IEC 14496-15:2017 §8.3.3).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct HevcNalArray {
    /// `array_completeness` flag — all NALs of this type are in the array.
    pub array_completeness: bool,
    /// `NAL_unit_type` — restricted to VPS/SPS/PPS/prefix SEI/suffix SEI.
    pub nal_unit_type: u8,
    /// The NAL units in this array.
    pub nalus: Vec<HevcNalUnit>,
}

impl HevcNalArray {
    /// Create a new HEVC NAL array.
    pub fn new(array_completeness: bool, nal_unit_type: u8, nalus: Vec<HevcNalUnit>) -> Self {
        Self {
            array_completeness,
            nal_unit_type,
            nalus,
        }
    }
}

impl Serialize for HevcNalArray {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        // header: 1 byte (array_completeness|reserved(1)|nal_unit_type(6)) + 2 bytes numNalus
        3 + self.nalus.iter().map(|n| n.serialized_len()).sum::<usize>()
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
        // First byte: [array_completeness(1), reserved(1)=0, NAL_unit_type(6)]
        let first_byte =
            (if self.array_completeness { 0x80u8 } else { 0u8 }) | (self.nal_unit_type & 0x3F);
        buf[cursor] = first_byte;
        cursor += 1;

        // numNalus as u16
        let count = self.nalus.len();
        buf[cursor..cursor + 2].copy_from_slice(&(count as u16).to_be_bytes());
        cursor += 2;

        for nalu in &self.nalus {
            cursor += nalu.serialize_into(&mut buf[cursor..])?;
        }

        Ok(cursor)
    }
}
