use dvb_common::Serialize;
use dvb_si::collect::{CollectError, CompletedEit, EitCollector, SectionSetCollector};
use dvb_si::descriptors::AnyDescriptor;
use dvb_si::descriptors::DescriptorLoop;
use dvb_si::tables::eit::{EitKind, EitSection, TABLE_ID_SCHEDULE_ACTUAL_FIRST as EIT_50};
use dvb_si::tables::nit::{NitKind, NitSection, NitTransportStream};
use dvb_si::tables::pat::{PatEntry, PatSection};

fn nit_section(
    section_number: u8,
    last_section_number: u8,
    network_descriptors: &[u8],
    transport_streams: Vec<NitTransportStream<'_>>,
) -> Vec<u8> {
    let nit = NitSection {
        kind: NitKind::Actual,
        network_id: 0x0001,
        version_number: 3,
        current_next_indicator: true,
        section_number,
        last_section_number,
        network_descriptors: DescriptorLoop::new(network_descriptors),
        transport_streams,
    };
    let mut bytes = vec![0u8; nit.serialized_len()];
    nit.serialize_into(&mut bytes).unwrap();
    bytes
}

fn eit_schedule_section(table_id: u8, last_table_id: u8) -> Vec<u8> {
    eit_schedule_section_with_version(table_id, last_table_id, 4)
}

fn eit_schedule_section_with_version(
    table_id: u8,
    last_table_id: u8,
    version_number: u8,
) -> Vec<u8> {
    let eit = EitSection {
        kind: EitKind::ScheduleActual,
        table_id,
        service_id: 0x0100,
        version_number,
        current_next_indicator: true,
        section_number: 0,
        last_section_number: 0,
        transport_stream_id: 0x2000,
        original_network_id: 0x0001,
        segment_last_section_number: 0,
        last_table_id,
        events: vec![],
    };
    let mut bytes = vec![0u8; eit.serialized_len()];
    eit.serialize_into(&mut bytes).unwrap();
    bytes
}

fn pat_section(section_number: u8, last_section_number: u8, pid: u16) -> Vec<u8> {
    pat_section_versioned(2, section_number, last_section_number, pid)
}

fn pat_section_versioned(
    version_number: u8,
    section_number: u8,
    last_section_number: u8,
    pid: u16,
) -> Vec<u8> {
    let pat = PatSection {
        transport_stream_id: 0x1111,
        version_number,
        current_next_indicator: true,
        section_number,
        last_section_number,
        entries: vec![PatEntry {
            program_number: section_number as u16 + 1,
            pid,
        }],
    };
    let mut bytes = vec![0u8; pat.serialized_len()];
    pat.serialize_into(&mut bytes).unwrap();
    bytes
}

#[test]
fn nit_collector_emits_only_when_all_sections_are_present() {
    let network_name = [0x40, 0x02, b'A', b'B'];
    let section0 = nit_section(
        0,
        1,
        &network_name,
        vec![NitTransportStream {
            transport_stream_id: 0x1000,
            original_network_id: 0x0001,
            descriptors: DescriptorLoop::new(&[]),
        }],
    );
    let section1 = nit_section(
        1,
        1,
        &network_name,
        vec![NitTransportStream {
            transport_stream_id: 0x1001,
            original_network_id: 0x0001,
            descriptors: DescriptorLoop::new(&[]),
        }],
    );

    let mut collector = SectionSetCollector::new();
    assert!(collector.push_section(&section0).unwrap().is_none());

    let complete_set = collector
        .push_section(&section1)
        .unwrap()
        .expect("second section completes NIT");
    let nit = complete_set.nit().unwrap();

    let tsids: Vec<_> = nit
        .transport_streams
        .iter()
        .map(|ts| ts.transport_stream_id)
        .collect();
    assert_eq!(tsids, [0x1000, 0x1001]);
}

#[test]
fn generic_complete_table_handles_pat_sections() {
    let section0 = pat_section(0, 1, 0x0100);
    let section1 = pat_section(1, 1, 0x0101);

    let mut collector = SectionSetCollector::new();
    assert!(collector.push_section(&section1).unwrap().is_none());

    let complete_set = collector
        .push_section(&section0)
        .unwrap()
        .expect("PAT completes when both sections are present");
    let pat = complete_set.table::<PatSection>().unwrap();

    assert_eq!(pat.meta().version_number, 2);
    let pids: Vec<_> = pat
        .sections()
        .iter()
        .flat_map(|section| section.entries.iter().map(|entry| entry.pid))
        .collect();
    assert_eq!(pids, [0x0100, 0x0101]);
}

#[test]
fn collector_rejects_crc_mismatch() {
    let mut section = pat_section(0, 0, 0x0100);
    let payload_byte = 10;
    section[payload_byte] ^= 0x01;

    let mut collector = SectionSetCollector::new();
    let err = collector.push_section(&section).unwrap_err();
    assert!(matches!(
        err,
        CollectError::Section(dvb_si::Error::CrcMismatch { .. })
    ));
}

#[test]
fn complete_nit_exposes_typed_descriptors() {
    let network_name = [0x40, 0x02, b'A', b'B'];
    let section = nit_section(0, 0, &network_name, vec![]);

    let mut collector = SectionSetCollector::new();
    let complete_set = collector
        .push_section(&section)
        .unwrap()
        .expect("single-section NIT completes immediately");
    let nit = complete_set.nit().unwrap();

    assert!(matches!(
        nit.network_descriptors.descriptors().first(),
        Some(Ok(AnyDescriptor::NetworkName(_)))
    ));
    assert_eq!(nit.network_descriptors.raw().raw(), &network_name);
}

#[test]
fn eit_schedule_collector_waits_for_all_table_ids_through_last_table_id() {
    let section50 = eit_schedule_section(EIT_50, EIT_50 + 1);
    let section51 = eit_schedule_section(EIT_50 + 1, EIT_50 + 1);

    let mut collector = EitCollector::new();
    assert!(collector.push_section(&section50).unwrap().is_none());

    let completed = collector
        .push_section(&section51)
        .unwrap()
        .expect("second schedule table_id completes EIT schedule");
    let CompletedEit::Schedule(schedule) = completed else {
        panic!("expected completed schedule EIT");
    };
    assert_eq!(schedule.first_table_id(), EIT_50);
    assert_eq!(schedule.last_table_id(), EIT_50 + 1);
    assert_eq!(schedule.table_sets().len(), 2);

    let tables = schedule.tables().unwrap();
    let table_ids: Vec<_> = tables.iter().map(|eit| eit.table_id).collect();
    assert_eq!(table_ids, [EIT_50, EIT_50 + 1]);
}

#[test]
fn eit_schedule_collector_allows_per_table_id_versions() {
    let section50 = eit_schedule_section_with_version(EIT_50, EIT_50 + 1, 4);
    let section51 = eit_schedule_section_with_version(EIT_50 + 1, EIT_50 + 1, 5);

    let mut collector = EitCollector::new();
    assert!(collector.push_section(&section50).unwrap().is_none());

    let completed = collector
        .push_section(&section51)
        .unwrap()
        .expect("second schedule table_id completes EIT schedule");
    let CompletedEit::Schedule(schedule) = completed else {
        panic!("expected completed schedule EIT");
    };

    let versions: Vec<_> = schedule.table_versions().collect();
    assert_eq!(versions, [(EIT_50, 4), (EIT_50 + 1, 5)]);
}

#[test]
fn eit_collector_retain_logical_prunes_schedule_state() {
    let section50 = eit_schedule_section(EIT_50, EIT_50 + 1);
    let section51 = eit_schedule_section(EIT_50 + 1, EIT_50 + 1);

    let mut collector = EitCollector::new();
    assert!(collector.push_section(&section50).unwrap().is_none());
    assert!(collector.push_section(&section51).unwrap().is_some());
    assert_eq!(collector.section_set_len(), 2);
    assert_eq!(collector.schedule_len(), 1);

    collector.retain_logical(|key| key.service_id != 0x0100);

    assert_eq!(collector.section_set_len(), 0);
    assert_eq!(collector.schedule_len(), 0);
}

// A complete set is emitted exactly once: re-pushing already-present sections
// of the same version must not re-emit (the `emitted` flag on the retained
// partial set). Pins the emit-once half of the 4.0 SectionSetCollector contract.
#[test]
fn collector_emits_complete_once_then_none() {
    let section0 = pat_section(0, 1, 0x0100);
    let section1 = pat_section(1, 1, 0x0101);

    let mut collector = SectionSetCollector::new();
    assert!(collector.push_section(&section0).unwrap().is_none());

    // First time every section is present -> emit.
    assert!(
        collector.push_section(&section1).unwrap().is_some(),
        "completing the set emits the complete table",
    );

    // Re-pushing already-present sections of the same version must NOT re-emit.
    assert!(
        collector.push_section(&section1).unwrap().is_none(),
        "re-pushing the completing section does not re-emit (emit-once)",
    );
    assert!(
        collector.push_section(&section0).unwrap().is_none(),
        "re-pushing an earlier section does not re-emit (emit-once)",
    );
}

// A new `version_number` for the same logical table drops the stale partial,
// re-collects, and emits the new complete table. The key excludes version but
// includes current_next_indicator, so cni is held constant here (a cni flip is
// a *different* logical table and keys separately). Asserts via the public API
// — `meta().version_number` and a follow-up emit-once check that the entry was
// reset in place (one key, no duplicate accumulation) — never `len()`.
#[test]
fn collector_reemits_on_version_bump() {
    let v1_section0 = pat_section_versioned(1, 0, 1, 0x0100);
    let v1_section1 = pat_section_versioned(1, 1, 1, 0x0101);
    let v2_section0 = pat_section_versioned(2, 0, 1, 0x0200);
    let v2_section1 = pat_section_versioned(2, 1, 1, 0x0201);

    let mut collector = SectionSetCollector::new();

    assert!(collector.push_section(&v1_section0).unwrap().is_none());
    let v1 = collector
        .push_section(&v1_section1)
        .unwrap()
        .expect("version 1 completes once both sections are present");
    assert_eq!(v1.meta().version_number, 1);

    // The new version resets the stale v1 partial in place; its first section
    // does not complete the set on its own.
    assert!(
        collector.push_section(&v2_section0).unwrap().is_none(),
        "first section of the new version re-collects from scratch",
    );
    let v2 = collector
        .push_section(&v2_section1)
        .unwrap()
        .expect("version 2 completes after re-collection");
    assert_eq!(v2.meta().version_number, 2);

    // Emit-once still holds for the re-collected version: the v2 partial reused
    // the single retained entry rather than accumulating a duplicate.
    assert!(
        collector.push_section(&v2_section1).unwrap().is_none(),
        "re-collected version is also emit-once (single retained entry per key)",
    );
}
