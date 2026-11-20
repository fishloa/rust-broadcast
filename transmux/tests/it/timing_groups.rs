//! Real-fixture round-trip tests for prft, sgpd, sbgp, and subs boxes (#435).
//!
//! Gate criteria (ungameable):
//! - `prft` from `fixtures/mp4/prft.mp4`: parse fields + byte-exact round-trip.
//! - `sgpd('roll')` + `sbgp` from `fixtures/mp4/aac-sgpd.mp4`: parse
//!   `roll_distance=-1` + byte-exact round-trip.
//! - `subs`: build from a spec vector, round-trip.
//! - Mutation: mutating a field changes the serialized bytes.

use broadcast_common::timescale::Timescale;
use broadcast_common::{Parse, Serialize};
use transmux::TrackId;
use transmux::{
    CompositionOffsetBox, CompositionToDecodeBox, CttsEntry, EditListBox, EditListEntry,
    GROUPING_TYPE_ROLL, ProducerReferenceTimeBox, SampleGroupDescriptionBox, SampleToGroupBox,
    SbgpEntry, SegmentIndexBox, SgpdEntry, SidxReference, SttsEntry, SubSampleDescriptor,
    SubSampleInformationBox, SubSampleReferenceTableBox, SubsEntry, SubsampleProperties,
    TimeToSampleBox,
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
// Oracle bytes from fixtures/mp4/PROVENANCE.md §#435
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
    let path = test_fixtures::path("mp4/prft.mp4");
    let data = std::fs::read(path).expect("fixture file must exist");

    // prft is a top-level box in this file, after moov.
    let prft_box = find_full_box(&data, b"prft");

    let parsed =
        ProducerReferenceTimeBox::parse(prft_box).expect("should parse prft from real fixture");

    assert_eq!(parsed.version, 1, "prft version");
    assert_eq!(parsed.flags, 0x000018, "prft flags");
    assert_eq!(
        parsed.reference_track_id,
        TrackId::new(1),
        "prft reference_track_id"
    );
    assert_eq!(
        parsed.ntp_timestamp, 0xedefe3e3_a7ae147a,
        "prft ntp_timestamp"
    );
    assert_eq!(parsed.media_time, 0x0000_0000_0000_1c20, "prft media_time");
}

#[test]
fn prft_real_fixture_byte_exact_round_trip() {
    let path = test_fixtures::path("mp4/prft.mp4");
    let data = std::fs::read(path).expect("fixture file must exist");

    let prft_box = find_full_box(&data, b"prft");
    let parsed = ProducerReferenceTimeBox::parse(prft_box).expect("should parse prft");

    let serialized = parsed.try_to_bytes().unwrap();
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
    let path = test_fixtures::path("mp4/prft.mp4");
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
    let path = test_fixtures::path("mp4/prft.mp4");
    let data = std::fs::read(path).expect("fixture file must exist");
    let prft_box = find_full_box(&data, b"prft");
    let mut parsed = ProducerReferenceTimeBox::parse(prft_box).expect("parse");

    let original = parsed.try_to_bytes().unwrap();
    parsed.media_time = 0xDEAD_BEEF_1234_5678;
    let mutated = parsed.try_to_bytes().unwrap();
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
    let path = test_fixtures::path("mp4/aac-sgpd.mp4");
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
    let path = test_fixtures::path("mp4/aac-sgpd.mp4");
    let data = std::fs::read(path).expect("fixture file must exist");

    let stbl = enter_chain(&data, &[b"moov", b"trak", b"mdia", b"minf", b"stbl"]);
    let sgpd_box = find_full_box(stbl, b"sgpd");

    let parsed = SampleGroupDescriptionBox::parse(sgpd_box).expect("parse sgpd");
    let serialized = parsed.try_to_bytes().unwrap();
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
    let path = test_fixtures::path("mp4/aac-sgpd.mp4");
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
    let path = test_fixtures::path("mp4/aac-sgpd.mp4");
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
    let path = test_fixtures::path("mp4/aac-sgpd.mp4");
    let data = std::fs::read(path).expect("fixture file must exist");

    let stbl = enter_chain(&data, &[b"moov", b"trak", b"mdia", b"minf", b"stbl"]);
    let sbgp_box = find_full_box(stbl, b"sbgp");

    let parsed = SampleToGroupBox::parse(sbgp_box).expect("parse sbgp");
    let serialized = parsed.try_to_bytes().unwrap();
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
    let path = test_fixtures::path("mp4/aac-sgpd.mp4");
    let data = std::fs::read(path).expect("fixture file must exist");
    let stbl = enter_chain(&data, &[b"moov", b"trak", b"mdia", b"minf", b"stbl"]);
    let sgpd_box = find_full_box(stbl, b"sgpd");

    let mut parsed = SampleGroupDescriptionBox::parse(sgpd_box).expect("parse sgpd");
    let original = parsed.try_to_bytes().unwrap();

    // Change roll_distance from -1 to -4
    parsed.entries[0] = SgpdEntry::Roll { roll_distance: -4 };
    let mutated = parsed.try_to_bytes().unwrap();
    assert_ne!(
        mutated, original,
        "mutating roll_distance must change serialized bytes"
    );
}

#[test]
fn sbgp_mutation_changes_bytes() {
    let path = test_fixtures::path("mp4/aac-sgpd.mp4");
    let data = std::fs::read(path).expect("fixture file must exist");
    let stbl = enter_chain(&data, &[b"moov", b"trak", b"mdia", b"minf", b"stbl"]);
    let sbgp_box = find_full_box(stbl, b"sbgp");

    let mut parsed = SampleToGroupBox::parse(sbgp_box).expect("parse sbgp");
    let original = parsed.try_to_bytes().unwrap();

    parsed.entries.push(SbgpEntry {
        sample_count: 999,
        group_description_index: 1,
    });
    let mutated = parsed.try_to_bytes().unwrap();
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

    let bytes = b.try_to_bytes().unwrap();
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
        parsed.try_to_bytes().unwrap(),
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
    let bytes = b.try_to_bytes().unwrap();
    let parsed = SubSampleInformationBox::parse(&bytes).expect("parse subs v1");
    assert_eq!(parsed, b);
    assert_eq!(
        parsed.try_to_bytes().unwrap(),
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
    let original = b.try_to_bytes().unwrap();
    let mut b2 = b;
    b2.entries[0].subsamples[0].subsample_size = 999;
    let mutated = b2.try_to_bytes().unwrap();
    assert_ne!(
        mutated, original,
        "mutating subsample_size must change serialized bytes"
    );
}

// ---------------------------------------------------------------------------
// ssrt long-form entry_count and per-entry guards
// (ISO/IEC 14496-12:2026 §8.7.10)
// ---------------------------------------------------------------------------

/// A long-form (`is_short_ref == 0`) table with zero entries is legal: the
/// body is the 4-byte version/flags plus a 16-bit `entry_count`.
#[test]
fn ssrt_long_form_empty_table_parses() {
    let empty: [u8; 16] = [
        0, 0, 0, 16, b's', b's', b'r', b't', 0, 0, 0, 0, 0x00, 0x00, 0x00, 0x00,
    ];
    let parsed = SubSampleReferenceTableBox::parse(&empty).unwrap();
    assert!(!parsed.is_short_ref);
    assert_eq!(parsed.flags, 0);
    assert!(parsed.entries.is_empty());
}

/// A long-form body that ends after one byte of the 16-bit `entry_count`
/// must be a typed error, not a read past the body.
#[test]
fn ssrt_body_ending_inside_entry_count_errors() {
    let cut: [u8; 13] = [0, 0, 0, 13, b's', b's', b'r', b't', 0, 0, 0, 0, 0x00];
    match SubSampleReferenceTableBox::parse(&cut) {
        Err(transmux::Error::BufferTooShort(e)) => {
            assert_eq!((e.need, e.have, e.what), (6, 5, "ssrt entry_count"));
        }
        other => panic!("expected BufferTooShort, got {other:?}"),
    }
}

/// A short-form body announcing one entry but carrying no entry byte must be
/// a typed error, not a read past the body.
#[test]
fn ssrt_short_form_count_without_entry_errors() {
    let cut: [u8; 13] = [0, 0, 0, 13, b's', b's', b'r', b't', 0, 0, 0, 0, 0x81];
    match SubSampleReferenceTableBox::parse(&cut) {
        Err(transmux::Error::BufferTooShort(e)) => {
            assert_eq!((e.need, e.have, e.what), (6, 5, "ssrt entry"));
        }
        other => panic!("expected BufferTooShort, got {other:?}"),
    }
}

/// The long-form `entry_count` is 15 bits: 300 entries must survive the high
/// byte rather than being truncated to 44.
#[test]
fn ssrt_long_form_entry_count_high_byte_round_trips() {
    let entry = SubsampleProperties {
        discardable: true,
        subsample_priority: None,
        codec_specific_parameters: None,
    };
    let b = SubSampleReferenceTableBox {
        flags: 0,
        is_short_ref: false,
        entries: vec![entry; 300],
    };
    let bytes = b.try_to_bytes().unwrap();
    assert_eq!(bytes.len(), 8 + 4 + 2 + 300);
    assert_eq!(&bytes[12..14], &[0x01, 0x2C]);
    let parsed = SubSampleReferenceTableBox::parse(&bytes).unwrap();
    assert_eq!(parsed.entries.len(), 300);
    assert_eq!(parsed, b);
}

// ---------------------------------------------------------------------------
// stts / ctts / cslg / elst body guards (ISO/IEC 14496-12:2026 §8.6.1, §8.6.6)
// ---------------------------------------------------------------------------

/// A body shorter than the 4-byte version/flags plus the 32-bit entry count
/// is rejected before any field read.
#[test]
fn timing_parse_body_short_bodies_are_errors() {
    for body in [&[][..], &[0u8, 0, 0][..]] {
        assert!(matches!(
            TimeToSampleBox::parse_body(body),
            Err(transmux::Error::BufferTooShort(_))
        ));
        assert!(matches!(
            CompositionOffsetBox::parse_body(body),
            Err(transmux::Error::BufferTooShort(_))
        ));
        assert!(matches!(
            CompositionToDecodeBox::parse_body(body),
            Err(transmux::Error::BufferTooShort(_))
        ));
        assert!(matches!(
            EditListBox::parse_body(body),
            Err(transmux::Error::BufferTooShort(_))
        ));
    }
}

/// An empty entry list is a valid 8-byte body for ctts and elst.
#[test]
fn timing_parse_body_empty_entry_lists_are_valid() {
    let ctts = CompositionOffsetBox::parse_body(&[0; 8]).unwrap();
    assert!(ctts.entries.is_empty());
    let elst = EditListBox::parse_body(&[0; 8]).unwrap();
    assert!(elst.entries.is_empty());
    let stts = TimeToSampleBox::parse_body(&[0; 8]).unwrap();
    assert!(stts.entries.is_empty());
}

/// Every byte of `sample_count` and `sample_delta` is read at its own index.
#[test]
fn stts_entry_fields_read_every_byte() {
    let mut body = [0u8; 24];
    body[4..8].copy_from_slice(&2u32.to_be_bytes());
    body[8..12].copy_from_slice(&0x0102_0304u32.to_be_bytes());
    body[12..16].copy_from_slice(&0x0506_0708u32.to_be_bytes());
    body[16..20].copy_from_slice(&0x090A_0B0Cu32.to_be_bytes());
    body[20..24].copy_from_slice(&0x0D0E_0F10u32.to_be_bytes());
    let parsed = TimeToSampleBox::parse_body(&body).unwrap();
    assert_eq!(
        parsed.entries,
        vec![
            SttsEntry {
                sample_count: 0x0102_0304,
                sample_delta: 0x0506_0708,
            },
            SttsEntry {
                sample_count: 0x090A_0B0C,
                sample_delta: 0x0D0E_0F10,
            },
        ]
    );
    assert_eq!(&parsed.try_to_bytes().unwrap()[8..], &body[..]);
}

/// Every byte of `sample_count` and `sample_offset` is read at its own index,
/// including a negative v1 offset.
#[test]
fn ctts_entry_fields_read_every_byte() {
    let mut body = [0u8; 24];
    body[0] = 1;
    body[4..8].copy_from_slice(&2u32.to_be_bytes());
    body[8..12].copy_from_slice(&0x0102_0304u32.to_be_bytes());
    body[12..16].copy_from_slice(&0x0506_0708u32.to_be_bytes());
    body[16..20].copy_from_slice(&0x090A_0B0Cu32.to_be_bytes());
    body[20..24].copy_from_slice(&0xFFFF_FFFEu32.to_be_bytes());
    let parsed = CompositionOffsetBox::parse_body(&body).unwrap();
    assert_eq!(
        parsed.entries,
        vec![
            CttsEntry {
                sample_count: 0x0102_0304,
                sample_offset: 0x0506_0708,
            },
            CttsEntry {
                sample_count: 0x090A_0B0C,
                sample_offset: -2,
            },
        ]
    );
    assert_eq!(&parsed.try_to_bytes().unwrap()[8..], &body[..]);
}

// ---------------------------------------------------------------------------
// elst entry guards (ISO/IEC 14496-12:2026 §8.6.6)
// ---------------------------------------------------------------------------

fn elst_body(version: u8, entry_count: u32, tail: usize) -> Vec<u8> {
    let mut body = vec![0u8; 8 + tail];
    body[0] = version;
    body[4..8].copy_from_slice(&entry_count.to_be_bytes());
    body
}

/// An entry shorter than its v0 (12) or v1 (20) wire size is rejected before
/// the read, whatever the cut point.
#[test]
fn elst_truncated_entry_is_an_error() {
    for tail in [4usize, 8, 11] {
        assert!(
            matches!(
                EditListBox::parse_body(&elst_body(0, 1, tail)),
                Err(transmux::Error::BufferTooShort(_))
            ),
            "v0 entry with {tail} bytes must be rejected"
        );
    }
    assert!(EditListBox::parse_body(&elst_body(0, 1, 12)).is_ok());
    assert!(matches!(
        EditListBox::parse_body(&elst_body(1, 1, 19)),
        Err(transmux::Error::BufferTooShort(_))
    ));
    assert!(EditListBox::parse_body(&elst_body(1, 1, 20)).is_ok());
}

/// Every byte of the 32-bit (v0) and 64-bit (v1) `segment_duration` and
/// `media_time` is read at its own index.
#[test]
fn elst_entry_fields_read_every_byte() {
    let mut body = [0u8; 20];
    body[4..8].copy_from_slice(&1u32.to_be_bytes());
    body[8..12].copy_from_slice(&0x0102_0304u32.to_be_bytes());
    body[12..16].copy_from_slice(&(-1i32).to_be_bytes());
    body[16..18].copy_from_slice(&1i16.to_be_bytes());
    let parsed = EditListBox::parse_body(&body).unwrap();
    assert_eq!(
        parsed.entries,
        vec![EditListEntry {
            segment_duration: 0x0102_0304,
            media_time: -1,
            media_rate_integer: 1,
            media_rate_fraction: 0,
        }]
    );
    assert_eq!(&parsed.try_to_bytes().unwrap()[8..], &body[..]);

    let mut body = [0u8; 28];
    body[0] = 1;
    body[4..8].copy_from_slice(&1u32.to_be_bytes());
    body[8..16].copy_from_slice(&0x0102_0304_0506_0708u64.to_be_bytes());
    body[16..24].copy_from_slice(&0x1112_1314_1516_1718i64.to_be_bytes());
    body[24..26].copy_from_slice(&1i16.to_be_bytes());
    let parsed = EditListBox::parse_body(&body).unwrap();
    assert_eq!(
        parsed.entries,
        vec![EditListEntry {
            segment_duration: 0x0102_0304_0506_0708,
            media_time: 0x1112_1314_1516_1718,
            media_rate_integer: 1,
            media_rate_fraction: 0,
        }]
    );
    assert_eq!(&parsed.try_to_bytes().unwrap()[8..], &body[..]);
}

/// An empty `edts`/`elst` is legal: a 16-byte box with `entry_count` 0.
#[test]
fn elst_empty_box_parses_and_round_trips() {
    let bytes: [u8; 16] = [
        0, 0, 0, 16, b'e', b'l', b's', b't', 0, 0, 0, 0, 0x00, 0x00, 0x00, 0x00,
    ];
    let parsed = EditListBox::parse(&bytes).unwrap();
    assert_eq!(parsed.version, 0);
    assert!(parsed.entries.is_empty());
    assert_eq!(parsed.serialized_len(), 16);
    assert_eq!(parsed.try_to_bytes().unwrap(), bytes);
}

/// A box shorter than the 8-byte header plus version/flags plus the entry
/// count is refused by the box guard itself.
#[test]
fn elst_box_short_of_its_count_errors() {
    let bytes: [u8; 8] = [0, 0, 0, 8, b'e', b'l', b's', b't'];
    match EditListBox::parse(&bytes) {
        Err(transmux::Error::BufferTooShort(e)) => {
            assert_eq!((e.need, e.have, e.what), (16, 8, "elst box"));
        }
        other => panic!("expected BufferTooShort, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// cslg field guards, widths and size (ISO/IEC 14496-12:2026 §8.6.1.4)
// ---------------------------------------------------------------------------

/// A payload shorter than the five fields is rejected with the size and the
/// field name the caller needs; trailing bytes beyond the fields are ignored.
#[test]
fn cslg_short_payload_reports_need_and_field() {
    let cases: [(u8, usize, usize, &str); 5] = [
        (0, 23, 24, "cslg v0 field"),
        (0, 19, 24, "cslg v0 field"),
        (0, 10, 24, "cslg v0 field"),
        (1, 39, 44, "cslg v1 field"),
        (1, 23, 44, "cslg v1 field"),
    ];
    for (version, body_len, need, what) in cases {
        let mut body = vec![0u8; body_len];
        body[0] = version;
        match CompositionToDecodeBox::parse_body(&body) {
            Err(transmux::Error::BufferTooShort(e)) => {
                assert_eq!((e.need, e.have, e.what), (need, body_len, what));
            }
            other => panic!("v{version} body {body_len}: expected BufferTooShort, got {other:?}"),
        }
    }

    let mut body = vec![0u8; 28];
    body[4..8].copy_from_slice(&0x0102_0304u32.to_be_bytes());
    let parsed = CompositionToDecodeBox::parse_body(&body).unwrap();
    assert_eq!(parsed.composition_to_dts_shift, 0x0102_0304);
    assert_eq!(parsed.composition_end_time, 0);
}

/// Every byte of each 32-bit (v0) and 64-bit (v1) field is read at its own
/// index, including signed fields.
#[test]
fn cslg_fields_read_every_byte() {
    let v0_values: [i32; 5] = [
        0x0102_0304,
        -0x0102_0304,
        0x0A0B_0C0D,
        0x1020_3040,
        0x7F6E_5D4C,
    ];
    let mut body = vec![0u8; 24];
    for (i, v) in v0_values.iter().enumerate() {
        body[4 + i * 4..8 + i * 4].copy_from_slice(&v.to_be_bytes());
    }
    let parsed = CompositionToDecodeBox::parse_body(&body).unwrap();
    assert_eq!(parsed.composition_to_dts_shift, i64::from(0x0102_0304));
    assert_eq!(
        parsed.least_decode_to_display_delta,
        i64::from(-0x0102_0304)
    );
    assert_eq!(parsed.greatest_decode_to_display_delta, 0x0A0B_0C0D);
    assert_eq!(parsed.composition_start_time, 0x1020_3040);
    assert_eq!(parsed.composition_end_time, 0x7F6E_5D4C);
    assert_eq!(&parsed.try_to_bytes().unwrap()[8..], &body[..]);

    let v1_values: [i64; 5] = [
        0x0102_0304_0506_0708,
        -0x0102_0304_0506_0708,
        0x0A0B_0C0D_0E0F_1011,
        0x1112_1314_1516_1718,
        0x2122_2324_2526_2728,
    ];
    let mut body = vec![0u8; 44];
    body[0] = 1;
    for (i, v) in v1_values.iter().enumerate() {
        body[4 + i * 8..12 + i * 8].copy_from_slice(&v.to_be_bytes());
    }
    let parsed = CompositionToDecodeBox::parse_body(&body).unwrap();
    assert_eq!(parsed.composition_to_dts_shift, 0x0102_0304_0506_0708);
    assert_eq!(parsed.least_decode_to_display_delta, -0x0102_0304_0506_0708);
    assert_eq!(
        parsed.greatest_decode_to_display_delta,
        0x0A0B_0C0D_0E0F_1011
    );
    assert_eq!(parsed.composition_start_time, 0x1112_1314_1516_1718);
    assert_eq!(parsed.composition_end_time, 0x2122_2324_2526_2728);
    assert_eq!(&parsed.try_to_bytes().unwrap()[8..], &body[..]);
}

/// The reported length, the box size field and the returned write count agree,
/// and a buffer one byte short is refused.
#[test]
fn cslg_serialized_len_and_size_field() {
    for (version, len) in [(0u8, 32usize), (1, 52)] {
        let b = CompositionToDecodeBox {
            version,
            flags: 0x000010,
            composition_to_dts_shift: -1,
            least_decode_to_display_delta: 2,
            greatest_decode_to_display_delta: 3,
            composition_start_time: 4,
            composition_end_time: 5,
        };
        assert_eq!(b.serialized_len(), len);
        let mut buf = vec![0u8; len];
        assert_eq!(b.serialize_into(&mut buf).unwrap(), len);
        assert_eq!(&buf[0..4], &(len as u32).to_be_bytes());
        assert_eq!(&buf[4..8], b"cslg");
        assert_eq!(b.try_to_bytes().unwrap(), buf);
        assert!(matches!(
            b.serialize_into(&mut buf[..len - 1]),
            Err(transmux::Error::OutputBufferTooSmall(_))
        ));
    }
}

// ---------------------------------------------------------------------------
// sidx guards, field widths and sub-byte ranges
// (ISO/IEC 14496-12:2026 §8.14.3)
// ---------------------------------------------------------------------------

/// A sidx body cut in the fixed part, before the count, or inside the last
/// reference entry is a typed error, never a read past the body.
#[test]
fn sidx_truncated_bodies_are_errors() {
    for len in [12usize, 19] {
        let mut body = vec![0u8; len];
        body[8..12].copy_from_slice(&90000u32.to_be_bytes());
        match SegmentIndexBox::parse_body(&body) {
            Err(transmux::Error::BufferTooShort(e)) => {
                assert_eq!((e.need, e.have, e.what), (20, len, "sidx body"));
            }
            other => panic!("body {len}: expected BufferTooShort, got {other:?}"),
        }
    }

    for len in [20usize, 21, 22, 23] {
        let mut body = vec![0u8; len];
        body[8..12].copy_from_slice(&90000u32.to_be_bytes());
        match SegmentIndexBox::parse_body(&body) {
            Err(transmux::Error::BufferTooShort(e)) => {
                assert_eq!((e.need, e.have, e.what), (24, len, "sidx reserved+count"));
            }
            other => panic!("body {len}: expected BufferTooShort, got {other:?}"),
        }
    }

    let mut body = vec![0u8; 42];
    body[8..12].copy_from_slice(&90000u32.to_be_bytes());
    body[22..24].copy_from_slice(&2u16.to_be_bytes());
    match SegmentIndexBox::parse_body(&body) {
        Err(transmux::Error::BufferTooShort(e)) => {
            assert_eq!((e.need, e.have, e.what), (48, 42, "sidx reference entry"));
        }
        other => panic!("expected BufferTooShort, got {other:?}"),
    }

    let mut body = vec![0u8; 24];
    body[8..12].copy_from_slice(&90000u32.to_be_bytes());
    let parsed = SegmentIndexBox::parse_body(&body).unwrap();
    assert!(parsed.references.is_empty());
}

/// Every byte of `reference_id`, `earliest_presentation_time`, `first_offset`
/// and the 16-bit `reference_count` is read at its own index.
#[test]
fn sidx_fields_read_every_byte() {
    let mut body = vec![0u8; 24];
    body[4..8].copy_from_slice(&0x0102_0304u32.to_be_bytes());
    body[8..12].copy_from_slice(&90000u32.to_be_bytes());
    body[12..16].copy_from_slice(&0x1122_3344u32.to_be_bytes());
    body[16..20].copy_from_slice(&0x0A0B_0C0Du32.to_be_bytes());
    let parsed = SegmentIndexBox::parse_body(&body).unwrap();
    assert_eq!(parsed.reference_id, 0x0102_0304);
    assert_eq!(parsed.earliest_presentation_time, 0x1122_3344);
    assert_eq!(parsed.first_offset, 0x0A0B_0C0D);
    assert!(parsed.references.is_empty());

    let mut body = vec![0u8; 32];
    body[0] = 1;
    body[4..8].copy_from_slice(&0x2122_2324u32.to_be_bytes());
    body[8..12].copy_from_slice(&90000u32.to_be_bytes());
    body[12..20].copy_from_slice(&0x0102_0304_0506_0708u64.to_be_bytes());
    body[20..28].copy_from_slice(&0x1112_1314_1516_1718u64.to_be_bytes());
    let parsed = SegmentIndexBox::parse_body(&body).unwrap();
    assert_eq!(parsed.reference_id, 0x2122_2324);
    assert_eq!(parsed.earliest_presentation_time, 0x0102_0304_0506_0708);
    assert_eq!(parsed.first_offset, 0x1112_1314_1516_1718);
    assert_eq!(&parsed.try_to_bytes().unwrap()[8..], &body[..]);

    let reference = SidxReference {
        reference_type: 1,
        referenced_size: 1000,
        subsegment_duration: 3003,
        starts_with_sap: 1,
        sap_type: 1,
        sap_delta_time: 0,
    };
    let b = SegmentIndexBox {
        version: 0,
        flags: 0,
        reference_id: 1,
        timescale: Timescale::try_new(90000).unwrap(),
        earliest_presentation_time: 0,
        first_offset: 0,
        references: vec![reference; 300],
    };
    let bytes = b.try_to_bytes().unwrap();
    assert_eq!(&bytes[30..32], &0x012Cu16.to_be_bytes());
    let parsed = SegmentIndexBox::parse(&bytes).unwrap();
    assert_eq!(parsed.references.len(), 300);
    assert_eq!(parsed, b);
}

/// `starts_with_SAP` is bit 31 alone: an entry without a SAP must not parse
/// as one, and an entry with one must still report it.
#[test]
fn sidx_starts_with_sap_is_a_single_bit() {
    let mut body = vec![0u8; 60];
    body[8..12].copy_from_slice(&90000u32.to_be_bytes());
    body[22..24].copy_from_slice(&3u16.to_be_bytes());
    body[32..36].copy_from_slice(&0x0000_0000u32.to_be_bytes());
    body[44..48].copy_from_slice(&0x1000_0000u32.to_be_bytes());
    body[56..60].copy_from_slice(&0x8000_0000u32.to_be_bytes());
    let parsed = SegmentIndexBox::parse_body(&body).unwrap();
    assert_eq!(parsed.references.len(), 3);
    assert_eq!(parsed.references[0].starts_with_sap, 0);
    assert_eq!(parsed.references[0].sap_type, 0);
    assert_eq!(parsed.references[1].starts_with_sap, 0);
    assert_eq!(parsed.references[1].sap_type, 1);
    assert_eq!(parsed.references[2].starts_with_sap, 1);
    assert_eq!(parsed.references[2].sap_type, 0);
}

/// Reference sub-byte fields are range-checked on write: an out-of-range
/// `reference_type`, `starts_with_sap` or `sap_type` is an error, not a
/// misframed entry, and the field maxima still round-trip.
fn sidx_with_reference(reference_type: u8, starts_with_sap: u8, sap_type: u8) -> SegmentIndexBox {
    SegmentIndexBox {
        version: 0,
        flags: 0,
        reference_id: 1,
        timescale: Timescale::try_new(90000).unwrap(),
        earliest_presentation_time: 0,
        first_offset: 0,
        references: vec![SidxReference {
            reference_type,
            referenced_size: 1,
            subsegment_duration: 1,
            starts_with_sap,
            sap_type,
            sap_delta_time: 0,
        }],
    }
}

fn assert_overflow(bad: SegmentIndexBox, field: &str, value: u64, max: u64) {
    match bad.try_to_bytes() {
        Err(transmux::Error::FieldOverflow(e)) => {
            assert_eq!(e.field, field);
            assert_eq!(e.value, value);
            assert_eq!(e.max, max);
        }
        other => panic!("{field} = {value}: expected FieldOverflow, got {other:?}"),
    }
}

#[test]
fn sidx_serialize_rejects_reference_type_2() {
    assert_overflow(sidx_with_reference(2, 0, 0), "reference_type", 2, 1);
}

#[test]
fn sidx_serialize_rejects_starts_with_sap_2() {
    assert_overflow(sidx_with_reference(0, 2, 0), "starts_with_sap", 2, 1);
}

#[test]
fn sidx_serialize_rejects_sap_type_8() {
    assert_overflow(sidx_with_reference(0, 0, 8), "sap_type", 8, 7);
}

#[test]
fn sidx_serialize_accepts_sub_byte_field_maxima() {
    let high = sidx_with_reference(1, 1, 7);
    let bytes = high.try_to_bytes().unwrap();
    assert_eq!(&bytes[32..36], &0x8000_0001u32.to_be_bytes());
    assert_eq!(&bytes[40..44], &0xF000_0000u32.to_be_bytes());
    assert_eq!(SegmentIndexBox::parse(&bytes).unwrap(), high);
}
