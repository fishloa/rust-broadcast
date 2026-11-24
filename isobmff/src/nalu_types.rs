//! NAL unit byte-slice newtypes and typed arrays for AVC/HEVC decoder config records.
//!
//! Each NAL unit is stored as a `Vec<u8>` of its raw bytes (header + payload).
//! Arrays carry them in wire order and compute their own serialized length.

use crate::error::{Error, Result};
use alloc::vec::Vec;
use broadcast_common::Serialize;
use broadcast_common::len::need_out;

/// `lengthSizeMinusOne` of an AVC/HEVC/VVC decoder configuration record must be 0, 1, or 3
/// (value 2 is invalid).
pub const VALID_LENGTH_SIZES: [u8; 3] = [0, 1, 3];

/// H.265 `PREFIX_SEI_NUT` — ITU-T H.265 Table 7-1 (type 39).
pub const HEVC_PREFIX_SEI_NUT: u8 = 39;
/// Mask of `nal_unit_type` in the two-byte HEVC NAL header (H.265 §7.3.1.2).
const HEVC_NAL_TYPE_MASK: u8 = 0x3F;

/// One-byte `array_completeness` flag in an `hvcC` NAL-array header.
const ARRAY_COMPLETENESS_FLAG: u8 = 0x80;
/// Byte size of an `hvcC` NAL-array header: one packed byte plus the 16-bit
/// `numNalus` count.
const HEVC_NAL_ARRAY_HEADER_LEN: usize = 1 + core::mem::size_of::<u16>();

/// H.265 `nal_unit_type` for a sequence parameter set (`SPS_NUT`) — ITU-T H.265
/// Table 7-1 (type 33).
pub const HEVC_SPS_NUT: u8 = 33;
/// H.265 `VPS_NUT` — ITU-T H.265 Table 7-1 (type 32).
pub const HEVC_VPS_NUT: u8 = 32;
/// H.265 `PPS_NUT` — ITU-T H.265 Table 7-1 (type 34).
pub const HEVC_PPS_NUT: u8 = 34;
/// H.265 `SUFFIX_SEI_NUT` — ITU-T H.265 Table 7-1 (type 40).
pub const HEVC_SUFFIX_SEI_NUT: u8 = 40;

// ---------------------------------------------------------------------------
// AVC: SPS, PPS, and SPSExt NAL units (sequenceParameterSetNALUnit)
// ---------------------------------------------------------------------------

/// A single AVC sequence parameter set (SPS) NAL unit — raw bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct AvcSps(pub Vec<u8>);

/// A single AVC picture parameter set (PPS) NAL unit — raw bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct AvcPps(pub Vec<u8>);

/// A single AVC sequence parameter set extension NAL unit — raw bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct AvcSpsExt(pub Vec<u8>);

/// One AVC NAL unit of an `AVCParameterSample` (ISO/IEC 14496-15:2017
/// §5.4.3.2) — raw bytes, NAL header included: a parameter set or a declarative
/// SEI.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct AvcNalUnit(pub Vec<u8>);

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
}

impl Serialize for HevcNalUnit {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        Self::LENGTH_FIELD_SIZE + self.0.len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;
        let len = broadcast_common::len::fit_u16(self.0.len(), "nalUnitLength")?;
        buf[..2].copy_from_slice(&len.to_be_bytes());
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

    /// Whether this array holds VPS, SPS or PPS NAL units.
    pub fn is_parameter_set(&self) -> bool {
        matches!(
            self.nal_unit_type,
            HEVC_VPS_NUT | HEVC_SPS_NUT | HEVC_PPS_NUT
        )
    }

    /// Whether `NAL_unit_type` is one of the five values the `hvcC` arrays are
    /// restricted to: VPS, SPS, PPS, prefix SEI and suffix SEI (ISO/IEC
    /// 14496-15:2017 §8.3.3.1.1, §8.3.3.1.3).
    pub fn has_permitted_nal_unit_type(&self) -> bool {
        self.is_parameter_set()
            || matches!(
                self.nal_unit_type,
                HEVC_PREFIX_SEI_NUT | HEVC_SUFFIX_SEI_NUT
            )
    }
}

impl Serialize for HevcNalArray {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        // header: 1 byte (array_completeness|reserved(1)|nal_unit_type(6)) + 2 bytes numNalus
        HEVC_NAL_ARRAY_HEADER_LEN + self.nalus.iter().map(|n| n.serialized_len()).sum::<usize>()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        need_out(buf, need)?;

        let mut cursor = 0usize;
        // First byte: [array_completeness(1), reserved(1)=0, NAL_unit_type(6)]
        let first_byte = (if self.array_completeness {
            ARRAY_COMPLETENESS_FLAG
        } else {
            0u8
        }) | (self.nal_unit_type & HEVC_NAL_TYPE_MASK);
        buf[cursor] = first_byte;
        cursor += 1;

        // numNalus as u16
        let count = broadcast_common::len::fit_u16(self.nalus.len(), "numNalus")?;
        buf[cursor..cursor + HevcNalUnit::LENGTH_FIELD_SIZE].copy_from_slice(&count.to_be_bytes());
        cursor += HevcNalUnit::LENGTH_FIELD_SIZE;

        for nalu in &self.nalus {
            cursor += nalu.serialize_into(&mut buf[cursor..])?;
        }

        Ok(cursor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A NAL unit of 65 536 bytes cannot fit `HevcNalUnit`'s 16-bit
    /// `nalUnitLength` field (#1129): unfixed, `(len as u16)` silently
    /// wrapped 65536 to 0 while the full-length payload was still copied.
    #[test]
    fn hevc_nal_unit_oversized_length_errors() {
        let nalu = HevcNalUnit(alloc::vec![0xAAu8; 65536]);
        let err = nalu.try_to_bytes().unwrap_err();
        assert!(
            matches!(
                err,
                Error::FieldOverflow(broadcast_common::len::FieldOverflow {
                    field: "nalUnitLength",
                    ..
                })
            ),
            "expected FieldOverflow for nalUnitLength, got {err:?}"
        );
    }

    /// The boundary: exactly 65 535 bytes (u16::MAX) still round-trips.
    #[test]
    fn hevc_nal_unit_max_length_round_trips() {
        let nalu = HevcNalUnit(alloc::vec![0xAAu8; 65535]);
        let bytes = nalu.try_to_bytes().unwrap();
        assert_eq!(u16::from_be_bytes([bytes[0], bytes[1]]), 65535);
    }

    /// 65 536 NAL units in one array cannot fit `HevcNalArray`'s 16-bit
    /// `numNalus` field (#1129): unfixed, `.len() as u16` wrapped.
    #[test]
    fn hevc_nal_array_oversized_count_errors() {
        let arr = HevcNalArray {
            array_completeness: false,
            nal_unit_type: 33,
            nalus: (0..65536).map(|_| HevcNalUnit(alloc::vec![0x42])).collect(),
        };
        let err = arr.try_to_bytes().unwrap_err();
        assert!(
            matches!(
                err,
                Error::FieldOverflow(broadcast_common::len::FieldOverflow {
                    field: "numNalus",
                    ..
                })
            ),
            "expected FieldOverflow for numNalus, got {err:?}"
        );
    }

    /// The boundary: exactly 65 535 NAL units still round-trips.
    #[test]
    fn hevc_nal_array_max_count_round_trips() {
        let arr = HevcNalArray {
            array_completeness: false,
            nal_unit_type: 33,
            nalus: (0..65535).map(|_| HevcNalUnit(alloc::vec![0x42])).collect(),
        };
        let bytes = arr.try_to_bytes().unwrap();
        assert_eq!(u16::from_be_bytes([bytes[1], bytes[2]]), 65535);
    }

    #[test]
    fn hevc_nal_array_serialized_len_sums_the_entries_and_keeps_the_header_bits() {
        let nalus = alloc::vec![
            HevcNalUnit(alloc::vec![0x42, 0x01, 0x0C]),
            HevcNalUnit(alloc::vec![0x42, 0x01, 0x0C, 0xAA, 0xBB]),
        ];
        let arr = HevcNalArray {
            array_completeness: true,
            nal_unit_type: 33,
            nalus,
        };
        assert_eq!(arr.serialized_len(), 3 + 2 + 3 + 2 + 5);
        let mut buf = alloc::vec![0u8; arr.serialized_len()];
        assert_eq!(arr.serialize_into(&mut buf).unwrap(), 15);
        assert_eq!(buf[0], 0xA1, "array_completeness | NAL_unit_type");
        assert_eq!(&buf[1..3], &[0x00, 0x02], "numNalus");

        let incomplete = HevcNalArray {
            array_completeness: false,
            nal_unit_type: 33,
            nalus: alloc::vec![HevcNalUnit(alloc::vec![0x42, 0x01, 0x0C])],
        };
        let mut buf = alloc::vec![0u8; incomplete.serialized_len()];
        incomplete.serialize_into(&mut buf).unwrap();
        assert_eq!(buf[0], 0x21, "no array_completeness bit");
    }
}
