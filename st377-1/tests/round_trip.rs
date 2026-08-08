//! Round-trip integration tests for the structural metadata types added
//! by issue #754 — Packages, Tracks, Sequences, Components, and OP1a.
//!
//! Each test constructs a typed Set, serializes it, parses the result,
//! and verifies both structural equality AND byte-identical reserialize
//! (the project's symmetric Parse/Serialize invariant).

use broadcast_common::{Parse, Serialize};
use st377_1::{
    EventTrack, FillerComponent, InterchangeObjectFields, MaterialPackage, MxfTimestamp, PackageId,
    Rational, Sequence, SourceClip, SourcePackage, StaticTrack, TimecodeComponent, TimelineTrack,
};

// ── Shared helpers ──────────────────────────────────────────────────────

fn ts() -> MxfTimestamp {
    MxfTimestamp {
        year: 2026,
        month: 8,
        day: 7,
        hour: 12,
        minute: 30,
        second: 0,
        msec_div4: 0,
    }
}

fn interchange(seed: u8) -> InterchangeObjectFields {
    InterchangeObjectFields {
        instance_uid: [seed; 16],
        generation_uid: None,
        object_class: None,
    }
}

/// SMPTE-RP 224 Picture essence data definition UL (test placeholder).
const PICTURE_DD: [u8; 16] = [
    0x06, 0x0E, 0x2B, 0x34, 0x04, 0x01, 0x01, 0x01, 0x01, 0x03, 0x02, 0x02, 0x01, 0x00, 0x00, 0x00,
];

/// SMPTE-RP 224 Timecode data definition UL (test placeholder).
const TIMECODE_DD: [u8; 16] = [
    0x06, 0x0E, 0x2B, 0x34, 0x04, 0x01, 0x01, 0x01, 0x01, 0x03, 0x02, 0x01, 0x01, 0x00, 0x00, 0x00,
];

/// Assert parse→serialize→byte-identical AND serialize→parse→equal.
fn assert_round_trip<T>(original: &T)
where
    T: for<'a> Parse<'a, Error = st377_1::Error>
        + Serialize<Error = st377_1::Error>
        + PartialEq
        + core::fmt::Debug,
{
    let bytes = original.to_bytes();
    let parsed = T::parse(&bytes).expect("parse round-trip");
    assert_eq!(&parsed, original, "parsed != original");

    let reserialized = parsed.to_bytes();
    assert_eq!(reserialized, bytes, "reserialize not byte-identical");
}

// ── Individual type round-trips ─────────────────────────────────────────

#[test]
fn material_package_round_trip() {
    let mp = MaterialPackage {
        interchange: interchange(0x10),
        package_uid: PackageId([0x20; 32]),
        name: Some("Material Timeline".into()),
        creation_date: ts(),
        modified_date: ts(),
        tracks: vec![[0x30; 16], [0x31; 16]],
        dark: Vec::new(),
    };
    assert_round_trip(&mp);
}

#[test]
fn source_package_round_trip() {
    let sp = SourcePackage {
        interchange: interchange(0x40),
        package_uid: PackageId([0x50; 32]),
        name: None,
        creation_date: ts(),
        modified_date: ts(),
        tracks: vec![[0x60; 16]],
        descriptor: [0x70; 16],
        dark: Vec::new(),
    };
    assert_round_trip(&sp);
}

#[test]
fn timeline_track_round_trip() {
    let tt = TimelineTrack {
        interchange: interchange(0x01),
        track_id: 1,
        track_number: 0x15010100,
        track_name: Some("Video Track".into()),
        sequence: [0x02; 16],
        edit_rate: Rational {
            numerator: 25,
            denominator: 1,
        },
        origin: 0,
        dark: Vec::new(),
    };
    assert_round_trip(&tt);
}

#[test]
fn event_track_round_trip() {
    let et = EventTrack {
        interchange: interchange(0x03),
        track_id: 3,
        track_number: 0,
        track_name: None,
        sequence: [0x04; 16],
        event_edit_rate: Rational {
            numerator: 25,
            denominator: 1,
        },
        event_origin: Some(100),
        dark: Vec::new(),
    };
    assert_round_trip(&et);
}

#[test]
fn event_track_no_origin_round_trip() {
    let et = EventTrack {
        interchange: interchange(0x03),
        track_id: 3,
        track_number: 0,
        track_name: None,
        sequence: [0x04; 16],
        event_edit_rate: Rational {
            numerator: 30000,
            denominator: 1001,
        },
        event_origin: None,
        dark: Vec::new(),
    };
    assert_round_trip(&et);
}

#[test]
fn static_track_round_trip() {
    let st = StaticTrack {
        interchange: interchange(0x05),
        track_id: 4,
        track_number: 0,
        track_name: Some("DM Static".into()),
        sequence: [0x06; 16],
        dark: Vec::new(),
    };
    assert_round_trip(&st);
}

#[test]
fn sequence_round_trip() {
    let seq = Sequence {
        interchange: interchange(0x07),
        data_definition: PICTURE_DD,
        duration: Some(250),
        structural_components: vec![[0x08; 16], [0x09; 16]],
        dark: Vec::new(),
    };
    assert_round_trip(&seq);
}

#[test]
fn sequence_no_duration_round_trip() {
    let seq = Sequence {
        interchange: interchange(0x07),
        data_definition: PICTURE_DD,
        duration: None,
        structural_components: vec![[0x08; 16]],
        dark: Vec::new(),
    };
    assert_round_trip(&seq);
}

#[test]
fn source_clip_round_trip() {
    let sc = SourceClip {
        interchange: interchange(0x0A),
        data_definition: PICTURE_DD,
        duration: Some(250),
        start_position: 0,
        source_package_id: PackageId([0x0B; 32]),
        source_track_id: 1,
        dark: Vec::new(),
    };
    assert_round_trip(&sc);
}

#[test]
fn source_clip_null_ref_round_trip() {
    let sc = SourceClip {
        interchange: interchange(0x0A),
        data_definition: PICTURE_DD,
        duration: Some(250),
        start_position: 0,
        source_package_id: PackageId::NULL,
        source_track_id: 0,
        dark: Vec::new(),
    };
    assert_round_trip(&sc);
    assert!(sc.source_package_id.is_null());
}

#[test]
fn timecode_component_round_trip() {
    let tc = TimecodeComponent {
        interchange: interchange(0x0C),
        data_definition: TIMECODE_DD,
        duration: Some(250),
        start_timecode: 90000,
        rounded_timecode_base: 25,
        drop_frame: false,
        dark: Vec::new(),
    };
    assert_round_trip(&tc);
}

#[test]
fn timecode_component_drop_frame_round_trip() {
    let tc = TimecodeComponent {
        interchange: interchange(0x0C),
        data_definition: TIMECODE_DD,
        duration: Some(1800),
        start_timecode: 0,
        rounded_timecode_base: 30,
        drop_frame: true,
        dark: Vec::new(),
    };
    assert_round_trip(&tc);
}

#[test]
fn filler_component_round_trip() {
    let f = FillerComponent {
        interchange: interchange(0x0D),
        data_definition: PICTURE_DD,
        duration: Some(50),
        dark: Vec::new(),
    };
    assert_round_trip(&f);
}

#[test]
fn filler_component_no_duration_round_trip() {
    let f = FillerComponent {
        interchange: interchange(0x0D),
        data_definition: PICTURE_DD,
        duration: None,
        dark: Vec::new(),
    };
    assert_round_trip(&f);
}

// ── OP1a helpers ────────────────────────────────────────────────────────

#[test]
fn op1a_default_round_trip() {
    let q = st377_1::op1a::Op1aQualifier::default();
    let ul = st377_1::op1a::op1a_ul(q);
    assert!(st377_1::op1a::is_op1a(&ul));
    let q2 = st377_1::op1a::Op1aQualifier::from_byte(ul[14]);
    assert!(!q2.external_essence());
    assert!(!q2.non_streamable());
    assert!(!q2.multi_track());
}

#[test]
fn op1a_full_qualifier_round_trip() {
    let q = st377_1::op1a::Op1aQualifier::default()
        .with_external_essence()
        .with_non_streamable()
        .with_multi_track();
    let ul = st377_1::op1a::op1a_ul(q);
    assert!(st377_1::op1a::is_op1a(&ul));
    assert_eq!(ul[14], 0x07);
}

// ── Full OP1a structure round-trip ──────────────────────────────────────

#[test]
fn full_op1a_structure_builds_and_round_trips() {
    use st377_1::{
        ContentStorage, EssenceContainerData, Identification, PartitionKind, PartitionPack,
        PartitionStatus, Preface, PrimerPack, RandomIndexPack, VERSION_1_3,
    };

    // -- Build the complete metadata graph --

    // Instance UIDs for cross-referencing.
    let preface_uid = [0x01; 16];
    let ident_uid = [0x02; 16];
    let cs_uid = [0x03; 16];
    let ecd_uid = [0x04; 16];
    let mp_uid_ref = [0x05; 16];
    let sp_uid_ref = [0x06; 16];
    let tt_uid = [0x07; 16];
    let seq_uid = [0x08; 16];
    let sc_uid = [0x09; 16];
    let tc_uid = [0x0A; 16];
    let desc_uid = [0xDD; 16];

    let op_pattern = st377_1::op1a::op1a_ul(st377_1::op1a::Op1aQualifier::default());
    let ec_label: [u8; 16] = [
        0x06, 0x0E, 0x2B, 0x34, 0x04, 0x01, 0x01, 0x01, 0x0D, 0x01, 0x03, 0x01, 0x02, 0x06, 0x01,
        0x00,
    ];

    let mp = MaterialPackage {
        interchange: InterchangeObjectFields {
            instance_uid: mp_uid_ref,
            generation_uid: None,
            object_class: None,
        },
        package_uid: PackageId([0x10; 32]),
        name: Some("Main".into()),
        creation_date: ts(),
        modified_date: ts(),
        tracks: vec![tt_uid],
        dark: Vec::new(),
    };
    assert_round_trip(&mp);

    let sp = SourcePackage {
        interchange: InterchangeObjectFields {
            instance_uid: sp_uid_ref,
            generation_uid: None,
            object_class: None,
        },
        package_uid: PackageId([0x20; 32]),
        name: None,
        creation_date: ts(),
        modified_date: ts(),
        tracks: vec![tt_uid],
        descriptor: desc_uid,
        dark: Vec::new(),
    };
    assert_round_trip(&sp);

    let tt = TimelineTrack {
        interchange: InterchangeObjectFields {
            instance_uid: tt_uid,
            generation_uid: None,
            object_class: None,
        },
        track_id: 1,
        track_number: 0x15010100,
        track_name: Some("Video".into()),
        sequence: seq_uid,
        edit_rate: Rational {
            numerator: 25,
            denominator: 1,
        },
        origin: 0,
        dark: Vec::new(),
    };
    assert_round_trip(&tt);

    let seq = Sequence {
        interchange: InterchangeObjectFields {
            instance_uid: seq_uid,
            generation_uid: None,
            object_class: None,
        },
        data_definition: PICTURE_DD,
        duration: Some(250),
        structural_components: vec![sc_uid, tc_uid],
        dark: Vec::new(),
    };
    assert_round_trip(&seq);

    let sc = SourceClip {
        interchange: InterchangeObjectFields {
            instance_uid: sc_uid,
            generation_uid: None,
            object_class: None,
        },
        data_definition: PICTURE_DD,
        duration: Some(250),
        start_position: 0,
        source_package_id: PackageId([0x20; 32]),
        source_track_id: 1,
        dark: Vec::new(),
    };
    assert_round_trip(&sc);

    let tc = TimecodeComponent {
        interchange: InterchangeObjectFields {
            instance_uid: tc_uid,
            generation_uid: None,
            object_class: None,
        },
        data_definition: TIMECODE_DD,
        duration: Some(250),
        start_timecode: 90000,
        rounded_timecode_base: 25,
        drop_frame: false,
        dark: Vec::new(),
    };
    assert_round_trip(&tc);

    let cs = ContentStorage {
        interchange: InterchangeObjectFields {
            instance_uid: cs_uid,
            generation_uid: None,
            object_class: None,
        },
        packages: vec![mp_uid_ref, sp_uid_ref],
        essence_container_data: Some(vec![ecd_uid]),
        dark: Vec::new(),
    };
    assert_round_trip(&cs);

    let ecd = EssenceContainerData {
        interchange: InterchangeObjectFields {
            instance_uid: ecd_uid,
            generation_uid: None,
            object_class: None,
        },
        linked_package_uid: PackageId([0x20; 32]),
        index_sid: Some(0),
        body_sid: 1,
        dark: Vec::new(),
    };
    assert_round_trip(&ecd);

    let gen_uid = [0xF0; 16];
    let ident = Identification {
        interchange: InterchangeObjectFields {
            instance_uid: ident_uid,
            generation_uid: None,
            object_class: None,
        },
        this_generation_uid: gen_uid,
        company_name: "Test".into(),
        product_name: "st377-1".into(),
        product_version: None,
        version_string: "0.3.0".into(),
        product_uid: [0xE0; 16],
        modification_date: ts(),
        toolkit_version: None,
        platform: None,
        dark: Vec::new(),
    };
    assert_round_trip(&ident);

    let preface = Preface {
        interchange: InterchangeObjectFields {
            instance_uid: preface_uid,
            generation_uid: Some(gen_uid),
            object_class: None,
        },
        last_modified_date: ts(),
        version: VERSION_1_3,
        object_model_version: Some(1),
        primary_package: Some(mp_uid_ref),
        identifications: vec![ident_uid],
        content_storage: cs_uid,
        operational_pattern: op_pattern,
        essence_containers: vec![ec_label],
        dm_schemes: Vec::new(),
        dark: Vec::new(),
    };
    assert_round_trip(&preface);

    // -- Verify the OP1a link is intact --
    assert!(st377_1::op1a::is_op1a(&preface.operational_pattern));

    // -- Partition Packs also round-trip --
    let header_pp = PartitionPack {
        kind: PartitionKind::Header,
        status: PartitionStatus::ClosedComplete,
        major_version: 1,
        minor_version: 3,
        kag_size: 1,
        this_partition: 0,
        previous_partition: 0,
        footer_partition: 0,
        header_byte_count: 0,
        index_byte_count: 0,
        index_sid: 0,
        body_offset: 0,
        body_sid: 0,
        operational_pattern: op_pattern,
        essence_containers: vec![ec_label],
    };
    assert_round_trip(&header_pp);

    let footer_pp = PartitionPack {
        kind: PartitionKind::Footer,
        status: PartitionStatus::ClosedComplete,
        major_version: 1,
        minor_version: 3,
        kag_size: 1,
        this_partition: 0,
        previous_partition: 0,
        footer_partition: 0,
        header_byte_count: 0,
        index_byte_count: 0,
        index_sid: 0,
        body_offset: 0,
        body_sid: 0,
        operational_pattern: op_pattern,
        essence_containers: vec![ec_label],
    };
    assert_round_trip(&footer_pp);

    let primer = PrimerPack {
        entries: vec![(
            0x3C0A,
            [
                0x06, 0x0E, 0x2B, 0x34, 0x01, 0x01, 0x01, 0x01, 0x01, 0x01, 0x15, 0x02, 0x00, 0x00,
                0x00, 0x00,
            ],
        )],
    };
    assert_round_trip(&primer);

    let rip = RandomIndexPack {
        partitions: vec![
            st377_1::PartitionLocation {
                body_sid: 0,
                byte_offset: 0,
            },
            st377_1::PartitionLocation {
                body_sid: 0,
                byte_offset: 9999,
            },
        ],
    };
    assert_round_trip(&rip);
}
