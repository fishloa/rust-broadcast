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
//!
//! Coverage now also includes the companion tables — `Container`,
//! `MpeDatagramSection`, `MpeFec`, `MpeIfec`, `ProtectionMessageSection`,
//! `DownloadableFontInfoSection` — plus a DSM-CC object-carousel case
//! (`UnMessage::Dii`).
#![cfg(feature = "serde")]

use dvb_common::Parse;
use dvb_si::carousel::{Dii, DiiModule, UnMessage};
use dvb_si::tables::{
    ait::{Ait, AitApplication, ApplicationIdentifier},
    bat::{Bat, BatTransportStream},
    cat::Cat,
    cit::Cit,
    container::Container,
    dit::Dit,
    downloadable_font_info::{DownloadableFontInfoSection, FontInfo},
    dsmcc::DsmccSection,
    eit::{Eit, EitEvent, EitKind},
    int::Int,
    mpe::MpeDatagramSection,
    mpe_fec::{MpeFec, RealTimeParameters as MpeFecRtp},
    mpe_ifec::{MpeIfec, RealTimeParameters as MpeIfecRtp},
    nit::{Nit, NitKind, NitTransportStream},
    pat::{Pat, PatEntry},
    pmt::{Pmt, PmtStream},
    protection_message::{ProtectionMessageBody, ProtectionMessageSection},
    rct::Rct,
    rnt::Rnt,
    rst::{Rst, RstEntry},
    sat::Sat,
    sdt::{Sdt, SdtKind, SdtService},
    sit::Sit,
    st::St,
    tdt::Tdt,
    tot::Tot,
    tsdt::Tsdt,
    unt::Unt,
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
            PatEntry {
                program_number: 0,
                pid: 0x10,
            },
            PatEntry {
                program_number: 1,
                pid: 0x100,
            },
            PatEntry {
                program_number: 2,
                pid: 0x200,
            },
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

#[test]
fn cat_serializes_to_valid_json() {
    let cat = Cat {
        version_number: 0,
        current_next_indicator: true,
        section_number: 0,
        last_section_number: 0,
        descriptors: vec![0x09, 0x04, 0x06, 0x48, 0xE0, 0x50],
    };
    let j = serde_json::to_string(&cat).expect("serialize CAT");
    assert_valid_json_with_keys(&j, &["version_number", "descriptors"]);
}

#[test]
fn tsdt_serializes_to_valid_json() {
    let tsdt = Tsdt {
        table_id_extension: 0xFFFF,
        version_number: 0,
        current_next_indicator: true,
        section_number: 0,
        last_section_number: 0,
        descriptors: vec![0x42, 0x00],
    };
    let j = serde_json::to_string(&tsdt).expect("serialize TSDT");
    assert_valid_json_with_keys(&j, &["table_id_extension", "descriptors"]);
}

#[test]
fn bat_serializes_to_valid_json() {
    let bat = Bat {
        bouquet_id: 0x0001,
        version_number: 0,
        current_next_indicator: true,
        section_number: 0,
        last_section_number: 0,
        bouquet_descriptors: &[],
        transport_streams: vec![BatTransportStream {
            transport_stream_id: 0x1234,
            original_network_id: 0x0020,
            descriptors: &[],
        }],
    };
    let j = serde_json::to_string(&bat).expect("serialize BAT");
    assert_valid_json_with_keys(&j, &["bouquet_id", "transport_streams"]);
}

#[test]
fn rst_serializes_to_valid_json() {
    let rst = Rst {
        entries: vec![RstEntry {
            transport_stream_id: 0x1234,
            original_network_id: 0x0020,
            service_id: 0x0001,
            event_id: 0x0001,
            running_status: 4,
        }],
    };
    let j = serde_json::to_string(&rst).expect("serialize RST");
    assert_valid_json_with_keys(&j, &["entries"]);
}

#[test]
fn dit_serializes_to_valid_json() {
    let dit = Dit {
        transition_flag: true,
    };
    let j = serde_json::to_string(&dit).expect("serialize DIT");
    assert_valid_json_with_keys(&j, &["transition_flag"]);
}

#[test]
fn sit_serializes_to_valid_json() {
    let sit = Sit {
        table_id_extension: 0xFFFF,
        version_number: 0,
        current_next_indicator: true,
        section_number: 0,
        last_section_number: 0,
        transmission_info_descriptors: vec![],
        service_loop: vec![],
    };
    let j = serde_json::to_string(&sit).expect("serialize SIT");
    assert_valid_json_with_keys(&j, &["table_id_extension", "service_loop"]);
}

#[test]
fn sat_serializes_to_valid_json() {
    let sat = Sat {
        satellite_table_id: 0,
        table_count: 0,
        version_number: 0,
        current_next_indicator: true,
        section_number: 0,
        last_section_number: 0,
        body: &[],
    };
    let j = serde_json::to_string(&sat).expect("serialize SAT");
    assert_valid_json_with_keys(&j, &["satellite_table_id", "body"]);
}

#[test]
fn unt_serializes_to_valid_json() {
    let unt = Unt {
        action_type: 0x01,
        oui_hash: 0x00,
        version_number: 0,
        current_next_indicator: true,
        section_number: 0,
        last_section_number: 0,
        oui: 0x00_00_01,
        processing_order: 0,
        common_descriptors: &[],
        platform_loop: &[],
    };
    let j = serde_json::to_string(&unt).expect("serialize UNT");
    assert_valid_json_with_keys(&j, &["action_type", "oui", "platform_loop"]);
}

#[test]
fn int_serializes_to_valid_json() {
    let int = Int {
        action_type: 0x01,
        platform_id_hash: 0x00,
        version_number: 0,
        current_next_indicator: true,
        section_number: 0,
        last_section_number: 0,
        platform_id: 0x00_00_01,
        processing_order: 0,
        platform_descriptors: &[],
        loops: &[],
    };
    let j = serde_json::to_string(&int).expect("serialize INT");
    assert_valid_json_with_keys(&j, &["action_type", "platform_id", "loops"]);
}

#[test]
fn rct_serializes_to_valid_json() {
    let rct = Rct {
        table_id_extension_flag: false,
        service_id: 0x0001,
        version_number: 0,
        current_next_indicator: true,
        section_number: 0,
        last_section_number: 0,
        year_offset: 0x07D3,
        link_count: 0,
        link_info_loop: &[],
        descriptors: &[],
    };
    let j = serde_json::to_string(&rct).expect("serialize RCT");
    assert_valid_json_with_keys(&j, &["service_id", "year_offset", "link_info_loop"]);
}

#[test]
fn cit_serializes_to_valid_json() {
    let cit = Cit {
        private_indicator: false,
        service_id: 0x0001,
        version_number: 0,
        current_next_indicator: true,
        section_number: 0,
        last_section_number: 0,
        transport_stream_id: 0x1234,
        original_network_id: 0x0020,
        prepend_strings: &[],
        crid_entries: &[],
    };
    let j = serde_json::to_string(&cit).expect("serialize CIT");
    assert_valid_json_with_keys(&j, &["service_id", "prepend_strings", "crid_entries"]);
}

#[test]
fn rnt_serializes_to_valid_json() {
    let rnt = Rnt {
        context_id: 0x0001,
        version_number: 0,
        current_next_indicator: true,
        section_number: 0,
        last_section_number: 0,
        context_id_type: 0,
        common_descriptors: &[],
        resolution_providers: &[],
    };
    let j = serde_json::to_string(&rnt).expect("serialize RNT");
    assert_valid_json_with_keys(&j, &["context_id", "resolution_providers"]);
}

// --- Companion tables + carousel: serialize emits valid JSON --------------

#[test]
fn container_serializes_to_valid_json() {
    let container = Container {
        private_indicator: true,
        container_id: 0xBEEF,
        version_number: 9,
        current_next_indicator: true,
        section_number: 0,
        last_section_number: 0,
        container_data: &[0xCA, 0xFE],
    };
    let j = serde_json::to_string(&container).expect("serialize Container");
    assert_valid_json_with_keys(&j, &["container_id", "version_number", "container_data"]);
}

#[test]
fn mpe_datagram_section_serializes_to_valid_json() {
    let mpe = MpeDatagramSection {
        section_syntax_indicator: false,
        private_indicator: true,
        mac_address: [0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F],
        payload_scrambling_control: 0b01,
        address_scrambling_control: 0b11,
        llc_snap_flag: true,
        current_next_indicator: true,
        section_number: 3,
        last_section_number: 7,
        payload: &[0xAB, 0xCD],
        checksum: [0xDE, 0xAD, 0xBE, 0xEF],
    };
    let j = serde_json::to_string(&mpe).expect("serialize MpeDatagramSection");
    assert_valid_json_with_keys(&j, &["mac_address", "payload", "checksum"]);
}

#[test]
fn mpe_fec_serializes_to_valid_json() {
    let mpe_fec = MpeFec {
        private_indicator: true,
        padding_columns: 12,
        current_next_indicator: true,
        section_number: 0,
        last_section_number: 0,
        real_time_parameters: MpeFecRtp {
            delta_t: 0x0ABC,
            table_boundary: true,
            frame_boundary: false,
            address: 0x0001_2345,
        },
        rs_data: &[0x01, 0x02],
    };
    let j = serde_json::to_string(&mpe_fec).expect("serialize MpeFec");
    assert_valid_json_with_keys(&j, &["padding_columns", "real_time_parameters", "rs_data"]);
}

#[test]
fn mpe_ifec_serializes_to_valid_json() {
    let mpe_ifec = MpeIfec {
        private_indicator: true,
        burst_number: 7,
        ifec_burst_size: 8,
        version: 3,
        current_next_indicator: true,
        section_number: 0,
        last_section_number: 0,
        real_time_parameters: MpeIfecRtp {
            delta_t: 0x0ABC,
            mpe_boundary: true,
            frame_boundary: false,
            prev_burst_size: 0x0001_2345,
        },
        ifec_data: &[0x01, 0x02],
    };
    let j = serde_json::to_string(&mpe_ifec).expect("serialize MpeIfec");
    assert_valid_json_with_keys(&j, &["burst_number", "real_time_parameters", "ifec_data"]);
}

#[test]
fn protection_message_section_serializes_to_valid_json() {
    let pms = ProtectionMessageSection {
        table_id_extension: 0x0100,
        version_number: 5,
        current_next_indicator: true,
        section_number: 0,
        last_section_number: 0,
        body: ProtectionMessageBody::CertificateCollection {
            certificates: vec![&[0x30, 0x82, 0x01, 0x02], &[0xAB, 0xCD]],
        },
    };
    let j = serde_json::to_string(&pms).expect("serialize ProtectionMessageSection");
    assert_valid_json_with_keys(&j, &["table_id_extension", "body"]);
}

#[test]
fn downloadable_font_info_section_serializes_to_valid_json() {
    let dfis = DownloadableFontInfoSection {
        font_id_extension: 0,
        font_id: 0x42,
        version_number: 0,
        current_next_indicator: true,
        section_number: 0,
        last_section_number: 0,
        font_info: vec![
            FontInfo::StyleWeight {
                style: 2,
                weight: 2,
            },
            FontInfo::FileUri {
                format: 1,
                uri: b"https://f.example/Droid.otf",
            },
        ],
    };
    let j = serde_json::to_string(&dfis).expect("serialize DownloadableFontInfoSection");
    assert_valid_json_with_keys(&j, &["font_id", "font_info"]);
}

#[test]
fn un_message_dii_serializes_to_valid_json() {
    let dii = UnMessage::Dii(Dii {
        transaction_id: 0x8002_0002,
        adaptation: &[],
        download_id: 0x0000_00AB,
        block_size: 4066,
        window_size: 0,
        ack_period: 0,
        t_c_download_window: 0,
        t_c_download_scenario: 0,
        compatibility_descriptor: &[],
        modules: vec![DiiModule {
            module_id: 1,
            module_size: 8000,
            module_version: 3,
            module_info: &[0xDE, 0xAD],
        }],
        private_data: &[],
    });
    // The enum is externally tagged: the JSON top-level object carries a
    // single `Dii` key whose value is the message body.
    let j = serde_json::to_string(&dii).expect("serialize UnMessage::Dii");
    assert_valid_json_with_keys(&j, &["Dii"]);
}
