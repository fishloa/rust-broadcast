//! Serde coverage for the dvb_si tables.
//!
//! Serde is **Serialize-only** across the workspace (3.0.1): JSON is a
//! display/export format and parsing FROM JSON is deliberately unsupported —
//! re-parse from the wire bytes instead. Every table is therefore exercised by
//! constructing it (directly or via a parsed fixture), calling
//! `serde_json::to_string`/`to_value`, and asserting the serialized shape
//! (valid JSON with the wire-relevant fields / typed sub-values).
//!
//! Coverage now also includes the companion tables — `ContainerSection`,
//! `MpeDatagramSection`, `MpeFecSection`, `MpeIfecSection`, `ProtectionMessageSection`,
//! `DownloadableFontInfoSection` — plus a DSM-CC object-carousel case
//! (`UnMessage::Dii`).
#![cfg(feature = "serde")]

use broadcast_common::Parse;
use dvb_si::carousel::{Dii, DiiModule, UnMessage};
use dvb_si::compatibility::CompatibilityDescriptor;
use dvb_si::descriptors::DescriptorLoop;
use dvb_si::tables::RunningStatus;
use dvb_si::tables::{
    ait::{AitApplication, AitSection, ApplicationIdentifier, ApplicationType, ControlCode},
    bat::{BatSection, BatTransportStream},
    cat::CatSection,
    cit::CitSection,
    container::ContainerSection,
    dit::DitSection,
    downloadable_font_info::{DownloadableFontInfoSection, FontInfo},
    dsmcc::DsmccSection,
    eit::{EitEvent, EitKind, EitSection},
    int::{IntActionType, IntSection},
    mpe::{Checksum, MacAddress, MpeDatagramSection},
    mpe_fec::{MpeFecSection, RealTimeParameters as MpeFecRtp},
    mpe_ifec::{MpeIfecSection, RealTimeParameters as MpeIfecRtp},
    nit::{NitKind, NitSection, NitTransportStream},
    pat::{PatEntry, PatSection},
    pmt::{PmtSection, PmtStream, StreamType},
    protection_message::{ProtectionMessageBody, ProtectionMessageSection},
    rct::RctSection,
    rnt::{ContextIdType, RntSection},
    rst::{RstEntry, RstSection},
    sat::SatBody,
    sat::SatSection,
    sdt::{SdtKind, SdtSection, SdtService},
    sit::SitSection,
    st::StSection,
    tdt::TdtSection,
    tot::TotSection,
    tsdt::TsdtSection,
    unt::{UntActionType, UntSection},
};
use dvb_si::text::{DvbText, LangCode};
use mpeg_ts::pid::Pid;

// --- Tables: serialize emits valid JSON -----------------------------------

#[test]
fn pat_serializes_to_valid_json() {
    let bytes: Vec<u8> = vec![
        0x00, 0xB0, 0x0D, // table_id, section_length=13
        0x12, 0x34, // tsid
        0xC1, 0x00, 0x00, // version=0, current_next=1, section/last
        0x00, 0x01, 0xE1, 0x00, // program 1 -> PMT PID 0x100
        0x00, 0x00, 0x00, 0x00, // CRC placeholder
    ];
    let parsed = PatSection::parse(&bytes).expect("parse PAT");
    let j = serde_json::to_string(&parsed).expect("serialize PAT");
    assert_valid_json_with_keys(&j, &["transport_stream_id", "entries"]);
}

#[test]
fn tdt_serializes_to_valid_json() {
    let bytes: Vec<u8> = vec![
        0x70, 0x70, 0x05, // table_id=0x70, section_length=5
        0xDA, 0x06, 0x12, 0x34, 0x56, // 5-byte UTC time
    ];
    let parsed = TdtSection::parse(&bytes).expect("parse TDT");
    let j = serde_json::to_string(&parsed).expect("serialize TDT");
    assert_valid_json_with_keys(&j, &["utc_time_raw"]);
}

#[test]
fn st_serializes_to_valid_json() {
    let bytes: Vec<u8> = vec![0x72, 0x30, 0x03, 0x00, 0x00, 0x00];
    let parsed = StSection::parse(&bytes).expect("parse ST");
    let j = serde_json::to_string(&parsed).expect("serialize ST");
    assert_valid_json_with_keys(&j, &["payload"]);
}

#[test]
fn pat_with_multiple_entries_serializes() {
    let p = PatSection::new(
        0x1234,
        5,
        true,
        0,
        0,
        vec![
            PatEntry::new(0, Pid::new(0x10)),
            PatEntry::new(1, Pid::new(0x100)),
            PatEntry::new(2, Pid::new(0x200)),
        ],
    );
    let v = serde_json::to_value(&p).expect("serialize");
    assert_eq!(v["transport_stream_id"], 0x1234);
    assert_eq!(v["entries"].as_array().unwrap().len(), 3);
    assert_eq!(v["entries"][1]["program_number"], 1);
    assert_eq!(v["entries"][1]["pid"], 0x100);
}

fn assert_valid_json_with_keys(j: &str, keys: &[&str]) {
    let v: serde_json::Value = serde_json::from_str(j).expect("emitted JSON parses");
    assert!(v.is_object(), "expected JSON object, got {j}");
    for k in keys {
        assert!(v.get(k).is_some(), "missing key {k} in {j}");
    }
}

#[test]
fn pmt_serializes_to_valid_json() {
    let pmt = PmtSection::new(
        1,
        0,
        true,
        0,
        0,
        Pid::new(0x64),
        DescriptorLoop::new(&[]),
        vec![PmtStream::new(
            StreamType::Mpeg2Video,
            Pid::new(0xC8),
            DescriptorLoop::new(&[]),
        )],
    );
    let j = serde_json::to_string(&pmt).expect("serialize PMT");
    assert_valid_json_with_keys(&j, &["program_number", "pcr_pid", "streams"]);
}

#[test]
fn sdt_serializes_to_valid_json() {
    let sdt = SdtSection::new(
        SdtKind::Actual,
        0x1234,
        0,
        true,
        0,
        0,
        0x0020,
        vec![SdtService::new(
            1,
            false,
            true,
            RunningStatus::Running,
            false,
            DescriptorLoop::new(&[]),
        )],
    );
    let j = serde_json::to_string(&sdt).expect("serialize SDT");
    assert_valid_json_with_keys(&j, &["kind", "transport_stream_id", "services"]);
}

/// 3.0 typed descriptor loop: an `SdtService` whose `descriptors`
/// `DescriptorLoop` carries a service_descriptor (tag 0x48) serializes such
/// that `services[0].descriptors[0].service.service_name` is the decoded name.
#[test]
fn sdt_service_descriptor_loop_serializes_decoded_name() {
    // service_descriptor: tag 0x48, len, service_type=0x01,
    // provider_name_length=3 "BBC", service_name_length=3 "ONE".
    let raw_loop: &[u8] = &[
        0x48, 0x09, 0x01, 0x03, b'B', b'B', b'C', 0x03, b'O', b'N', b'E',
    ];
    let sdt = SdtSection::new(
        SdtKind::Actual,
        0x1234,
        0,
        true,
        0,
        0,
        0x0020,
        vec![SdtService::new(
            1,
            false,
            true,
            RunningStatus::Running,
            false,
            DescriptorLoop::new(raw_loop),
        )],
    );
    let v = serde_json::to_value(&sdt).expect("serialize SDT");
    assert_eq!(
        v["services"][0]["descriptors"][0]["service"]["service_name"],
        "ONE"
    );
    assert_eq!(
        v["services"][0]["descriptors"][0]["service"]["provider_name"],
        "BBC"
    );
}

#[test]
fn eit_serializes_to_valid_json() {
    let eit = EitSection::new(
        EitKind::PresentFollowingActual,
        0x4E,
        0x0001,
        0,
        true,
        0,
        0,
        0x1234,
        0x0020,
        0,
        0x4E,
        vec![EitEvent::new(
            0x0001,
            [0xDA, 0x06, 0x12, 0x34, 0x56],
            [0x00, 0x00, 0x10],
            RunningStatus::Running,
            false,
            DescriptorLoop::new(&[]),
        )],
    );
    let j = serde_json::to_string(&eit).expect("serialize EIT");
    assert_valid_json_with_keys(&j, &["kind", "service_id", "events"]);
}

#[test]
fn nit_serializes_to_valid_json() {
    let nit = NitSection::new(
        NitKind::Actual,
        0x0020,
        0,
        true,
        0,
        0,
        DescriptorLoop::new(&[]),
        vec![NitTransportStream::new(
            0x1234,
            0x0020,
            DescriptorLoop::new(&[]),
        )],
    );
    let j = serde_json::to_string(&nit).expect("serialize NIT");
    assert_valid_json_with_keys(&j, &["kind", "network_id", "transport_streams"]);
}

#[test]
fn tot_serializes_to_valid_json() {
    let tot = TotSection::new([0xDA, 0x06, 0x12, 0x34, 0x56], DescriptorLoop::new(&[]));
    let j = serde_json::to_string(&tot).expect("serialize TOT");
    assert_valid_json_with_keys(&j, &["utc_time_raw", "descriptors"]);
}

#[test]
fn ait_serializes_to_valid_json() {
    let ait = AitSection::new(
        ApplicationType::HbbTv,
        false,
        0,
        true,
        0,
        0,
        DescriptorLoop::new(&[]),
        vec![AitApplication::new(
            ApplicationIdentifier {
                organisation_id: 0x0001_0001,
                application_id: 0x0001,
            },
            ControlCode::Autostart,
            DescriptorLoop::new(&[]),
        )],
    );
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
    let cat = CatSection::new(
        0,
        true,
        0,
        0,
        DescriptorLoop::new(&[0x09, 0x04, 0x06, 0x48, 0xE0, 0x50]),
    );
    let j = serde_json::to_string(&cat).expect("serialize CAT");
    assert_valid_json_with_keys(&j, &["version_number", "descriptors"]);
}

#[test]
fn tsdt_serializes_to_valid_json() {
    let tsdt = TsdtSection::new(0xFFFF, 0, true, 0, 0, DescriptorLoop::new(&[0x42, 0x00]));
    let j = serde_json::to_string(&tsdt).expect("serialize TSDT");
    assert_valid_json_with_keys(&j, &["table_id_extension", "descriptors"]);
}

#[test]
fn bat_serializes_to_valid_json() {
    let bat = BatSection::new(
        0x0001,
        0,
        true,
        0,
        0,
        DescriptorLoop::new(&[]),
        vec![BatTransportStream::new(
            0x1234,
            0x0020,
            DescriptorLoop::new(&[]),
        )],
    );
    let j = serde_json::to_string(&bat).expect("serialize BAT");
    assert_valid_json_with_keys(&j, &["bouquet_id", "transport_streams"]);
}

#[test]
fn rst_serializes_to_valid_json() {
    let rst = RstSection::new(vec![RstEntry::new(
        0x1234,
        0x0020,
        0x0001,
        0x0001,
        RunningStatus::Running,
    )]);
    let j = serde_json::to_string(&rst).expect("serialize RST");
    assert_valid_json_with_keys(&j, &["entries"]);
}

#[test]
fn dit_serializes_to_valid_json() {
    let dit = DitSection::new(true);
    let j = serde_json::to_string(&dit).expect("serialize DIT");
    assert_valid_json_with_keys(&j, &["transition_flag"]);
}

#[test]
fn sit_serializes_to_valid_json() {
    use dvb_si::tables::sit::SitService;
    let sit = SitSection::new(
        0xFFFF,
        0,
        true,
        0,
        0,
        DescriptorLoop::new(&[]),
        vec![SitService::new(
            0x0001,
            RunningStatus::Running,
            DescriptorLoop::new(&[
                0x48, 0x09, 0x01, 0x03, b'B', b'B', b'C', 0x03, b'O', b'N', b'E',
            ]),
        )],
    );
    let v = serde_json::to_value(&sit).expect("serialize SIT");
    assert!(v.is_object(), "expected JSON object, got {v}");
    assert!(v["transmission_info_descriptors"].is_array());
    assert_eq!(v["services"][0]["service_id"], 1);
    assert_eq!(v["services"][0]["running_status"], "Running");
    // Typed descriptors render inside the service.
    assert_eq!(
        v["services"][0]["descriptors"][0]["service"]["service_name"],
        "ONE"
    );
}

#[test]
fn sat_serializes_to_valid_json() {
    let sat = SatSection::new(0, true, 0, 0, true, 0, 0, SatBody::Raw(vec![]));
    let j = serde_json::to_string(&sat).expect("serialize SAT");
    assert_valid_json_with_keys(&j, &["satellite_table_id", "body"]);
}

#[test]
fn unt_serializes_to_valid_json() {
    let unt = UntSection::new(
        UntActionType::SystemSoftwareUpdate,
        0x00,
        0,
        true,
        0,
        0,
        0x00_00_01,
        0,
        dvb_si::descriptors::unt::UntDescriptorLoop::new(&[]),
        vec![],
    );
    let j = serde_json::to_string(&unt).expect("serialize UNT");
    assert_valid_json_with_keys(&j, &["action_type", "oui", "platforms"]);
}

#[test]
fn int_serializes_to_valid_json() {
    let int = IntSection::new(
        IntActionType::IpMacStreamLocation,
        0x00,
        0,
        true,
        0,
        0,
        0x00_00_01,
        0,
        dvb_si::descriptors::int::IntDescriptorLoop::new(&[]),
        vec![],
    );
    let j = serde_json::to_string(&int).expect("serialize INT");
    assert_valid_json_with_keys(&j, &["action_type", "platform_id", "loops"]);
}

#[test]
fn rct_serializes_to_valid_json() {
    let rct = RctSection::new(
        false,
        0x0001,
        0,
        true,
        0,
        0,
        0x07D3,
        vec![],
        DescriptorLoop::new(&[]),
    );
    let j = serde_json::to_string(&rct).expect("serialize RCT");
    assert_valid_json_with_keys(&j, &["service_id", "year_offset", "links"]);
}

#[test]
fn cit_serializes_to_valid_json() {
    let cit = CitSection::new(
        false,
        0x0001,
        0,
        true,
        0,
        0,
        0x1234,
        0x0020,
        DvbText::new(&[]),
        vec![],
    );
    let j = serde_json::to_string(&cit).expect("serialize CIT");
    assert_valid_json_with_keys(&j, &["service_id", "prepend_strings", "crid_entries"]);
}

#[test]
fn rnt_serializes_to_valid_json() {
    let rnt = RntSection::new(
        0x0001,
        0,
        true,
        0,
        0,
        ContextIdType::BouquetId,
        dvb_si::descriptors::rnt::RntDescriptorLoop::new(&[]),
        vec![],
    );
    let j = serde_json::to_string(&rnt).expect("serialize RNT");
    assert_valid_json_with_keys(&j, &["context_id", "resolution_providers"]);
}

// --- Companion tables + carousel: serialize emits valid JSON --------------

#[test]
fn container_serializes_to_valid_json() {
    let container = ContainerSection::new(true, 0xBEEF, 9, true, 0, 0, &[0xCA, 0xFE]);
    let j = serde_json::to_string(&container).expect("serialize ContainerSection");
    assert_valid_json_with_keys(&j, &["container_id", "version_number", "container_data"]);
}

#[test]
fn mpe_datagram_section_serializes_to_valid_json() {
    let mpe = {
        let mut __v = MpeDatagramSection::new(
            MacAddress([0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F]),
            3,
            7,
            &[0xAB, 0xCD],
        );
        __v.section_syntax_indicator = false;
        __v.private_indicator = true;
        __v.payload_scrambling_control = 0b01;
        __v.address_scrambling_control = 0b11;
        __v.llc_snap_flag = true;
        __v.current_next_indicator = true;
        __v.checksum = Checksum([0xDE, 0xAD, 0xBE, 0xEF]);
        __v
    };
    let j = serde_json::to_string(&mpe).expect("serialize MpeDatagramSection");
    assert_valid_json_with_keys(&j, &["mac_address", "payload", "checksum"]);
}

#[test]
fn mpe_fec_serializes_to_valid_json() {
    let mpe_fec = MpeFecSection::new(
        true,
        12,
        true,
        0,
        0,
        MpeFecRtp {
            delta_t: 0x0ABC,
            table_boundary: true,
            frame_boundary: false,
            address: 0x0001_2345,
        },
        &[0x01, 0x02],
    );
    let j = serde_json::to_string(&mpe_fec).expect("serialize MpeFecSection");
    assert_valid_json_with_keys(&j, &["padding_columns", "real_time_parameters", "rs_data"]);
}

#[test]
fn mpe_ifec_serializes_to_valid_json() {
    let mpe_ifec = MpeIfecSection::new(
        true,
        7,
        8,
        3,
        true,
        0,
        0,
        MpeIfecRtp {
            delta_t: 0x0ABC,
            mpe_boundary: true,
            frame_boundary: false,
            prev_burst_size: 0x0001_2345,
        },
        &[0x01, 0x02],
    );
    let j = serde_json::to_string(&mpe_ifec).expect("serialize MpeIfecSection");
    assert_valid_json_with_keys(&j, &["burst_number", "real_time_parameters", "ifec_data"]);
}

#[test]
fn protection_message_section_serializes_to_valid_json() {
    let pms = ProtectionMessageSection::new(
        0x0100,
        5,
        true,
        0,
        0,
        ProtectionMessageBody::certificate_collection(vec![
            &[0x30, 0x82, 0x01, 0x02],
            &[0xAB, 0xCD],
        ]),
    );
    let j = serde_json::to_string(&pms).expect("serialize ProtectionMessageSection");
    assert_valid_json_with_keys(&j, &["table_id_extension", "body"]);
}

#[test]
fn downloadable_font_info_section_serializes_to_valid_json() {
    let dfis = DownloadableFontInfoSection::new(
        0,
        0x42,
        0,
        true,
        0,
        0,
        vec![
            FontInfo::style_weight(2, 2),
            FontInfo::file_uri(1, b"https://f.example/Droid.otf"),
        ],
    );
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
        compatibility_descriptor: CompatibilityDescriptor {
            descriptors: vec![],
        },
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

// ── decoded-text serde: DvbText/LangCode fields serialize as decoded
//    strings (serialize-only; re-parse from wire bytes to reconstruct) ──

#[test]
fn short_event_serializes_decoded_strings() {
    use dvb_si::descriptors::short_event::ShortEventDescriptor;
    let d = ShortEventDescriptor {
        language_code: LangCode(*b"fre"),
        event_name: DvbText::new(b"Journal"),
        text: DvbText::new(b"20h"),
        trailing: Vec::new(),
    };
    let j = serde_json::to_value(&d).unwrap();
    assert_eq!(j["language_code"], "fre");
    assert_eq!(j["event_name"], "Journal");
    assert_eq!(j["text"], "20h");
}

#[test]
fn service_serializes_decoded_strings() {
    use dvb_si::descriptors::service::{ServiceDescriptor, ServiceType};
    let d = ServiceDescriptor {
        service_type: ServiceType::AvcHdDigitalTelevision,
        provider_name: DvbText::new(b"BBC"),
        service_name: DvbText::new(b"BBC ONE HD"),
        trailing: Vec::new(),
    };
    let j = serde_json::to_value(&d).unwrap();
    assert_eq!(j["provider_name"], "BBC");
    assert_eq!(j["service_name"], "BBC ONE HD");
}

#[test]
fn parental_rating_serializes_lang_code_as_string() {
    use dvb_si::descriptors::parental_rating::{ParentalRatingDescriptor, RatingEntry};
    let d = ParentalRatingDescriptor {
        entries: vec![
            RatingEntry {
                country_code: LangCode(*b"FRA"),
                rating: 0x05,
            },
            RatingEntry {
                country_code: LangCode(*b"GBR"),
                rating: 0x01,
            },
        ],
    };
    let v = serde_json::to_value(&d).unwrap();
    // LangCode serializes as a plain string (serialize-only).
    assert_eq!(v["entries"][0]["country_code"], "FRA");
    assert_eq!(v["entries"][0]["rating"], 0x05);
    assert_eq!(v["entries"][1]["country_code"], "GBR");
    assert_eq!(v["entries"][1]["rating"], 0x01);
}
