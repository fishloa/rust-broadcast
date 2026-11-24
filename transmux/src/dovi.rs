//! Dolby Vision in ISOBMFF — configuration record and `dvcC`/`dvvC` boxes.
//!
//! Implements the `DOVIDecoderConfigurationRecord`, the profile/compatibility
//! enums and the sample-entry FourCCs of the Dolby Vision profiles/levels
//! specification. The MPEG-2 TS `DOVI_video_stream_descriptor` lives in
//! [`crate::dovi_descriptor`].

use crate::box_types::BOX_TYPE_SIZE;
use crate::error::{Error, Result};
use crate::wire_cursor::u16_at;
use alloc::string::String;
use broadcast_common::error::InvalidValue;
use broadcast_common::len::{need_in, need_out};
use broadcast_common::{Parse, Serialize};

/// FourCC of the Dolby Vision configuration box for `dv_profile <= 7`.
pub const DVCC_FOURCC: [u8; 4] = *b"dvcC";
/// FourCC of the Dolby Vision configuration box for `dv_profile > 7`.
pub const DVVC_FOURCC: [u8; 4] = *b"dvvC";
/// FourCC of the Dolby Vision HEVC sample entry over `hvc1`.
pub const DVH1_FOURCC: [u8; 4] = *b"dvh1";
/// FourCC of the Dolby Vision AVC sample entry over `avc1`.
pub const DVA1_FOURCC: [u8; 4] = *b"dva1";
/// FourCC of the Dolby Vision HEVC sample entry over `hev1`.
pub const DVHE_FOURCC: [u8; 4] = *b"dvhe";

/// Highest `dv_profile` that uses `dvcC` rather than `dvvC` (ISOBMFF spec 3.1).
pub const MAX_DVCC_PROFILE: u8 = 7;
/// Dolby Vision profile 9 is the only AVC-based profile of Table 1; profiles
/// 4, 5, 7 and 8 are HEVC.
pub const AVC_PROFILE: u8 = 9;
pub(crate) const HEVC_PROFILES: [u8; 4] = [4, 5, 7, 8];

/// `DOVIDecoderConfigurationRecord` length in bytes (ISOBMFF spec 3.2).
pub const DOVI_RECORD_LEN: usize = 24;

pub(crate) const LEVEL_BITS: u32 = 6;
pub(crate) const LEVEL_MASK: u16 = (1 << LEVEL_BITS) - 1;
pub(crate) const PROFILE_SHIFT: u32 = 9;
pub(crate) const LEVEL_SHIFT: u32 = 3;
pub(crate) const RPU_FLAG: u16 = 0b100;
pub(crate) const EL_FLAG: u16 = 0b010;
pub(crate) const BL_FLAG: u16 = 0b001;
pub(crate) const CCID_SHIFT: u32 = 4;
pub(crate) const CCID_MASK: u8 = 0x0F;

/// `dv_bl_signal_compatibility_id` (profiles-levels spec, Table 1 and the
/// compatibility-ID list in section 2.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum DoviBlSignalCompatibilityId {
    /// 0: no cross-compatibility; the base layer is Dolby Vision specific.
    None,
    /// 1: HDR10 compatible.
    Hdr10,
    /// 2: SDR compatible.
    Sdr,
    /// 4: HLG compatible.
    Hlg,
    /// 6: Ultra HD Blu-ray HDR compatible.
    UhdBdHdr,
    /// Any other 4-bit value. Build it through [`Self::from_code`]: a defined
    /// code wrapped here serializes unchanged but parses back as its named
    /// variant.
    Reserved(u8),
}

impl DoviBlSignalCompatibilityId {
    /// The 4-bit wire value.
    pub fn code(self) -> u8 {
        match self {
            Self::None => 0,
            Self::Hdr10 => 1,
            Self::Sdr => 2,
            Self::Hlg => 4,
            Self::UhdBdHdr => 6,
            Self::Reserved(v) => v,
        }
    }

    /// Decode a 4-bit wire value.
    pub fn from_code(code: u8) -> Self {
        match code & CCID_MASK {
            0 => Self::None,
            1 => Self::Hdr10,
            2 => Self::Sdr,
            4 => Self::Hlg,
            6 => Self::UhdBdHdr,
            v => Self::Reserved(v),
        }
    }

    /// Spec label.
    pub fn name(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Hdr10 => "HDR10",
            Self::Sdr => "SDR",
            Self::Hlg => "HLG",
            Self::UhdBdHdr => "UHD BD HDR",
            Self::Reserved(_) => "reserved",
        }
    }
}

broadcast_common::impl_spec_display!(DoviBlSignalCompatibilityId, Reserved);

/// How the Dolby Vision stream is announced at the ISOBMFF sample-entry level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum DoviSampleEntryKind {
    /// Base layer decodes as SDR/HDR on its own (ISOBMFF spec 8.1.1): the
    /// plain `hvc1` entry carries the configuration box.
    CompatibleBaseLayer,
    /// Base layer is not SDR/HDR compliant (8.1.2): the `dvh1` entry.
    DolbyVisionEntry,
}

impl DoviSampleEntryKind {
    /// Spec label.
    pub fn name(&self) -> &'static str {
        match self {
            Self::CompatibleBaseLayer => "compatible base layer",
            Self::DolbyVisionEntry => "Dolby Vision sample entry",
        }
    }
}

broadcast_common::impl_spec_display!(DoviSampleEntryKind);

/// `DOVIDecoderConfigurationRecord` (ISOBMFF spec 3.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DoviDecoderConfigurationRecord {
    /// `dv_version_major`.
    pub version_major: u8,
    /// `dv_version_minor`.
    pub version_minor: u8,
    /// `dv_profile` (7 bits).
    pub profile: u8,
    /// `dv_level` (6 bits).
    pub level: u8,
    /// `rpu_present_flag`.
    pub rpu_present: bool,
    /// `el_present_flag`.
    pub el_present: bool,
    /// `bl_present_flag`.
    pub bl_present: bool,
    /// `dv_bl_signal_compatibility_id`.
    pub bl_signal_compatibility_id: DoviBlSignalCompatibilityId,
    /// The 28 + 128 reserved bits of the record, captured on parse and
    /// re-emitted verbatim: byte 0 holds the low nibble that shares a byte with
    /// `dv_bl_signal_compatibility_id`, bytes 1.. the rest. A hand-built record
    /// uses [`DOVI_RESERVED_ZERO`], the spec value.
    pub reserved: [u8; DOVI_RESERVED_LEN],
}

/// Length of [`DoviDecoderConfigurationRecord::reserved`].
pub const DOVI_RESERVED_LEN: usize = DOVI_RECORD_LEN - RESERVED_OFFSET;
/// The reserved bits as the spec defines them.
pub const DOVI_RESERVED_ZERO: [u8; DOVI_RESERVED_LEN] = [0; DOVI_RESERVED_LEN];
pub(crate) const RESERVED_OFFSET: usize = 4;

/// Width of a four-character code in bytes.
/// Byte offset of the `dv_profile`/`dv_level`/flag word in the record.
pub(crate) const CONFIG_WORD_OFFSET: usize = 2;
/// Byte offset of `dv_bl_signal_compatibility_id` in the record.
pub(crate) const CCID_OFFSET: usize = RESERVED_OFFSET;
/// Width of `dv_profile` once the record's word layout is accounted for.
pub(crate) const PROFILE_BITS: u32 = u16::BITS - PROFILE_SHIFT;
/// Bytes of the reserved tail that closes each descriptor variant.
pub(crate) const RESERVED_TAIL_LEN: usize = 1;

pub(crate) fn invalid(field: &'static str, value: u64, reason: &'static str) -> Error {
    Error::InvalidValue(InvalidValue {
        field,
        value,
        reason,
    })
}

pub(crate) fn pack_config_word(r: &DoviDecoderConfigurationRecord) -> u16 {
    (u16::from(r.profile) << PROFILE_SHIFT)
        | ((u16::from(r.level) & LEVEL_MASK) << LEVEL_SHIFT)
        | if r.rpu_present { RPU_FLAG } else { 0 }
        | if r.el_present { EL_FLAG } else { 0 }
        | if r.bl_present { BL_FLAG } else { 0 }
}

pub(crate) fn unpack_config_word(
    version_major: u8,
    version_minor: u8,
    word: u16,
    ccid: u8,
    reserved: [u8; DOVI_RESERVED_LEN],
) -> DoviDecoderConfigurationRecord {
    DoviDecoderConfigurationRecord {
        version_major,
        version_minor,
        profile: (word >> PROFILE_SHIFT) as u8,
        level: ((word >> LEVEL_SHIFT) & LEVEL_MASK) as u8,
        rpu_present: word & RPU_FLAG != 0,
        el_present: word & EL_FLAG != 0,
        bl_present: word & BL_FLAG != 0,
        bl_signal_compatibility_id: DoviBlSignalCompatibilityId::from_code(ccid),
        reserved,
    }
}

impl DoviDecoderConfigurationRecord {
    /// The configuration box FourCC the record belongs in (ISOBMFF spec 3.1).
    pub fn box_fourcc(&self) -> [u8; BOX_TYPE_SIZE] {
        if self.profile <= MAX_DVCC_PROFILE {
            DVCC_FOURCC
        } else {
            DVVC_FOURCC
        }
    }

    /// Whether the base layer decodes to a standards-compliant signal on its
    /// own (`dv_bl_signal_compatibility_id != 0`).
    pub fn is_base_layer_compatible(&self) -> bool {
        self.bl_signal_compatibility_id != DoviBlSignalCompatibilityId::None
    }

    /// Whether this crate can signal the stream: single track, base layer and
    /// RPU present, no enhancement layer (EL carriage is not implemented).
    pub fn is_single_layer(&self) -> bool {
        self.bl_present && self.rpu_present && !self.el_present
    }

    /// HEVC-based profile (Table 1 profiles 4, 5, 7, 8).
    pub fn is_hevc_profile(&self) -> bool {
        HEVC_PROFILES.contains(&self.profile)
    }

    /// AVC-based profile (Table 1 profile 9).
    pub fn is_avc_profile(&self) -> bool {
        self.profile == AVC_PROFILE
    }

    /// Whether an HEVC track carrying this record is written with the
    /// `dvh1` Dolby Vision sample entry (ISOBMFF spec 8.1.2) and therefore
    /// announced as `dvh1.PP.LL` in DASH `codecs`
    /// (`private/specs-verified/dolby/vision/dolby-vision-streams-in-mpeg-dash-v2.0-2018-11.md`
    /// 4.3.2).
    pub fn uses_dolby_hevc_entry(&self) -> bool {
        self.is_single_layer()
            && self.is_hevc_profile()
            && self.sample_entry_kind() == DoviSampleEntryKind::DolbyVisionEntry
    }

    /// Sample entry choice per ISOBMFF spec 8.1.1 / 8.1.2.
    pub fn sample_entry_kind(&self) -> DoviSampleEntryKind {
        if self.is_base_layer_compatible() {
            DoviSampleEntryKind::CompatibleBaseLayer
        } else {
            DoviSampleEntryKind::DolbyVisionEntry
        }
    }

    /// The record found in a `dvcC` / `dvvC` sample-entry child box
    /// (`(fourcc, body)`); the first that parses wins.
    pub fn from_sample_entry_boxes<'a>(
        boxes: impl IntoIterator<Item = (&'a [u8; BOX_TYPE_SIZE], &'a [u8])>,
    ) -> Option<Self> {
        boxes
            .into_iter()
            .filter(|(fourcc, _)| **fourcc == DVCC_FOURCC || **fourcc == DVVC_FOURCC)
            .find_map(|(_, body)| Self::parse(body).ok())
    }

    /// Dolby Vision codec string `dvh1.PP.LL` for an HEVC profile or
    /// `dva1.PP.LL` for the AVC profile (profiles-levels spec Table 1 and the
    /// codec_type table, profile then level, two decimal digits each); `None`
    /// for a profile the verified text does not define.
    pub fn codec_string(&self) -> Option<String> {
        let fourcc = if self.is_hevc_profile() {
            DVH1_FOURCC
        } else if self.is_avc_profile() {
            DVA1_FOURCC
        } else {
            return None;
        };
        Some(alloc::format!(
            "{}.{:02}.{:02}",
            core::str::from_utf8(&fourcc).ok()?,
            self.profile,
            self.level
        ))
    }
}

impl<'a> Parse<'a> for DoviDecoderConfigurationRecord {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        need_in(bytes, DOVI_RECORD_LEN, "DOVIDecoderConfigurationRecord")?;
        if bytes.len() > DOVI_RECORD_LEN {
            return Err(invalid(
                "DOVIDecoderConfigurationRecord",
                bytes.len() as u64,
                "longer than the defined syntax",
            ));
        }
        let word = u16_at(bytes, CONFIG_WORD_OFFSET);
        let mut reserved = [0u8; DOVI_RESERVED_LEN];
        reserved.copy_from_slice(&bytes[RESERVED_OFFSET..]);
        reserved[0] &= CCID_MASK;
        Ok(unpack_config_word(
            bytes[0],
            bytes[1],
            word,
            bytes[CCID_OFFSET] >> CCID_SHIFT,
            reserved,
        ))
    }
}

impl Serialize for DoviDecoderConfigurationRecord {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        DOVI_RECORD_LEN
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        need_out(buf, DOVI_RECORD_LEN)?;
        if self.profile >= 1 << PROFILE_BITS {
            return Err(invalid(
                "dv_profile",
                u64::from(self.profile),
                "does not fit 7 bits",
            ));
        }
        if u16::from(self.level) > LEVEL_MASK {
            return Err(invalid(
                "dv_level",
                u64::from(self.level),
                "does not fit 6 bits",
            ));
        }
        let ccid = self.bl_signal_compatibility_id.code();
        if ccid > CCID_MASK {
            return Err(invalid(
                "dv_bl_signal_compatibility_id",
                u64::from(ccid),
                "does not fit 4 bits",
            ));
        }
        if self.reserved[0] & !CCID_MASK != 0 {
            return Err(invalid(
                "DOVIDecoderConfigurationRecord.reserved",
                u64::from(self.reserved[0]),
                "the first reserved byte holds only 4 bits",
            ));
        }
        buf[0] = self.version_major;
        buf[1] = self.version_minor;
        buf[CONFIG_WORD_OFFSET..RESERVED_OFFSET]
            .copy_from_slice(&pack_config_word(self).to_be_bytes());
        buf[RESERVED_OFFSET..DOVI_RECORD_LEN].copy_from_slice(&self.reserved);
        buf[CCID_OFFSET] |= ccid << CCID_SHIFT;
        Ok(DOVI_RECORD_LEN)
    }
}
