//! Real-fixture round-trip tests for prft, sgpd, sbgp, and subs boxes (#435).
//!
//! Gate criteria (ungameable):
//! - `prft` from `fixtures/mp4/prft.mp4`: parse fields + byte-exact round-trip.
//! - `sgpd('roll')` + `sbgp` from `fixtures/mp4/aac_sgpd.mp4`: parse
//!   `roll_distance=-1` + byte-exact round-trip.
//! - `subs`: build from a spec vector, round-trip.
//! - Mutation: mutating a field changes the serialized bytes.

use broadcast_common::{Parse, Serialize};
use transmux::{
    GROUPING_TYPE_ROLL, ProducerReferenceTimeBox, SampleGroupDescriptionBox, SampleToGroupBox,
    SbgpEntry, SgpdEntry, SubSampleDescriptor, SubSampleInformationBox, SubsEntry,
};

// ---------------------------------------------------------------------------
// Helper: find a box in a flat container body by four-CC.
//
// Returns the full box bytes (including the 8-byte header).
// ---------------------------------------------------------------------------

fn find_full_box<'a>(body: &'a [u8], four_cc: &[u8; 4]) -> &'a [u8] {
    let mut off = 0usize;
    while off + 8 <= body.len() {
        let size =
            u32::from_be_bytes([body[off], body[off + 1], body[off + 2], body[off + 3]]) as usize;
        let ty = &body[off + 4..off + 8];
        if size < 8 {
            break;
        }
        if ty == four_cc {
            return &body[off..off + size];
        }
        off += size;
    }
    panic!("box {:?} not found", core::str::from_utf8(four_cc).unwrap());
}

/// Navigate a container hierarchy: given a chain of four-CCs, descend into
/// each container and return the body of the final one.
fn enter_chain<'a>(data: &'a [u8], types: &[&[u8; 4]]) -> &'a [u8] {
    let mut current = data;
    for &ty in types {
        let mut off = 0usize;
        loop {
            if off + 8 > current.len() {
                panic!("box {:?} not found", core::str::from_utf8(ty).unwrap());
            }
            let size = u32::from_be_bytes([
                current[off],
                current[off + 1],
                current[off + 2],
                current[off + 3],
            ]) as usize;
            let box_ty = &current[off + 4..off + 8];
            if size < 8 {
                panic!(
                    "box {:?} not found (bad size)",
                    core::str::from_utf8(ty).unwrap()
                );
            }
            if box_ty == ty {
                current = &current[off + 8..off + size];
                break;
            }
            off += size;
        }
    }
    current
}

// ---------------------------------------------------------------------------
// Oracle bytes from FMP4-GAPS-ORACLE.md §#435
// ---------------------------------------------------------------------------

// prft body: `01 000018 00000001 edefe3e3a7ae147a 0000000000001c20`
// (version=1, flags=0x000018, ref_track_id=1, ntp_timestamp=0xedefe3e3a7ae147a,
//  media_time=0x0000000000001c20)
const PRFT_ORACLE_BODY: &[u8] = &[
    0x01, 0x00, 0x00, 0x18, // version=1, flags=0x000018
    0x00, 0x00, 0x00, 0x01, // reference_track_ID=1
    0xed, 0xef, 0xe3, 0xe3, 0xa7, 0xae, 0x14, 0x7a, // ntp_timestamp
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x1c, 0x20, // media_time (v1, u64)
];

// sgpd body: `01 000000 726f6c6c 00000002 00000001 ffff`
// (version=1, flags=0, grouping_type='roll', default_length=2, entry_count=1,
//  roll_distance=0xffff=-1)
const SGPD_ORACLE_BODY: &[u8] = &[
    0x01, 0x00, 0x00, 0x00, // version=1, flags=0
    0x72, 0x6f, 0x6c, 0x6c, // grouping_type='roll'
    0x00, 0x00, 0x00, 0x02, // default_length=2
    0x00, 0x00, 0x00, 0x01, // entry_count=1
    0xff, 0xff, // roll_distance=-1
];

// ---------------------------------------------------------------------------
// prft: real fixture parse + byte-exact round-trip
// ---------------------------------------------------------------------------

#[test]
fn prft_real_fixture_parse_fields() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/mp4/prft.mp4");
    let data = std::fs::read(path).expect("fixture file must exist");

    // prft is a top-level box in this file, after moov.
    let prft_box = find_full_box(&data, b"prft");

    let parsed =
        ProducerReferenceTimeBox::parse(prft_box).expect("should parse prft from real fixture");

    assert_eq!(parsed.version, 1, "prft version");
    assert_eq!(parsed.flags, 0x000018, "prft flags");
    assert_eq!(parsed.reference_track_id, 1, "prft reference_track_id");
    assert_eq!(
        parsed.ntp_timestamp, 0xedefe3e3_a7ae147a,
        "prft ntp_timestamp"
    );
    assert_eq!(parsed.media_time, 0x0000_0000_0000_1c20, "prft media_time");
}

#[test]
fn prft_real_fixture_byte_exact_round_trip() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/mp4/prft.mp4");
    let data = std::fs::read(path).expect("fixture file must exist");

    let prft_box = find_full_box(&data, b"prft");
    let parsed = ProducerReferenceTimeBox::parse(prft_box).expect("should parse prft");

    let serialized = parsed.to_bytes();
    assert_eq!(
        serialized.len(),
        prft_box.len(),
        "prft serialized length must match fixture"
    );
    assert_eq!(
        &serialized[..],
        prft_box,
        "prft round-trip must be byte-identical to fixture"
    );
}

#[test]
fn prft_oracle_body_matches() {
    // Verify that the oracle body bytes from the spec doc match what we parse
    // from the real file.
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/mp4/prft.mp4");
    let data = std::fs::read(path).expect("fixture file must exist");
    let prft_box = find_full_box(&data, b"prft");
    // body = full box bytes after the 8-byte header
    let body = &prft_box[8..];
    assert_eq!(body, PRFT_ORACLE_BODY, "prft body must match oracle");
}

// ---------------------------------------------------------------------------
// prft: mutation-proof (field change → bytes change)
// ---------------------------------------------------------------------------

#[test]
fn prft_mutation_changes_bytes() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/mp4/prft.mp4");
    let data = std::fs::read(path).expect("fixture file must exist");
    let prft_box = find_full_box(&data, b"prft");
    let mut parsed = ProducerReferenceTimeBox::parse(prft_box).expect("parse");

    let original = parsed.to_bytes();
    parsed.media_time = 0xDEAD_BEEF_1234_5678;
    let mutated = parsed.to_bytes();
    assert_ne!(
        mutated, original,
        "mutating media_time must change serialized bytes"
    );
}

// ---------------------------------------------------------------------------
// sgpd('roll'): real fixture parse + byte-exact round-trip
// ---------------------------------------------------------------------------

#[test]
fn sgpd_roll_real_fixture_parse_fields() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/mp4/aac_sgpd.mp4");
    let data = std::fs::read(path).expect("fixture file must exist");

    let stbl = enter_chain(&data, &[b"moov", b"trak", b"mdia", b"minf", b"stbl"]);
    let sgpd_box = find_full_box(stbl, b"sgpd");

    let parsed =
        SampleGroupDescriptionBox::parse(sgpd_box).expect("should parse sgpd from real fixture");

    assert_eq!(parsed.version, 1, "sgpd version");
    assert_eq!(parsed.flags, 0, "sgpd flags");
    assert_eq!(
        parsed.grouping_type, GROUPING_TYPE_ROLL,
        "sgpd grouping_type"
    );
    assert_eq!(parsed.default_length, 2, "sgpd default_length");
    assert_eq!(parsed.entries.len(), 1, "sgpd entry_count");
    assert_eq!(
        parsed.entries[0],
        SgpdEntry::Roll { roll_distance: -1 },
        "sgpd roll_distance"
    );
}

#[test]
fn sgpd_roll_real_fixture_byte_exact_round_trip() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/mp4/aac_sgpd.mp4");
    let data = std::fs::read(path).expect("fixture file must exist");

    let stbl = enter_chain(&data, &[b"moov", b"trak", b"mdia", b"minf", b"stbl"]);
    let sgpd_box = find_full_box(stbl, b"sgpd");

    let parsed = SampleGroupDescriptionBox::parse(sgpd_box).expect("parse sgpd");
    let serialized = parsed.to_bytes();
    assert_eq!(
        serialized.len(),
        sgpd_box.len(),
        "sgpd serialized length must match fixture"
    );
    assert_eq!(
        &serialized[..],
        sgpd_box,
        "sgpd round-trip must be byte-identical to fixture"
    );
}

#[test]
fn sgpd_oracle_body_matches() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/mp4/aac_sgpd.mp4");
    let data = std::fs::read(path).expect("fixture file must exist");
    let stbl = enter_chain(&data, &[b"moov", b"trak", b"mdia", b"minf", b"stbl"]);
    let sgpd_box = find_full_box(stbl, b"sgpd");
    let body = &sgpd_box[8..];
    assert_eq!(body, SGPD_ORACLE_BODY, "sgpd body must match oracle");
}

// ---------------------------------------------------------------------------
// sbgp: real fixture parse + byte-exact round-trip
// ---------------------------------------------------------------------------

#[test]
fn sbgp_real_fixture_parse_fields() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/mp4/aac_sgpd.mp4");
    let data = std::fs::read(path).expect("fixture file must exist");

    let stbl = enter_chain(&data, &[b"moov", b"trak", b"mdia", b"minf", b"stbl"]);
    let sbgp_box = find_full_box(stbl, b"sbgp");

    let parsed = SampleToGroupBox::parse(sbgp_box).expect("should parse sbgp from real fixture");

    assert_eq!(parsed.version, 0, "sbgp version");
    assert_eq!(
        parsed.grouping_type, GROUPING_TYPE_ROLL,
        "sbgp grouping_type"
    );
    assert!(
        parsed.grouping_type_parameter.is_none(),
        "sbgp no gtp in v0"
    );
    // At least one entry present
    assert!(!parsed.entries.is_empty(), "sbgp must have entries");
}

#[test]
fn sbgp_real_fixture_byte_exact_round_trip() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/mp4/aac_sgpd.mp4");
    let data = std::fs::read(path).expect("fixture file must exist");

    let stbl = enter_chain(&data, &[b"moov", b"trak", b"mdia", b"minf", b"stbl"]);
    let sbgp_box = find_full_box(stbl, b"sbgp");

    let parsed = SampleToGroupBox::parse(sbgp_box).expect("parse sbgp");
    let serialized = parsed.to_bytes();
    assert_eq!(
        serialized.len(),
        sbgp_box.len(),
        "sbgp serialized length must match fixture"
    );
    assert_eq!(
        &serialized[..],
        sbgp_box,
        "sbgp round-trip must be byte-identical to fixture"
    );
}

// ---------------------------------------------------------------------------
// sgpd+sbgp: mutation-proof
// ---------------------------------------------------------------------------

#[test]
fn sgpd_mutation_changes_bytes() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/mp4/aac_sgpd.mp4");
    let data = std::fs::read(path).expect("fixture file must exist");
    let stbl = enter_chain(&data, &[b"moov", b"trak", b"mdia", b"minf", b"stbl"]);
    let sgpd_box = find_full_box(stbl, b"sgpd");

    let mut parsed = SampleGroupDescriptionBox::parse(sgpd_box).expect("parse sgpd");
    let original = parsed.to_bytes();

    // Change roll_distance from -1 to -4
    parsed.entries[0] = SgpdEntry::Roll { roll_distance: -4 };
    let mutated = parsed.to_bytes();
    assert_ne!(
        mutated, original,
        "mutating roll_distance must change serialized bytes"
    );
}

#[test]
fn sbgp_mutation_changes_bytes() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/mp4/aac_sgpd.mp4");
    let data = std::fs::read(path).expect("fixture file must exist");
    let stbl = enter_chain(&data, &[b"moov", b"trak", b"mdia", b"minf", b"stbl"]);
    let sbgp_box = find_full_box(stbl, b"sbgp");

    let mut parsed = SampleToGroupBox::parse(sbgp_box).expect("parse sbgp");
    let original = parsed.to_bytes();

    parsed.entries.push(SbgpEntry {
        sample_count: 999,
        group_description_index: 1,
    });
    let mutated = parsed.to_bytes();
    assert_ne!(
        mutated, original,
        "adding a sbgp entry must change serialized bytes"
    );
}

// ---------------------------------------------------------------------------
// subs: spec vector round-trip (§8.7.7 Table example — two samples, v0)
// ---------------------------------------------------------------------------

#[test]
fn subs_spec_vector_round_trip_v0() {
    // Spec-derived vector: 1 entry, 2 sub-samples.
    // sample_delta=1, subsample 1: size=1500 prio=255 disc=0 csp=0
    // subsample 2: size=200 prio=128 disc=1 csp=0
    let b = SubSampleInformationBox {
        version: 0,
        flags: 0,
        entries: vec![SubsEntry {
            sample_delta: 1,
            subsamples: vec![
                SubSampleDescriptor {
                    subsample_size: 1500,
                    subsample_priority: 255,
                    discardable: 0,
                    codec_specific_parameters: 0,
                },
                SubSampleDescriptor {
                    subsample_size: 200,
                    subsample_priority: 128,
                    discardable: 1,
                    codec_specific_parameters: 0,
                },
            ],
        }],
    };

    let bytes = b.to_bytes();
    let parsed = SubSampleInformationBox::parse(&bytes).expect("parse subs v0");

    assert_eq!(parsed.version, 0);
    assert_eq!(parsed.entries.len(), 1);
    assert_eq!(parsed.entries[0].sample_delta, 1);
    assert_eq!(parsed.entries[0].subsamples.len(), 2);
    assert_eq!(parsed.entries[0].subsamples[0].subsample_size, 1500);
    assert_eq!(parsed.entries[0].subsamples[0].subsample_priority, 255);
    assert_eq!(parsed.entries[0].subsamples[0].discardable, 0);
    assert_eq!(parsed.entries[0].subsamples[1].subsample_size, 200);
    assert_eq!(parsed.entries[0].subsamples[1].subsample_priority, 128);
    assert_eq!(parsed.entries[0].subsamples[1].discardable, 1);

    // Byte-exact round-trip
    assert_eq!(
        parsed.to_bytes(),
        bytes,
        "subs round-trip must be byte-identical"
    );
}

#[test]
fn subs_spec_vector_round_trip_v1() {
    // v1: subsample_size is u32
    let b = SubSampleInformationBox {
        version: 1,
        flags: 0,
        entries: vec![SubsEntry {
            sample_delta: 5,
            subsamples: vec![SubSampleDescriptor {
                subsample_size: 0x0001_0000,
                subsample_priority: 200,
                discardable: 0,
                codec_specific_parameters: 0x1234_5678,
            }],
        }],
    };
    let bytes = b.to_bytes();
    let parsed = SubSampleInformationBox::parse(&bytes).expect("parse subs v1");
    assert_eq!(parsed, b);
    assert_eq!(
        parsed.to_bytes(),
        bytes,
        "subs v1 round-trip byte-identical"
    );
}

#[test]
fn subs_mutation_changes_bytes() {
    let b = SubSampleInformationBox {
        version: 0,
        flags: 0,
        entries: vec![SubsEntry {
            sample_delta: 1,
            subsamples: vec![SubSampleDescriptor {
                subsample_size: 100,
                subsample_priority: 255,
                discardable: 0,
                codec_specific_parameters: 0,
            }],
        }],
    };
    let original = b.to_bytes();
    let mut b2 = b;
    b2.entries[0].subsamples[0].subsample_size = 999;
    let mutated = b2.to_bytes();
    assert_ne!(
        mutated, original,
        "mutating subsample_size must change serialized bytes"
    );
}
