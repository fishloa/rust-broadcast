//! Application Information Table — ETSI TS 102 809 §5.3.4.
//!
//! AIT carries application metadata for HbbTV / interactive-TV services.
//! Carried on a per-service PID with table_id 0x74.

use crate::descriptors::DescriptorLoop;
use crate::error::{Error, Result};
use dvb_common::{Parse, Serialize};

/// AIT table_id (ETSI TS 102 809 §5.3.4).
pub const TABLE_ID: u8 = 0x74;
/// AIT has no well-known PID — it is service-specific.
pub const PID: u16 = 0x0000;

const MIN_HEADER_LEN: usize = 3;
const EXTENSION_HEADER_LEN: usize = 5;
const COMMON_DESC_LEN_BYTES: usize = 2;
const APP_LOOP_LEN_BYTES: usize = 2;
const CRC_LEN: usize = 4;
const APP_HEADER_LEN: usize = 9;
const MIN_SECTION_LEN: usize =
    MIN_HEADER_LEN + EXTENSION_HEADER_LEN + COMMON_DESC_LEN_BYTES + APP_LOOP_LEN_BYTES + CRC_LEN;

/// Application control code — ETSI TS 102 809 §5.2.4.1 Table 3.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum ControlCode {
    /// 0x00 — reserved for future use.
    Reserved,
    /// 0x01 — AUTOSTART: started on service selection unless already running.
    Autostart,
    /// 0x02 — PRESENT: allowed to run but not auto-started.
    Present,
    /// 0x03 — DESTROY: stopped gracefully, no restart.
    Destroy,
    /// 0x04 — KILL: stopped immediately, no restart.
    Kill,
    /// 0x05 — PREFETCH: files cached, app not started.
    Prefetch,
    /// 0x06 — REMOTE: application not hosted by the current service.
    Remote,
    /// 0x07 — DISABLED: application shall not be available to the user.
    Disabled,
    /// 0x08 — PLAYBACK_AUTOSTART: autostart for playback services.
    PlaybackAutostart,
    /// Catch-all for reserved / unallocated wire values.
    Unallocated(u8),
}

impl ControlCode {
    #[must_use]
    /// Decode from the wire value.  Every value maps (lossless).
    pub fn from_u8(v: u8) -> Self {
        match v {
            0x00 => Self::Reserved,
            0x01 => Self::Autostart,
            0x02 => Self::Present,
            0x03 => Self::Destroy,
            0x04 => Self::Kill,
            0x05 => Self::Prefetch,
            0x06 => Self::Remote,
            0x07 => Self::Disabled,
            0x08 => Self::PlaybackAutostart,
            _ => Self::Unallocated(v),
        }
    }

    #[must_use]
    /// Encode to the wire value.  Inverse of `from_u8` / `from_u16`.
    pub fn to_u8(self) -> u8 {
        match self {
            Self::Reserved => 0x00,
            Self::Autostart => 0x01,
            Self::Present => 0x02,
            Self::Destroy => 0x03,
            Self::Kill => 0x04,
            Self::Prefetch => 0x05,
            Self::Remote => 0x06,
            Self::Disabled => 0x07,
            Self::PlaybackAutostart => 0x08,
            Self::Unallocated(v) => v,
        }
    }

    #[must_use]
    /// Human-readable spec display name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Reserved => "Reserved",
            Self::Autostart => "AUTOSTART",
            Self::Present => "PRESENT",
            Self::Destroy => "DESTROY",
            Self::Kill => "KILL",
            Self::Prefetch => "PREFETCH",
            Self::Remote => "REMOTE",
            Self::Disabled => "DISABLED",
            Self::PlaybackAutostart => "PLAYBACK_AUTOSTART",
            Self::Unallocated(_) => "Unallocated",
        }
    }
}

/// Application type — ETSI TS 102 809 §5.2.4.2 Tables 2-3 (application_type).
///
/// 15-bit field identifying the application environment.
/// Verified entries from the DVB Services registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum ApplicationType {
    /// 0x0001 — DVB-J.
    DvbJ,
    /// 0x0002 — DVB-HTML.
    DvbHtml,
    /// 0x0010 — HbbTV.
    HbbTv,
    /// 0x0011 — OIPF DAE.
    OipfDae,
    /// Other values below `0x8000` — reserved for DVB use.
    Reserved(u16),
    /// `0x8000`..`0xFFFF` — user defined.
    UserDefined(u16),
}

impl ApplicationType {
    #[must_use]
    /// Decode from the wire value.  Every value maps (lossless).
    pub fn from_u16(v: u16) -> Self {
        match v {
            0x0001 => Self::DvbJ,
            0x0002 => Self::DvbHtml,
            0x0010 => Self::HbbTv,
            0x0011 => Self::OipfDae,
            v if v < 0x8000 => Self::Reserved(v),
            _ => Self::UserDefined(v),
        }
    }

    #[must_use]
    /// Encode to the wire value.  Inverse of `from_u16`.
    pub fn to_u16(self) -> u16 {
        match self {
            Self::DvbJ => 0x0001,
            Self::DvbHtml => 0x0002,
            Self::HbbTv => 0x0010,
            Self::OipfDae => 0x0011,
            Self::Reserved(v) | Self::UserDefined(v) => v,
        }
    }

    #[must_use]
    /// Human-readable spec display name.
    pub fn name(self) -> &'static str {
        match self {
            Self::DvbJ => "DVB-J",
            Self::DvbHtml => "DVB-HTML",
            Self::HbbTv => "HbbTV",
            Self::OipfDae => "OIPF DAE",
            Self::Reserved(_) => "Reserved",
            Self::UserDefined(_) => "User Defined",
        }
    }
}

/// 48-bit application identifier: organisation_id + application_id.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ApplicationIdentifier {
    /// 32-bit organisation_id.
    pub organisation_id: u32,
    /// 16-bit application_id.
    pub application_id: u16,
}

/// One application entry in the AIT application loop.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct AitApplication<'a> {
    /// Application identifier.
    pub identifier: ApplicationIdentifier,
    /// Application control code (1 = autostart, etc.).
    pub control_code: ControlCode,
    /// Raw descriptor bytes for this application.
    /// Per-application descriptor loop. Serializes as the typed descriptor
    /// sequence; `.raw()` yields the wire bytes.
    pub descriptors: DescriptorLoop<'a>,
}

/// Application Information Table.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct AitSection<'a> {
    /// 15-bit application_type (e.g. 0x0010 for HbbTV).
    pub application_type: ApplicationType,
    /// Test application flag (bit 15 of the extension field).
    pub test_application_flag: bool,
    /// 5-bit version_number.
    pub version_number: u8,
    /// current_next_indicator bit.
    pub current_next_indicator: bool,
    /// section_number in the sub-table sequence.
    pub section_number: u8,
    /// last_section_number in the sub-table sequence.
    pub last_section_number: u8,
    /// Raw common descriptor bytes.
    /// Common descriptor loop. Serializes as the typed descriptor sequence;
    /// `.raw()` yields the wire bytes.
    pub common_descriptors: DescriptorLoop<'a>,
    /// Applications in wire order.
    pub applications: Vec<AitApplication<'a>>,
}

impl<'a> Parse<'a> for AitSection<'a> {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let min_len = MIN_HEADER_LEN
            + EXTENSION_HEADER_LEN
            + COMMON_DESC_LEN_BYTES
            + APP_LOOP_LEN_BYTES
            + CRC_LEN;
        if bytes.len() < min_len {
            return Err(Error::BufferTooShort {
                need: min_len,
                have: bytes.len(),
                what: "AitSection",
            });
        }

        if bytes[0] != TABLE_ID {
            return Err(Error::UnexpectedTableId {
                table_id: bytes[0],
                what: "AitSection",
                expected: &[TABLE_ID],
            });
        }

        let section_length = ((bytes[1] & 0x0F) as u16) << 8 | bytes[2] as u16;
        let total = super::check_section_length(
            bytes.len(),
            MIN_HEADER_LEN,
            section_length as usize,
            MIN_SECTION_LEN,
        )?;

        let test_application_flag = (bytes[3] & 0x80) != 0;
        let application_type_raw = (((bytes[3] & 0x7F) as u16) << 8) | (bytes[4] as u16);
        let application_type = ApplicationType::from_u16(application_type_raw);
        let version_number = (bytes[5] >> 1) & 0x1F;
        let current_next_indicator = (bytes[5] & 0x01) != 0;
        let section_number = bytes[6];
        let last_section_number = bytes[7];

        let common_descriptors_length = (((bytes[8] & 0x0F) as usize) << 8) | bytes[9] as usize;
        let common_desc_start = MIN_HEADER_LEN + EXTENSION_HEADER_LEN + COMMON_DESC_LEN_BYTES;
        let common_desc_end = common_desc_start + common_descriptors_length;
        let app_loop_end = total - CRC_LEN;
        if common_desc_end > app_loop_end {
            return Err(Error::SectionLengthOverflow {
                declared: common_descriptors_length,
                available: app_loop_end.saturating_sub(common_desc_start),
            });
        }
        let common_descriptors = DescriptorLoop::new(&bytes[common_desc_start..common_desc_end]);

        let app_loop_length =
            (((bytes[common_desc_end] & 0x0F) as usize) << 8) | bytes[common_desc_end + 1] as usize;
        let app_loop_start = common_desc_end + APP_LOOP_LEN_BYTES;
        let app_loop_actual_end = app_loop_start + app_loop_length;
        if app_loop_actual_end > app_loop_end {
            return Err(Error::SectionLengthOverflow {
                declared: app_loop_length,
                available: app_loop_end.saturating_sub(app_loop_start),
            });
        }

        let mut applications = Vec::new();
        let mut pos = app_loop_start;
        while pos + APP_HEADER_LEN <= app_loop_actual_end {
            let organisation_id = ((bytes[pos] as u32) << 24)
                | ((bytes[pos + 1] as u32) << 16)
                | ((bytes[pos + 2] as u32) << 8)
                | (bytes[pos + 3] as u32);
            let application_id = u16::from_be_bytes([bytes[pos + 4], bytes[pos + 5]]);
            let control_code = ControlCode::from_u8(bytes[pos + 6]);
            let app_desc_length =
                (((bytes[pos + 7] & 0x0F) as usize) << 8) | bytes[pos + 8] as usize;
            let app_desc_start = pos + APP_HEADER_LEN;
            let app_desc_end = app_desc_start + app_desc_length;
            if app_desc_end > app_loop_actual_end {
                return Err(Error::SectionLengthOverflow {
                    declared: app_desc_length,
                    available: app_loop_actual_end.saturating_sub(app_desc_start),
                });
            }
            applications.push(AitApplication {
                identifier: ApplicationIdentifier {
                    organisation_id,
                    application_id,
                },
                control_code,
                descriptors: DescriptorLoop::new(&bytes[app_desc_start..app_desc_end]),
            });
            pos = app_desc_end;
        }

        Ok(AitSection {
            application_type,
            test_application_flag,
            version_number,
            current_next_indicator,
            section_number,
            last_section_number,
            common_descriptors,
            applications,
        })
    }
}

impl Serialize for AitSection<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        let app_bytes: usize = self
            .applications
            .iter()
            .map(|a| APP_HEADER_LEN + a.descriptors.len())
            .sum();
        MIN_HEADER_LEN
            + EXTENSION_HEADER_LEN
            + COMMON_DESC_LEN_BYTES
            + self.common_descriptors.len()
            + APP_LOOP_LEN_BYTES
            + app_bytes
            + CRC_LEN
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }

        let section_length: u16 = (len - MIN_HEADER_LEN) as u16;
        let app_type_raw = self.application_type.to_u16();
        buf[0] = TABLE_ID;
        buf[1] = super::SECTION_B1_FLAGS_DVB | ((section_length >> 8) as u8 & 0x0F);
        buf[2] = (section_length & 0xFF) as u8;
        buf[3] = (u8::from(self.test_application_flag) << 7) | ((app_type_raw >> 8) as u8 & 0x7F);
        buf[4] = (app_type_raw & 0xFF) as u8;
        buf[5] = 0xC0 | ((self.version_number & 0x1F) << 1) | u8::from(self.current_next_indicator);
        buf[6] = self.section_number;
        buf[7] = self.last_section_number;

        let cdl = self.common_descriptors.len() as u16;
        buf[8] = 0xF0 | ((cdl >> 8) as u8 & 0x0F);
        buf[9] = (cdl & 0xFF) as u8;

        let common_desc_start = MIN_HEADER_LEN + EXTENSION_HEADER_LEN + COMMON_DESC_LEN_BYTES;
        buf[common_desc_start..common_desc_start + self.common_descriptors.len()]
            .copy_from_slice(self.common_descriptors.raw());

        let app_loop_start = common_desc_start + self.common_descriptors.len();
        let app_bytes: usize = self
            .applications
            .iter()
            .map(|a| APP_HEADER_LEN + a.descriptors.len())
            .sum();
        let apl = app_bytes as u16;
        buf[app_loop_start] = 0xF0 | ((apl >> 8) as u8 & 0x0F);
        buf[app_loop_start + 1] = (apl & 0xFF) as u8;

        let mut pos = app_loop_start + APP_LOOP_LEN_BYTES;
        for app in &self.applications {
            buf[pos..pos + 4].copy_from_slice(&app.identifier.organisation_id.to_be_bytes());
            buf[pos + 4..pos + 6].copy_from_slice(&app.identifier.application_id.to_be_bytes());
            buf[pos + 6] = app.control_code.to_u8();
            let adl = app.descriptors.len() as u16;
            buf[pos + 7] = 0xF0 | ((adl >> 8) as u8 & 0x0F);
            buf[pos + 8] = (adl & 0xFF) as u8;
            let desc_start = pos + APP_HEADER_LEN;
            buf[desc_start..desc_start + app.descriptors.len()]
                .copy_from_slice(app.descriptors.raw());
            pos = desc_start + app.descriptors.len();
        }

        let crc_pos = len - CRC_LEN;
        let crc = dvb_common::crc32_mpeg2::compute(&buf[..crc_pos]);
        buf[crc_pos..len].copy_from_slice(&crc.to_be_bytes());
        Ok(len)
    }
}
impl<'a> crate::traits::TableDef<'a> for AitSection<'a> {
    const TABLE_ID_RANGES: &'static [(u8, u8)] = &[(TABLE_ID, TABLE_ID)];
    const NAME: &'static str = "APPLICATION_INFORMATION";
}

#[cfg(test)]
mod tests {
    use super::*;

    fn build_ait(
        application_type: u16,
        test_flag: bool,
        version: u8,
        common_descriptors: &[u8],
        applications: &[(u32, u16, u8, Vec<u8>)],
    ) -> Vec<u8> {
        let app_bytes: usize = applications
            .iter()
            .map(|(_, _, _, d)| APP_HEADER_LEN + d.len())
            .sum();
        let section_length: u16 = (EXTENSION_HEADER_LEN
            + COMMON_DESC_LEN_BYTES
            + common_descriptors.len()
            + APP_LOOP_LEN_BYTES
            + app_bytes
            + CRC_LEN) as u16;
        let mut v = vec![
            TABLE_ID,
            super::super::SECTION_B1_FLAGS_DVB | ((section_length >> 8) as u8 & 0x0F),
            (section_length & 0xFF) as u8,
            (u8::from(test_flag) << 7) | ((application_type >> 8) as u8 & 0x7F),
            (application_type & 0xFF) as u8,
            0xC0 | ((version & 0x1F) << 1) | 0x01,
            0,
            0,
        ];
        let cdl = common_descriptors.len() as u16;
        v.push(0xF0 | ((cdl >> 8) as u8 & 0x0F));
        v.push((cdl & 0xFF) as u8);
        v.extend_from_slice(common_descriptors);
        let apl = app_bytes as u16;
        v.push(0xF0 | ((apl >> 8) as u8 & 0x0F));
        v.push((apl & 0xFF) as u8);
        for &(org_id, app_id, cc, ref desc) in applications {
            v.extend_from_slice(&org_id.to_be_bytes());
            v.extend_from_slice(&app_id.to_be_bytes());
            v.push(cc);
            let adl = desc.len() as u16;
            v.push(0xF0 | ((adl >> 8) as u8 & 0x0F));
            v.push((adl & 0xFF) as u8);
            v.extend_from_slice(desc);
        }
        v.extend_from_slice(&[0, 0, 0, 0]);
        v
    }

    #[test]
    fn parse_rejects_wrong_table_id() {
        let mut bytes = build_ait(0x0010, false, 0, &[], &[]);
        bytes[0] = 0x00;
        let err = AitSection::parse(&bytes).unwrap_err();
        assert!(matches!(
            err,
            Error::UnexpectedTableId { table_id: 0x00, .. }
        ));
    }

    #[test]
    fn parse_rejects_short_buffer() {
        let err = AitSection::parse(&[0x74, 0x00]).unwrap_err();
        assert!(matches!(err, Error::BufferTooShort { .. }));
    }

    #[test]
    fn parse_empty_ait_no_applications() {
        let bytes = build_ait(0x0010, false, 5, &[], &[]);
        let ait = AitSection::parse(&bytes).expect("parse");
        assert_eq!(ait.application_type, ApplicationType::HbbTv);
        assert!(!ait.test_application_flag);
        assert_eq!(ait.version_number, 5);
        assert!(ait.current_next_indicator);
        assert_eq!(ait.section_number, 0);
        assert_eq!(ait.last_section_number, 0);
        assert_eq!(ait.common_descriptors.len(), 0);
        assert_eq!(ait.applications.len(), 0);
    }

    #[test]
    fn parse_test_application_flag_extracted() {
        let bytes = build_ait(0x0010, true, 0, &[], &[]);
        let ait = AitSection::parse(&bytes).unwrap();
        assert!(ait.test_application_flag);
    }

    #[test]
    fn parse_common_descriptors_preserved() {
        let desc = vec![0x00, 0x02, 0xAA, 0xBB];
        let bytes = build_ait(0x0010, false, 0, &desc, &[]);
        let ait = AitSection::parse(&bytes).unwrap();
        assert_eq!(ait.common_descriptors.raw(), &desc[..]);
    }

    #[test]
    fn parse_single_application() {
        let desc = vec![0x02, 0x03, 0xCC, 0xDD, 0xEE];
        let bytes = build_ait(
            0x0010,
            false,
            0,
            &[],
            &[(0x12345678, 0xABCD, 0x01, desc.clone())],
        );
        let ait = AitSection::parse(&bytes).unwrap();
        assert_eq!(ait.applications.len(), 1);
        assert_eq!(ait.applications[0].identifier.organisation_id, 0x12345678);
        assert_eq!(ait.applications[0].identifier.application_id, 0xABCD);
        assert_eq!(ait.applications[0].control_code, ControlCode::Autostart);
        assert_eq!(ait.applications[0].descriptors.raw(), &desc[..]);
    }

    #[test]
    fn parse_multiple_applications_preserve_order() {
        let bytes = build_ait(
            0x0010,
            false,
            0,
            &[],
            &[
                (0x00000001, 0x0001, 0x01, vec![]),
                (0x00000002, 0x0002, 0x02, vec![0x01]),
                (0x00000003, 0x0003, 0x03, vec![0x02, 0x03]),
            ],
        );
        let ait = AitSection::parse(&bytes).unwrap();
        assert_eq!(ait.applications.len(), 3);
        assert_eq!(ait.applications[0].identifier.organisation_id, 1);
        assert_eq!(ait.applications[1].identifier.organisation_id, 2);
        assert_eq!(ait.applications[2].identifier.organisation_id, 3);
    }

    #[test]
    fn serialize_round_trip_empty() {
        let ait = AitSection {
            application_type: ApplicationType::HbbTv,
            test_application_flag: false,
            version_number: 3,
            current_next_indicator: true,
            section_number: 0,
            last_section_number: 0,
            common_descriptors: DescriptorLoop::new(&[]),
            applications: vec![],
        };
        let mut buf = vec![0u8; ait.serialized_len()];
        ait.serialize_into(&mut buf).unwrap();
        let reparsed = AitSection::parse(&buf).unwrap();
        assert_eq!(ait, reparsed);
    }

    #[test]
    fn serialize_round_trip_with_applications() {
        let desc1: [u8; 2] = [0xAA, 0xBB];
        let ait = AitSection {
            application_type: ApplicationType::HbbTv,
            test_application_flag: true,
            version_number: 7,
            current_next_indicator: true,
            section_number: 1,
            last_section_number: 2,
            common_descriptors: DescriptorLoop::new(&[0x01, 0x00]),
            applications: vec![
                AitApplication {
                    identifier: ApplicationIdentifier {
                        organisation_id: 0x12345678,
                        application_id: 0xABCD,
                    },
                    control_code: ControlCode::Autostart,
                    descriptors: DescriptorLoop::new(&desc1),
                },
                AitApplication {
                    identifier: ApplicationIdentifier {
                        organisation_id: 0x87654321,
                        application_id: 0x00EF,
                    },
                    control_code: ControlCode::Present,
                    descriptors: DescriptorLoop::new(&[]),
                },
            ],
        };
        let mut buf = vec![0u8; ait.serialized_len()];
        ait.serialize_into(&mut buf).unwrap();
        let reparsed = AitSection::parse(&buf).unwrap();
        assert_eq!(ait, reparsed);
    }

    #[test]
    fn parse_rejects_zero_section_length() {
        let mut buf = vec![0u8; 64];
        buf[0] = TABLE_ID;
        buf[1] = 0xF0;
        buf[2] = 0x00;
        for b in &mut buf[3..] {
            *b = 0xFF;
        }
        assert!(matches!(
            AitSection::parse(&buf).unwrap_err(),
            Error::SectionLengthOverflow { .. }
        ));
    }

    #[test]
    fn control_code_full_range_round_trip() {
        for byte in 0u8..=0xFF {
            let cc = ControlCode::from_u8(byte);
            assert_eq!(
                cc.to_u8(),
                byte,
                "ControlCode round-trip failed for {byte:#04x}"
            );
        }
    }

    #[test]
    fn control_code_named_values() {
        assert_eq!(ControlCode::Autostart.to_u8(), 0x01);
        assert_eq!(ControlCode::Kill.to_u8(), 0x04);
        assert_eq!(ControlCode::Prefetch.to_u8(), 0x05);
        assert_eq!(ControlCode::PlaybackAutostart.to_u8(), 0x08);
    }

    #[test]
    fn application_type_full_range_round_trip() {
        for at in 0u16..=0xFFFF {
            let app = ApplicationType::from_u16(at);
            assert_eq!(
                app.to_u16(),
                at,
                "ApplicationType round-trip failed for {at:#06x}"
            );
        }
    }
}
