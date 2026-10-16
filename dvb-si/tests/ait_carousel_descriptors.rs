//! Typed AIT/MHP application descriptors and DVB data-carousel descriptors
//! (issue #1294): real-capture sections first, spec-vector round trips for
//! everything the captures do not carry.
//!
//! Real data (private submodule, skip loudly when absent):
//! `sections/hotbird-ait-carousel/` (Hot Bird 13E MHP) and
//! `sections/france-tnt-hbbtv-carousel/` (France TNT HbbTV) — complete AIT /
//! DSI / DII sections, see `private/fixtures/sections/PROVENANCE.md`. The
//! survey recorded there found NO real sample of AIT tags
//! 0x06/0x08-0x0D/0x10/0x11/0x14 nor of carousel tags 0x02-0x08/0x0A/0x0B in
//! any capture, so those are covered by the syntax-table vectors below (which
//! cite the spec clause).

use broadcast_common::{Parse, Serialize};
use dvb_si::carousel::UnMessage;
use dvb_si::carousel::biop::message::ModuleInfo;
use dvb_si::descriptors::ait::{
    AnyAitDescriptor, ApplicationIconsDescriptor, ApplicationRecordingDescriptor,
    ApplicationStorageDescriptor, DiiLocation, DiiLocationDescriptor,
    DvbHtmlApplicationBoundaryDescriptor, DvbHtmlApplicationLocationDescriptor,
    GraphicsConfiguration, GraphicsConstraintsDescriptor, IconFlags, IconFormat,
    IpSignallingDescriptor, PrefetchDescriptor, PrefetchEntry, RecordingLabel, StorageProperties,
    StorageProperty, parse_ait_loop,
};
use dvb_si::descriptors::carousel::{
    AnyCarouselDescriptor, CarouselDescriptorLoop, CompressedModuleDescriptor, Crc32Descriptor,
    EstDownloadTimeDescriptor, GroupLinkDescriptor, InfoDescriptor, LinkPosition,
    LocationDescriptor, ModuleLinkDescriptor, NameDescriptor, SsuModuleType,
    SsuModuleTypeDescriptor, SubgroupAssociationDescriptor, TypeDescriptor, parse_carousel_loop,
};
use dvb_si::tables::ait::AitSection;
use dvb_si::tables::dsmcc::DsmccSection;
use dvb_si::text::{DvbText, LangCode};

fn ser<T: Serialize<Error = dvb_si::Error>>(v: &T) -> Vec<u8> {
    let mut out = vec![0u8; v.serialized_len()];
    let n = v.serialize_into(&mut out).unwrap();
    assert_eq!(n, out.len());
    out
}

fn section(rel: &str) -> Option<Vec<u8>> {
    let path = test_fixtures::require(rel, "ait_carousel_descriptors")?;
    Some(std::fs::read(path).expect("read section fixture"))
}

// ---------------------------------------------------------------- real data

fn walk_ait(bytes: &[u8]) -> Vec<String> {
    let ait = AitSection::parse(bytes).expect("real AIT section parses");
    assert_eq!(ser(&ait), bytes, "AIT section must round-trip byte-exactly");
    let mut seen = Vec::new();
    let mut loops = vec![ait.common_ait_descriptors()];
    loops.extend(ait.applications.iter().map(|a| a.ait_descriptors()));
    for l in loops {
        for d in l.iter() {
            seen.push(
                d.expect("every real AIT descriptor parses")
                    .name()
                    .to_owned(),
            );
        }
    }
    seen
}

#[test]
fn hotbird_real_ait_section_walks_typed_and_round_trips() {
    let Some(bytes) = section("sections/hotbird-ait-carousel/ait-pid1ec7-off275232.bin") else {
        return;
    };
    let seen = walk_ait(&bytes);
    eprintln!("ait_carousel_descriptors: hotbird AIT descriptors present: {seen:?}");
    // Real data carries the DVB-J pair (0x03/0x04) and application/name/transport.
    for n in [
        "APPLICATION",
        "APPLICATION_NAME",
        "TRANSPORT_PROTOCOL",
        "DVB_J_APPLICATION",
        "DVB_J_APPLICATION_LOCATION",
    ] {
        assert!(seen.iter().any(|s| s == n), "{n} missing: {seen:?}");
    }
    assert!(
        !seen.iter().any(|n| n == "UNKNOWN"),
        "no Unknown AIT descriptors expected: {seen:?}"
    );
}

#[test]
fn france_real_ait_section_walks_typed_and_round_trips() {
    let Some(bytes) = section("sections/france-tnt-hbbtv-carousel/ait-pid010e-off19364.bin") else {
        return;
    };
    let seen = walk_ait(&bytes);
    eprintln!("ait_carousel_descriptors: france AIT descriptors present: {seen:?}");
    assert!(!seen.iter().any(|n| n == "UNKNOWN"));
}

fn dii_module_infos(bytes: &[u8]) -> Vec<Vec<u8>> {
    let sec = DsmccSection::parse(bytes).expect("DSM-CC section");
    assert_eq!(
        ser(&sec),
        bytes,
        "DSM-CC section must round-trip byte-exactly"
    );
    match UnMessage::parse(sec.payload).expect("DSM-CC message") {
        UnMessage::Dii(d) => d.modules.iter().map(|m| m.module_info.to_vec()).collect(),
        other => panic!("expected DII, got {other:?}"),
    }
}

#[test]
fn hotbird_real_dii_module_info_has_empty_typed_user_info() {
    let Some(bytes) = section("sections/hotbird-ait-carousel/dii1-pid1e9e-off42488.bin") else {
        return;
    };
    let infos = dii_module_infos(&bytes);
    assert!(!infos.is_empty());
    for raw in &infos {
        let mi = ModuleInfo::parse(raw).expect("BIOP ModuleInfo");
        eprintln!(
            "ait_carousel_descriptors: hotbird DII moduleInfo userInfo bytes: {}",
            mi.user_info.len()
        );
        assert!(mi.user_info.is_empty());
        assert!(mi.compressed_module_descriptor().is_none());
        assert_eq!(ser(&mi), *raw);
    }
}

#[test]
fn france_real_dii_user_info_types_compressed_module_and_keeps_unallocated_tag() {
    let Some(bytes) = section("sections/france-tnt-hbbtv-carousel/dii1-pid010f-off114680.bin")
    else {
        return;
    };
    let infos = dii_module_infos(&bytes);
    let mut compressed = 0;
    for raw in &infos {
        let mi = ModuleInfo::parse(raw).expect("BIOP ModuleInfo");
        assert_eq!(ser(&mi), *raw, "ModuleInfo must round-trip byte-exactly");
        let items: Vec<_> = mi.user_info.iter().collect();
        eprintln!(
            "ait_carousel_descriptors: france DII userInfo: {:?}",
            items
                .iter()
                .map(|d| d.as_ref().map(|d| d.name()))
                .collect::<Vec<_>>()
        );
        for d in &items {
            match d.as_ref().expect("real userInfo descriptor parses") {
                AnyCarouselDescriptor::CompressedModule(c) => {
                    compressed += 1;
                    assert_eq!(c.original_size, 0xEB2B);
                    assert_eq!(c.compression_method, 0x78);
                }
                // MHP caching_priority_descriptor: not allocated in EN 301 192.
                AnyCarouselDescriptor::Unknown { tag: 0x71, body } => {
                    assert_eq!(*body, &[0xFFu8, 0xFF][..])
                }
                other => panic!("unexpected carousel descriptor {other:?}"),
            }
        }
        assert!(mi.compressed_module_descriptor().is_some());
    }
    assert!(compressed >= 1);
}

#[test]
fn tsduck_test_062_plain_module_info_types_type_descriptor() {
    use mpeg_ts::ts::{SectionReassembler, TS_PACKET_SIZE, TsPacket};
    use std::collections::BTreeMap;
    let Some(path) = test_fixtures::require("tsduck/input/test-062.ts", "ait_carousel_descriptors")
    else {
        return;
    };
    let data = std::fs::read(path).expect("read capture");
    let mut reasm: BTreeMap<u16, SectionReassembler> = BTreeMap::new();
    let mut types = Vec::new();
    for chunk in data.chunks(TS_PACKET_SIZE) {
        if chunk.len() != TS_PACKET_SIZE || chunk[0] != 0x47 {
            continue;
        }
        let Ok(pkt) = TsPacket::parse(chunk) else {
            continue;
        };
        let Some(payload) = pkt.payload else { continue };
        let r = reasm.entry(pkt.header.pid.value()).or_default();
        r.feed(payload, pkt.header.pusi);
        while let Some(s) = r.pop_section() {
            if s[0] != 0x3B {
                continue;
            }
            let Ok(sec) = DsmccSection::parse(&s) else {
                continue;
            };
            if let Ok(UnMessage::Dii(d)) = UnMessage::parse(sec.payload) {
                for m in &d.modules {
                    for item in m.descriptors().iter().flatten() {
                        if let AnyCarouselDescriptor::Type(t) = item {
                            types.push(t.media_type.raw().to_vec());
                        }
                    }
                }
            }
        }
    }
    eprintln!(
        "ait_carousel_descriptors: test-062 type_descriptors: {:?}",
        types
            .iter()
            .map(|t| String::from_utf8_lossy(t).into_owned())
            .collect::<Vec<_>>()
    );
    assert!(
        types.iter().any(|t| t == b"text/css"),
        "expected a real type_descriptor \"text/css\" in test-062"
    );
}

// ------------------------------------------------- spec-syntax vectors, AIT

#[test]
fn ait_loop_with_every_new_descriptor_walks_typed_and_round_trips() {
    let icons = ApplicationIconsDescriptor {
        icon_locator: DvbText::new(b"/icons"),
        icon_flags: IconFlags::default()
            .with(IconFormat::Square32x32)
            .with(IconFormat::Broadcast24x32Dar16x9),
        reserved_future_use: &[],
    };
    let recording = ApplicationRecordingDescriptor {
        scheduled_recording_flag: true,
        labels: vec![RecordingLabel::new(
            b"lbl",
            StorageProperties::CriticalToStore,
        )],
        component_tags: vec![0x10],
        private: &[0xAA],
        ..Default::default()
    };
    let storage = ApplicationStorageDescriptor {
        storage_property: StorageProperty::StandAlone,
        version: 5,
        priority: 9,
        ..Default::default()
    };
    let graphics = GraphicsConstraintsDescriptor {
        can_run_without_visible_ui: true,
        graphics_configurations: vec![GraphicsConfiguration::FullScreen1280x720],
        ..Default::default()
    };
    let prefetch = PrefetchDescriptor {
        transport_protocol_label: 1,
        entries: vec![PrefetchEntry {
            label: b"img",
            prefetch_priority: 50,
        }],
    };
    let dii = DiiLocationDescriptor {
        transport_protocol_label: 1,
        locations: vec![DiiLocation::new(0x0102, 0x0304)],
    };
    let ip = IpSignallingDescriptor {
        platform_id: 0x00ABCD,
    };
    let html_loc = DvbHtmlApplicationLocationDescriptor {
        physical_root: DvbText::new(b"application/"),
        initial_path: DvbText::new(b"main/index.htm"),
    };
    let html_bound = DvbHtmlApplicationBoundaryDescriptor {
        label: DvbText::new(b"l"),
        regular_expression: DvbText::new(b".*"),
    };
    // Ascending tag order, as a sender would emit.
    let mut lp = Vec::new();
    lp.extend(ser(&recording));
    lp.extend(ser(&html_loc));
    lp.extend(ser(&html_bound));
    lp.extend(ser(&icons));
    lp.extend(ser(&prefetch));
    lp.extend(ser(&dii));
    lp.extend(ser(&storage));
    lp.extend(ser(&ip));
    lp.extend(ser(&graphics));

    let items: Vec<_> = parse_ait_loop(&lp).map(|d| d.expect("typed")).collect();
    assert_eq!(items.len(), 9);
    let names: Vec<_> = items.iter().map(AnyAitDescriptor::name).collect();
    assert_eq!(
        names,
        [
            "APPLICATION_RECORDING",
            "DVB_HTML_APPLICATION_LOCATION",
            "DVB_HTML_APPLICATION_BOUNDARY",
            "APPLICATION_ICONS",
            "PREFETCH",
            "DII_LOCATION",
            "APPLICATION_STORAGE",
            "IP_SIGNALLING",
            "GRAPHICS_CONSTRAINTS"
        ]
    );
    assert!(matches!(&items[0], AnyAitDescriptor::ApplicationRecording(d) if *d == recording));
    assert!(matches!(&items[3], AnyAitDescriptor::ApplicationIcons(d) if *d == icons));
    assert!(matches!(&items[6], AnyAitDescriptor::ApplicationStorage(d) if *d == storage));
    assert!(matches!(&items[8], AnyAitDescriptor::GraphicsConstraints(d) if *d == graphics));
    // Re-serialize every typed entry: the loop is reproduced byte-for-byte.
    let mut again = Vec::new();
    for i in &items {
        match i {
            AnyAitDescriptor::ApplicationRecording(d) => again.extend(ser(d)),
            AnyAitDescriptor::DvbHtmlApplicationLocation(d) => again.extend(ser(d)),
            AnyAitDescriptor::DvbHtmlApplicationBoundary(d) => again.extend(ser(d)),
            AnyAitDescriptor::ApplicationIcons(d) => again.extend(ser(d)),
            AnyAitDescriptor::Prefetch(d) => again.extend(ser(d)),
            AnyAitDescriptor::DiiLocation(d) => again.extend(ser(d)),
            AnyAitDescriptor::ApplicationStorage(d) => again.extend(ser(d)),
            AnyAitDescriptor::IpSignalling(d) => again.extend(ser(d)),
            AnyAitDescriptor::GraphicsConstraints(d) => again.extend(ser(d)),
            other => panic!("unexpected {other:?}"),
        }
    }
    assert_eq!(again, lp);
}

// --------------------------------------------- spec-syntax vectors, carousel

#[test]
fn carousel_loop_with_every_descriptor_walks_typed_and_round_trips() {
    let ty = TypeDescriptor {
        media_type: DvbText::new(b"text/plain"),
    };
    let name = NameDescriptor {
        name: DvbText::new(b"index"),
    };
    let info = InfoDescriptor {
        language_code: LangCode(*b"eng"),
        text: DvbText::new(b"hello"),
    };
    let link = ModuleLinkDescriptor {
        position: LinkPosition::Intermediate,
        module_id: 0x0102,
    };
    let crc = Crc32Descriptor {
        crc_32: 0x0376_E6E7,
    };
    let loc = LocationDescriptor { location_tag: 0x42 };
    let est = EstDownloadTimeDescriptor {
        est_download_time: 30,
    };
    let group = GroupLinkDescriptor {
        position: LinkPosition::Last,
        group_id: 0xDEAD_BEEF,
    };
    let comp = CompressedModuleDescriptor {
        compression_method: 8,
        original_size: 1234,
    };
    let ssu = SsuModuleTypeDescriptor {
        ssu_module_type: SsuModuleType::Data,
    };
    let sub = SubgroupAssociationDescriptor {
        oui: 0x00015A,
        subgroup_tag: 3,
    };
    let mut lp = Vec::new();
    lp.extend(ser(&ty));
    lp.extend(ser(&name));
    lp.extend(ser(&info));
    lp.extend(ser(&link));
    lp.extend(ser(&crc));
    lp.extend(ser(&loc));
    lp.extend(ser(&est));
    lp.extend(ser(&group));
    lp.extend(ser(&comp));
    lp.extend(ser(&ssu));
    lp.extend(ser(&sub));
    lp.extend_from_slice(&[0x80, 0x01, 0x55]); // private descriptor (Table 49)

    let l = CarouselDescriptorLoop::new(&lp);
    let items: Vec<_> = parse_carousel_loop(&lp)
        .map(|d| d.expect("typed"))
        .collect();
    assert_eq!(l.iter().count(), 12);
    assert_eq!(items.len(), 12);
    assert!(matches!(&items[0], AnyCarouselDescriptor::Type(d) if *d == ty));
    assert!(matches!(&items[3], AnyCarouselDescriptor::ModuleLink(d) if *d == link));
    assert!(matches!(&items[8], AnyCarouselDescriptor::CompressedModule(d) if *d == comp));
    assert!(matches!(&items[10], AnyCarouselDescriptor::SubgroupAssociation(d) if *d == sub));
    assert!(matches!(
        &items[11],
        AnyCarouselDescriptor::Unknown { tag: 0x80, .. }
    ));
    let mut again = Vec::new();
    for i in &items {
        match i {
            AnyCarouselDescriptor::Type(d) => again.extend(ser(d)),
            AnyCarouselDescriptor::Name(d) => again.extend(ser(d)),
            AnyCarouselDescriptor::Info(d) => again.extend(ser(d)),
            AnyCarouselDescriptor::ModuleLink(d) => again.extend(ser(d)),
            AnyCarouselDescriptor::Crc32(d) => again.extend(ser(d)),
            AnyCarouselDescriptor::Location(d) => again.extend(ser(d)),
            AnyCarouselDescriptor::EstDownloadTime(d) => again.extend(ser(d)),
            AnyCarouselDescriptor::GroupLink(d) => again.extend(ser(d)),
            AnyCarouselDescriptor::CompressedModule(d) => again.extend(ser(d)),
            AnyCarouselDescriptor::SsuModuleType(d) => again.extend(ser(d)),
            AnyCarouselDescriptor::SubgroupAssociation(d) => again.extend(ser(d)),
            AnyCarouselDescriptor::Unknown { tag, body } => {
                again.push(*tag);
                again.push(u8::try_from(body.len()).unwrap());
                again.extend_from_slice(body);
            }
            other => panic!("unexpected {other:?}"),
        }
    }
    assert_eq!(again, lp);
}

#[test]
fn dii_module_and_dsi_group_info_expose_typed_loops() {
    use dvb_si::carousel::{DiiModule, GroupInfo};
    use dvb_si::compatibility::CompatibilityDescriptor;
    let loop_bytes = [
        0x07, 0x04, 0x00, 0x00, 0x00, 0x0A, 0x0B, 0x05, 0x00, 0x01, 0x5A, 0x00, 0x07,
    ];
    let m = DiiModule {
        module_id: 1,
        module_size: 10,
        module_version: 0,
        module_info: &loop_bytes,
    };
    let kinds: Vec<_> = m.descriptors().iter().map(|d| d.unwrap().name()).collect();
    assert_eq!(kinds, ["EST_DOWNLOAD_TIME", "SUBGROUP_ASSOCIATION"]);
    let g = GroupInfo {
        group_id: 1,
        group_size: 10,
        group_compatibility: CompatibilityDescriptor {
            descriptors: Vec::new(),
        },
        group_info: &loop_bytes,
        private_data: &[],
    };
    assert_eq!(g.descriptors().iter().count(), 2);
}
