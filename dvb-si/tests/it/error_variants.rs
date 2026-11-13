use broadcast_common::Serialize;
use dvb_si::Error;
use dvb_si::collect::{CollectError, EitCollector, SectionSetCollector};
use dvb_si::tables::eit::{EitKind, EitSection, TABLE_ID_SCHEDULE_ACTUAL_FIRST as EIT_50};
use dvb_si::tables::pat::{PatEntry, PatSection};
use mpeg_ts::pid::Pid;

fn pat_section(version: u8, section_number: u8, last_section_number: u8, pid: u16) -> Vec<u8> {
    let pat = PatSection::new(
        0x1111,
        version,
        true,
        section_number,
        last_section_number,
        vec![PatEntry::new(1, Pid::new(pid))],
    );
    let mut bytes = vec![0u8; pat.serialized_len()];
    pat.serialize_into(&mut bytes).unwrap();
    bytes
}

fn eit_schedule(table_id: u8, last_table_id: u8) -> Vec<u8> {
    let eit = EitSection::new(
        EitKind::ScheduleActual,
        table_id,
        0x0001,
        dvb_si::tables::SectionVersion::new(1, true, 0, 0),
        0x2000,
        0x0001,
        0,
        last_table_id,
        vec![],
    );
    let mut bytes = vec![0u8; eit.serialized_len()];
    eit.serialize_into(&mut bytes).unwrap();
    bytes
}

#[test]
fn a_short_form_section_cannot_be_collected() {
    let tdt = [0x70, 0x70, 0x05, 0xE4, 0x09, 0x12, 0x34, 0x56];
    let expected = CollectError::ShortFormSection { table_id: 0x70 };
    assert_eq!(
        SectionSetCollector::new().push_section(tdt).unwrap_err(),
        expected
    );
    assert_eq!(EitCollector::new().push_section(tdt).unwrap_err(), expected);
}

#[test]
fn a_section_number_beyond_last_section_number_is_out_of_range() {
    let section = pat_section(2, 2, 1, 0x0100);
    let expected = CollectError::SectionNumberOutOfRange {
        table_id: 0x00,
        section_number: 2,
        last_section_number: 1,
    };
    assert_eq!(
        SectionSetCollector::new()
            .push_section(&section)
            .unwrap_err(),
        expected
    );
}

#[test]
fn different_bytes_for_the_same_slot_and_version_conflict() {
    let mut collector = SectionSetCollector::new();
    collector
        .push_section(pat_section(2, 0, 1, 0x0100))
        .unwrap();
    assert_eq!(
        collector
            .push_section(pat_section(2, 0, 1, 0x0200))
            .unwrap_err(),
        CollectError::ConflictingSection {
            table_id: 0x00,
            section_number: 0,
        }
    );
}

#[test]
fn an_eit_schedule_table_id_beyond_last_table_id_is_out_of_range() {
    let section = eit_schedule(EIT_50 + 1, EIT_50);
    assert_eq!(
        EitCollector::new().push_section(&section).unwrap_err(),
        CollectError::EitTableIdOutOfRange {
            table_id: EIT_50 + 1,
            first_table_id: EIT_50,
            last_table_id: EIT_50,
        }
    );
}

#[test]
fn a_ts_packet_without_the_sync_byte_converts_to_invalid_sync() {
    let ts_err = mpeg_ts::ts::TsPacket::parse(&[0u8; 188]).unwrap_err();
    assert_eq!(
        Error::from(ts_err),
        Error::InvalidSync(broadcast_common::error::InvalidSync {
            what: "TS sync byte",
            expected: 0x47,
            found: 0,
        })
    );
}
