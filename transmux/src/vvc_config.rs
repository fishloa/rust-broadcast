//! VVC / H.266 decoder configuration record (`vvcC`) — ISO/IEC 14496-15:2022 §11.
//!
//! The `VvcDecoderConfigurationRecord` carries the DCI/OPI/VPS/SPS/PPS NAL unit
//! arrays for H.266/VVC video, preceded by a config prefix and an optional
//! `VvcPTLRecord` (profile/tier/level). It is a NAL-array container directly
//! analogous to `hvcC` ([`crate::hevc_config`]), but the VVC NAL header is two
//! bytes and the record is carried in a **FullBox** (a 4-byte version + flags
//! prefix precedes the record body).
//!
//! # Wire layout — ISO/IEC 14496-15:2022 §11.3.2.1, `private/specs-verified/mpeg/transmux/codec/vvc-h266.md`
//!
//! ```text
//! aligned(8) class VvcDecoderConfigurationRecord {
//!   bit(5) reserved = '11111'b;
//!   unsigned int(2) LengthSizeMinusOne;
//!   unsigned int(1) ptl_present_flag;
//!   if (ptl_present_flag) {
//!     unsigned int(9) ols_idx;
//!     unsigned int(3) num_sublayers;
//!     unsigned int(2) constant_frame_rate;
//!     unsigned int(2) chroma_format_idc;
//!     unsigned int(3) bit_depth_minus8;
//!     bit(5) reserved = '11111'b;
//!     VvcPTLRecord(num_sublayers) track_ptl;
//!     unsigned int(16) max_picture_width;
//!     unsigned int(16) max_picture_height;
//!     unsigned int(16) avg_frame_rate;
//!   }
//!   unsigned int(8) num_of_arrays;
//!   for (j=0; j<num_of_arrays; j++) {
//!     unsigned int(1) array_completeness;
//!     bit(2) reserved = 0;
//!     unsigned int(5) NAL_unit_type;
//!     if (NAL_unit_type != DCI && NAL_unit_type != OPI)
//!       unsigned int(16) num_nalus;
//!     for (i=0; i<num_nalus; i++) { unsigned int(16) nalUnitLength; bit(8*len) nalUnit; }
//!   }
//! }
//!
//! aligned(8) class VvcPTLRecord(num_sublayers) {
//!   bit(2) reserved = 0;
//!   unsigned int(6) num_bytes_constraint_info;
//!   unsigned int(7) general_profile_idc;
//!   unsigned int(1) general_tier_flag;
//!   unsigned int(8) general_level_idc;
//!   unsigned int(1) ptl_frame_only_constraint_flag;
//!   unsigned int(1) ptl_multilayer_enabled_flag;
//!   if (num_bytes_constraint_info > 0)
//!     unsigned int(8*num_bytes_constraint_info - 2) general_constraint_info;
//!   else
//!     bit(6) reserved = 0;
//!   if (num_sublayers > 1) {
//!     for (i=num_sublayers-2; i>=0; i--) unsigned int(1) ptl_sublayer_level_present_flag[i];
//!     for (i=num_sublayers-2; i>=0; i--) if (present[i]) unsigned int(8) sublayer_level_idc[i];
//!   }
//!   unsigned int(8) ptl_num_sub_profiles;
//!   for (j=0; j<ptl_num_sub_profiles; j++) unsigned int(32) general_sub_profile_idc[j];
//! }
//! ```
//!
//! # VVC NAL unit types (ITU-T H.266 Table 5)
//!
//! `nal_unit_type` occupies bits `[7:3]` of the second NAL-header byte. The
//! parameter-set / config types are `OPI=12`, `DCI=13`, `VPS=14`, `SPS=15`,
//! `PPS=16`, `PREFIX_APS=17`. (The `private/specs-verified/mpeg/transmux/codec/vvc-h266.md` numbering
//! predates the final H.266 assignments; the values here follow the published
//! ITU-T H.266 Table 5 and are validated against the committed real fixture.)

use crate::box_types::{BOX_HEADER_MIN_SIZE, FULLBOX_EXTRA_SIZE, FULLBOX_FLAGS_OFFSET};
use crate::error::{Error, Result};
use crate::nalu_types::VALID_LENGTH_SIZES;
use alloc::vec::Vec;
use broadcast_common::bits::{BitError, BitWriter};
use broadcast_common::error::{BufferTooShort, InvalidValue};
use broadcast_common::len::{need_in, need_out};
use broadcast_common::wire::u24_at;
use broadcast_common::{Parse, Serialize};
use core::fmt;

// ---------------------------------------------------------------------------
// Field widths, masks and fixed sizes
// ---------------------------------------------------------------------------

/// Bit widths of the fixed-width `u(n)` fields read or written below.
const U2_BITS: usize = 2;
const U3_BITS: usize = 3;
const U5_BITS: usize = 5;
const U6_BITS: usize = 6;
const U7_BITS: usize = 7;
const U9_BITS: usize = 9;
const U16_BITS: usize = u16::BITS as usize;

/// Masks for the fixed-width fields written below.
const U2_MASK: u64 = 0x3;
const U3_MASK: u64 = 0x7;
const U5_MASK: u64 = 0x1F;
const U6_MASK: u64 = 0x3F;
const U7_MASK: u64 = 0x7F;
const U9_MASK: u64 = 0x1FF;

/// `'11111'` reserved pattern preceding `LengthSizeMinusOne`.
const VVC_RESERVED_5_BITS: u64 = 0x1F;
/// Byte width of a `num_nalus` / `nalUnitLength` field.
const NALU_LENGTH_FIELD_LEN: usize = core::mem::size_of::<u16>();
/// Fixed `VvcPTLRecord` bytes before `general_constraint_info`.
const VVC_PTL_FIELDS_LEN: usize = 3;
/// Bytes of `max_picture_width` + `max_picture_height` + `avg_frame_rate`.
const VVC_PICTURE_INFO_LEN: usize = 3 * core::mem::size_of::<u16>();
/// Leading bits of `general_constraint_info` already read as flags.
const CONSTRAINT_INFO_LEADING_FLAGS_BITS: usize = 2;

// ---------------------------------------------------------------------------
// VvcNalUnitType — ITU-T H.266 Table 5 (parameter-set / config NAL types)
// ---------------------------------------------------------------------------

/// The `NAL_unit_type` values that appear in a `vvcC` NAL-unit array —
/// ITU-T H.266 §7.4.2.2 Table 5. Only the parameter-set / config types are
/// enumerated; any other value maps to [`VvcNalUnitType::Other`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum VvcNalUnitType {
    /// Operating point information (`OPI_NUT` = 12).
    Opi,
    /// Decoding capability information (`DCI_NUT` = 13).
    Dci,
    /// Video parameter set (`VPS_NUT` = 14).
    Vps,
    /// Sequence parameter set (`SPS_NUT` = 15).
    Sps,
    /// Picture parameter set (`PPS_NUT` = 16).
    Pps,
    /// Prefix adaptation parameter set (`PREFIX_APS_NUT` = 17).
    PrefixAps,
    /// Any other NAL unit type (carried by its raw 5-bit value).
    Other(u8),
}

impl VvcNalUnitType {
    /// `OPI_NUT` — operating point information.
    pub const OPI: u8 = 12;
    /// `DCI_NUT` — decoding capability information.
    pub const DCI: u8 = 13;
    /// `VPS_NUT` — video parameter set.
    pub const VPS: u8 = 14;
    /// `SPS_NUT` — sequence parameter set.
    pub const SPS: u8 = 15;
    /// `PPS_NUT` — picture parameter set.
    pub const PPS: u8 = 16;
    /// `PREFIX_APS_NUT` — prefix adaptation parameter set.
    pub const PREFIX_APS: u8 = 17;

    /// Decode a 5-bit `nal_unit_type` value into a typed variant.
    pub fn from_u8(v: u8) -> Self {
        match v {
            Self::OPI => VvcNalUnitType::Opi,
            Self::DCI => VvcNalUnitType::Dci,
            Self::VPS => VvcNalUnitType::Vps,
            Self::SPS => VvcNalUnitType::Sps,
            Self::PPS => VvcNalUnitType::Pps,
            Self::PREFIX_APS => VvcNalUnitType::PrefixAps,
            other => VvcNalUnitType::Other(other),
        }
    }

    /// The 5-bit `nal_unit_type` value on the wire.
    pub fn to_u8(self) -> u8 {
        match self {
            VvcNalUnitType::Opi => Self::OPI,
            VvcNalUnitType::Dci => Self::DCI,
            VvcNalUnitType::Vps => Self::VPS,
            VvcNalUnitType::Sps => Self::SPS,
            VvcNalUnitType::Pps => Self::PPS,
            VvcNalUnitType::PrefixAps => Self::PREFIX_APS,
            VvcNalUnitType::Other(v) => v,
        }
    }

    /// Whether an array of this NAL type carries a `num_nalus` field. Per
    /// ISO/IEC 14496-15:2022 §11.3.2.1, DCI and OPI arrays omit `num_nalus`
    /// (they carry exactly one NAL unit).
    pub fn has_num_nalus_field(self) -> bool {
        !matches!(self, VvcNalUnitType::Dci | VvcNalUnitType::Opi)
    }

    /// The ITU-T H.266 Table 5 token for this NAL unit type.
    pub fn name(&self) -> &'static str {
        match self {
            VvcNalUnitType::Opi => "OPI_NUT",
            VvcNalUnitType::Dci => "DCI_NUT",
            VvcNalUnitType::Vps => "VPS_NUT",
            VvcNalUnitType::Sps => "SPS_NUT",
            VvcNalUnitType::Pps => "PPS_NUT",
            VvcNalUnitType::PrefixAps => "PREFIX_APS_NUT",
            VvcNalUnitType::Other(_) => "reserved",
        }
    }
}

broadcast_common::impl_spec_display!(VvcNalUnitType, Other);

// ---------------------------------------------------------------------------
// VvcNalArray — one NAL-unit array in the record
// ---------------------------------------------------------------------------

/// One VVC NAL array in the decoder config record — one iteration of the
/// `num_of_arrays` loop (ISO/IEC 14496-15:2022 §11.3.2.1).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct VvcNalArray {
    /// `array_completeness` — all NALs of this type are in the array.
    pub array_completeness: bool,
    /// `NAL_unit_type` (5 bits, ITU-T H.266 Table 5).
    pub nal_unit_type: u8,
    /// The NAL units in this array (each stored raw: 2-byte header + payload).
    pub nalus: Vec<Vec<u8>>,
}

impl VvcNalArray {
    /// Create a new VVC NAL array.
    pub fn new(array_completeness: bool, nal_unit_type: u8, nalus: Vec<Vec<u8>>) -> Self {
        Self {
            array_completeness,
            nal_unit_type,
            nalus,
        }
    }

    /// The typed NAL unit type ([`VvcNalUnitType`]).
    pub fn kind(&self) -> VvcNalUnitType {
        VvcNalUnitType::from_u8(self.nal_unit_type)
    }

    /// Serialized length of this array (header + optional count + NAL units).
    fn serialized_len(&self) -> usize {
        // 1 byte (completeness|reserved|nal_unit_type) + optional 2 (num_nalus)
        // + per-nalu: 2 (nalUnitLength) + data.
        let count_field = if self.kind().has_num_nalus_field() {
            NALU_LENGTH_FIELD_LEN
        } else {
            0
        };
        1 + count_field
            + self
                .nalus
                .iter()
                .map(|n| NALU_LENGTH_FIELD_LEN + n.len())
                .sum::<usize>()
    }
}

// ---------------------------------------------------------------------------
// VvcPtlRecord — profile/tier/level record (§11.3.2.1 VvcPTLRecord)
// ---------------------------------------------------------------------------

/// `VvcPTLRecord` — profile/tier/level record embedded in the config record
/// when `ptl_present_flag` is set (ISO/IEC 14496-15:2022 §11.3.2.1).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct VvcPtlRecord {
    /// `num_bytes_constraint_info` (6 bits): length in bytes of the constraint
    /// info block; the block occupies `8*num_bytes_constraint_info - 2` bits
    /// (the leading two bits being `ptl_frame_only_constraint_flag` and
    /// `ptl_multilayer_enabled_flag`).
    pub num_bytes_constraint_info: u8,
    /// `general_profile_idc` (7 bits) — ITU-T H.266 Annex A.
    pub general_profile_idc: u8,
    /// `general_tier_flag` (1 bit).
    pub general_tier_flag: bool,
    /// `general_level_idc` (8 bits).
    pub general_level_idc: u8,
    /// `ptl_frame_only_constraint_flag` (1 bit).
    pub ptl_frame_only_constraint_flag: bool,
    /// `ptl_multilayer_enabled_flag` (1 bit).
    pub ptl_multilayer_enabled_flag: bool,
    /// The `general_constraint_info` payload (the `8*num_bytes_constraint_info
    /// - 2` bits after the two leading flags), MSB-first and left-aligned into
    /// `num_bytes_constraint_info` bytes — the low two bits of the last byte
    /// are unused and zero. This is the byte layout the RFC 6381 `CTA-` suffix
    /// prints. Empty when `num_bytes_constraint_info == 0`.
    ///
    /// A byte vector rather than a `u64`: `num_bytes_constraint_info` is a
    /// 6-bit field (≤ 63), so the payload can be up to 502 bits and does not
    /// fit any fixed-width integer.
    pub general_constraint_info: Vec<u8>,
    /// `ptl_sublayer_level_present_flag[i]` for `i = num_sublayers-2 .. 0`
    /// (present only when `num_sublayers > 1`), most-significant first.
    pub sublayer_level_present: Vec<bool>,
    /// `sublayer_level_idc[i]` for each present sublayer, in the same order as
    /// [`sublayer_level_present`](Self::sublayer_level_present).
    pub sublayer_level_idc: Vec<u8>,
    /// `general_sub_profile_idc[j]` (32 bits each), `ptl_num_sub_profiles` of them.
    pub sub_profile_idc: Vec<u32>,
}

// ---------------------------------------------------------------------------
// VvcDecoderConfigurationRecord
// ---------------------------------------------------------------------------

/// `VvcDecoderConfigurationRecord` — H.266/VVC decoder config
/// (ISO/IEC 14496-15:2022 §11.3.2.1).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct VvcDecoderConfigurationRecord {
    /// `LengthSizeMinusOne` ∈ {0,1,3}: NAL length-prefix size − 1.
    pub length_size_minus_one: u8,
    /// `ptl_present_flag` (1 bit).
    pub ptl_present: bool,
    /// `ols_idx` (9 bits) — output layer set index (present when `ptl_present`).
    pub ols_idx: u16,
    /// `num_sublayers` (3 bits) — present when `ptl_present`.
    pub num_sublayers: u8,
    /// `constant_frame_rate` (2 bits) — present when `ptl_present`.
    pub constant_frame_rate: u8,
    /// `chroma_format_idc` (2 bits) — present when `ptl_present`.
    pub chroma_format_idc: u8,
    /// `bit_depth_minus8` (3 bits) — present when `ptl_present`.
    pub bit_depth_minus8: u8,
    /// The `VvcPTLRecord` — present iff `ptl_present`.
    pub ptl: Option<VvcPtlRecord>,
    /// `max_picture_width` (16 bits) — present when `ptl_present`.
    pub max_picture_width: u16,
    /// `max_picture_height` (16 bits) — present when `ptl_present`.
    pub max_picture_height: u16,
    /// `avg_frame_rate` (16 bits) — present when `ptl_present`.
    pub avg_frame_rate: u16,
    /// The NAL unit arrays (DCI/OPI/VPS/SPS/PPS…).
    pub arrays: Vec<VvcNalArray>,
}

impl VvcDecoderConfigurationRecord {
    /// Whether `v` is a valid `LengthSizeMinusOne` (0, 1, or 3).
    pub fn is_valid_length_size(v: u8) -> bool {
        VALID_LENGTH_SIZES.contains(&v)
    }

    /// The first SPS NAL unit (raw bytes, 2-byte header + RBSP), if any.
    pub fn sps(&self) -> Option<&[u8]> {
        self.arrays
            .iter()
            .find(|a| a.kind() == VvcNalUnitType::Sps)
            .and_then(|a| a.nalus.first())
            .map(|n| n.as_slice())
    }
}

impl<'a> Parse<'a> for VvcDecoderConfigurationRecord {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let mut r = VvcBitReader::new(bytes);

        // reserved(5) + LengthSizeMinusOne(2) + ptl_present_flag(1)
        let _reserved0 = r.bits(U5_BITS, "vvcC reserved")?;
        let length_size_minus_one = r.bits(U2_BITS, "LengthSizeMinusOne")? as u8;
        if !Self::is_valid_length_size(length_size_minus_one) {
            return Err(Error::InvalidValue(InvalidValue {
                field: "LengthSizeMinusOne",
                value: length_size_minus_one as u64,
                reason: "must be 0, 1, or 3 (value 2 is invalid per ISO/IEC 14496-15:2022 §11.3.2.1)",
            }));
        }
        let ptl_present = r.flag("ptl_present_flag")?;

        let (
            ols_idx,
            num_sublayers,
            constant_frame_rate,
            chroma_format_idc,
            bit_depth_minus8,
            ptl,
            max_picture_width,
            max_picture_height,
            avg_frame_rate,
        ) = if ptl_present {
            let ols_idx = r.bits(U9_BITS, "ols_idx")? as u16;
            let num_sublayers = r.bits(U3_BITS, "num_sublayers")? as u8;
            let constant_frame_rate = r.bits(U2_BITS, "constant_frame_rate")? as u8;
            let chroma_format_idc = r.bits(U2_BITS, "chroma_format_idc")? as u8;
            let bit_depth_minus8 = r.bits(U3_BITS, "bit_depth_minus8")? as u8;
            let _reserved1 = r.bits(U5_BITS, "vvcC reserved")?;

            let ptl = parse_ptl(&mut r, num_sublayers)?;

            let max_picture_width = r.bits(U16_BITS, "max_picture_width")? as u16;
            let max_picture_height = r.bits(U16_BITS, "max_picture_height")? as u16;
            let avg_frame_rate = r.bits(U16_BITS, "avg_frame_rate")? as u16;
            (
                ols_idx,
                num_sublayers,
                constant_frame_rate,
                chroma_format_idc,
                bit_depth_minus8,
                Some(ptl),
                max_picture_width,
                max_picture_height,
                avg_frame_rate,
            )
        } else {
            (0, 0, 0, 0, 0, None, 0, 0, 0)
        };

        // num_of_arrays(8), then the arrays (byte-aligned here).
        let num_arrays = r.bits(u8::BITS as usize, "num_of_arrays")? as usize;
        let mut arrays = Vec::with_capacity(num_arrays);
        for _ in 0..num_arrays {
            // array_completeness(1) + reserved(2) + NAL_unit_type(5)
            let array_completeness = r.flag("array_completeness")?;
            let _reserved = r.bits(U2_BITS, "vvcC array reserved")?;
            let nal_unit_type = r.bits(U5_BITS, "NAL_unit_type")? as u8;
            let kind = VvcNalUnitType::from_u8(nal_unit_type);

            let num_nalus = if kind.has_num_nalus_field() {
                r.bits(U16_BITS, "num_nalus")? as usize
            } else {
                1
            };

            let mut nalus = Vec::with_capacity(num_nalus);
            for _ in 0..num_nalus {
                let nal_len = r.bits(U16_BITS, "nalUnitLength")? as usize;
                let nalu = r.take_bytes(nal_len, "nalUnit")?;
                nalus.push(nalu);
            }
            arrays.push(VvcNalArray {
                array_completeness,
                nal_unit_type,
                nalus,
            });
        }

        Ok(Self {
            length_size_minus_one,
            ptl_present,
            ols_idx,
            num_sublayers,
            constant_frame_rate,
            chroma_format_idc,
            bit_depth_minus8,
            ptl,
            max_picture_width,
            max_picture_height,
            avg_frame_rate,
            arrays,
        })
    }
}

impl VvcDecoderConfigurationRecord {
    /// Validate the record's internal consistency before serializing.
    ///
    /// `general_constraint_info` is the payload the writer reads
    /// `num_bytes_constraint_info` bytes of; if it is shorter, indexing it would
    /// panic, so a caller-constructed record with a mismatched length is
    /// [`Error::InvalidValue`] instead.
    fn validate(&self) -> Result<()> {
        if let Some(ptl) = &self.ptl {
            let want = usize::from(ptl.num_bytes_constraint_info);
            if ptl.general_constraint_info.len() < want {
                return Err(Error::InvalidValue(InvalidValue {
                    field: "general_constraint_info",
                    value: ptl.general_constraint_info.len() as u64,
                    reason: "shorter than num_bytes_constraint_info",
                }));
            }
        }
        Ok(())
    }
}

impl Serialize for VvcDecoderConfigurationRecord {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        let mut n = 1usize; // reserved+LengthSizeMinusOne+ptl_present byte
        if self.ptl_present {
            // ols_idx(9)+num_sublayers(3)+constant_frame_rate(2)+chroma(2) = 16 bits = 2 bytes
            // bit_depth_minus8(3)+reserved(5) = 1 byte
            n += VVC_PTL_FIELDS_LEN;
            if let Some(ptl) = &self.ptl {
                n += ptl_serialized_len(ptl);
            }
            n += VVC_PICTURE_INFO_LEN;
        }
        n += 1; // num_of_arrays
        n += self
            .arrays
            .iter()
            .map(|a| a.serialized_len())
            .sum::<usize>();
        n
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        self.validate()?;
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut w = VvcBitWriter::new(buf);

        // reserved(5)='11111' + LengthSizeMinusOne(2) + ptl_present_flag(1)
        w.bits(VVC_RESERVED_5_BITS, U5_BITS as u32)?;
        w.bits(self.length_size_minus_one as u64 & U2_MASK, U2_BITS as u32)?;
        w.flag(self.ptl_present)?;

        if self.ptl_present {
            w.bits(self.ols_idx as u64 & U9_MASK, U9_BITS as u32)?;
            w.bits(self.num_sublayers as u64 & U3_MASK, U3_BITS as u32)?;
            w.bits(self.constant_frame_rate as u64 & U2_MASK, U2_BITS as u32)?;
            w.bits(self.chroma_format_idc as u64 & U2_MASK, U2_BITS as u32)?;
            w.bits(self.bit_depth_minus8 as u64 & U3_MASK, U3_BITS as u32)?;
            w.bits(VVC_RESERVED_5_BITS, U5_BITS as u32)?; // reserved '11111'

            if let Some(ptl) = &self.ptl {
                write_ptl(&mut w, ptl)?;
            }

            w.bits(self.max_picture_width as u64, u16::BITS)?;
            w.bits(self.max_picture_height as u64, u16::BITS)?;
            w.bits(self.avg_frame_rate as u64, u16::BITS)?;
        }

        w.bits(self.arrays.len() as u64 & u64::from(u8::MAX), u8::BITS)?;
        for arr in &self.arrays {
            w.flag(arr.array_completeness)?;
            w.bits(0, U2_BITS as u32)?; // reserved
            w.bits(arr.nal_unit_type as u64 & U5_MASK, U5_BITS as u32)?;
            let kind = arr.kind();
            if kind.has_num_nalus_field() {
                w.bits(arr.nalus.len() as u64 & u64::from(u16::MAX), u16::BITS)?;
            }
            for nalu in &arr.nalus {
                w.bits(nalu.len() as u64 & u64::from(u16::MAX), u16::BITS)?;
                w.bytes(nalu)?;
            }
        }

        Ok(w.finish())
    }
}

impl fmt::Display for VvcDecoderConfigurationRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.ptl {
            Some(ptl) => write!(
                f,
                "VVC(profile_idc={}, tier={}, level={}, lenSize={}, arrays={})",
                ptl.general_profile_idc,
                if ptl.general_tier_flag {
                    "high"
                } else {
                    "main"
                },
                ptl.general_level_idc,
                self.length_size_minus_one + 1,
                self.arrays.len(),
            ),
            None => write!(
                f,
                "VVC(no PTL, lenSize={}, arrays={})",
                self.length_size_minus_one + 1,
                self.arrays.len(),
            ),
        }
    }
}

// ---------------------------------------------------------------------------
// VvcConfigurationBox — wraps the record inside a FullBox('vvcC')
// ---------------------------------------------------------------------------

/// The `vvcC` box is a **FullBox**: a 1-byte version + 3-byte flags precede the
/// [`VvcDecoderConfigurationRecord`] (ISO/IEC 14496-15:2022 §11.3.2).
const VVCC_VERSION: u8 = 0;

/// Configuration box for VVC — wraps [`VvcDecoderConfigurationRecord`] in a
/// FullBox `'vvcC'` (ISO/IEC 14496-15:2022 §11.3.2).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct VvcConfigurationBox {
    /// FullBox version (0).
    pub version: u8,
    /// FullBox flags (24 bits; 0).
    pub flags: u32,
    /// The decoder configuration record.
    pub config: VvcDecoderConfigurationRecord,
}

impl VvcConfigurationBox {
    /// Parse a `vvcC` box from its body bytes (after the 8-byte box header) —
    /// the body is the 4-byte FullBox version+flags followed by the record.
    pub fn parse_body(body: &[u8]) -> Result<Self> {
        need_in(body, FULLBOX_EXTRA_SIZE, "vvcC FullBox header")?;
        let version = body[0];
        let flags = u24_at(body, FULLBOX_FLAGS_OFFSET);
        let config = VvcDecoderConfigurationRecord::parse(&body[4..])?;
        Ok(Self {
            version,
            flags,
            config,
        })
    }

    /// Create from a pre-parsed record (version 0, flags 0).
    pub fn new(config: VvcDecoderConfigurationRecord) -> Self {
        Self {
            version: VVCC_VERSION,
            flags: 0,
            config,
        }
    }
}

impl Serialize for VvcConfigurationBox {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        // 8-byte box header + 4-byte FullBox header + record.
        crate::box_types::box_len(
            BOX_HEADER_MIN_SIZE + FULLBOX_EXTRA_SIZE + self.config.serialized_len(),
        )
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        self.config.validate()?;
        let need = self.serialized_len();
        need_out(buf, need)?;
        let mut cursor = 0usize;
        cursor += crate::box_types::write_full_box_header(
            buf,
            b"vvcC",
            need,
            self.version,
            self.flags,
            false,
        )?;
        cursor += self.config.serialize_into(&mut buf[cursor..])?;
        Ok(cursor)
    }
}

// ---------------------------------------------------------------------------
// VvcPTLRecord parse / serialize
// ---------------------------------------------------------------------------

fn parse_ptl(r: &mut VvcBitReader, num_sublayers: u8) -> Result<VvcPtlRecord> {
    // reserved(2) + num_bytes_constraint_info(6)
    let _reserved = r.bits(U2_BITS, "VvcPTLRecord reserved")?;
    let num_bytes_constraint_info = r.bits(U6_BITS, "num_bytes_constraint_info")? as u8;
    // general_profile_idc(7) + general_tier_flag(1)
    let general_profile_idc = r.bits(U7_BITS, "general_profile_idc")? as u8;
    let general_tier_flag = r.flag("general_tier_flag")?;
    let general_level_idc = r.bits(u8::BITS as usize, "general_level_idc")? as u8;
    let ptl_frame_only_constraint_flag = r.flag("ptl_frame_only_constraint_flag")?;
    let ptl_multilayer_enabled_flag = r.flag("ptl_multilayer_enabled_flag")?;

    // general_constraint_info: the two flags above are the leading bits of the
    // constraint-info block, so the remaining payload is 8*n - 2 bits and the
    // payload bytes are MSB-first, left-aligned into n bytes.
    let general_constraint_info = if num_bytes_constraint_info > 0 {
        let bits = (num_bytes_constraint_info as usize) * u8::BITS as usize
            - CONSTRAINT_INFO_LEADING_FLAGS_BITS;
        r.bits_bytes(bits, "general_constraint_info")?
    } else {
        // The two leading flags stand alone; a 6-bit reserved field pads to a byte.
        let _reserved = r.bits(U6_BITS, "VvcPTLRecord reserved")?;
        Vec::new()
    };

    let mut sublayer_level_present = Vec::new();
    let mut sublayer_level_idc = Vec::new();
    if num_sublayers > 1 {
        // ptl_sublayer_level_present_flag[i] for i = num_sublayers-2 .. 0.
        let flag_count = (num_sublayers - 1) as usize;
        for _ in 0..flag_count {
            sublayer_level_present.push(r.flag("ptl_sublayer_level_present_flag[i]")?);
        }
        // ptl_reserved_zero_bit padding to a byte boundary (§11.3.2.1).
        r.align()?;
        for &present in &sublayer_level_present {
            if present {
                sublayer_level_idc.push(r.bits(u8::BITS as usize, "sublayer_level_idc[i]")? as u8);
            }
        }
    }

    let num_sub_profiles = r.bits(u8::BITS as usize, "ptl_num_sub_profiles")? as usize;
    let mut sub_profile_idc = Vec::with_capacity(num_sub_profiles);
    for _ in 0..num_sub_profiles {
        sub_profile_idc.push(r.bits(u32::BITS as usize, "general_sub_profile_idc[j]")? as u32);
    }

    Ok(VvcPtlRecord {
        num_bytes_constraint_info,
        general_profile_idc,
        general_tier_flag,
        general_level_idc,
        ptl_frame_only_constraint_flag,
        ptl_multilayer_enabled_flag,
        general_constraint_info,
        sublayer_level_present,
        sublayer_level_idc,
        sub_profile_idc,
    })
}

fn write_ptl(w: &mut VvcBitWriter, ptl: &VvcPtlRecord) -> Result<()> {
    w.bits(0, U2_BITS as u32)?; // reserved
    w.bits(
        ptl.num_bytes_constraint_info as u64 & U6_MASK,
        U6_BITS as u32,
    )?;
    w.bits(ptl.general_profile_idc as u64 & U7_MASK, U7_BITS as u32)?;
    w.flag(ptl.general_tier_flag)?;
    w.bits(ptl.general_level_idc as u64, u8::BITS)?;
    w.flag(ptl.ptl_frame_only_constraint_flag)?;
    w.flag(ptl.ptl_multilayer_enabled_flag)?;

    if ptl.num_bytes_constraint_info > 0 {
        let bits = (ptl.num_bytes_constraint_info as usize) * u8::BITS as usize
            - CONSTRAINT_INFO_LEADING_FLAGS_BITS;
        w.bits_from_bytes(&ptl.general_constraint_info, bits)?;
    } else {
        w.bits(0, U6_BITS as u32)?; // reserved
    }

    if !ptl.sublayer_level_present.is_empty() {
        for &present in &ptl.sublayer_level_present {
            w.flag(present)?;
        }
        w.align()?; // ptl_reserved_zero_bit
        let mut idc = ptl.sublayer_level_idc.iter();
        for &present in &ptl.sublayer_level_present {
            if present && let Some(&v) = idc.next() {
                w.bits(v as u64, u8::BITS)?;
            }
        }
    }

    w.bits(
        ptl.sub_profile_idc.len() as u64 & u64::from(u8::MAX),
        u8::BITS,
    )?;
    for &sp in &ptl.sub_profile_idc {
        w.bits(sp as u64, u32::BITS)?;
    }
    Ok(())
}

/// Serialized length of a [`VvcPtlRecord`] in bytes. The record is byte-aligned
/// at both ends by construction (§11.3.2.1), so its bit length is a multiple of 8.
fn ptl_serialized_len(ptl: &VvcPtlRecord) -> usize {
    // reserved(2)+num_bytes_constraint_info(6)=1 byte,
    // general_profile_idc(7)+general_tier_flag(1)=1 byte, general_level_idc(8)=1 byte.
    // Then 2 flags + (8*n-2) constraint bits (n>0) OR 2 flags + 6 reserved (n==0):
    //   both cases = n bytes when n>0, else 1 byte.
    let mut bits = VVC_PTL_FIELDS_LEN * u8::BITS as usize;
    if ptl.num_bytes_constraint_info > 0 {
        bits += CONSTRAINT_INFO_LEADING_FLAGS_BITS
            + ((ptl.num_bytes_constraint_info as usize) * u8::BITS as usize
                - CONSTRAINT_INFO_LEADING_FLAGS_BITS);
    } else {
        bits += u8::BITS as usize;
    }
    if !ptl.sublayer_level_present.is_empty() {
        // sublayer flags padded to a byte, then one byte per present sublayer.
        bits += ptl.sublayer_level_present.len().div_ceil(u8::BITS as usize) * u8::BITS as usize;
        bits += ptl.sublayer_level_idc.len() * u8::BITS as usize;
    }
    bits += u8::BITS as usize; // ptl_num_sub_profiles
    bits += ptl.sub_profile_idc.len() * u32::BITS as usize;
    bits / u8::BITS as usize
}

// ---------------------------------------------------------------------------
// Bit reader / writer (MSB-first, over the raw record bytes — no RBSP unescape)
// ---------------------------------------------------------------------------

/// Maps a shared [`BitError`] onto this crate's typed errors.
fn bit_error(what: &'static str, e: BitError) -> Error {
    match e {
        BitError::OutOfBounds { .. } => {
            let mut short: BufferTooShort = e.into();
            short.what = what;
            Error::BufferTooShort(short)
        }
        BitError::TooManyBits { requested } => Error::InvalidValue(InvalidValue {
            field: what,
            value: u64::from(requested),
            reason: "field wider than 64 bits",
        }),
        BitError::ValueTooWide { value, .. } => Error::InvalidValue(InvalidValue {
            field: what,
            value,
            reason: "value does not fit the field width",
        }),
        _ => Error::InvalidValue(InvalidValue {
            field: what,
            value: 0,
            reason: "bit reader error",
        }),
    }
}

/// Bytes needed to hold `n` more bits starting at `bit_pos`.
fn bytes_needed(bit_pos: usize, n: usize) -> usize {
    bit_pos.saturating_add(n).div_ceil(u8::BITS as usize)
}

/// Writes the low `n` bits of `value` after a `fit_bits` range check.
fn write_bits_checked(w: &mut BitWriter<'_>, value: u64, n: u32, what: &'static str) -> Result<()> {
    let value = broadcast_common::len::fit_bits(value, n, what)?;
    w.write_bits(value, n).map_err(|e| bit_error(what, e))
}

/// MSB-first bit reader over the `vvcC` record bytes: the shared
/// [`broadcast_common::bits::BitReader`] with this crate's typed errors.
struct VvcBitReader<'a> {
    data: &'a [u8],
    inner: broadcast_common::bits::BitReader<'a>,
}

impl<'a> VvcBitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            inner: broadcast_common::bits::BitReader::new(data),
        }
    }

    fn bits(&mut self, n: usize, what: &'static str) -> Result<u64> {
        let width = u32::try_from(n).ok().filter(|&n| n <= u64::BITS);
        let Some(width) = width else {
            return Err(Error::InvalidValue(InvalidValue {
                field: what,
                value: n as u64,
                reason: "bit count > 64",
            }));
        };
        let need = bytes_needed(self.inner.bits_read(), n);
        self.inner.read_bits(width).map_err(|_| {
            Error::BufferTooShort(BufferTooShort {
                need,
                have: self.data.len(),
                what,
            })
        })
    }

    fn flag(&mut self, what: &'static str) -> Result<bool> {
        Ok(self.bits(1, what)? != 0)
    }

    /// Read `n` bits (n > 64 allowed) MSB-first, left-aligned into
    /// `ceil(n / 8)` bytes — the low `ceil(n/8)*8 - n` bits are zero. Used for
    /// a field wider than any integer width (`general_constraint_info`, whose
    /// `num_bytes_constraint_info`-derived size can reach 502 bits).
    fn bits_bytes(&mut self, n: usize, what: &'static str) -> Result<Vec<u8>> {
        let mut out = alloc::vec![0u8; n.div_ceil(u8::BITS as usize)];
        for i in 0..n {
            if self.bits(1, what)? != 0 {
                out[i / u8::BITS as usize] |=
                    1 << (u8::BITS as usize - 1 - (i % u8::BITS as usize));
            }
        }
        Ok(out)
    }

    /// Consume padding bits up to the next byte boundary, reading them so a
    /// record that ends mid-byte without its padding is a short buffer.
    fn align(&mut self) -> Result<()> {
        while !self.inner.is_byte_aligned() {
            self.bits(1, "ptl_reserved_zero_bit")?;
        }
        Ok(())
    }

    /// Take `len` whole bytes (the reader is byte-aligned at NAL-unit reads).
    fn take_bytes(&mut self, len: usize, what: &'static str) -> Result<Vec<u8>> {
        debug_assert!(
            self.inner.is_byte_aligned(),
            "take_bytes requires byte alignment"
        );
        let start = self.inner.bits_read() / u8::BITS as usize;
        let end = start.saturating_add(len);
        if end > self.data.len() {
            return Err(Error::BufferTooShort(BufferTooShort {
                need: end,
                have: self.data.len(),
                what,
            }));
        }
        self.inner
            .skip_bits(len * u8::BITS as usize)
            .map_err(|e| bit_error(what, e))?;
        Ok(self.data[start..end].to_vec())
    }
}

/// MSB-first bit writer over a caller-provided buffer: the shared
/// [`BitWriter`] (which clears every bit it writes, so a dirty buffer cannot
/// leave stale bits in a written zero, r04-W45) with this crate's typed errors.
struct VvcBitWriter<'a> {
    inner: BitWriter<'a>,
}

impl<'a> VvcBitWriter<'a> {
    fn new(buf: &'a mut [u8]) -> Self {
        Self {
            inner: BitWriter::new(buf),
        }
    }

    fn bits(&mut self, val: u64, n: u32) -> Result<()> {
        write_bits_checked(&mut self.inner, val, n, "vvcC field")
    }

    fn flag(&mut self, v: bool) -> Result<()> {
        self.bits(u64::from(v), 1)
    }

    /// Write the first `n` bits of an MSB-first, left-aligned byte vector
    /// (`n > 64` allowed) — the inverse of
    /// [`VvcBitReader::bits_bytes`]. The trailing padding bits of the last
    /// byte are ignored, so a vector round-tripped through
    /// `bits_bytes` writes back identically.
    fn bits_from_bytes(&mut self, data: &[u8], n: usize) -> Result<()> {
        for i in 0..n {
            let byte = data
                .get(i / u8::BITS as usize)
                .ok_or(Error::BufferTooShort(BufferTooShort {
                    need: n.div_ceil(u8::BITS as usize),
                    have: data.len(),
                    what: "general_constraint_info",
                }))?;
            self.bits(
                u64::from((byte >> (u8::BITS as usize - 1 - (i % u8::BITS as usize))) & 1),
                1,
            )?;
        }
        Ok(())
    }

    /// Advance to the next byte boundary, writing zero padding bits.
    fn align(&mut self) -> Result<()> {
        self.inner
            .align_to_byte()
            .map_err(|e| bit_error("vvcC padding", e))
    }

    fn bytes(&mut self, data: &[u8]) -> Result<()> {
        debug_assert!(
            self.inner.is_byte_aligned(),
            "bytes() requires byte alignment"
        );
        for &b in data {
            self.bits(u64::from(b), u8::BITS)?;
        }
        Ok(())
    }

    fn finish(&self) -> usize {
        self.inner.bits_written().div_ceil(u8::BITS as usize)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use broadcast_common::error::{BufferTooShort, InvalidValue};

    use super::*;
    use crate::codec_config_ext::VvcRecordCodecInfo;

    #[test]
    fn the_record_bit_reader_keeps_its_cursor_and_typed_errors() {
        let data = [0xA5u8, 0x5A, 0xFF];
        let mut r = VvcBitReader::new(&data);
        assert_eq!(r.bits(4, "x").unwrap(), 0xA);
        assert_eq!(r.bits(0, "x").unwrap(), 0);
        assert!(matches!(
            r.bits(65, "x"),
            Err(Error::InvalidValue(InvalidValue {
                field: "x",
                value: 65,
                ..
            }))
        ));
        assert_eq!(r.bits(12, "x").unwrap(), 0x55A);
        assert!(r.flag("x").unwrap());
        assert_eq!(r.inner.bits_read(), 17);
        r.align().unwrap();
        assert_eq!(r.inner.bits_read(), 24);
        assert_eq!(
            r.bits(1, "tail").unwrap_err(),
            Error::BufferTooShort(BufferTooShort {
                need: 4,
                have: 3,
                what: "tail",
            })
        );
        assert_eq!(
            r.inner.bits_read(),
            24,
            "a failed read does not move the cursor"
        );

        let mut r = VvcBitReader::new(&data);
        assert_eq!(r.bits_bytes(12, "x").unwrap(), [0xA5, 0x50]);
        r.align().unwrap();
        assert_eq!(r.take_bytes(1, "x").unwrap(), [0xFF]);
        assert_eq!(
            r.take_bytes(1, "bytes").unwrap_err(),
            Error::BufferTooShort(BufferTooShort {
                need: 4,
                have: 3,
                what: "bytes",
            })
        );

        let mut r = VvcBitReader::new(&data);
        assert_eq!(
            r.take_bytes(usize::MAX, "bytes").unwrap_err(),
            Error::BufferTooShort(BufferTooShort {
                need: usize::MAX,
                have: 3,
                what: "bytes",
            })
        );
        assert_eq!(r.inner.bits_read(), 0, "a refused take moves no cursor");
    }
    use broadcast_common::{Parse, Serialize};

    // The vvcC record body walked out of the committed real fixture
    // (fixtures/mp4/frag/vvc.frag.mp4): the 4-byte FullBox header + record.
    // This is the byte-exact oracle.
    fn fixture_vvcc_body() -> Vec<u8> {
        let hex = "00000000ff00655f010220800000014000f00000028f00010044\
007900ab02208000008028203c46a00527ffac2136563040827004a1164883521e8\
f56a4bc97a89422c81168412421c43425c8e54330463c50000003001000000300c1\
88900001000c0081000014101e22241f4100";
        test_fixtures::hex_bytes(hex)
    }

    #[test]
    fn test_vvcc_parse_fixture_fields() {
        let body = fixture_vvcc_body();
        let boxed = VvcConfigurationBox::parse_body(&body).unwrap();
        assert_eq!(boxed.version, 0);
        assert_eq!(boxed.flags, 0);
        let r = &boxed.config;
        assert_eq!(r.length_size_minus_one, 3);
        assert!(r.ptl_present);
        assert_eq!(r.num_sublayers, 6);
        assert_eq!(r.constant_frame_rate, 1);
        assert_eq!(r.chroma_format_idc, 1);
        assert_eq!(r.bit_depth_minus8, 2);
        assert_eq!(r.max_picture_width, 320);
        assert_eq!(r.max_picture_height, 240);
        assert_eq!(r.avg_frame_rate, 0);
        assert_eq!(r.arrays.len(), 2);
        assert_eq!(r.arrays[0].kind(), VvcNalUnitType::Sps);
        assert_eq!(r.arrays[1].kind(), VvcNalUnitType::Pps);
        let ptl = r.ptl.as_ref().unwrap();
        assert_eq!(ptl.general_profile_idc, 1);
        assert!(!ptl.general_tier_flag);
        assert_eq!(ptl.general_level_idc, 32);
        assert_eq!(ptl.num_bytes_constraint_info, 1);
    }

    #[test]
    fn test_vvcc_round_trip_fixture() {
        let body = fixture_vvcc_body();
        // Parse the record (skip the 4-byte FullBox header) and re-serialize it.
        let record = VvcDecoderConfigurationRecord::parse(&body[4..]).unwrap();
        let serialized = record.try_to_bytes().unwrap();
        assert_eq!(
            serialized,
            &body[4..],
            "vvcC record round-trip must be byte-identical"
        );
    }

    #[test]
    fn test_vvcc_box_round_trip_fixture() {
        // Whole vvcC box (header + FullBox + record) round-trips byte-exactly.
        let body = fixture_vvcc_body();
        let boxed = VvcConfigurationBox::parse_body(&body).unwrap();
        let full = boxed.try_to_bytes().unwrap();
        // The re-serialized box has the 8-byte box header + FullBox + record;
        // its body (bytes 8..) must equal the parsed body.
        assert_eq!(&full[8..], &body[..]);
        assert_eq!(&full[4..8], b"vvcC");
    }

    #[test]
    fn test_vvcc_field_mutation_changes_bytes() {
        // Not a raw passthrough: mutating a decoded field changes the output.
        let body = fixture_vvcc_body();
        let mut record = VvcDecoderConfigurationRecord::parse(&body[4..]).unwrap();
        let before = record.try_to_bytes().unwrap();
        record.max_picture_width = 640;
        let after = record.try_to_bytes().unwrap();
        assert_ne!(
            before, after,
            "mutating max_picture_width must change bytes"
        );
        let reparsed = VvcDecoderConfigurationRecord::parse(&after).unwrap();
        assert_eq!(reparsed.max_picture_width, 640);
    }

    #[test]
    fn test_vvcc_dimensions_from_sps() {
        let body = fixture_vvcc_body();
        let record = VvcDecoderConfigurationRecord::parse(&body[4..]).unwrap();
        assert_eq!(record.dimensions(), Some((320, 240)));
    }

    /// Build a `profile_tier_level` bitstream (via the real writer) for a PTL
    /// record and hand it back to the real reader.
    fn ptl_round_trip(ptl: &VvcPtlRecord, num_sublayers: u8) -> VvcPtlRecord {
        let len = ptl_serialized_len(ptl);
        let mut buf = alloc::vec![0u8; len];
        {
            let mut w = VvcBitWriter::new(&mut buf);
            write_ptl(&mut w, ptl).unwrap();
            assert_eq!(w.finish(), len, "declared and written length must agree");
        }
        let mut r = VvcBitReader::new(&buf);
        let parsed = parse_ptl(&mut r, num_sublayers).expect("parse written PTL");
        assert_eq!(
            r.inner.bits_read(),
            len * 8,
            "reader must consume the whole record"
        );
        parsed
    }

    fn base_ptl() -> VvcPtlRecord {
        VvcPtlRecord {
            num_bytes_constraint_info: 0,
            general_profile_idc: 1,
            general_tier_flag: false,
            general_level_idc: 32,
            ptl_frame_only_constraint_flag: false,
            ptl_multilayer_enabled_flag: false,
            general_constraint_info: Vec::new(),
            sublayer_level_present: Vec::new(),
            sublayer_level_idc: Vec::new(),
            sub_profile_idc: Vec::new(),
        }
    }

    #[test]
    fn test_ptl_constraint_info_wider_than_8_bytes_round_trips() {
        // `num_bytes_constraint_info` is a 6-bit field (ITU-T H.266 §7.3.3.1 /
        // ISO/IEC 14496-15:2022 §11.3.2.1), so the payload can exceed 64 bits.
        // 12 bytes → 94 payload bits, which previously made the reader fail
        // with `n > 64` and rejected the whole vvcC.
        let ptl = VvcPtlRecord {
            num_bytes_constraint_info: 12,
            // 12-byte payload, left-aligned with the low 2 bits unused.
            general_constraint_info: alloc::vec![
                0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xAA, 0xFC,
            ],
            ..base_ptl()
        };
        let parsed = ptl_round_trip(&ptl, 1);
        assert_eq!(parsed.num_bytes_constraint_info, 12);
        assert_eq!(
            parsed.general_constraint_info, ptl.general_constraint_info,
            "the 94-bit payload must survive parse → serialize byte-exactly"
        );
        // The RFC 6381 CTA- suffix prints the payload bytes verbatim.
        assert_eq!(
            crate::sps::rfc6381_vvc1(1, false, 32, &parsed.general_constraint_info, 12),
            "vvc1.1.L32.CTA-00112233445566778899AAFC"
        );
    }

    #[test]
    fn test_ptl_constraint_info_full_6bit_width_round_trips() {
        // The widest legal block: num_bytes_constraint_info = 63 → 502 bits.
        let mut payload = alloc::vec![0u8; 63];
        payload[0] = 0xAB;
        payload[62] = 0xC0; // low 2 bits unused
        let ptl = VvcPtlRecord {
            num_bytes_constraint_info: 63,
            general_constraint_info: payload,
            general_profile_idc: 5,
            general_level_idc: 100,
            ..base_ptl()
        };
        let parsed = ptl_round_trip(&ptl, 1);
        assert_eq!(parsed.general_constraint_info, ptl.general_constraint_info);
        assert_eq!(ptl_serialized_len(&parsed), ptl_serialized_len(&ptl),);
    }

    #[test]
    fn test_ptl_constraint_info_absent_is_empty() {
        let ptl = VvcPtlRecord {
            num_bytes_constraint_info: 0,
            general_constraint_info: Vec::new(),
            ..base_ptl()
        };
        let parsed = ptl_round_trip(&ptl, 1);
        assert!(parsed.general_constraint_info.is_empty());
    }

    /// A record whose `general_constraint_info` is shorter than
    /// `num_bytes_constraint_info` must be an `Err` on serialize, not a panic
    /// indexing out of bounds (r04-W42 follow-up).
    #[test]
    fn short_general_constraint_info_errors_not_panics() {
        let ptl = VvcPtlRecord {
            num_bytes_constraint_info: 8,
            // Only 2 bytes for an 8-byte field.
            general_constraint_info: alloc::vec![0xAA, 0xBB],
            ..base_ptl()
        };
        let record = VvcDecoderConfigurationRecord {
            length_size_minus_one: 3,
            ptl_present: true,
            ols_idx: 0,
            num_sublayers: 1,
            constant_frame_rate: 0,
            chroma_format_idc: 1,
            bit_depth_minus8: 0,
            ptl: Some(ptl),
            max_picture_width: 640,
            max_picture_height: 360,
            avg_frame_rate: 0,
            arrays: Vec::new(),
        };
        assert!(matches!(
            record.try_to_bytes(),
            Err(Error::InvalidValue(InvalidValue {
                field: "general_constraint_info",
                ..
            }))
        ));
        // And through the box path.
        let boxed = VvcConfigurationBox::new(record);
        assert!(matches!(
            boxed.try_to_bytes(),
            Err(Error::InvalidValue(InvalidValue {
                field: "general_constraint_info",
                ..
            }))
        ));
    }

    /// r04-W45: the `VvcBitWriter` must clear bits it writes as zero rather than
    /// only OR-ing set bits in, so a reused/dirty output buffer cannot leave
    /// garbage in the reserved/`pre_defined` fields. Writing the same PTL into a
    /// 0xFF-filled buffer must equal writing it into a zeroed one.
    #[test]
    fn ptl_written_into_a_dirty_buffer_matches_a_zeroed_one() {
        let ptl = VvcPtlRecord {
            num_bytes_constraint_info: 0,
            general_profile_idc: 1,
            general_level_idc: 32,
            sublayer_level_present: alloc::vec![true, false, true],
            sublayer_level_idc: alloc::vec![10, 20],
            sub_profile_idc: alloc::vec![0x1234_5678],
            ..base_ptl()
        };
        let len = ptl_serialized_len(&ptl);
        let mut clean = alloc::vec![0u8; len];
        {
            let mut w = VvcBitWriter::new(&mut clean);
            write_ptl(&mut w, &ptl).unwrap();
        }
        let mut dirty = alloc::vec![0xFFu8; len];
        {
            let mut w = VvcBitWriter::new(&mut dirty);
            write_ptl(&mut w, &ptl).unwrap();
        }
        assert_eq!(
            dirty, clean,
            "reserved/padding bits must be written, so a dirty buffer matches a zeroed one"
        );
        // And the result still parses back to the same record.
        let mut r = VvcBitReader::new(&dirty);
        let parsed = parse_ptl(&mut r, 4).expect("parse dirty-written PTL");
        assert_eq!(parsed.sub_profile_idc, ptl.sub_profile_idc);
        assert_eq!(parsed.sublayer_level_present, ptl.sublayer_level_present);
    }

    /// A buffer shorter than the record is a typed error, not an index panic.
    #[test]
    fn ptl_written_into_a_short_buffer_is_an_error_not_a_panic() {
        let ptl = base_ptl();
        let len = ptl_serialized_len(&ptl);
        for short in 0..len {
            let mut buf = alloc::vec![0u8; short];
            let mut w = VvcBitWriter::new(&mut buf);
            assert!(
                matches!(write_ptl(&mut w, &ptl), Err(Error::BufferTooShort(..))),
                "{short}-byte buffer for a {len}-byte PTL"
            );
        }
    }

    /// `general_constraint_info` shorter than `num_bytes_constraint_info`
    /// claims is rejected by the writer itself, not only by `validate`.
    #[test]
    fn short_constraint_info_is_an_error_not_a_panic() {
        let ptl = VvcPtlRecord {
            num_bytes_constraint_info: 2,
            general_constraint_info: alloc::vec![0xAA],
            ..base_ptl()
        };
        let mut buf = alloc::vec![0u8; 64];
        let mut w = VvcBitWriter::new(&mut buf);
        assert!(matches!(
            write_ptl(&mut w, &ptl),
            Err(Error::BufferTooShort(BufferTooShort {
                what: "general_constraint_info",
                ..
            }))
        ));
    }
}
