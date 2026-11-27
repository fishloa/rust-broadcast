//! Real-fixture round-trip tests for prft, sgpd, sbgp, and subs boxes (#435).
//!
//! Gate criteria (ungameable):
//! - `prft` from `fixtures/mp4/prft.mp4`: parse fields + byte-exact round-trip.
//! - `sgpd('roll')` + `sbgp` from `fixtures/mp4/aac-sgpd.mp4`: parse
//!   `roll_distance=-1` + byte-exact round-trip.
//! - `subs`: build from a spec vector, round-trip.
//! - Mutation: mutating a field changes the serialized bytes.

use broadcast_core::timescale::Timescale;
use broadcast_core::{Parse, Serialize};
use isobmff::Error;
use isobmff::TrackId;
use isobmff::sample_groups::{
    CodecSpecificParameters, GROUPING_TYPE_ROLL, ProducerReferenceTimeBox,
    SampleGroupDescriptionBox, SampleToGroupBox, SbgpEntry, SgpdEntry, SubSampleDescriptor,
    SubSampleInformationBox, SubSampleInformationBoxV2, SubSampleReferenceTableBox, SubsEntry,
    SubsV2Entry, SubsV2Source, SubsV2Subsample, SubsampleProperties,
};
use isobmff::timing::{
    CompositionOffsetBox, CompositionToDecodeBox, CttsEntry, EditListBox, EditListEntry,
    SegmentIndexBox, SidxReference, SttsEntry, TimeToSampleBox,
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

/// `sgpd` version 1 with `default_length == 0` states each entry's width in a
/// `description_length` prefix (ISO/IEC 14496-12:2026 §8.9.3.2), so uniform
/// entries must not be folded back into the shared-length form. The bytes are
/// `fixtures/mp4/aac-sgpd.mp4`'s `sgpd` (box at offset 37695, its
/// `default_length` field at 37711, `entry_count` at 37715) rewritten to state
/// `default_length = 0` and prefix the one entry with its 2-byte width.
#[test]
fn sgpd_v1_zero_default_length_with_uniform_entries_keeps_the_form() {
    let data = std::fs::read(test_fixtures::path("mp4/aac-sgpd.mp4")).expect("fixture file");
    let stbl = enter_chain(&data, &[b"moov", b"trak", b"mdia", b"minf", b"stbl"]);
    let sgpd_box = find_full_box(stbl, b"sgpd");
    let mut bytes = sgpd_box[..24].to_vec();
    bytes[16..20].copy_from_slice(&0u32.to_be_bytes());
    bytes.extend_from_slice(&2u32.to_be_bytes());
    bytes.extend_from_slice(&sgpd_box[24..]);
    let len = u32::try_from(bytes.len()).expect("box length");
    bytes[..4].copy_from_slice(&len.to_be_bytes());
    assert_eq!(
        bytes[16..20],
        [0, 0, 0, 0],
        "mutated default_length is zero"
    );
    let parsed = SampleGroupDescriptionBox::parse(&bytes).expect("parse");
    assert_eq!(parsed.default_length, 0);
    assert_eq!(parsed.entries, vec![SgpdEntry::Roll { roll_distance: -1 }]);
    assert_eq!(parsed.try_to_bytes().expect("serialize"), bytes);
}

/// A version-1 `sgpd` may declare a non-zero `default_length` with no entries
/// (ISO/IEC 14496-12:2026 §8.9.3.2); the value is the file's, not the
/// serializer's to re-derive. Same fixture and box as above, with
/// `entry_count` (offset 37715) zeroed and the entry's two bytes dropped.
#[test]
fn sgpd_v1_non_zero_default_length_with_no_entries_keeps_the_value() {
    let data = std::fs::read(test_fixtures::path("mp4/aac-sgpd.mp4")).expect("fixture file");
    let stbl = enter_chain(&data, &[b"moov", b"trak", b"mdia", b"minf", b"stbl"]);
    let sgpd_box = find_full_box(stbl, b"sgpd");
    let mut bytes = sgpd_box[..20].to_vec();
    bytes.extend_from_slice(&0u32.to_be_bytes());
    let len = u32::try_from(bytes.len()).expect("box length");
    bytes[..4].copy_from_slice(&len.to_be_bytes());
    assert_eq!(bytes[16..20], [0, 0, 0, 2], "default_length stays 2");
    let parsed = SampleGroupDescriptionBox::parse(&bytes).expect("parse");
    assert_eq!(parsed.default_length, 2);
    assert!(parsed.entries.is_empty());
    assert_eq!(parsed.try_to_bytes().expect("serialize"), bytes);
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
        Err(isobmff::Error::BufferTooShort(e)) => {
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
        Err(isobmff::Error::BufferTooShort(e)) => {
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
        reserved: 0,
    };
    let b = SubSampleReferenceTableBox {
        version: 0,
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
            Err(isobmff::Error::BufferTooShort(_))
        ));
        assert!(matches!(
            CompositionOffsetBox::parse_body(body),
            Err(isobmff::Error::BufferTooShort(_))
        ));
        assert!(matches!(
            CompositionToDecodeBox::parse_body(body),
            Err(isobmff::Error::BufferTooShort(_))
        ));
        assert!(matches!(
            EditListBox::parse_body(body),
            Err(isobmff::Error::BufferTooShort(_))
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
                Err(isobmff::Error::BufferTooShort(_))
            ),
            "v0 entry with {tail} bytes must be rejected"
        );
    }
    assert!(EditListBox::parse_body(&elst_body(0, 1, 12)).is_ok());
    assert!(matches!(
        EditListBox::parse_body(&elst_body(1, 1, 19)),
        Err(isobmff::Error::BufferTooShort(_))
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
        Err(isobmff::Error::BufferTooShort(e)) => {
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
            Err(isobmff::Error::BufferTooShort(e)) => {
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
            Err(isobmff::Error::OutputBufferTooSmall(_))
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
            Err(isobmff::Error::BufferTooShort(e)) => {
                assert_eq!((e.need, e.have, e.what), (20, len, "sidx body"));
            }
            other => panic!("body {len}: expected BufferTooShort, got {other:?}"),
        }
    }

    for len in [20usize, 21, 22, 23] {
        let mut body = vec![0u8; len];
        body[8..12].copy_from_slice(&90000u32.to_be_bytes());
        match SegmentIndexBox::parse_body(&body) {
            Err(isobmff::Error::BufferTooShort(e)) => {
                assert_eq!((e.need, e.have, e.what), (24, len, "sidx reserved+count"));
            }
            other => panic!("body {len}: expected BufferTooShort, got {other:?}"),
        }
    }

    let mut body = vec![0u8; 42];
    body[8..12].copy_from_slice(&90000u32.to_be_bytes());
    body[22..24].copy_from_slice(&2u16.to_be_bytes());
    match SegmentIndexBox::parse_body(&body) {
        Err(isobmff::Error::BufferTooShort(e)) => {
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
    let b = SegmentIndexBox::new(
        0,
        0,
        1,
        Timescale::try_new(90000).unwrap(),
        0,
        0,
        vec![reference; 300],
    );
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
    SegmentIndexBox::new(
        0,
        0,
        1,
        Timescale::try_new(90000).unwrap(),
        0,
        0,
        vec![SidxReference {
            reference_type,
            referenced_size: 1,
            subsegment_duration: 1,
            starts_with_sap,
            sap_type,
            sap_delta_time: 0,
        }],
    )
}

fn assert_overflow(bad: SegmentIndexBox, field: &str, value: u64, max: u64) {
    match bad.try_to_bytes() {
        Err(isobmff::Error::FieldOverflow(e)) => {
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
// ---------------------------------------------------------------------------
// sbgp: the minimal empty box and every truncated prefix (#3375)
// ---------------------------------------------------------------------------

const SBGP_EMPTY_V0: &[u8] = &[
    0, 0, 0, 20, b's', b'b', b'g', b'p', 0, 0, 0, 0, b'r', b'o', b'l', b'l', 0, 0, 0, 0,
];

fn assert_buffer_too_short<T: core::fmt::Debug>(
    r: Result<T, Error>,
    need: usize,
    have: usize,
    what: &str,
) {
    match r {
        Err(Error::BufferTooShort(b)) => {
            assert_eq!((b.need, b.have, b.what), (need, have, what), "shortfall")
        }
        other => panic!("expected BufferTooShort, got {other:?}"),
    }
}

#[test]
fn sbgp_empty_box_parses_and_round_trips() {
    let parsed = SampleToGroupBox::parse(SBGP_EMPTY_V0).expect("20-byte empty sbgp");
    assert_eq!(parsed.version, 0);
    assert_eq!(parsed.grouping_type, GROUPING_TYPE_ROLL);
    assert!(parsed.entries.is_empty(), "entry_count is zero");
    assert_eq!(parsed.serialized_len(), 20);
    assert_eq!(parsed.try_to_bytes().unwrap(), SBGP_EMPTY_V0);
}

#[test]
fn sbgp_truncated_bodies_report_the_exact_shortfall() {
    for cut in 0..12 {
        assert_buffer_too_short(
            SampleToGroupBox::parse_body(&[0u8; 12][..cut]),
            12,
            cut,
            "sbgp body",
        );
    }

    let mut v1 = vec![1u8, 0, 0, 0];
    v1.extend_from_slice(b"roll");
    assert_buffer_too_short(SampleToGroupBox::parse_body(&v1), 12, 8, "sbgp body");

    let mut v1_gtp = v1.clone();
    v1_gtp.extend_from_slice(&[0x11, 0x12, 0x13, 0x14]);
    assert_buffer_too_short(
        SampleToGroupBox::parse_body(&v1_gtp),
        16,
        12,
        "sbgp entry_count",
    );

    let mut v0_one_entry = vec![0u8, 0, 0, 0];
    v0_one_entry.extend_from_slice(b"roll");
    v0_one_entry.extend_from_slice(&[0, 0, 0, 1]);
    assert_buffer_too_short(
        SampleToGroupBox::parse_body(&v0_one_entry),
        20,
        12,
        "sbgp entry",
    );
    let mut entry_short = v0_one_entry.clone();
    entry_short.push(0x01);
    assert_buffer_too_short(
        SampleToGroupBox::parse_body(&entry_short),
        20,
        13,
        "sbgp entry",
    );

    let mut v1_one_entry = v1_gtp.clone();
    v1_one_entry.extend_from_slice(&[0, 0, 0, 1]);
    assert_buffer_too_short(
        SampleToGroupBox::parse_body(&v1_one_entry),
        24,
        16,
        "sbgp entry",
    );

    assert_buffer_too_short(SampleToGroupBox::parse(&[0u8; 12]), 16, 12, "sbgp box");
}

/// An `entry_count` above 65 535 is read from all four of its bytes: a body
/// declaring `0x0001_0002` entries but holding two must run out at the third.
#[test]
fn sbgp_entry_count_high_bytes_are_read() {
    let mut body = vec![0u8; 4];
    body.extend_from_slice(b"roll");
    body.extend_from_slice(&[0x00, 0x01, 0x00, 0x02]);
    body.extend_from_slice(&[0, 0, 0, 1, 0, 0, 0, 1]);
    body.extend_from_slice(&[0, 0, 0, 2, 0, 0, 0, 2]);
    assert_buffer_too_short(SampleToGroupBox::parse_body(&body), 36, 28, "sbgp entry");
}

// ---------------------------------------------------------------------------
// sbgp: every entry field keeps its own bytes (#3376)
// ---------------------------------------------------------------------------

const SBGP_DISTINCT_V1: &[u8] = &[
    0, 0, 0, 40, b's', b'b', b'g', b'p', // box header
    1, 0, 0, 0, // version 1, flags 0
    b'r', b'o', b'l', b'l', // grouping_type
    0x11, 0x12, 0x13, 0x14, // grouping_type_parameter
    0, 0, 0, 2, // entry_count
    0x01, 0x02, 0x03, 0x04, 0x0A, 0x0B, 0x0C, 0x0D, // entry 0
    0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, // entry 1
];

#[test]
fn sbgp_distinct_field_bytes_round_trip() {
    let parsed = SampleToGroupBox::parse(SBGP_DISTINCT_V1).expect("v1 sbgp with two entries");
    assert_eq!(parsed.version, 1);
    assert_eq!(parsed.grouping_type_parameter, Some(0x1112_1314));
    assert_eq!(
        parsed.entries,
        vec![
            SbgpEntry {
                sample_count: 0x0102_0304,
                group_description_index: 0x0A0B_0C0D,
            },
            SbgpEntry {
                sample_count: 0x0506_0708,
                group_description_index: 0x090A_0B0C,
            },
        ]
    );
    assert_eq!(parsed.try_to_bytes().unwrap(), SBGP_DISTINCT_V1);
}

// ---------------------------------------------------------------------------
// subs v0/v1: serialized_len, entry wire length and the box size field (#3377)
// ---------------------------------------------------------------------------

const SUBS_V0_GOLDEN: &[u8] = &[
    0, 0, 0, 38, b's', b'u', b'b', b's', // box header
    0, 0, 0, 0, // version 0, flags 0
    0, 0, 0, 1, // entry_count
    0x01, 0x02, 0x03, 0x04, // sample_delta
    0, 2, // subsample_count
    0x0A, 0x0B, 0x0C, 0x0D, 0xAA, 0xBB, 0xCC, 0xDD, // size, prio, disc, csp
    0x0E, 0x0F, 0x10, 0x11, 0x12, 0x34, 0x56, 0x78, // size, prio, disc, csp
];

const SUBS_V1_GOLDEN: &[u8] = &[
    0, 0, 0, 42, b's', b'u', b'b', b's', // box header
    1, 0, 0, 0, // version 1, flags 0
    0, 0, 0, 1, // entry_count
    0x01, 0x02, 0x03, 0x04, // sample_delta
    0, 2, // subsample_count
    0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F, 0x11, 0x12, 0x13, 0x14, // subsample 0
    0x15, 0x16, 0x17, 0x18, 0x19, 0x1A, 0x1B, 0x1C, 0x1D, 0x1E, // subsample 1
];

fn subs_v0_golden_struct() -> SubSampleInformationBox {
    SubSampleInformationBox {
        version: 0,
        flags: 0,
        entries: vec![SubsEntry {
            sample_delta: 0x0102_0304,
            subsamples: vec![
                SubSampleDescriptor {
                    subsample_size: 0x0A0B,
                    subsample_priority: 0x0C,
                    discardable: 0x0D,
                    codec_specific_parameters: 0xAABB_CCDD,
                },
                SubSampleDescriptor {
                    subsample_size: 0x0E0F,
                    subsample_priority: 0x10,
                    discardable: 0x11,
                    codec_specific_parameters: 0x1234_5678,
                },
            ],
        }],
    }
}

fn subs_v1_golden_struct() -> SubSampleInformationBox {
    SubSampleInformationBox {
        version: 1,
        flags: 0,
        entries: vec![SubsEntry {
            sample_delta: 0x0102_0304,
            subsamples: vec![
                SubSampleDescriptor {
                    subsample_size: 0x0A0B_0C0D,
                    subsample_priority: 0x0E,
                    discardable: 0x0F,
                    codec_specific_parameters: 0x1112_1314,
                },
                SubSampleDescriptor {
                    subsample_size: 0x1516_1718,
                    subsample_priority: 0x19,
                    discardable: 0x1A,
                    codec_specific_parameters: 0x1B1C_1D1E,
                },
            ],
        }],
    }
}

#[test]
fn subs_v0_and_v1_goldens_and_size_field() {
    for (golden, b) in [
        (SUBS_V0_GOLDEN, subs_v0_golden_struct()),
        (SUBS_V1_GOLDEN, subs_v1_golden_struct()),
    ] {
        let bytes = b.try_to_bytes().unwrap();
        assert_eq!(bytes, golden, "serialize must match the hand-built golden");
        assert_eq!(b.serialized_len(), golden.len());
        assert_eq!(
            u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize,
            bytes.len(),
            "the box size field must cover the whole box"
        );
        let parsed = SubSampleInformationBox::parse(golden).unwrap();
        assert_eq!(parsed, b);
        assert_eq!(parsed.try_to_bytes().unwrap(), golden);
    }
}

/// A `subs` `entry_count` above 65 535 is read from all four of its bytes: a
/// body declaring `0x0001_0002` entries but holding two must run out at the
/// third.
#[test]
fn subs_entry_count_high_bytes_are_read() {
    let mut body = vec![0u8, 0, 0, 0];
    body.extend_from_slice(&[0x00, 0x01, 0x00, 0x02]);
    body.extend_from_slice(&[0, 0, 0, 1, 0, 0]);
    body.extend_from_slice(&[0, 0, 0, 2, 0, 0]);
    assert_buffer_too_short(
        SubSampleInformationBox::parse_body(&body),
        26,
        20,
        "subs entry header",
    );
}

// ---------------------------------------------------------------------------
// subs v0/v1: an entry with zero subsamples is legal, and one-byte
// truncations report the exact shortfall (#3378)
// ---------------------------------------------------------------------------

const SUBS_V0_BODY: &[u8] = &[
    0, 0, 0, 0, // version 0, flags 0
    0, 0, 0, 1, // entry_count
    0x01, 0x02, 0x03, 0x04, // sample_delta
    0, 2, // subsample_count
    0x0A, 0x0B, 0x0C, 0x0D, 0xAA, 0xBB, 0xCC, 0xDD, // subsample 0
    0x0E, 0x0F, 0x10, 0x11, 0x12, 0x34, 0x56, 0x78, // subsample 1
];

const SUBS_V1_BODY: &[u8] = &[
    1, 0, 0, 0, // version 1, flags 0
    0, 0, 0, 1, // entry_count
    0x01, 0x02, 0x03, 0x04, // sample_delta
    0, 2, // subsample_count
    0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F, 0x11, 0x12, 0x13, 0x14, // subsample 0
    0x15, 0x16, 0x17, 0x18, 0x19, 0x1A, 0x1B, 0x1C, 0x1D, 0x1E, // subsample 1
];

#[test]
fn subs_entry_with_zero_subsamples_ending_the_body_round_trips() {
    for version in [0u8, 1] {
        let b = SubSampleInformationBox {
            version,
            flags: 0,
            entries: vec![SubsEntry {
                sample_delta: 0x0102_0304,
                subsamples: vec![],
            }],
        };
        let bytes = b.try_to_bytes().unwrap();
        assert_eq!(bytes.len(), 22);
        let parsed = SubSampleInformationBox::parse(&bytes).unwrap();
        assert_eq!(parsed.entries.len(), 1);
        assert!(parsed.entries[0].subsamples.is_empty());
        assert_eq!(parsed.try_to_bytes().unwrap(), bytes);
    }
}

#[test]
fn subs_truncated_bodies_report_the_exact_shortfall() {
    for body in [SUBS_V0_BODY, SUBS_V1_BODY] {
        assert_buffer_too_short(
            SubSampleInformationBox::parse_body(&body[..7]),
            8,
            7,
            "subs body",
        );
        assert_buffer_too_short(
            SubSampleInformationBox::parse_body(&body[..13]),
            14,
            13,
            "subs entry header",
        );
        let first_subsample_end = 14 + 8 + if body == SUBS_V1_BODY { 2 } else { 0 };
        assert_buffer_too_short(
            SubSampleInformationBox::parse_body(&body[..first_subsample_end - 1]),
            first_subsample_end,
            first_subsample_end - 1,
            "subs subsample",
        );
        assert_buffer_too_short(
            SubSampleInformationBox::parse_body(&body[..body.len() - 1]),
            body.len(),
            body.len() - 1,
            "subs subsample",
        );
        assert!(SubSampleInformationBox::parse_body(body).is_ok());
    }
}

// ---------------------------------------------------------------------------
// subs: entry_count, sample_delta and v1 subsample_size keep their own
// bytes (#3379)
// ---------------------------------------------------------------------------

#[test]
fn subs_v1_wide_fields_keep_every_byte() {
    let mut entries = vec![SubsEntry {
        sample_delta: 0x0102_0304,
        subsamples: vec![SubSampleDescriptor {
            subsample_size: 0x0A0B_0C0D,
            subsample_priority: 0x0E,
            discardable: 0x0F,
            codec_specific_parameters: 0x1112_1314,
        }],
    }];
    for i in 1..257u32 {
        entries.push(SubsEntry {
            sample_delta: i,
            subsamples: vec![],
        });
    }
    let b = SubSampleInformationBox {
        version: 1,
        flags: 1,
        entries,
    };
    let bytes = b.try_to_bytes().unwrap();
    let parsed = SubSampleInformationBox::parse(&bytes).expect("257 subs entries");
    assert_eq!(parsed.entries.len(), 257);
    assert_eq!(parsed.entries[0].sample_delta, 0x0102_0304);
    assert_eq!(parsed.entries[0].subsamples[0].subsample_size, 0x0A0B_0C0D);
    assert_eq!(parsed.entries[0].subsamples[0].subsample_priority, 0x0E);
    assert_eq!(parsed.try_to_bytes().unwrap(), bytes);
}

// ---------------------------------------------------------------------------
// subs v2 / ssrt: the cursor advances past the codec-specific parameters,
// and a short property payload is a reported shortfall (#3381)
// ---------------------------------------------------------------------------

const SUBS_V2_TWO_SUBSAMPLES: &[u8] = &[
    0, 0, 0, 34, b's', b'u', b'b', b's', // box header
    2, 0, 0, 0, // version 2, flags 0
    0, 0, 0, 1,    // entry_count
    0xC0, // 16-bit size, 7-bit reference
    0, 0, 0, 1, // sample_delta
    0, 2, // subsample_count
    0x00, 0x10, 0x38, 0xAA, 0x11, 0x22, 0x33, // size, flags, prio, csp
    0x00, 0x20, 0x20, 0xBB, // size, flags, prio
];

const SSRT_TWO_ENTRIES: &[u8] = &[
    0, 0, 0, 21, b's', b's', b'r', b't', // box header
    0, 0, 0, 0, // flags
    0, 2, // entry_count (15-bit form)
    0x70, 0xAA, 0x11, 0x22, 0x33, // priority + 3-byte csp
    0x40, 0xBB, // priority only
];

#[test]
fn subs_v2_and_ssrt_advance_past_the_codec_specific_parameters() {
    let parsed = SubSampleInformationBoxV2::parse(SUBS_V2_TWO_SUBSAMPLES).unwrap();
    assert_eq!(
        parsed.entries[0].subsamples,
        vec![
            SubsV2Subsample {
                subsample_size: 0x10,
                source: SubsV2Source::Explicit(SubsampleProperties {
                    discardable: false,
                    subsample_priority: Some(0xAA),
                    codec_specific_parameters: Some(CodecSpecificParameters {
                        len: 3,
                        value: 0x0011_2233,
                    }),
                    reserved: 0,
                }),
            },
            SubsV2Subsample {
                subsample_size: 0x20,
                source: SubsV2Source::Explicit(SubsampleProperties {
                    discardable: false,
                    subsample_priority: Some(0xBB),
                    codec_specific_parameters: None,
                    reserved: 0,
                }),
            },
        ]
    );
    assert_eq!(parsed.try_to_bytes().unwrap(), SUBS_V2_TWO_SUBSAMPLES);

    let ssrt = SubSampleReferenceTableBox::parse(SSRT_TWO_ENTRIES).unwrap();
    assert_eq!(
        ssrt.entries,
        vec![
            SubsampleProperties {
                discardable: false,
                subsample_priority: Some(0xAA),
                codec_specific_parameters: Some(CodecSpecificParameters {
                    len: 3,
                    value: 0x0011_2233,
                }),
                reserved: 0,
            },
            SubsampleProperties {
                discardable: false,
                subsample_priority: Some(0xBB),
                codec_specific_parameters: None,
                reserved: 0,
            },
        ]
    );
    assert_eq!(ssrt.try_to_bytes().unwrap(), SSRT_TWO_ENTRIES);
}

/// No committed fixture carries a `subs` or an `ssrt` box (the box audit lists
/// both as typed boxes with no real occurrence), so these are spec-layout
/// vectors. `subs` version 2's byte after `entry_count` is `is_size_16_bits`,
/// `is_short_ref` and six reserved bits (ISO/IEC 14496-12:2026 §8.7.7.3):
/// every bit is the file's, so all of them come back.
#[test]
fn subs_v2_keeps_the_reserved_header_bits() {
    let bytes: &[u8] = &[
        0, 0, 0, 17, b's', b'u', b'b', b's', // box header
        2, 0, 0, 0, // version 2, flags 0
        0, 0, 0, 0,    // entry_count
        0xFF, // 16-bit size, 7-bit reference, six reserved bits set
    ];
    let parsed = SubSampleInformationBoxV2::parse(bytes).expect("parse");
    assert_eq!(parsed.reserved, 0x3F);
    assert_eq!(parsed.try_to_bytes().expect("serialize"), bytes);
}

/// An explicit version-2 sub-sample property byte ends with two reserved bits
/// after `csp_length_minus_1` (§8.7.7.3), and an `ssrt` entry byte with three
/// after it (§8.7.10). Both are carried.
#[test]
fn subs_v2_and_ssrt_keep_their_property_reserved_bits() {
    let subs: &[u8] = &[
        0, 0, 0, 26, b's', b'u', b'b', b's', // box header
        2, 0, 0, 0, // version 2, flags 0
        0, 0, 0, 1,    // entry_count
        0xC0, // 16-bit size, 7-bit reference
        0, 0, 0, 1, // sample_delta
        0, 1, // subsample_count
        0, 4,    // subsample_size
        0x03, // both reserved bits set, no property
    ];
    let parsed = SubSampleInformationBoxV2::parse(subs).expect("parse subs");
    assert_eq!(parsed.try_to_bytes().expect("serialize subs"), subs);

    let ssrt: &[u8] = &[
        0, 0, 0, 14, b's', b's', b'r', b't', // box header
        0, 0, 0, 0,    // flags
        0x81, // 7-bit entry_count = 1
        0x07, // three reserved bits set
    ];
    let parsed = SubSampleReferenceTableBox::parse(ssrt).expect("parse ssrt");
    assert_eq!(parsed.entries[0].reserved, 0x07);
    assert_eq!(parsed.try_to_bytes().expect("serialize ssrt"), ssrt);
}

/// `ssrt` is a version 0 box, but the version byte a file states is what a
/// rewrite must emit: rewriting it to 0 changes bytes the crate parsed.
#[test]
fn ssrt_keeps_the_fullbox_version_it_parsed() {
    let bytes: &[u8] = &[
        0, 0, 0, 14, b's', b's', b'r', b't', // box header
        1, 0, 0, 0, // version 1, flags 0
        0, 0, // entry_count
    ];
    let parsed = SubSampleReferenceTableBox::parse(bytes).expect("parse");
    assert_eq!(parsed.version, 1);
    assert_eq!(parsed.try_to_bytes().expect("serialize"), bytes);
}

#[test]
fn subs_v2_and_ssrt_property_truncations_report_the_exact_shortfall() {
    let subs_after_flags: &[u8] = &[
        0, 0, 0, 26, b's', b'u', b'b', b's', 2, 0, 0, 0, 0, 0, 0, 1, 0xC0, 0, 0, 0, 1, 0, 1, 0,
        0x10, 0x38,
    ];
    assert_buffer_too_short(
        SubSampleInformationBoxV2::parse(subs_after_flags),
        19,
        18,
        "subsample_priority",
    );

    let subs_two_csp_bytes: &[u8] = &[
        0, 0, 0, 28, b's', b'u', b'b', b's', 2, 0, 0, 0, 0, 0, 0, 1, 0xC0, 0, 0, 0, 1, 0, 1, 0,
        0x10, 0x18, 0x11, 0x22,
    ];
    assert_buffer_too_short(
        SubSampleInformationBoxV2::parse(subs_two_csp_bytes),
        21,
        20,
        "codec_specific_parameters",
    );

    let ssrt_after_flags: &[u8] = &[0, 0, 0, 15, b's', b's', b'r', b't', 0, 0, 0, 0, 0, 2, 0x70];
    assert_buffer_too_short(
        SubSampleReferenceTableBox::parse(ssrt_after_flags),
        8,
        7,
        "subsample_priority",
    );

    let ssrt_two_csp_bytes: &[u8] = &[
        0, 0, 0, 17, b's', b's', b'r', b't', 0, 0, 0, 0, 0, 2, 0x30, 0x11, 0x22,
    ];
    assert_buffer_too_short(
        SubSampleReferenceTableBox::parse(ssrt_two_csp_bytes),
        10,
        9,
        "codec_specific_parameters",
    );
}

// ---------------------------------------------------------------------------
// subs v2: the minimal empty box and one-byte truncations in every size and
// reference form (#3382)
// ---------------------------------------------------------------------------

const SUBS_V2_EMPTY: &[u8] = &[
    0, 0, 0, 17, b's', b'u', b'b', b's', 2, 0, 0, 0, 0, 0, 0, 0, 0x00,
];

const SUBS_V2_SIZE16_SHORT_REF: &[u8] = &[
    0, 0, 0, 26, b's', b'u', b'b', b's', 2, 0, 0, 0, 0, 0, 0, 1, 0xC0, 0, 0, 0, 0x0A, 0, 1, 0,
    0x64, 0x85,
];

const SUBS_V2_SIZE16_LONG_REF: &[u8] = &[
    0, 0, 0, 27, b's', b'u', b'b', b's', 2, 0, 0, 0, 0, 0, 0, 1, 0x80, 0, 0, 0, 0x0A, 0, 1, 0,
    0x64, 0x81, 0x05,
];

const SUBS_V2_SIZE32_SHORT_REF: &[u8] = &[
    0, 0, 0, 28, b's', b'u', b'b', b's', 2, 0, 0, 0, 0, 0, 0, 1, 0x40, 0, 0, 0, 0x0A, 0, 1, 0, 0,
    1, 0x2C, 0x07,
];

const SUBS_V2_SIZE32_LONG_REF: &[u8] = &[
    0, 0, 0, 29, b's', b'u', b'b', b's', 2, 0, 0, 0, 0, 0, 0, 1, 0x00, 0, 0, 0, 0x0A, 0, 1, 0, 0,
    1, 0x2C, 0x81, 0x07,
];

#[test]
fn subs_v2_empty_box_parses() {
    let parsed = SubSampleInformationBoxV2::parse(SUBS_V2_EMPTY).expect("17-byte empty subs v2");
    assert!(parsed.entries.is_empty());
    assert!(!parsed.is_size_16_bits);
    assert!(!parsed.is_short_ref);
    assert_eq!(parsed.try_to_bytes().unwrap(), SUBS_V2_EMPTY);
}

#[test]
fn subs_v2_truncated_bodies_report_the_exact_shortfall() {
    assert_buffer_too_short(
        SubSampleInformationBoxV2::parse_body(&SUBS_V2_EMPTY[8..16]),
        9,
        8,
        "subs v2 body",
    );
    assert_buffer_too_short(
        SubSampleInformationBoxV2::parse_body(&SUBS_V2_EMPTY[8..15]),
        9,
        7,
        "subs v2 body",
    );

    let header_short: &[u8] = &[
        0, 0, 0, 22, b's', b'u', b'b', b's', 2, 0, 0, 0, 0, 0, 0, 1, 0xC0, 0, 0, 0, 0x0A, 0,
    ];
    assert_buffer_too_short(
        SubSampleInformationBoxV2::parse_body(&header_short[8..]),
        15,
        14,
        "subs v2 entry header",
    );

    for (golden, need, what) in [
        (SUBS_V2_SIZE16_SHORT_REF, 18usize, "subs v2 subsample"),
        (SUBS_V2_SIZE16_LONG_REF, 19, "subs v2 reference_idx"),
        (SUBS_V2_SIZE32_SHORT_REF, 20, "subs v2 subsample"),
        (SUBS_V2_SIZE32_LONG_REF, 21, "subs v2 reference_idx"),
    ] {
        let cut = &golden[..golden.len() - 1];
        assert_buffer_too_short(
            SubSampleInformationBoxV2::parse_body(&cut[8..]),
            need,
            need - 1,
            what,
        );
    }

    assert_buffer_too_short(
        SubSampleInformationBoxV2::parse(&[0u8; 12]),
        16,
        12,
        "subs box",
    );
}

// ---------------------------------------------------------------------------
// subs v2: entry_count, sample_delta and subsample_count keep their own
// bytes (#3383)
// ---------------------------------------------------------------------------

#[test]
fn subs_v2_wide_fields_keep_every_byte() {
    let mut entries = vec![SubsV2Entry {
        sample_delta: 0x0B0C_0D0E,
        subsamples: vec![
            SubsV2Subsample {
                subsample_size: 0x64,
                source: SubsV2Source::Reference { reference_idx: 5 },
            },
            SubsV2Subsample {
                subsample_size: 0x65,
                source: SubsV2Source::Reference { reference_idx: 6 },
            },
            SubsV2Subsample {
                subsample_size: 0x66,
                source: SubsV2Source::Reference { reference_idx: 7 },
            },
        ],
    }];
    entries.push(SubsV2Entry {
        sample_delta: 0x0102_0304,
        subsamples: (0..258)
            .map(|_| SubsV2Subsample {
                subsample_size: 7,
                source: SubsV2Source::Reference { reference_idx: 1 },
            })
            .collect(),
    });
    for i in 2..257u32 {
        entries.push(SubsV2Entry {
            sample_delta: i,
            subsamples: vec![],
        });
    }
    let b = SubSampleInformationBoxV2 {
        flags: 0,
        is_size_16_bits: true,
        is_short_ref: true,
        reserved: 0,
        entries,
    };
    let bytes = b.try_to_bytes().unwrap();
    let parsed = SubSampleInformationBoxV2::parse(&bytes).expect("257 subs v2 entries");
    assert_eq!(parsed.entries.len(), 257);
    assert_eq!(parsed.entries[1].sample_delta, 0x0102_0304);
    assert_eq!(parsed.entries[1].subsamples.len(), 258);
    assert_eq!(
        u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize,
        bytes.len()
    );
    assert_eq!(parsed.try_to_bytes().unwrap(), bytes);
}

// ---------------------------------------------------------------------------
// ssrt: a largesize header leaves only the body guard between the tail and
// out-of-bounds reads (#3384)
// ---------------------------------------------------------------------------

#[test]
fn ssrt_largesize_truncations_report_the_exact_shortfall() {
    for total in 16..=20usize {
        let mut box_bytes = vec![0, 0, 0, 1, b's', b's', b'r', b't'];
        box_bytes.extend_from_slice(&(total as u64).to_be_bytes());
        box_bytes.resize(total, 0);
        assert_buffer_too_short(
            SubSampleReferenceTableBox::parse(&box_bytes),
            5,
            total - 16,
            "ssrt body",
        );
    }
    let mut single_body = vec![0, 0, 0, 1, b's', b's', b'r', b't'];
    single_body.extend_from_slice(&17u64.to_be_bytes());
    single_body.push(0x00);
    assert_eq!(single_body.len(), 17);
    assert_buffer_too_short(
        SubSampleReferenceTableBox::parse(&single_body),
        5,
        1,
        "ssrt body",
    );
}
