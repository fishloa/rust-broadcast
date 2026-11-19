//! Public accessors and setters that the rest of the suite never calls: a
//! blank `MacAddress` display, a no-op `set_utc_time_decoded`, an
//! always-true `is_empty` or a hard-coded eviction count would otherwise ship
//! green.

use broadcast_common::Serialize;
use broadcast_common::time::MjdBcdDateTime;
use dvb_si::collect::{EitCollector, SectionSetCollector};
use dvb_si::descriptors::{AnyAitDescriptor, DescriptorLoop};
use dvb_si::tables::ait::{AitSection, ApplicationType};
use dvb_si::tables::eit::{EitKind, EitSection, TABLE_ID_SCHEDULE_ACTUAL_FIRST};
use dvb_si::tables::mpe::{Checksum, MacAddress};
use dvb_si::tables::pat::{PatEntry, PatSection};
use dvb_si::tables::tdt::TdtSection;
use dvb_si::tables::tot::TotSection;
use mpeg_ts::pid::Pid;

fn utc(year: u16, month: u8, day: u8, hour: u8, minute: u8, second: u8) -> MjdBcdDateTime {
    MjdBcdDateTime {
        year,
        month,
        day,
        hour,
        minute,
        second,
    }
}

#[test]
fn mac_address_display_and_byte_views() {
    let mac = MacAddress([0x01, 0x02, 0x03, 0x04, 0x05, 0x06]);
    assert_eq!(mac.to_string(), "01:02:03:04:05:06");
    assert_eq!(mac.as_bytes(), &[0x01, 0x02, 0x03, 0x04, 0x05, 0x06]);
    let via_as_ref: &[u8; 6] = mac.as_ref();
    assert_eq!(via_as_ref, &[0x01, 0x02, 0x03, 0x04, 0x05, 0x06]);
}

#[test]
fn checksum_byte_views() {
    let checksum = Checksum([0xDE, 0xAD, 0xBE, 0xEF]);
    assert_eq!(checksum.as_bytes(), &[0xDE, 0xAD, 0xBE, 0xEF]);
    let via_as_ref: &[u8; 4] = checksum.as_ref();
    assert_eq!(via_as_ref, &[0xDE, 0xAD, 0xBE, 0xEF]);
}

#[test]
fn tdt_set_utc_time_encodes_mjd_bcd_on_the_wire() {
    let mut tdt = TdtSection::new([0xE4, 0x09, 0x12, 0x34, 0x56]);
    tdt.set_utc_time_decoded(utc(2024, 12, 31, 23, 59, 59))
        .unwrap();
    assert_eq!(tdt.utc_time_decoded(), Some(utc(2024, 12, 31, 23, 59, 59)));

    let mut buf = vec![0u8; tdt.serialized_len()];
    tdt.serialize_into(&mut buf).unwrap();
    assert_eq!(&buf[3..8], &[0xED, 0x03, 0x23, 0x59, 0x59]);
}

#[test]
fn tot_set_utc_time_encodes_mjd_bcd_on_the_wire() {
    let mut tot = TotSection::new([0xE4, 0x09, 0x12, 0x34, 0x56], DescriptorLoop::new(&[]));
    tot.set_utc_time_decoded(utc(2024, 12, 31, 23, 59, 59))
        .unwrap();
    assert_eq!(tot.utc_time_decoded(), Some(utc(2024, 12, 31, 23, 59, 59)));

    let mut buf = vec![0u8; tot.serialized_len()];
    tot.serialize_into(&mut buf).unwrap();
    assert_eq!(&buf[3..8], &[0xED, 0x03, 0x23, 0x59, 0x59]);
}

#[test]
fn set_utc_time_rejects_unrepresentable_date_and_keeps_the_field() {
    let original = [0xE4, 0x09, 0x12, 0x34, 0x56];

    let mut tdt = TdtSection::new(original);
    assert_eq!(
        tdt.set_utc_time_decoded(utc(1900, 1, 1, 0, 0, 0)),
        Err(dvb_si::Error::ValueOutOfRange {
            field: "TdtSection::utc_time",
            reason: "date not representable in 16-bit MJD",
        })
    );
    assert_eq!(tdt.utc_time_raw(), original);

    let mut tot = TotSection::new(original, DescriptorLoop::new(&[]));
    assert_eq!(
        tot.set_utc_time_decoded(utc(1900, 1, 1, 0, 0, 0)),
        Err(dvb_si::Error::ValueOutOfRange {
            field: "TotSection::utc_time",
            reason: "date not representable in 16-bit MJD",
        })
    );
    assert_eq!(tot.utc_time_raw(), original);
}

#[test]
fn ait_common_ait_descriptors_walks_the_typed_loop() {
    let common = [0x16u8, 0x01, 0x01];
    let ait = AitSection::new(
        ApplicationType::HbbTv,
        false,
        dvb_si::tables::SectionVersion::new(1, true, 0, 0),
        DescriptorLoop::new(&common),
        vec![],
    );

    let walker = ait.common_ait_descriptors();
    assert_eq!(walker.raw(), &common[..]);
    let items: Vec<_> = walker.iter().collect();
    assert_eq!(items.len(), 1);
    match items[0].as_ref().unwrap() {
        AnyAitDescriptor::ApplicationUsage(usage) => assert_eq!(usage.usage_type, 0x01),
        other => panic!("expected ApplicationUsage, got {other:?}"),
    }
}

fn pat_section(section_number: u8, last_section_number: u8) -> Vec<u8> {
    let pat = PatSection::new(
        0x1111,
        2,
        true,
        section_number,
        last_section_number,
        vec![PatEntry::new(
            u16::from(section_number) + 1,
            Pid::from_wire_masked(0x0100),
        )],
    );
    let mut bytes = vec![0u8; pat.serialized_len()];
    pat.serialize_into(&mut bytes).unwrap();
    bytes
}

#[test]
fn collector_is_empty_tracks_retained_partial_sets() {
    let mut collector = SectionSetCollector::new();
    assert!(collector.is_empty());

    collector.push_section(pat_section(0, 1)).unwrap();
    assert_eq!(collector.len(), 1);
    assert!(!collector.is_empty());

    collector.clear();
    assert_eq!(collector.len(), 0);
    assert!(collector.is_empty());
}

fn eit_schedule_section(service_id: u16) -> Vec<u8> {
    let eit = EitSection::new(
        EitKind::ScheduleActual,
        TABLE_ID_SCHEDULE_ACTUAL_FIRST,
        service_id,
        dvb_si::tables::SectionVersion::new(4, true, 0, 0),
        0x2000,
        0x0001,
        0,
        TABLE_ID_SCHEDULE_ACTUAL_FIRST,
        vec![],
    );
    let mut bytes = vec![0u8; eit.serialized_len()];
    eit.serialize_into(&mut bytes).unwrap();
    bytes
}

#[test]
fn eit_schedule_eviction_count_starts_at_zero_then_counts() {
    let mut collector = EitCollector::new().with_max_logical_keys(1);
    assert_eq!(collector.schedules_evicted_for_capacity(), 0);

    assert!(
        collector
            .push_section(eit_schedule_section(0x0100))
            .unwrap()
            .is_some()
    );
    assert_eq!(collector.schedules_evicted_for_capacity(), 0);

    assert!(
        collector
            .push_section(eit_schedule_section(0x0200))
            .unwrap()
            .is_some()
    );
    assert_eq!(collector.schedules_evicted_for_capacity(), 1);
}
