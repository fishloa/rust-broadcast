//! VVC / H.266 decoder configuration record (`vvcC`) — ISO/IEC 14496-15:2022 §11.
//!
//! The `VvcDecoderConfigurationRecord` carries the DCI/OPI/VPS/SPS/PPS NAL unit
//! arrays for H.266/VVC video, preceded by a config prefix and an optional
//! `VvcPTLRecord` (profile/tier/level). It is a NAL-array container directly
//! analogous to `hvcC` ([`crate::hevc_config`]), but the VVC NAL header is two
//! bytes and the record is carried in a **FullBox** (a 4-byte version + flags
//! prefix precedes the record body).
//!
//! # Wire layout — ISO/IEC 14496-15:2022 §11.3.2.1, `transmux/docs/codec/vvc-h266.md`
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
//! `PPS=16`, `PREFIX_APS=17`. (The `transmux/docs/codec/vvc-h266.md` numbering
//! predates the final H.266 assignments; the values here follow the published
//! ITU-T H.266 Table 5 and are validated against the committed real fixture.)

use crate::error::{Error, Result};
use crate::sps::rfc6381_vvc1;
use alloc::string::String;
use alloc::vec::Vec;
use broadcast_common::{Parse, Serialize};
use core::fmt;

/// `LengthSizeMinusOne` must be 0, 1, or 3 (value 2 is invalid).
const VALID_LENGTH_SIZES: [u8; 3] = [0, 1, 3];

// ---------------------------------------------------------------------------
// VvcNalUnitType — ITU-T H.266 Table 5 (parameter-set / config NAL types)
// ---------------------------------------------------------------------------

/// The `NAL_unit_type` values that appear in a `vvcC` NAL-unit array —
/// ITU-T H.266 §7.4.2.2 Table 5. Only the parameter-set / config types are
/// enumerated; any other value maps to [`VvcNalUnitType::Other`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
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
            2
        } else {
            0
        };
        1 + count_field + self.nalus.iter().map(|n| 2 + n.len()).sum::<usize>()
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
    /// - 2` bits after the two leading flags), right-aligned in this `u64`.
    /// Zero when `num_bytes_constraint_info == 0`.
    pub general_constraint_info: u64,
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

    /// Coded dimensions from the first SPS, if one is present and decodable.
    pub fn dimensions(&self) -> Option<(u16, u16)> {
        let sps = self.sps()?;
        let info = crate::sps::decode_vvc_sps(sps).ok()?;
        Some((info.width as u16, info.height as u16))
    }

    /// RFC 6381 codec string (`vvc1.…`) built from the profile/tier/level fields
    /// carried in this record — see [`crate::sps::rfc6381_vvc1`].
    pub fn rfc6381(&self) -> String {
        match &self.ptl {
            Some(ptl) => rfc6381_vvc1(
                ptl.general_profile_idc,
                ptl.general_tier_flag,
                ptl.general_level_idc,
                ptl.general_constraint_info,
                ptl.num_bytes_constraint_info,
            ),
            // No PTL record: emit the bare sample-entry FourCC (RFC 6381 §3.3).
            None => String::from("vvc1"),
        }
    }
}

impl<'a> Parse<'a> for VvcDecoderConfigurationRecord {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let mut r = VvcBitReader::new(bytes);

        // reserved(5) + LengthSizeMinusOne(2) + ptl_present_flag(1)
        let _reserved0 = r.bits(5, "vvcC reserved")?;
        let length_size_minus_one = r.bits(2, "LengthSizeMinusOne")? as u8;
        if !Self::is_valid_length_size(length_size_minus_one) {
            return Err(Error::InvalidValue {
                field: "LengthSizeMinusOne",
                value: length_size_minus_one as u64,
                reason: "must be 0, 1, or 3 (value 2 is invalid per ISO/IEC 14496-15:2022 §11.3.2.1)",
            });
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
            let ols_idx = r.bits(9, "ols_idx")? as u16;
            let num_sublayers = r.bits(3, "num_sublayers")? as u8;
            let constant_frame_rate = r.bits(2, "constant_frame_rate")? as u8;
            let chroma_format_idc = r.bits(2, "chroma_format_idc")? as u8;
            let bit_depth_minus8 = r.bits(3, "bit_depth_minus8")? as u8;
            let _reserved1 = r.bits(5, "vvcC reserved")?;

            let ptl = parse_ptl(&mut r, num_sublayers)?;

            let max_picture_width = r.bits(16, "max_picture_width")? as u16;
            let max_picture_height = r.bits(16, "max_picture_height")? as u16;
            let avg_frame_rate = r.bits(16, "avg_frame_rate")? as u16;
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
        let num_arrays = r.bits(8, "num_of_arrays")? as usize;
        let mut arrays = Vec::with_capacity(num_arrays);
        for _ in 0..num_arrays {
            // array_completeness(1) + reserved(2) + NAL_unit_type(5)
            let array_completeness = r.flag("array_completeness")?;
            let _reserved = r.bits(2, "vvcC array reserved")?;
            let nal_unit_type = r.bits(5, "NAL_unit_type")? as u8;
            let kind = VvcNalUnitType::from_u8(nal_unit_type);

            let num_nalus = if kind.has_num_nalus_field() {
                r.bits(16, "num_nalus")? as usize
            } else {
                1
            };

            let mut nalus = Vec::with_capacity(num_nalus);
            for _ in 0..num_nalus {
                let nal_len = r.bits(16, "nalUnitLength")? as usize;
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

impl Serialize for VvcDecoderConfigurationRecord {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        let mut n = 1usize; // reserved+LengthSizeMinusOne+ptl_present byte
        if self.ptl_present {
            // ols_idx(9)+num_sublayers(3)+constant_frame_rate(2)+chroma(2) = 16 bits = 2 bytes
            // bit_depth_minus8(3)+reserved(5) = 1 byte
            n += 3;
            if let Some(ptl) = &self.ptl {
                n += ptl_serialized_len(ptl);
            }
            n += 6; // max_picture_width(2) + max_picture_height(2) + avg_frame_rate(2)
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
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        let mut w = VvcBitWriter::new(buf);

        // reserved(5)='11111' + LengthSizeMinusOne(2) + ptl_present_flag(1)
        w.bits(0x1F, 5);
        w.bits(self.length_size_minus_one as u64 & 0x3, 2);
        w.flag(self.ptl_present);

        if self.ptl_present {
            w.bits(self.ols_idx as u64 & 0x1FF, 9);
            w.bits(self.num_sublayers as u64 & 0x7, 3);
            w.bits(self.constant_frame_rate as u64 & 0x3, 2);
            w.bits(self.chroma_format_idc as u64 & 0x3, 2);
            w.bits(self.bit_depth_minus8 as u64 & 0x7, 3);
            w.bits(0x1F, 5); // reserved '11111'

            if let Some(ptl) = &self.ptl {
                write_ptl(&mut w, ptl);
            }

            w.bits(self.max_picture_width as u64, 16);
            w.bits(self.max_picture_height as u64, 16);
            w.bits(self.avg_frame_rate as u64, 16);
        }

        w.bits(self.arrays.len() as u64 & 0xFF, 8);
        for arr in &self.arrays {
            w.flag(arr.array_completeness);
            w.bits(0, 2); // reserved
            w.bits(arr.nal_unit_type as u64 & 0x1F, 5);
            let kind = arr.kind();
            if kind.has_num_nalus_field() {
                w.bits(arr.nalus.len() as u64 & 0xFFFF, 16);
            }
            for nalu in &arr.nalus {
                w.bits(nalu.len() as u64 & 0xFFFF, 16);
                w.bytes(nalu);
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
        if body.len() < 4 {
            return Err(Error::BufferTooShort {
                need: 4,
                have: body.len(),
                what: "vvcC FullBox header",
            });
        }
        let version = body[0];
        let flags = u32::from_be_bytes([0, body[1], body[2], body[3]]);
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
        8 + 4 + self.config.serialized_len()
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
        buf[cursor..cursor + 4].copy_from_slice(&(need as u32).to_be_bytes());
        cursor += 4;
        buf[cursor..cursor + 4].copy_from_slice(b"vvcC");
        cursor += 4;
        // FullBox version + flags.
        buf[cursor] = self.version;
        cursor += 1;
        buf[cursor..cursor + 3].copy_from_slice(&self.flags.to_be_bytes()[1..]);
        cursor += 3;
        cursor += self.config.serialize_into(&mut buf[cursor..])?;
        Ok(cursor)
    }
}

// ---------------------------------------------------------------------------
// VvcPTLRecord parse / serialize
// ---------------------------------------------------------------------------

fn parse_ptl(r: &mut VvcBitReader, num_sublayers: u8) -> Result<VvcPtlRecord> {
    // reserved(2) + num_bytes_constraint_info(6)
    let _reserved = r.bits(2, "VvcPTLRecord reserved")?;
    let num_bytes_constraint_info = r.bits(6, "num_bytes_constraint_info")? as u8;
    // general_profile_idc(7) + general_tier_flag(1)
    let general_profile_idc = r.bits(7, "general_profile_idc")? as u8;
    let general_tier_flag = r.flag("general_tier_flag")?;
    let general_level_idc = r.bits(8, "general_level_idc")? as u8;
    let ptl_frame_only_constraint_flag = r.flag("ptl_frame_only_constraint_flag")?;
    let ptl_multilayer_enabled_flag = r.flag("ptl_multilayer_enabled_flag")?;

    // general_constraint_info: the two flags above are the leading bits of the
    // constraint-info block, so the remaining payload is 8*n - 2 bits.
    let general_constraint_info = if num_bytes_constraint_info > 0 {
        let bits = (num_bytes_constraint_info as usize) * 8 - 2;
        r.bits(bits, "general_constraint_info")?
    } else {
        // The two leading flags stand alone; a 6-bit reserved field pads to a byte.
        let _reserved = r.bits(6, "VvcPTLRecord reserved")?;
        0
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
        r.align("ptl_reserved_zero_bit")?;
        for &present in &sublayer_level_present {
            if present {
                sublayer_level_idc.push(r.bits(8, "sublayer_level_idc[i]")? as u8);
            }
        }
    }

    let num_sub_profiles = r.bits(8, "ptl_num_sub_profiles")? as usize;
    let mut sub_profile_idc = Vec::with_capacity(num_sub_profiles);
    for _ in 0..num_sub_profiles {
        sub_profile_idc.push(r.bits(32, "general_sub_profile_idc[j]")? as u32);
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

fn write_ptl(w: &mut VvcBitWriter, ptl: &VvcPtlRecord) {
    w.bits(0, 2); // reserved
    w.bits(ptl.num_bytes_constraint_info as u64 & 0x3F, 6);
    w.bits(ptl.general_profile_idc as u64 & 0x7F, 7);
    w.flag(ptl.general_tier_flag);
    w.bits(ptl.general_level_idc as u64, 8);
    w.flag(ptl.ptl_frame_only_constraint_flag);
    w.flag(ptl.ptl_multilayer_enabled_flag);

    if ptl.num_bytes_constraint_info > 0 {
        let bits = (ptl.num_bytes_constraint_info as usize) * 8 - 2;
        w.bits(ptl.general_constraint_info, bits);
    } else {
        w.bits(0, 6); // reserved
    }

    if !ptl.sublayer_level_present.is_empty() {
        for &present in &ptl.sublayer_level_present {
            w.flag(present);
        }
        w.align(); // ptl_reserved_zero_bit
        let mut idc = ptl.sublayer_level_idc.iter();
        for &present in &ptl.sublayer_level_present {
            if present {
                if let Some(&v) = idc.next() {
                    w.bits(v as u64, 8);
                }
            }
        }
    }

    w.bits(ptl.sub_profile_idc.len() as u64 & 0xFF, 8);
    for &sp in &ptl.sub_profile_idc {
        w.bits(sp as u64, 32);
    }
}

/// Serialized length of a [`VvcPtlRecord`] in bytes. The record is byte-aligned
/// at both ends by construction (§11.3.2.1), so its bit length is a multiple of 8.
fn ptl_serialized_len(ptl: &VvcPtlRecord) -> usize {
    // reserved(2)+num_bytes_constraint_info(6)=1 byte,
    // general_profile_idc(7)+general_tier_flag(1)=1 byte, general_level_idc(8)=1 byte.
    // Then 2 flags + (8*n-2) constraint bits (n>0) OR 2 flags + 6 reserved (n==0):
    //   both cases = n bytes when n>0, else 1 byte.
    let mut bits = 3 * 8;
    if ptl.num_bytes_constraint_info > 0 {
        bits += 2 + ((ptl.num_bytes_constraint_info as usize) * 8 - 2);
    } else {
        bits += 8;
    }
    if !ptl.sublayer_level_present.is_empty() {
        // sublayer flags padded to a byte, then one byte per present sublayer.
        bits += ptl.sublayer_level_present.len().div_ceil(8) * 8;
        bits += ptl.sublayer_level_idc.len() * 8;
    }
    bits += 8; // ptl_num_sub_profiles
    bits += ptl.sub_profile_idc.len() * 32;
    bits / 8
}

// ---------------------------------------------------------------------------
// Bit reader / writer (MSB-first, over the raw record bytes — no RBSP unescape)
// ---------------------------------------------------------------------------

/// MSB-first bit reader over the `vvcC` record bytes.
struct VvcBitReader<'a> {
    data: &'a [u8],
    bit_pos: usize,
}

impl<'a> VvcBitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, bit_pos: 0 }
    }

    fn bits(&mut self, n: usize, what: &'static str) -> Result<u64> {
        if n > 64 || self.bit_pos + n > self.data.len() * 8 {
            return Err(Error::BufferTooShort {
                need: (self.bit_pos + n).div_ceil(8),
                have: self.data.len(),
                what,
            });
        }
        let mut val = 0u64;
        for _ in 0..n {
            let byte_idx = self.bit_pos / 8;
            let bit_in_byte = 7 - (self.bit_pos % 8);
            let bit = ((self.data[byte_idx] >> bit_in_byte) & 1) as u64;
            val = (val << 1) | bit;
            self.bit_pos += 1;
        }
        Ok(val)
    }

    fn flag(&mut self, what: &'static str) -> Result<bool> {
        Ok(self.bits(1, what)? != 0)
    }

    /// Consume padding bits up to the next byte boundary.
    fn align(&mut self, what: &'static str) -> Result<()> {
        while self.bit_pos % 8 != 0 {
            let _ = self.bits(1, what)?;
        }
        Ok(())
    }

    /// Take `len` whole bytes (the reader is byte-aligned at NAL-unit reads).
    fn take_bytes(&mut self, len: usize, what: &'static str) -> Result<Vec<u8>> {
        debug_assert_eq!(self.bit_pos % 8, 0, "take_bytes requires byte alignment");
        let start = self.bit_pos / 8;
        let end = start + len;
        if end > self.data.len() {
            return Err(Error::BufferTooShort {
                need: end,
                have: self.data.len(),
                what,
            });
        }
        self.bit_pos += len * 8;
        Ok(self.data[start..end].to_vec())
    }
}

/// MSB-first bit writer over a caller-provided buffer.
struct VvcBitWriter<'a> {
    buf: &'a mut [u8],
    bit_pos: usize,
}

impl<'a> VvcBitWriter<'a> {
    fn new(buf: &'a mut [u8]) -> Self {
        Self { buf, bit_pos: 0 }
    }

    fn bits(&mut self, val: u64, n: usize) {
        for i in (0..n).rev() {
            let bit = ((val >> i) & 1) as u8;
            let byte_idx = self.bit_pos / 8;
            let bit_in_byte = 7 - (self.bit_pos % 8);
            if bit != 0 {
                self.buf[byte_idx] |= 1 << bit_in_byte;
            }
            self.bit_pos += 1;
        }
    }

    fn flag(&mut self, v: bool) {
        self.bits(v as u64, 1);
    }

    /// Advance to the next byte boundary (bits already zeroed by the caller's buffer).
    fn align(&mut self) {
        while self.bit_pos % 8 != 0 {
            self.bit_pos += 1;
        }
    }

    fn bytes(&mut self, data: &[u8]) {
        debug_assert_eq!(self.bit_pos % 8, 0, "bytes() requires byte alignment");
        let start = self.bit_pos / 8;
        self.buf[start..start + data.len()].copy_from_slice(data);
        self.bit_pos += data.len() * 8;
    }

    fn finish(&self) -> usize {
        self.bit_pos.div_ceil(8)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use broadcast_common::{Parse, Serialize};

    // The vvcC record body walked out of the committed real fixture
    // (fixtures/mp4/frag/vvc.frag.mp4): the 4-byte FullBox header + record.
    // This is the byte-exact oracle.
    fn fixture_vvcc_body() -> Vec<u8> {
        let hex = "00000000ff00655f010220800000014000f00000028f00010044\
007900ab02208000008028203c46a00527ffac2136563040827004a1164883521e8\
f56a4bc97a89422c81168412421c43425c8e54330463c50000003001000000300c1\
88900001000c0081000014101e22241f4100";
        hex_to_bytes(hex)
    }

    fn hex_to_bytes(s: &str) -> Vec<u8> {
        let clean: Vec<u8> = s.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
        clean
            .chunks(2)
            .map(|c| {
                let hi = (c[0] as char).to_digit(16).unwrap() as u8;
                let lo = (c[1] as char).to_digit(16).unwrap() as u8;
                (hi << 4) | lo
            })
            .collect()
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
        let serialized = record.to_bytes();
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
        let full = boxed.to_bytes();
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
        let before = record.to_bytes();
        record.max_picture_width = 640;
        let after = record.to_bytes();
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
}
