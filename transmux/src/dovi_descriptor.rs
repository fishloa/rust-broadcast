//! `DOVI_video_stream_descriptor` — MPEG-2 TS carriage of a Dolby Vision
//! configuration record.
//!
//! The record itself and its ISOBMFF boxes live in [`isobmff::dovi`].

use crate::error::{Error, Result};
use crate::ts_demux::DESCRIPTOR_HEADER_LEN;
use alloc::vec::Vec;
use broadcast_core::len::{need_in, need_out};
use broadcast_core::wire::u16_at;
use broadcast_core::{Parse, Serialize};
use isobmff::dovi::{
    BL_FLAG, CCID_MASK, CCID_OFFSET, CCID_SHIFT, CONFIG_WORD_OFFSET, DOVI_RESERVED_ZERO,
    DoviDecoderConfigurationRecord, LEVEL_MASK, PROFILE_BITS, RESERVED_OFFSET, RESERVED_TAIL_LEN,
    invalid, pack_config_word, unpack_config_word,
};

/// MPEG-2 TS `DOVI_video_stream_descriptor` tag (TS spec Table 3-2).
pub const DOVI_VIDEO_STREAM_DESCRIPTOR_TAG: u8 = 0xB0;
/// MPEG-2 registration descriptor tag (ISO/IEC 13818-1 Table 2-45).
pub const REGISTRATION_DESCRIPTOR_TAG: u8 = 0x05;
/// `format_identifier` "DOVI" of the registration descriptor (TS spec Table 3-1).
pub const DOVI_FORMAT_IDENTIFIER: [u8; 4] = *b"DOVI";

const TS_DESCRIPTOR_BASE_LEN: usize = 5;
const TS_DESCRIPTOR_DEPENDENCY_LEN: usize = 2;
const PID_SHIFT: u32 = 3;
const PID_MAX: u16 = 0x1FFF;

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
        need_in(
            bytes,
            TS_DESCRIPTOR_BASE_LEN,
            "DOVI_video_stream_descriptor",
        )?;
        let word = u16_at(bytes, CONFIG_WORD_OFFSET);
        let bl_present = word & BL_FLAG != 0;
        let mut dependency_reserved = 0u8;
        let (dependency_pid, tail) = if bl_present {
            (None, TS_DESCRIPTOR_BASE_LEN - RESERVED_TAIL_LEN)
        } else {
            need_in(
                bytes,
                TS_DESCRIPTOR_BASE_LEN + TS_DESCRIPTOR_DEPENDENCY_LEN,
                "DOVI_video_stream_descriptor dependency_pid",
            )?;
            let raw = u16_at(bytes, CCID_OFFSET);
            dependency_reserved = (raw & ((1 << PID_SHIFT) - 1)) as u8;
            (
                Some(raw >> PID_SHIFT),
                TS_DESCRIPTOR_BASE_LEN + TS_DESCRIPTOR_DEPENDENCY_LEN - RESERVED_TAIL_LEN,
            )
        };
        need_in(
            bytes,
            tail + RESERVED_TAIL_LEN,
            "DOVI_video_stream_descriptor",
        )?;
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
        need_out(buf, need)?;
        let ccid = self.record.bl_signal_compatibility_id.code();
        if ccid > CCID_MASK
            || self.record.level > LEVEL_MASK as u8
            || self.record.profile >= 1 << PROFILE_BITS
        {
            return Err(invalid(
                "DOVI_video_stream_descriptor",
                u64::from(ccid),
                "field does not fit its bit width",
            )
            .into());
        }
        buf[0] = self.record.version_major;
        buf[1] = self.record.version_minor;
        buf[CONFIG_WORD_OFFSET..RESERVED_OFFSET]
            .copy_from_slice(&pack_config_word(&self.record).to_be_bytes());
        let tail = match (self.record.bl_present, self.dependency_pid) {
            (true, None) => TS_DESCRIPTOR_BASE_LEN - RESERVED_TAIL_LEN,
            (false, Some(pid)) if pid <= PID_MAX => {
                if self.dependency_reserved >= 1 << PID_SHIFT {
                    return Err(invalid(
                        "dependency_reserved",
                        u64::from(self.dependency_reserved),
                        "does not fit 3 bits",
                    )
                    .into());
                }
                buf[CCID_OFFSET..CCID_OFFSET + TS_DESCRIPTOR_DEPENDENCY_LEN].copy_from_slice(
                    &((pid << PID_SHIFT) | u16::from(self.dependency_reserved)).to_be_bytes(),
                );
                TS_DESCRIPTOR_BASE_LEN + TS_DESCRIPTOR_DEPENDENCY_LEN - RESERVED_TAIL_LEN
            }
            _ => {
                return Err(invalid(
                    "dependency_pid",
                    self.dependency_pid.map_or(0, u64::from),
                    "present exactly when bl_present_flag is 0, and 13 bits",
                )
                .into());
            }
        };
        if self.reserved > CCID_MASK {
            return Err(invalid(
                "DOVI_video_stream_descriptor.reserved",
                u64::from(self.reserved),
                "does not fit 4 bits",
            )
            .into());
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

/// Frame a `DOVI_video_stream_descriptor` for a PMT ES_info loop
/// (`tag | length | body`); the inverse of [`find_video_stream_descriptor`].
///
/// # Errors
/// An error if the descriptor does not serialise or its body exceeds the 8-bit
/// `descriptor_length`.
pub fn video_stream_descriptor_bytes(d: &DoviVideoStreamDescriptor) -> Result<Vec<u8>> {
    let body = d.try_to_bytes()?;
    let length = broadcast_core::len::fit_u8(body.len(), "descriptor_length")?;
    let mut out = Vec::with_capacity(DESCRIPTOR_HEADER_LEN + body.len());
    out.push(DOVI_VIDEO_STREAM_DESCRIPTOR_TAG);
    out.push(length);
    out.extend_from_slice(&body);
    Ok(out)
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
        if off + DESCRIPTOR_HEADER_LEN > es_info.len() {
            return None;
        }
        let tag = es_info[off];
        let len = usize::from(es_info[off + 1]);
        let body = es_info.get(off + DESCRIPTOR_HEADER_LEN..off + DESCRIPTOR_HEADER_LEN + len)?;
        off += DESCRIPTOR_HEADER_LEN + len;
        Some((tag, body))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use isobmff::dovi::*;

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
        assert!(matches!(
            DoviDecoderConfigurationRecord::parse(&P81_RECORD[..23]),
            Err(isobmff::Error::BufferTooShort(..))
        ));
        let mut long = P81_RECORD.to_vec();
        long.push(0);
        assert!(matches!(
            DoviDecoderConfigurationRecord::parse(&long),
            Err(isobmff::Error::InvalidValue(..))
        ));
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
    fn descriptor_without_base_layer_cut_before_its_pid_is_short() {
        let body = [0x01, 0x00, 0x0E, 0x1A, 0x00];
        assert!(matches!(
            DoviVideoStreamDescriptor::parse(&body),
            Err(crate::error::Error::BufferTooShort(..))
        ));
    }

    #[test]
    fn descriptor_cut_before_the_compatibility_id_is_short() {
        let body = [0x01, 0x00, 0x10, 0x1D];
        assert!(matches!(
            DoviVideoStreamDescriptor::parse(&body),
            Err(crate::error::Error::BufferTooShort(..))
        ));
        let complete = [0x01, 0x00, 0x10, 0x1D, 0x10];
        let d = DoviVideoStreamDescriptor::parse(&complete).unwrap();
        assert!(d.extension.is_empty());
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
    fn record_serialize_accepts_the_field_maxima() {
        let mut r = DoviDecoderConfigurationRecord::parse(&P81_RECORD).unwrap();
        r.level = 63;
        r.bl_signal_compatibility_id = DoviBlSignalCompatibilityId::Reserved(15);
        let bytes = r.try_to_bytes().unwrap();
        assert_eq!(&bytes[2..4], &[0x11, 0xFD]);
        assert_eq!(bytes[4], 0xF0);
        let back = DoviDecoderConfigurationRecord::parse(&bytes).unwrap();
        assert_eq!(back.level, 63);
        assert_eq!(back.bl_signal_compatibility_id.code(), 15);
    }

    #[test]
    fn record_serialize_rejects_a_level_above_six_bits() {
        let mut r = DoviDecoderConfigurationRecord::parse(&P81_RECORD).unwrap();
        r.level = 64;
        assert!(matches!(
            r.try_to_bytes(),
            Err(isobmff::Error::InvalidValue(..))
        ));
    }

    #[test]
    fn record_serialize_rejects_a_compatibility_id_above_four_bits() {
        let mut r = DoviDecoderConfigurationRecord::parse(&P81_RECORD).unwrap();
        r.bl_signal_compatibility_id = DoviBlSignalCompatibilityId::Reserved(16);
        assert!(matches!(
            r.try_to_bytes(),
            Err(isobmff::Error::InvalidValue(..))
        ));
    }

    #[test]
    fn compatibility_id_labels() {
        assert_eq!(DoviBlSignalCompatibilityId::from_code(4).name(), "HLG");
        assert_eq!(DoviBlSignalCompatibilityId::Reserved(9).code(), 9);
    }

    #[test]
    fn sample_entry_kind_labels_are_pinned() {
        for (kind, label) in [
            (
                DoviSampleEntryKind::CompatibleBaseLayer,
                "compatible base layer",
            ),
            (
                DoviSampleEntryKind::DolbyVisionEntry,
                "Dolby Vision sample entry",
            ),
        ] {
            assert_eq!(kind.name(), label);
            assert_eq!(kind.to_string(), label);
        }
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

    fn descriptor_of(
        record: DoviDecoderConfigurationRecord,
        pid: Option<u16>,
    ) -> DoviVideoStreamDescriptor {
        DoviVideoStreamDescriptor {
            record,
            dependency_pid: pid,
            dependency_reserved: 0,
            reserved: 0,
            extension: Vec::new(),
        }
    }

    fn record_without_base_layer() -> DoviDecoderConfigurationRecord {
        let mut r = DoviDecoderConfigurationRecord::parse(&P81_RECORD).unwrap();
        r.bl_present = false;
        r
    }

    #[test]
    fn descriptor_dependency_pid_accepts_its_13_bit_maximum() {
        let d = descriptor_of(record_without_base_layer(), Some(0x1FFF));
        let bytes = d.try_to_bytes().unwrap();
        assert_eq!(&bytes[4..6], &[0xFF, 0xF8]);
        assert_eq!(
            DoviVideoStreamDescriptor::parse(&bytes)
                .unwrap()
                .dependency_pid,
            Some(0x1FFF)
        );
    }

    #[test]
    fn descriptor_rejects_a_dependency_pid_above_13_bits() {
        let d = descriptor_of(record_without_base_layer(), Some(0x2000));
        assert!(matches!(
            d.try_to_bytes(),
            Err(crate::error::Error::InvalidValue(..))
        ));
    }

    #[test]
    fn descriptor_serialize_accepts_the_field_maxima() {
        let mut record = DoviDecoderConfigurationRecord::parse(&P81_RECORD).unwrap();
        record.level = 63;
        record.profile = 127;
        record.bl_signal_compatibility_id = DoviBlSignalCompatibilityId::Reserved(15);
        let d = descriptor_of(record, None);
        let bytes = d.try_to_bytes().unwrap();
        assert_eq!(&bytes[2..4], &[0xFF, 0xFD]);
        assert_eq!(bytes[4], 0xF0);
        let back = DoviVideoStreamDescriptor::parse(&bytes).unwrap();
        assert_eq!(back.record.level, 63);
        assert_eq!(back.record.profile, 127);
        assert_eq!(back.record.bl_signal_compatibility_id.code(), 15);
    }

    #[test]
    fn descriptor_rejects_a_level_above_six_bits() {
        let mut record = DoviDecoderConfigurationRecord::parse(&P81_RECORD).unwrap();
        record.level = 64;
        assert!(matches!(
            descriptor_of(record, None).try_to_bytes(),
            Err(crate::error::Error::InvalidValue(..))
        ));
    }

    #[test]
    fn descriptor_rejects_a_profile_above_seven_bits() {
        let mut record = DoviDecoderConfigurationRecord::parse(&P81_RECORD).unwrap();
        record.profile = 128;
        assert!(matches!(
            descriptor_of(record, None).try_to_bytes(),
            Err(crate::error::Error::InvalidValue(..))
        ));
    }

    #[test]
    fn descriptor_rejects_a_compatibility_id_above_four_bits() {
        let mut record = DoviDecoderConfigurationRecord::parse(&P81_RECORD).unwrap();
        record.bl_signal_compatibility_id = DoviBlSignalCompatibilityId::Reserved(16);
        assert!(matches!(
            descriptor_of(record, None).try_to_bytes(),
            Err(crate::error::Error::InvalidValue(..))
        ));
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

    #[test]
    fn video_stream_descriptor_bytes_frames_the_body_and_find_recovers_it() {
        let with_pid = [0x01, 0x00, 0x0E, 0x1A, 0x12, 0x37, 0x4B];
        let d = DoviVideoStreamDescriptor::parse(&with_pid).unwrap();
        let es_info = video_stream_descriptor_bytes(&d).unwrap();
        assert_eq!(es_info[0], DOVI_VIDEO_STREAM_DESCRIPTOR_TAG);
        assert_eq!(usize::from(es_info[1]), with_pid.len());
        assert_eq!(&es_info[2..], &with_pid);
        assert_eq!(find_video_stream_descriptor(&es_info), Some(d));

        let mut after_other = alloc::vec![0x05, 0x01, 0xAA];
        after_other.extend_from_slice(&es_info);
        assert_eq!(
            find_video_stream_descriptor(&after_other),
            find_video_stream_descriptor(&es_info)
        );
    }

    #[test]
    fn the_dovi_registration_descriptor_is_recognised_by_tag_and_body() {
        let dovi = [0x05, 0x04, b'D', b'O', b'V', b'I'];
        assert!(has_registration_descriptor(&dovi));
        let hevc = [0x05, 0x04, b'H', b'E', b'V', b'C'];
        assert!(!has_registration_descriptor(&hevc));
        let wrong_tag = [0x06, 0x04, b'D', b'O', b'V', b'I'];
        assert!(!has_registration_descriptor(&wrong_tag));
        assert!(!has_registration_descriptor(&[]));
    }
}
