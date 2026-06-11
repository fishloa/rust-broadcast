//! Event Information Table — ETSI EN 300 468 §5.2.4.
//!
//! EIT carries programme event metadata. Four variants distinguished by
//! table_id:
//! - `0x4E` — Present/Following for the actual TS
//! - `0x4F` — Present/Following for another TS
//! - `0x50..=0x5F` — Schedule sub-tables for the actual TS
//! - `0x60..=0x6F` — Schedule sub-tables for another TS

use crate::descriptors::DescriptorLoop;
use crate::error::{Error, Result};
use dvb_common::{Parse, Serialize};

/// table_id for present/following on the actual TS.
pub const TABLE_ID_PF_ACTUAL: u8 = 0x4E;
/// table_id for present/following on other TSes.
pub const TABLE_ID_PF_OTHER: u8 = 0x4F;
/// First table_id in the schedule range for the actual TS.
pub const TABLE_ID_SCHEDULE_ACTUAL_FIRST: u8 = 0x50;
/// Last table_id in the schedule range for the actual TS (inclusive).
pub const TABLE_ID_SCHEDULE_ACTUAL_LAST: u8 = 0x5F;
/// First table_id in the schedule range for other TSes.
pub const TABLE_ID_SCHEDULE_OTHER_FIRST: u8 = 0x60;
/// Last table_id in the schedule range for other TSes (inclusive).
pub const TABLE_ID_SCHEDULE_OTHER_LAST: u8 = 0x6F;
/// Well-known PID on which EIT is carried.
pub const PID: u16 = 0x0012;

const MIN_HEADER_LEN: usize = 3;
const EXTENSION_HEADER_LEN: usize = 5;
/// Bytes after the extension header: transport_stream_id(2) + original_network_id(2) + segment_last_section_number(1)
/// + last_table_id(1) = 6 bytes between the section header and the first event.
const POST_EXTENSION_LEN: usize = 6;
const CRC_LEN: usize = 4;
const MIN_SECTION_LEN: usize = MIN_HEADER_LEN + EXTENSION_HEADER_LEN + POST_EXTENSION_LEN + CRC_LEN;
const EVENT_HEADER_LEN: usize = 12;

/// EIT variant distinguished by table_id range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum EitKind {
    /// Present/Following, actual TS.
    PresentFollowingActual,
    /// Present/Following, other TS.
    PresentFollowingOther,
    /// Schedule, actual TS — table_id `0x50..=0x5F`.
    ScheduleActual,
    /// Schedule, other TS — table_id `0x60..=0x6F`.
    ScheduleOther,
}

impl EitKind {
    /// Classify a table_id byte into a kind, if recognised.
    #[must_use]
    pub fn from_table_id(table_id: u8) -> Option<Self> {
        match table_id {
            TABLE_ID_PF_ACTUAL => Some(Self::PresentFollowingActual),
            TABLE_ID_PF_OTHER => Some(Self::PresentFollowingOther),
            TABLE_ID_SCHEDULE_ACTUAL_FIRST..=TABLE_ID_SCHEDULE_ACTUAL_LAST => {
                Some(Self::ScheduleActual)
            }
            TABLE_ID_SCHEDULE_OTHER_FIRST..=TABLE_ID_SCHEDULE_OTHER_LAST => {
                Some(Self::ScheduleOther)
            }
            _ => None,
        }
    }
}

/// One event in the EIT.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct EitEvent<'a> {
    /// 16-bit event_id.
    pub event_id: u16,
    /// 40-bit start_time: 16-bit MJD followed by 24-bit BCD UTC (HHMMSS).
    pub start_time_raw: [u8; 5],
    /// 24-bit BCD duration HHMMSS.
    pub duration_raw: [u8; 3],
    /// 3-bit running_status.
    pub running_status: u8,
    /// free_CA_mode flag.
    pub free_ca_mode: bool,
    /// Descriptor loop for this event. Serializes as the typed descriptor
    /// sequence; `.raw()` yields the wire bytes.
    pub descriptors: DescriptorLoop<'a>,
}

/// Event Information Table.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "yoke", derive(yoke::Yokeable))]
pub struct EitSection<'a> {
    /// Variant based on table_id.
    pub kind: EitKind,
    /// Raw table_id byte as parsed (for schedule sub-tables, identifies the slot).
    pub table_id: u8,
    /// service_id the events belong to (table_id_extension).
    pub service_id: u16,
    /// 5-bit version_number.
    pub version_number: u8,
    /// current_next_indicator bit.
    pub current_next_indicator: bool,
    /// section_number.
    pub section_number: u8,
    /// last_section_number.
    pub last_section_number: u8,
    /// transport_stream_id the events are carried on.
    pub transport_stream_id: u16,
    /// original_network_id.
    pub original_network_id: u16,
    /// segment_last_section_number.
    pub segment_last_section_number: u8,
    /// last_table_id (for schedule sub-table grouping).
    pub last_table_id: u8,
    /// Events in wire order.
    pub events: Vec<EitEvent<'a>>,
}

impl<'a> Parse<'a> for EitSection<'a> {
    type Error = crate::error::Error;
    fn parse(bytes: &'a [u8]) -> Result<Self> {
        let min_len = MIN_HEADER_LEN + EXTENSION_HEADER_LEN + POST_EXTENSION_LEN + CRC_LEN;
        if bytes.len() < min_len {
            return Err(Error::BufferTooShort {
                need: min_len,
                have: bytes.len(),
                what: "EitSection",
            });
        }

        let table_id = bytes[0];
        let kind = EitKind::from_table_id(table_id).ok_or(Error::UnexpectedTableId {
            table_id,
            what: "EitSection",
            expected: &[
                TABLE_ID_PF_ACTUAL,
                TABLE_ID_PF_OTHER,
                TABLE_ID_SCHEDULE_ACTUAL_FIRST,
                TABLE_ID_SCHEDULE_OTHER_FIRST,
            ],
        })?;

        let section_length = ((bytes[1] & 0x0F) as u16) << 8 | bytes[2] as u16;
        let total = super::check_section_length(
            bytes.len(),
            MIN_HEADER_LEN,
            section_length as usize,
            MIN_SECTION_LEN,
        )?;

        let service_id = u16::from_be_bytes([bytes[3], bytes[4]]);
        let version_number = (bytes[5] >> 1) & 0x1F;
        let current_next_indicator = (bytes[5] & 0x01) != 0;
        let section_number = bytes[6];
        let last_section_number = bytes[7];

        let transport_stream_id = u16::from_be_bytes([bytes[8], bytes[9]]);
        let original_network_id = u16::from_be_bytes([bytes[10], bytes[11]]);
        let segment_last_section_number = bytes[12];
        let last_table_id = bytes[13];

        let events_start = MIN_HEADER_LEN + EXTENSION_HEADER_LEN + POST_EXTENSION_LEN;
        let events_end = total - CRC_LEN;
        let mut events = Vec::new();
        let mut pos = events_start;
        while pos + EVENT_HEADER_LEN <= events_end {
            let event_id = u16::from_be_bytes([bytes[pos], bytes[pos + 1]]);
            let start_time_raw = [
                bytes[pos + 2],
                bytes[pos + 3],
                bytes[pos + 4],
                bytes[pos + 5],
                bytes[pos + 6],
            ];
            let duration_raw = [bytes[pos + 7], bytes[pos + 8], bytes[pos + 9]];
            let status_and_len_hi = bytes[pos + 10];
            let running_status = (status_and_len_hi >> 5) & 0x07;
            let free_ca_mode = (status_and_len_hi & 0x10) != 0;
            let descriptors_loop_length =
                (((status_and_len_hi & 0x0F) as usize) << 8) | bytes[pos + 11] as usize;
            let desc_start = pos + EVENT_HEADER_LEN;
            let desc_end = desc_start + descriptors_loop_length;
            if desc_end > events_end {
                return Err(Error::SectionLengthOverflow {
                    declared: descriptors_loop_length,
                    available: events_end.saturating_sub(desc_start),
                });
            }
            events.push(EitEvent {
                event_id,
                start_time_raw,
                duration_raw,
                running_status,
                free_ca_mode,
                descriptors: DescriptorLoop::new(&bytes[desc_start..desc_end]),
            });
            pos = desc_end;
        }

        if pos != events_end {
            return Err(Error::BufferTooShort {
                need: events_end - pos,
                have: 0,
                what: "EitSection trailing event bytes",
            });
        }

        Ok(EitSection {
            kind,
            table_id,
            service_id,
            version_number,
            current_next_indicator,
            section_number,
            last_section_number,
            transport_stream_id,
            original_network_id,
            segment_last_section_number,
            last_table_id,
            events,
        })
    }
}

impl Serialize for EitSection<'_> {
    type Error = crate::error::Error;
    fn serialized_len(&self) -> usize {
        let ev_bytes: usize = self
            .events
            .iter()
            .map(|e| EVENT_HEADER_LEN + e.descriptors.len())
            .sum();
        MIN_HEADER_LEN + EXTENSION_HEADER_LEN + POST_EXTENSION_LEN + ev_bytes + CRC_LEN
    }

    fn serialize_into(&self, buf: &mut [u8]) -> Result<usize> {
        let len = self.serialized_len();
        if buf.len() < len {
            return Err(Error::OutputBufferTooSmall {
                need: len,
                have: buf.len(),
            });
        }
        let section_length_usize = len - MIN_HEADER_LEN;
        if section_length_usize > 0x0FFF {
            return Err(Error::SectionLengthOverflow {
                declared: section_length_usize,
                available: 0x0FFF,
            });
        }
        let section_length: u16 = section_length_usize as u16;
        buf[0] = self.table_id;
        buf[1] = super::SECTION_B1_FLAGS_DVB | ((section_length >> 8) as u8 & 0x0F);
        buf[2] = (section_length & 0xFF) as u8;
        buf[3..5].copy_from_slice(&self.service_id.to_be_bytes());
        buf[5] = 0xC0 | ((self.version_number & 0x1F) << 1) | u8::from(self.current_next_indicator);
        buf[6] = self.section_number;
        buf[7] = self.last_section_number;
        buf[8..10].copy_from_slice(&self.transport_stream_id.to_be_bytes());
        buf[10..12].copy_from_slice(&self.original_network_id.to_be_bytes());
        buf[12] = self.segment_last_section_number;
        buf[13] = self.last_table_id;

        let mut pos = MIN_HEADER_LEN + EXTENSION_HEADER_LEN + POST_EXTENSION_LEN;
        for ev in &self.events {
            buf[pos..pos + 2].copy_from_slice(&ev.event_id.to_be_bytes());
            buf[pos + 2..pos + 7].copy_from_slice(&ev.start_time_raw);
            buf[pos + 7..pos + 10].copy_from_slice(&ev.duration_raw);
            let dll = ev.descriptors.len() as u16;
            buf[pos + 10] = ((ev.running_status & 0x07) << 5)
                | (u8::from(ev.free_ca_mode) << 4)
                | ((dll >> 8) as u8 & 0x0F);
            buf[pos + 11] = (dll & 0xFF) as u8;
            let desc_start = pos + EVENT_HEADER_LEN;
            buf[desc_start..desc_start + ev.descriptors.len()]
                .copy_from_slice(ev.descriptors.raw());
            pos = desc_start + ev.descriptors.len();
        }

        let crc_pos = len - CRC_LEN;
        let crc = dvb_common::crc32_mpeg2::compute(&buf[..crc_pos]);
        buf[crc_pos..len].copy_from_slice(&crc.to_be_bytes());
        Ok(len)
    }
}
impl<'a> crate::traits::TableDef<'a> for EitSection<'a> {
    const TABLE_ID_RANGES: &'static [(u8, u8)] =
        &[(TABLE_ID_PF_ACTUAL, TABLE_ID_SCHEDULE_OTHER_LAST)];
    const NAME: &'static str = "EVENT_INFORMATION";
}

impl EitEvent<'_> {
    /// Decode the 24-bit BCD `duration` (HHMMSS) to a [`core::time::Duration`].
    ///
    /// Returns `None` if the BCD nibbles are out of range. Available without the
    /// `chrono` feature — a duration is a plain elapsed-seconds value.
    #[must_use]
    pub fn duration(&self) -> Option<core::time::Duration> {
        dvb_common::time::decode_bcd_duration(self.duration_raw)
    }

    /// Set the event duration, encoding it into the 24-bit BCD `duration` field.
    ///
    /// # Errors
    /// [`ValueOutOfRange`](crate::Error::ValueOutOfRange) if the duration
    /// is 100 hours or longer (the `HH` field holds only two BCD digits).
    pub fn set_duration(&mut self, duration: core::time::Duration) -> crate::Result<()> {
        self.duration_raw = dvb_common::time::encode_bcd_duration(duration).ok_or(
            crate::Error::ValueOutOfRange {
                field: "EitEvent::duration",
                reason: "duration must be < 100 hours",
            },
        )?;
        Ok(())
    }
}

#[cfg(feature = "chrono")]
impl EitEvent<'_> {
    /// Decode `start_time_raw` (16-bit MJD + 24-bit BCD UTC) to a UTC datetime.
    ///
    /// Returns `None` if the date/time fields are out of range. MJD→calendar
    /// conversion per ETSI EN 300 468 Annex C.
    #[must_use]
    pub fn start_time(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        dvb_common::time::decode_mjd_bcd_utc(self.start_time_raw)
    }

    /// Set the event start time, encoding it into the 40-bit `start_time` field.
    ///
    /// # Errors
    /// [`ValueOutOfRange`](crate::Error::ValueOutOfRange) if the date is
    /// outside the representable 16-bit MJD range.
    pub fn set_start_time(
        &mut self,
        start_time: chrono::DateTime<chrono::Utc>,
    ) -> crate::Result<()> {
        self.start_time_raw = dvb_common::time::encode_mjd_bcd_utc(start_time).ok_or(
            crate::Error::ValueOutOfRange {
                field: "EitEvent::start_time",
                reason: "date not representable in 16-bit MJD",
            },
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type TestEvent = (u16, [u8; 5], [u8; 3], u8, bool, Vec<u8>);

    fn build_eit(
        table_id: u8,
        service_id: u16,
        version: u8,
        tsid: u16,
        onid: u16,
        events: &[TestEvent],
    ) -> Vec<u8> {
        let ev_bytes: usize = events
            .iter()
            .map(|(_, _, _, _, _, d)| EVENT_HEADER_LEN + d.len())
            .sum();
        let section_length: u16 =
            (EXTENSION_HEADER_LEN + POST_EXTENSION_LEN + ev_bytes + CRC_LEN) as u16;
        let mut v = Vec::new();
        v.push(table_id);
        v.push(super::super::SECTION_B1_FLAGS_DVB | ((section_length >> 8) as u8 & 0x0F));
        v.push((section_length & 0xFF) as u8);
        v.extend_from_slice(&service_id.to_be_bytes());
        v.push(0xC0 | ((version & 0x1F) << 1) | 0x01);
        v.push(0);
        v.push(0);
        v.extend_from_slice(&tsid.to_be_bytes());
        v.extend_from_slice(&onid.to_be_bytes());
        v.push(0);
        v.push(table_id);
        for (eid, start, dur, rs, fca, desc) in events {
            v.extend_from_slice(&eid.to_be_bytes());
            v.extend_from_slice(start);
            v.extend_from_slice(dur);
            let dll = desc.len() as u16;
            v.push(((*rs & 0x07) << 5) | (u8::from(*fca) << 4) | ((dll >> 8) as u8 & 0x0F));
            v.push((dll & 0xFF) as u8);
            v.extend_from_slice(desc);
        }
        v.extend_from_slice(&[0, 0, 0, 0]);
        v
    }

    #[test]
    fn parse_pf_actual_and_other_map_to_correct_kind() {
        for (tid, expected) in [
            (TABLE_ID_PF_ACTUAL, EitKind::PresentFollowingActual),
            (TABLE_ID_PF_OTHER, EitKind::PresentFollowingOther),
        ] {
            let bytes = build_eit(tid, 1, 0, 0x20, 0x30, &[]);
            assert_eq!(EitSection::parse(&bytes).unwrap().kind, expected);
        }
    }

    #[test]
    fn schedule_tables_0x50_through_0x5f_all_decode_as_schedule_actual() {
        for tid in TABLE_ID_SCHEDULE_ACTUAL_FIRST..=TABLE_ID_SCHEDULE_ACTUAL_LAST {
            let bytes = build_eit(tid, 1, 0, 0x20, 0x30, &[]);
            assert_eq!(
                EitSection::parse(&bytes).unwrap().kind,
                EitKind::ScheduleActual
            );
        }
    }

    #[test]
    fn schedule_tables_0x60_through_0x6f_all_decode_as_schedule_other() {
        for tid in TABLE_ID_SCHEDULE_OTHER_FIRST..=TABLE_ID_SCHEDULE_OTHER_LAST {
            let bytes = build_eit(tid, 1, 0, 0x20, 0x30, &[]);
            assert_eq!(
                EitSection::parse(&bytes).unwrap().kind,
                EitKind::ScheduleOther
            );
        }
    }

    #[test]
    fn event_loop_with_descriptor_bytes_preserved() {
        let desc = vec![0x4D, 0x05, 0x01, 0x02, 0x03, 0x04, 0x05];
        let bytes = build_eit(
            TABLE_ID_PF_ACTUAL,
            1,
            0,
            0x20,
            0x30,
            &[(
                42,
                [0xDF, 0xA1, 0x12, 0x34, 0x56],
                [0x00, 0x30, 0x00],
                4,
                false,
                desc.clone(),
            )],
        );
        let eit = EitSection::parse(&bytes).unwrap();
        assert_eq!(eit.events.len(), 1);
        assert_eq!(eit.events[0].event_id, 42);
        assert_eq!(eit.events[0].descriptors.raw(), &desc[..]);
    }

    #[test]
    fn running_status_extracted() {
        let bytes = build_eit(
            TABLE_ID_PF_ACTUAL,
            1,
            0,
            0x20,
            0x30,
            &[(1, [0; 5], [0; 3], 2, false, vec![])],
        );
        assert_eq!(
            EitSection::parse(&bytes).unwrap().events[0].running_status,
            2
        );
    }

    #[test]
    fn free_ca_mode_flag_extracted() {
        let bytes = build_eit(
            TABLE_ID_PF_ACTUAL,
            1,
            0,
            0x20,
            0x30,
            &[(1, [0; 5], [0; 3], 0, true, vec![])],
        );
        assert!(EitSection::parse(&bytes).unwrap().events[0].free_ca_mode);
    }

    #[test]
    fn serialize_round_trip_preserves_all_events() {
        let desc1: [u8; 2] = [0x54, 0x00];
        let eit = EitSection {
            kind: EitKind::PresentFollowingActual,
            table_id: TABLE_ID_PF_ACTUAL,
            service_id: 0x0100,
            version_number: 3,
            current_next_indicator: true,
            section_number: 0,
            last_section_number: 0,
            transport_stream_id: 0x1234,
            original_network_id: 0x0020,
            segment_last_section_number: 0,
            last_table_id: TABLE_ID_PF_ACTUAL,
            events: vec![
                EitEvent {
                    event_id: 1,
                    start_time_raw: [0xDF, 0xA1, 0x12, 0x34, 0x56],
                    duration_raw: [0x00, 0x30, 0x00],
                    running_status: 4,
                    free_ca_mode: false,
                    descriptors: DescriptorLoop::new(&desc1),
                },
                EitEvent {
                    event_id: 2,
                    start_time_raw: [0xDF, 0xA1, 0x13, 0x00, 0x00],
                    duration_raw: [0x01, 0x00, 0x00],
                    running_status: 1,
                    free_ca_mode: true,
                    descriptors: DescriptorLoop::new(&[]),
                },
            ],
        };
        let mut buf = vec![0u8; eit.serialized_len()];
        eit.serialize_into(&mut buf).unwrap();
        let re = EitSection::parse(&buf).unwrap();
        assert_eq!(eit, re);
    }

    #[test]
    fn zero_events_is_valid() {
        let bytes = build_eit(TABLE_ID_PF_ACTUAL, 1, 0, 0x20, 0x30, &[]);
        let eit = EitSection::parse(&bytes).unwrap();
        assert_eq!(eit.events.len(), 0);
    }

    #[test]
    #[cfg(feature = "chrono")]
    fn event_start_time_decodes_to_utc_datetime() {
        // MJD 59945 is 2023-01-01 per ETSI EN 300 468 Annex C; BCD time 12:34:56.
        let mjd: u16 = 59945;
        let ev = EitEvent {
            event_id: 1,
            start_time_raw: [(mjd >> 8) as u8, (mjd & 0xFF) as u8, 0x12, 0x34, 0x56],
            duration_raw: [0, 0, 0],
            running_status: 0,
            free_ca_mode: false,
            descriptors: DescriptorLoop::new(&[]),
        };
        let dt = ev.start_time().unwrap();
        use chrono::Datelike;
        assert_eq!(dt.year(), 2023);
        assert_eq!(dt.month(), 1);
        assert_eq!(dt.day(), 1);
        use chrono::Timelike;
        assert_eq!(dt.hour(), 12);
        assert_eq!(dt.minute(), 34);
        assert_eq!(dt.second(), 56);
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let bytes = build_eit(0x00, 1, 0, 0x20, 0x30, &[]);
        let err = EitSection::parse(&bytes).unwrap_err();
        assert!(matches!(
            err,
            Error::UnexpectedTableId { table_id: 0x00, .. }
        ));
    }

    #[test]
    fn parse_rejects_truncated_header() {
        let bytes = [0x4Eu8, 0xF0, 0x00];
        let err = EitSection::parse(&bytes).unwrap_err();
        assert!(matches!(
            err,
            Error::BufferTooShort {
                need: 18,
                have: 3,
                ..
            }
        ));
    }

    #[test]
    fn parse_rejects_event_descriptor_loop_overflow() {
        let section_length: u16 =
            (EXTENSION_HEADER_LEN + POST_EXTENSION_LEN + EVENT_HEADER_LEN + CRC_LEN) as u16;
        let mut v = Vec::new();
        v.push(TABLE_ID_PF_ACTUAL);
        v.push(super::super::SECTION_B1_FLAGS_DVB | ((section_length >> 8) as u8 & 0x0F));
        v.push((section_length & 0xFF) as u8);
        v.extend_from_slice(&1u16.to_be_bytes());
        v.push(0xC1);
        v.push(0);
        v.push(0);
        v.extend_from_slice(&0x0020u16.to_be_bytes());
        v.extend_from_slice(&0x0030u16.to_be_bytes());
        v.push(0);
        v.push(TABLE_ID_PF_ACTUAL);
        v.extend_from_slice(&1u16.to_be_bytes());
        v.extend_from_slice(&[0u8; 5]);
        v.extend_from_slice(&[0u8; 3]);
        v.push(0x00);
        v.push(0x0A);
        v.extend_from_slice(&[0u8; 4]);
        // descriptor_loop_length=10 but events_end is at the CRC start:
        // the declared 10 bytes overflow past the CRC boundary.
        let err = EitSection::parse(&v).unwrap_err();
        assert!(matches!(
            err,
            Error::SectionLengthOverflow { declared: 10, .. }
        ));
    }

    #[test]
    fn structured_fields_segment_and_last_table_id_preserved() {
        let desc: [u8; 2] = [0x54, 0x00];
        let bytes = build_eit(
            TABLE_ID_SCHEDULE_ACTUAL_FIRST,
            0x0100,
            7,
            0x0020,
            0x0030,
            &[(
                42,
                [0xDF, 0xA1, 0x12, 0x34, 0x56],
                [0x00, 0x30, 0x00],
                4,
                false,
                desc.to_vec(),
            )],
        );
        let eit = EitSection::parse(&bytes).unwrap();
        assert_eq!(eit.kind, EitKind::ScheduleActual);
        assert_eq!(eit.table_id, TABLE_ID_SCHEDULE_ACTUAL_FIRST);
        assert_eq!(eit.service_id, 0x0100);
        assert_eq!(eit.version_number, 7);
        assert!(eit.current_next_indicator);
        assert_eq!(eit.section_number, 0);
        assert_eq!(eit.last_section_number, 0);
        assert_eq!(eit.transport_stream_id, 0x0020);
        assert_eq!(eit.original_network_id, 0x0030);
        assert_eq!(eit.segment_last_section_number, 0);
        assert_eq!(eit.last_table_id, TABLE_ID_SCHEDULE_ACTUAL_FIRST);
        assert_eq!(eit.events.len(), 1);
        assert_eq!(eit.events[0].event_id, 42);
        assert_eq!(eit.events[0].running_status, 4);
        assert!(!eit.events[0].free_ca_mode);
        // 12-bit descriptor loop length decoded correctly: 2 bytes of desc.
        assert_eq!(eit.events[0].descriptors.raw(), &desc[..]);
    }

    #[test]
    fn parse_rejects_zero_section_length() {
        let mut buf = vec![0u8; 64];
        buf[0] = TABLE_ID_PF_ACTUAL;
        buf[1] = 0xF0;
        buf[2] = 0x00;
        for b in &mut buf[3..] {
            *b = 0xFF;
        }
        assert!(matches!(
            EitSection::parse(&buf).unwrap_err(),
            Error::SectionLengthOverflow { .. }
        ));
    }

    #[test]
    fn parse_rejects_trailing_slack_bytes() {
        let section_length: u16 =
            (EXTENSION_HEADER_LEN + POST_EXTENSION_LEN + EVENT_HEADER_LEN + 1 + CRC_LEN) as u16;
        let mut v = Vec::new();
        v.push(TABLE_ID_PF_ACTUAL);
        v.push(super::super::SECTION_B1_FLAGS_DVB | ((section_length >> 8) as u8 & 0x0F));
        v.push((section_length & 0xFF) as u8);
        v.extend_from_slice(&1u16.to_be_bytes());
        v.push(0xC1);
        v.push(0);
        v.push(0);
        v.extend_from_slice(&0x0020u16.to_be_bytes());
        v.extend_from_slice(&0x0030u16.to_be_bytes());
        v.push(0);
        v.push(TABLE_ID_PF_ACTUAL);
        v.extend_from_slice(&1u16.to_be_bytes());
        v.extend_from_slice(&[0u8; 5]);
        v.extend_from_slice(&[0u8; 3]);
        v.push(0x00);
        v.push(0x00);
        v.push(0xFF);
        v.extend_from_slice(&[0u8; 4]);
        let err = EitSection::parse(&v).unwrap_err();
        assert!(matches!(
            err,
            Error::BufferTooShort {
                what: "EitSection trailing event bytes",
                ..
            }
        ));
    }
}
