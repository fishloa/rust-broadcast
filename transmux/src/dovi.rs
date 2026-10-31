//! Dolby Vision decoder configuration: the `DOVIDecoderConfigurationRecord`
//! carried by the ISOBMFF `dvcC` / `dvvC` boxes and by the MPEG-2 TS
//! `DOVI_video_stream_descriptor`.
//!
//! - Record and boxes: `private/specs-verified/dolby/vision/dolby-vision-streams-in-isobmff-v2.0-2017-12.md`
//!   chapter 3 (3.2 syntax, 3.3 semantics); box type `dvcC` for
//!   `dv_profile <= 7`, `dvvC` for `dv_profile > 7` (3.1).
//! - Sample entry choice: same document 8.1.1 (SDR/HDR compliant base layer:
//!   `hev1`/`hvc1` carrying the box) and 8.1.2 (non-compliant base layer:
//!   `dvhe`/`dvh1`).
//! - TS descriptor (tag `0xB0`, Table 3-2), DOVI registration descriptor
//!   (Table 3-1) and the `stream_type` rules (3.1.1, 3.1.2):
//!   `private/specs-verified/dolby/vision/dolby-vision-streams-in-mpeg2-ts-v1.2.md`.
//! - Profile IDs, compatibility IDs, level IDs, codec-string form:
//!   `private/specs-verified/dolby/vision/dolby-vision-profiles-levels-v1.3.2-2019-09.md`
//!   Tables 1-3.
//!
//! HLS `SUPPLEMENTAL-CODECS` brands and `VIDEO-RANGE` pairing are not
//! implemented here; `hls-runtime` consumes this module for them.
//!
//! Not covered (no verified source text in the private spec set): the `dvwC`
//! box and profile 10, the RPU payload syntax, the EL configuration boxes
//! (`hvcE`/`avcE`) and dual-PID/dual-track carriage.

use crate::error::{Error, Result};
use alloc::string::String;
use alloc::vec::Vec;
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

/// MPEG-2 TS `DOVI_video_stream_descriptor` tag (TS spec Table 3-2).
pub const DOVI_VIDEO_STREAM_DESCRIPTOR_TAG: u8 = 0xB0;
/// MPEG-2 registration descriptor tag (ISO/IEC 13818-1 Table 2-45).
pub const REGISTRATION_DESCRIPTOR_TAG: u8 = 0x05;
/// `format_identifier` "DOVI" of the registration descriptor (TS spec Table 3-1).
pub const DOVI_FORMAT_IDENTIFIER: [u8; 4] = *b"DOVI";

/// Highest `dv_profile` that uses `dvcC` rather than `dvvC` (ISOBMFF spec 3.1).
pub const MAX_DVCC_PROFILE: u8 = 7;
/// Dolby Vision profile 9 is the only AVC-based profile of Table 1; profiles
/// 4, 5, 7 and 8 are HEVC.
pub const AVC_PROFILE: u8 = 9;
const HEVC_PROFILES: [u8; 4] = [4, 5, 7, 8];

/// `DOVIDecoderConfigurationRecord` length in bytes (ISOBMFF spec 3.2).
pub const DOVI_RECORD_LEN: usize = 24;

const TS_DESCRIPTOR_BASE_LEN: usize = 5;
const TS_DESCRIPTOR_DEPENDENCY_LEN: usize = 2;
const LEVEL_BITS: u32 = 6;
const LEVEL_MASK: u16 = (1 << LEVEL_BITS) - 1;
const PROFILE_SHIFT: u32 = 9;
const LEVEL_SHIFT: u32 = 3;
const RPU_FLAG: u16 = 0b100;
const EL_FLAG: u16 = 0b010;
const BL_FLAG: u16 = 0b001;
const CCID_SHIFT: u32 = 4;
const CCID_MASK: u8 = 0x0F;
const PID_SHIFT: u32 = 3;
const PID_MAX: u16 = 0x1FFF;

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
const RESERVED_OFFSET: usize = 4;

fn invalid(field: &'static str, value: u64, reason: &'static str) -> Error {
    Error::InvalidValue {
        field,
        value,
        reason,
    }
}

fn pack_config_word(r: &DoviDecoderConfigurationRecord) -> u16 {
    (u16::from(r.profile) << PROFILE_SHIFT)
        | ((u16::from(r.level) & LEVEL_MASK) << LEVEL_SHIFT)
        | if r.rpu_present { RPU_FLAG } else { 0 }
        | if r.el_present { EL_FLAG } else { 0 }
        | if r.bl_present { BL_FLAG } else { 0 }
}

fn unpack_config_word(
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
    pub fn box_fourcc(&self) -> [u8; 4] {
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
        boxes: impl IntoIterator<Item = (&'a [u8; 4], &'a [u8])>,
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
        if bytes.len() < DOVI_RECORD_LEN {
            return Err(Error::BufferTooShort {
                need: DOVI_RECORD_LEN,
                have: bytes.len(),
                what: "DOVIDecoderConfigurationRecord",
            });
        }
        if bytes.len() > DOVI_RECORD_LEN {
            return Err(invalid(
                "DOVIDecoderConfigurationRecord",
                bytes.len() as u64,
                "longer than the defined syntax",
            ));
        }
        let word = u16::from_be_bytes([bytes[2], bytes[3]]);
        let mut reserved = [0u8; DOVI_RESERVED_LEN];
        reserved.copy_from_slice(&bytes[RESERVED_OFFSET..]);
        reserved[0] &= CCID_MASK;
        Ok(unpack_config_word(
            bytes[0],
            bytes[1],
            word,
            bytes[4] >> CCID_SHIFT,
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
        if buf.len() < DOVI_RECORD_LEN {
            return Err(Error::OutputBufferTooSmall {
                need: DOVI_RECORD_LEN,
                have: buf.len(),
            });
        }
        if self.profile >= 1 << (16 - PROFILE_SHIFT) {
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
        buf[2..4].copy_from_slice(&pack_config_word(self).to_be_bytes());
        buf[RESERVED_OFFSET..DOVI_RECORD_LEN].copy_from_slice(&self.reserved);
        buf[4] |= ccid << CCID_SHIFT;
        Ok(DOVI_RECORD_LEN)
    }
}

/// `DOVI_video_stream_descriptor` body (TS spec Table 3-2), without the
/// leading `descriptor_tag` / `descriptor_length`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DoviVideoStreamDescriptor {
    /// The configuration carried by the descriptor.
    pub record: DoviDecoderConfigurationRecord,
    /// `dependency_pid`, present exactly when `bl_present_flag` is 0.
    pub dependency_pid: Option<u16>,
    /// The 3 reserved bits after `dependency_pid` (0 when absent), captured
    /// on parse and re-emitted verbatim.
    pub dependency_reserved: u8,
    /// The 4 reserved bits closing the descriptor, captured on parse and
    /// re-emitted verbatim.
    pub reserved: u8,
    /// Bytes after the defined syntax within `descriptor_length`, preserved
    /// verbatim (TS spec Table 3-2 `descriptor_length` counts every byte
    /// following the length field).
    pub extension: Vec<u8>,
}

impl<'a> Parse<'a> for DoviVideoStreamDescriptor {
    type Error = Error;

    fn parse(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < TS_DESCRIPTOR_BASE_LEN {
            return Err(Error::BufferTooShort {
                need: TS_DESCRIPTOR_BASE_LEN,
                have: bytes.len(),
                what: "DOVI_video_stream_descriptor",
            });
        }
        let word = u16::from_be_bytes([bytes[2], bytes[3]]);
        let bl_present = word & BL_FLAG != 0;
        let mut dependency_reserved = 0u8;
        let (dependency_pid, tail) = if bl_present {
            (None, 4)
        } else {
            if bytes.len() < TS_DESCRIPTOR_BASE_LEN + TS_DESCRIPTOR_DEPENDENCY_LEN {
                return Err(Error::BufferTooShort {
                    need: TS_DESCRIPTOR_BASE_LEN + TS_DESCRIPTOR_DEPENDENCY_LEN,
                    have: bytes.len(),
                    what: "DOVI_video_stream_descriptor dependency_pid",
                });
            }
            let raw = u16::from_be_bytes([bytes[4], bytes[5]]);
            dependency_reserved = (raw & ((1 << PID_SHIFT) - 1)) as u8;
            (Some(raw >> PID_SHIFT), 6)
        };
        if bytes.len() < tail + 1 {
            return Err(Error::BufferTooShort {
                need: tail + 1,
                have: bytes.len(),
                what: "DOVI_video_stream_descriptor",
            });
        }
        Ok(Self {
            record: unpack_config_word(
                bytes[0],
                bytes[1],
                word,
                bytes[tail] >> CCID_SHIFT,
                DOVI_RESERVED_ZERO,
            ),
            dependency_pid,
            dependency_reserved,
            reserved: bytes[tail] & CCID_MASK,
            extension: bytes[tail + 1..].to_vec(),
        })
    }
}

impl Serialize for DoviVideoStreamDescriptor {
    type Error = Error;

    fn serialized_len(&self) -> usize {
        TS_DESCRIPTOR_BASE_LEN
            + if self.record.bl_present {
                0
            } else {
                TS_DESCRIPTOR_DEPENDENCY_LEN
            }
            + self.extension.len()
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let need = self.serialized_len();
        if buf.len() < need {
            return Err(Error::OutputBufferTooSmall {
                need,
                have: buf.len(),
            });
        }
        let ccid = self.record.bl_signal_compatibility_id.code();
        if ccid > CCID_MASK || self.record.level > LEVEL_MASK as u8 || self.record.profile >= 1 << 7
        {
            return Err(invalid(
                "DOVI_video_stream_descriptor",
                u64::from(ccid),
                "field does not fit its bit width",
            ));
        }
        buf[0] = self.record.version_major;
        buf[1] = self.record.version_minor;
        buf[2..4].copy_from_slice(&pack_config_word(&self.record).to_be_bytes());
        let tail = match (self.record.bl_present, self.dependency_pid) {
            (true, None) => 4,
            (false, Some(pid)) if pid <= PID_MAX => {
                if self.dependency_reserved >= 1 << PID_SHIFT {
                    return Err(invalid(
                        "dependency_reserved",
                        u64::from(self.dependency_reserved),
                        "does not fit 3 bits",
                    ));
                }
                buf[4..6].copy_from_slice(
                    &((pid << PID_SHIFT) | u16::from(self.dependency_reserved)).to_be_bytes(),
                );
                6
            }
            _ => {
                return Err(invalid(
                    "dependency_pid",
                    self.dependency_pid.map_or(0, u64::from),
                    "present exactly when bl_present_flag is 0, and 13 bits",
                ));
            }
        };
        if self.reserved > CCID_MASK {
            return Err(invalid(
                "DOVI_video_stream_descriptor.reserved",
                u64::from(self.reserved),
                "does not fit 4 bits",
            ));
        }
        buf[tail] = (ccid << CCID_SHIFT) | self.reserved;
        buf[tail + 1..need].copy_from_slice(&self.extension);
        Ok(need)
    }
}

/// Walk a PMT ES_info loop for the `DOVI_video_stream_descriptor`.
pub fn find_video_stream_descriptor(es_info: &[u8]) -> Option<DoviVideoStreamDescriptor> {
    descriptor_bodies(es_info)
        .find(|(tag, _)| *tag == DOVI_VIDEO_STREAM_DESCRIPTOR_TAG)
        .and_then(|(_, body)| DoviVideoStreamDescriptor::parse(body).ok())
}

/// Whether an ES_info loop carries the DOVI registration descriptor.
pub fn has_registration_descriptor(es_info: &[u8]) -> bool {
    descriptor_bodies(es_info).any(|(tag, body)| {
        tag == REGISTRATION_DESCRIPTOR_TAG && body.starts_with(&DOVI_FORMAT_IDENTIFIER)
    })
}

fn descriptor_bodies(es_info: &[u8]) -> impl Iterator<Item = (u8, &[u8])> {
    let mut off = 0usize;
    core::iter::from_fn(move || {
        if off + 2 > es_info.len() {
            return None;
        }
        let tag = es_info[off];
        let len = usize::from(es_info[off + 1]);
        let body = es_info.get(off + 2..off + 2 + len)?;
        off += 2 + len;
        Some((tag, body))
    })
}

/// Build the ES_info bytes for a descriptor (tag + length + body).
pub fn video_stream_descriptor_bytes(d: &DoviVideoStreamDescriptor) -> Result<Vec<u8>> {
    let body = d.try_to_bytes()?;
    let mut out = Vec::with_capacity(2 + body.len());
    out.push(DOVI_VIDEO_STREAM_DESCRIPTOR_TAG);
    out.push(
        broadcast_common::len::fit_u8(body.len(), "descriptor_length").map_err(|_| {
            invalid(
                "descriptor_length",
                body.len() as u64,
                "does not fit 8 bits",
            )
        })?,
    );
    out.extend_from_slice(&body);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const P81_RECORD: [u8; 24] = [
        0x01, 0x00, 0x10, 0x1D, 0x10, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    ];

    #[test]
    fn record_round_trips() {
        let r = DoviDecoderConfigurationRecord::parse(&P81_RECORD).unwrap();
        assert_eq!(r.profile, 8);
        assert_eq!(r.level, 3);
        assert_eq!(r.try_to_bytes().unwrap(), P81_RECORD);
        assert_eq!(r.box_fourcc(), DVVC_FOURCC);
        assert_eq!(r.codec_string().as_deref(), Some("dvh1.08.03"));
    }

    #[test]
    fn reserved_bits_are_preserved_byte_exact() {
        let mut bytes = P81_RECORD;
        bytes[4] |= 0x05;
        bytes[6] = 0xA5;
        bytes[23] = 0xFF;
        let r = DoviDecoderConfigurationRecord::parse(&bytes).unwrap();
        assert_eq!(
            r.bl_signal_compatibility_id,
            DoviBlSignalCompatibilityId::Hdr10
        );
        assert_eq!(r.try_to_bytes().unwrap(), bytes);
        assert_eq!(r.reserved[0], 0x05);
    }

    #[test]
    fn lengths_are_rejected() {
        assert!(DoviDecoderConfigurationRecord::parse(&P81_RECORD[..23]).is_err());
        let mut long = P81_RECORD.to_vec();
        long.push(0);
        assert!(DoviDecoderConfigurationRecord::parse(&long).is_err());
    }

    #[test]
    fn descriptor_reserved_bits_are_preserved_byte_exact() {
        let with_pid = [0x01, 0x00, 0x0E, 0x1A, 0x12, 0x37, 0x4B];
        let d = DoviVideoStreamDescriptor::parse(&with_pid).unwrap();
        assert_eq!(d.dependency_pid, Some(0x246));
        assert_eq!(d.dependency_reserved, 0x7);
        assert_eq!(d.reserved, 0xB);
        assert_eq!(d.try_to_bytes().unwrap(), with_pid);
        let single = [0x01, 0x00, 0x10, 0x1D, 0x1F];
        let d = DoviVideoStreamDescriptor::parse(&single).unwrap();
        assert_eq!(d.reserved, 0xF);
        assert_eq!(
            d.record.bl_signal_compatibility_id,
            DoviBlSignalCompatibilityId::Hdr10
        );
        assert_eq!(d.try_to_bytes().unwrap(), single);
    }

    #[test]
    fn descriptor_with_dependency_pid_round_trips() {
        let d = DoviVideoStreamDescriptor {
            record: DoviDecoderConfigurationRecord {
                version_major: 1,
                version_minor: 0,
                profile: 7,
                level: 6,
                rpu_present: true,
                el_present: true,
                bl_present: false,
                bl_signal_compatibility_id: DoviBlSignalCompatibilityId::UhdBdHdr,
                reserved: DOVI_RESERVED_ZERO,
            },
            dependency_pid: Some(0x1234),
            dependency_reserved: 0,
            reserved: 0,
            extension: Vec::new(),
        };
        let bytes = d.try_to_bytes().unwrap();
        assert_eq!(bytes.len(), 7);
        assert_eq!(DoviVideoStreamDescriptor::parse(&bytes).unwrap(), d);
    }

    #[test]
    fn profile_84_is_representable() {
        let r = DoviDecoderConfigurationRecord {
            version_major: 1,
            version_minor: 0,
            profile: 8,
            level: 7,
            rpu_present: true,
            el_present: false,
            bl_present: true,
            bl_signal_compatibility_id: DoviBlSignalCompatibilityId::Hlg,
            reserved: DOVI_RESERVED_ZERO,
        };
        let back = DoviDecoderConfigurationRecord::parse(&r.try_to_bytes().unwrap()).unwrap();
        assert_eq!(back, r);
        assert_eq!(back.codec_string().as_deref(), Some("dvh1.08.07"));
    }

    #[test]
    fn compatibility_id_labels() {
        assert_eq!(DoviBlSignalCompatibilityId::from_code(4).name(), "HLG");
        assert_eq!(DoviBlSignalCompatibilityId::Reserved(9).code(), 9);
    }

    #[test]
    fn profile_classification_is_table_1_only() {
        let mut r = DoviDecoderConfigurationRecord::parse(&P81_RECORD).unwrap();
        for (profile, hevc, avc) in [
            (4, true, false),
            (5, true, false),
            (7, true, false),
            (8, true, false),
            (9, false, true),
            (6, false, false),
            (10, false, false),
            (20, false, false),
        ] {
            r.profile = profile;
            assert_eq!(r.is_hevc_profile(), hevc, "profile {profile}");
            assert_eq!(r.is_avc_profile(), avc, "profile {profile}");
        }
    }

    #[test]
    fn codec_string_follows_profile_family() {
        let mut r = DoviDecoderConfigurationRecord::parse(&P81_RECORD).unwrap();
        r.profile = 5;
        r.level = 3;
        assert_eq!(r.codec_string().as_deref(), Some("dvh1.05.03"));
        r.profile = 9;
        r.level = 4;
        assert_eq!(r.codec_string().as_deref(), Some("dva1.09.04"));
        r.profile = 10;
        assert_eq!(r.codec_string(), None);
    }

    #[test]
    fn descriptor_extension_bytes_are_tolerated_and_preserved() {
        let body = [0x01, 0x00, 0x10, 0x1D, 0x10, 0xAA, 0xBB];
        let d = DoviVideoStreamDescriptor::parse(&body).unwrap();
        assert_eq!(d.extension, [0xAA, 0xBB]);
        assert_eq!(d.record.profile, 8);
        assert_eq!(d.try_to_bytes().unwrap(), body);
        let mut es_info = alloc::vec![DOVI_VIDEO_STREAM_DESCRIPTOR_TAG, body.len() as u8];
        es_info.extend_from_slice(&body);
        assert!(find_video_stream_descriptor(&es_info).is_some());
    }
}
