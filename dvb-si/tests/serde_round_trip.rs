//! Serde coverage for the dvb_si tables.
//!
//! Owned tables (no `&'a [u8]` fields) are exercised with a full
//! `to_string` -> `from_str` JSON round-trip and `assert_eq!`.
//!
//! Borrowed tables cannot do a true JSON round-trip: JSON encodes `&[u8]`
//! as an array of integers, which is not contiguous in the wire form and
//! therefore cannot be borrowed back during deserialize. For those, this
//! file constructs the struct directly, calls `serde_json::to_string`,
//! and verifies the output is valid JSON containing the wire-relevant
//! top-level fields. Round-trip via a borrowing-friendly format
//! (postcard, bincode) is a separate concern.
#![cfg(feature = "serde")]

use dvb_common::Parse;
use dvb_si::tables::{
    ait::{Ait, AitApplication, ApplicationIdentifier},
    dsmcc::DsmccSection,
    eit::{Eit, EitEvent, EitKind},
    nit::{Nit, NitKind, NitTransportStream},
    pat::{Pat, PatEntry},
    pmt::{Pmt, PmtStream},
    sdt::{Sdt, SdtKind, SdtService},
    st::St,
    tdt::Tdt,
    tot::Tot,
};

// --- Owned tables: full JSON round-trip via a parsed fixture --------------

#[test]
fn pat_round_trips_via_json() {
    let bytes: Vec<u8> = vec![
        0x00, 0xB0, 0x0D, // table_id, section_length=13
        0x12, 0x34, // tsid
        0xC1, 0x00, 0x00, // version=0, current_next=1, section/last
        0x00, 0x01, 0xE1, 0x00, // program 1 -> PMT PID 0x100
        0x00, 0x00, 0x00, 0x00, // CRC placeholder
    ];
    let parsed = Pat::parse(&bytes).expect("parse PAT");
    let j = serde_json::to_string(&parsed).expect("serialize PAT");
    let back: Pat = serde_json::from_str(&j).expect("deserialize PAT");
    assert_eq!(parsed, back);
}

#[test]
fn tdt_round_trips_via_json() {
    let bytes: Vec<u8> = vec![
        0x70, 0x70, 0x05, // table_id=0x70, section_length=5
        0xDA, 0x06, 0x12, 0x34, 0x56, // 5-byte UTC time
    ];
    let parsed = Tdt::parse(&bytes).expect("parse TDT");
    let j = serde_json::to_string(&parsed).expect("serialize TDT");
    let back: Tdt = serde_json::from_str(&j).expect("deserialize TDT");
    assert_eq!(parsed, back);
}

#[test]
fn st_round_trips_via_json() {
    let bytes: Vec<u8> = vec![0x72, 0x30, 0x03, 0x00, 0x00, 0x00];
    let parsed = St::parse(&bytes).expect("parse ST");
    let j = serde_json::to_string(&parsed).expect("serialize ST");
    let back: St = serde_json::from_str(&j).expect("deserialize ST");
    assert_eq!(parsed, back);
}

#[test]
fn pat_with_multiple_entries_round_trips() {
    let p = Pat {
        transport_stream_id: 0x1234,
        version_number: 5,
        current_next_indicator: true,
        section_number: 0,
        last_section_number: 0,
        entries: vec![
            PatEntry { program_number: 0, pid: 0x10 },
            PatEntry { program_number: 1, pid: 0x100 },
            PatEntry { program_number: 2, pid: 0x200 },
        ],
    };
    let j = serde_json::to_string(&p).expect("serialize");
    let back: Pat = serde_json::from_str(&j).expect("deserialize");
    assert_eq!(p, back);
}

// --- Borrowed tables: serialize emits valid JSON --------------------------

fn assert_valid_json_with_keys(j: &str, keys: &[&str]) {
    let v: serde_json::Value = serde_json::from_str(j).expect("emitted JSON parses");
    assert!(v.is_object(), "expected JSON object, got {j}");
    for k in keys {
        assert!(v.get(k).is_some(), "missing key {k} in {j}");
    }
}

#[test]
fn pmt_serializes_to_valid_json() {
    let pmt = Pmt {
        program_number: 1,
        version_number: 0,
        current_next_indicator: true,
        pcr_pid: 0x64,
        program_info: &[],
        streams: vec![PmtStream {
            stream_type: 0x02,
            elementary_pid: 0xC8,
            es_info: &[],
        }],
    };
    let j = serde_json::to_string(&pmt).expect("serialize PMT");
    assert_valid_json_with_keys(&j, &["program_number", "pcr_pid", "streams"]);
}

#[test]
fn sdt_serializes_to_valid_json() {
    let sdt = Sdt {
        kind: SdtKind::Actual,
        transport_stream_id: 0x1234,
        version_number: 0,
        current_next_indicator: true,
        section_number: 0,
        last_section_number: 0,
        original_network_id: 0x0020,
        services: vec![SdtService {
            service_id: 1,
            eit_schedule_flag: false,
            eit_present_following_flag: true,
            running_status: 4,
            free_ca_mode: false,
            descriptors: &[],
        }],
    };
    let j = serde_json::to_string(&sdt).expect("serialize SDT");
    assert_valid_json_with_keys(&j, &["kind", "transport_stream_id", "services"]);
}

#[test]
fn eit_serializes_to_valid_json() {
    let eit = Eit {
        kind: EitKind::PresentFollowingActual,
        table_id: 0x4E,
        service_id: 0x0001,
        version_number: 0,
        current_next_indicator: true,
        section_number: 0,
        last_section_number: 0,
        transport_stream_id: 0x1234,
        original_network_id: 0x0020,
        segment_last_section_number: 0,
        last_table_id: 0x4E,
        events: vec![EitEvent {
            event_id: 0x0001,
            start_time_raw: [0xDA, 0x06, 0x12, 0x34, 0x56],
            duration_raw: [0x00, 0x00, 0x10],
            running_status: 4,
            free_ca_mode: false,
            descriptors: &[],
        }],
    };
    let j = serde_json::to_string(&eit).expect("serialize EIT");
    assert_valid_json_with_keys(&j, &["kind", "service_id", "events"]);
}

#[test]
fn nit_serializes_to_valid_json() {
    let nit = Nit {
        kind: NitKind::Actual,
        network_id: 0x0020,
        version_number: 0,
        current_next_indicator: true,
        section_number: 0,
        last_section_number: 0,
        network_descriptors: &[],
        transport_streams: vec![NitTransportStream {
            transport_stream_id: 0x1234,
            original_network_id: 0x0020,
            descriptors: &[],
        }],
    };
    let j = serde_json::to_string(&nit).expect("serialize NIT");
    assert_valid_json_with_keys(&j, &["kind", "network_id", "transport_streams"]);
}

#[test]
fn tot_serializes_to_valid_json() {
    let tot = Tot {
        utc_time_raw: [0xDA, 0x06, 0x12, 0x34, 0x56],
        descriptors: &[],
    };
    let j = serde_json::to_string(&tot).expect("serialize TOT");
    assert_valid_json_with_keys(&j, &["utc_time_raw", "descriptors"]);
}

#[test]
fn ait_serializes_to_valid_json() {
    let ait = Ait {
        application_type: 0x0010,
        version_number: 0,
        current_next_indicator: true,
        test_application_flag: false,
        section_number: 0,
        last_section_number: 0,
        common_descriptors: &[],
        applications: vec![AitApplication {
            identifier: ApplicationIdentifier {
                organisation_id: 0x0001_0001,
                application_id: 0x0001,
            },
            control_code: 1,
            descriptors: &[],
        }],
    };
    let j = serde_json::to_string(&ait).expect("serialize AIT");
    assert_valid_json_with_keys(&j, &["application_type", "applications"]);
}

#[test]
fn dsmcc_section_serializes_to_valid_json() {
    let bytes: Vec<u8> = vec![
        0x3A, 0xB0, 0x09, // table_id=0x3A (DSM-CC), section_length=9
        0x12, 0x34, 0xC1, 0x00, 0x00, // ext header
        0x00, 0x00, 0x00, 0x00, // CRC placeholder
    ];
    let section = DsmccSection::parse(&bytes).expect("parse DSM-CC section");
    let j = serde_json::to_string(&section).expect("serialize DSM-CC");
    assert_valid_json_with_keys(&j, &["table_id", "extension_id", "payload"]);
}
