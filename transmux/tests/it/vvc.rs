//! Real-fixture tests for VVC / H.266 (`vvc1` / `vvcC`) in the transmux hub.
//!
//! Oracle: `fixtures/mp4/frag/vvc.frag.mp4` — a real vvenc H.266 bitstream muxed
//! by ffmpeg (`-tag:v vvc1`), moof-framed, 320x240, carrying VPS/SPS/PPS in a
//! `vvcC` box. The `vvcC` box body (FullBox header + `VvcDecoderConfigurationRecord`)
//! is the byte-exact oracle. Every test walks the box tree — no hardcoded offsets.

use broadcast_core::{Parse, Serialize};
use bytes::Bytes;
use isobmff::vvc_config::{
    VvcConfigurationBox, VvcDecoderConfigurationRecord, VvcNalArray, VvcNalUnitType, VvcPtlRecord,
};
use transmux::codec_config_ext::VvcRecordCodecInfo;
use transmux::pipeline::CodecConfig;

const FIXTURE: &str = "mp4/frag/vvc.frag.mp4";

fn read_fixture() -> Vec<u8> {
    std::fs::read(test_fixtures::path(FIXTURE)).expect("read vvc.frag.mp4 fixture")
}

/// Walk boxes at `data`, recursing into container boxes, to find the first box
/// of `four_cc` and return its **full bytes** (8-byte header + body). Containers
/// like `moov`/`trak`/`mdia`/`minf`/`stbl` are recursed; `stsd` is a FullBox
/// container so its 8 bytes of (header) + 8 bytes (version/flags/count) are
/// skipped before recursing into its sample entries.
fn find_box_full<'a>(data: &'a [u8], four_cc: &[u8; 4]) -> Option<&'a [u8]> {
    const CONTAINERS: &[&[u8; 4]] = &[b"moov", b"trak", b"mdia", b"minf", b"stbl"];
    let mut off = 0usize;
    while off + 8 <= data.len() {
        let size =
            u32::from_be_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]]) as usize;
        if size < 8 || off + size > data.len() {
            break;
        }
        let ty: &[u8; 4] = data[off + 4..off + 8].try_into().unwrap();
        if ty == four_cc {
            return Some(&data[off..off + size]);
        }
        if CONTAINERS.contains(&ty) {
            if let Some(found) = find_box_full(&data[off + 8..off + size], four_cc) {
                return Some(found);
            }
        } else if ty == b"stsd" {
            // FullBox(4) + entry_count(4) then sample entries.
            if let Some(found) = find_box_full(&data[off + 16..off + size], four_cc) {
                return Some(found);
            }
        } else if ty == b"vvc1" || ty == b"vvi1" {
            // Sample entry: VisualSampleEntry fixed fields are 8 (header already
            // consumed) → the child config boxes start after 8 (box header) + 78.
            if let Some(found) = find_box_full(&data[off + 8 + 78..off + size], four_cc) {
                return Some(found);
            }
        }
        off += size;
    }
    None
}

/// The `vvcC` box body (bytes after the 8-byte box header) walked from the mp4.
fn oracle_vvcc_body(mp4: &[u8]) -> Vec<u8> {
    let vvcc = find_box_full(mp4, b"vvcC").expect("vvcC box in fixture");
    vvcc[8..].to_vec()
}

// ---------------------------------------------------------------------------
// Test 1 — enumeration + config byte-exact
// ---------------------------------------------------------------------------

#[test]
fn vvc_demux_yields_vvc_config_byte_exact() {
    let mp4 = read_fixture();
    let oracle = oracle_vvcc_body(&mp4);

    let media =
        test_harness::collect::media_from_mp4(&mp4, test_harness::collect::CollectMode::Seekable)
            .expect("demux vvc fixture");
    assert!(!media.tracks.is_empty(), "expected at least one track");

    let CodecConfig::Vvc { config, .. } = media.tracks[0].config() else {
        panic!(
            "track 0 is not CodecConfig::Vvc: {:?}",
            media.tracks[0].config()
        );
    };

    // The reconstructed vvcC box serialized back must equal the source box body
    // (FullBox header + record). `config` is the VvcConfigurationBox; its full
    // serialization is `[box header][FullBox+record]`, so bytes 8.. == oracle.
    let full = config.try_to_bytes().unwrap();
    assert_eq!(&full[4..8], b"vvcC");
    assert_eq!(
        &full[8..],
        &oracle[..],
        "reconstructed vvcC body must be byte-identical to the source"
    );
}

// ---------------------------------------------------------------------------
// Test 2 — dimensions parsed from the SPS in the vvcC NAL array
// ---------------------------------------------------------------------------

#[test]
fn vvc_dimensions_from_sps() {
    let mp4 = read_fixture();
    let media =
        test_harness::collect::media_from_mp4(&mp4, test_harness::collect::CollectMode::Seekable)
            .expect("demux");

    let track = &media.tracks[0];
    let CodecConfig::Vvc {
        config,
        width,
        height,
    } = track.config()
    else {
        panic!("expected Vvc");
    };

    // width/height on the CodecConfig came from decoding the SPS in the array.
    assert_eq!((*width, *height), (320, 240));

    // And decoding the SPS directly (not the sample-entry visual dims) agrees.
    let dims = config
        .config
        .dimensions()
        .expect("SPS present + decodable in the vvcC array");
    assert_eq!(dims, (320, 240));
}

// ---------------------------------------------------------------------------
// Test 3 — sample fidelity: demux → IR → CmafMuxStage → re-demux, bytes identical
// ---------------------------------------------------------------------------

#[test]
fn vvc_sample_round_trip_through_cmaf() {
    let mp4 = read_fixture();
    let media =
        test_harness::collect::media_from_mp4(&mp4, test_harness::collect::CollectMode::Seekable)
            .expect("demux");

    let orig_samples: Vec<Bytes> = media.tracks[0]
        .samples
        .iter()
        .map(|s| s.data.clone())
        .collect();
    assert!(!orig_samples.is_empty(), "expected coded samples");

    let remuxed = crate::cmaf::package(&media).expect("mux back to CMAF");

    let media2 = test_harness::collect::media_from_mp4(
        &remuxed,
        test_harness::collect::CollectMode::Seekable,
    )
    .expect("re-demux");
    let round_samples: Vec<Bytes> = media2.tracks[0]
        .samples
        .iter()
        .map(|s| s.data.clone())
        .collect();

    assert_eq!(
        round_samples.len(),
        orig_samples.len(),
        "sample count must match"
    );
    assert_eq!(
        round_samples, orig_samples,
        "VVC coded sample bytes must be byte-identical"
    );
}

// ---------------------------------------------------------------------------
// Test 4 — output path: muxed init segment carries a vvc1 whose vvcC == source
// ---------------------------------------------------------------------------

#[test]
fn vvc_output_init_segment_carries_vvc1_vvcc() {
    let mp4 = read_fixture();
    let oracle = oracle_vvcc_body(&mp4);

    let media =
        test_harness::collect::media_from_mp4(&mp4, test_harness::collect::CollectMode::Seekable)
            .expect("demux");

    let remuxed = crate::cmaf::package(&media).expect("mux");

    // The output must contain a vvc1 sample entry (in its moov→…→stsd).
    let vvc1 = find_box_full(&remuxed, b"vvc1").expect("vvc1 in muxed init segment");
    assert_eq!(&vvc1[4..8], b"vvc1");

    // And the vvcC inside it equals the source vvcC body.
    let vvcc = find_box_full(&remuxed, b"vvcC").expect("vvcC in muxed init segment");
    assert_eq!(
        &vvcc[8..],
        &oracle[..],
        "muxed vvcC must equal the source vvcC"
    );
}

// ---------------------------------------------------------------------------
// Test 5 — vvcC round-trip symmetry + not-a-passthrough (field mutation bites)
// ---------------------------------------------------------------------------

#[test]
fn vvc_record_round_trip_and_mutation() {
    let mp4 = read_fixture();
    let oracle = oracle_vvcc_body(&mp4);

    // Parse the vvcC box body (FullBox header + record) then re-serialize.
    let boxed = VvcConfigurationBox::parse_body(&oracle).expect("parse vvcC body");
    // The whole box (header + FullBox + record) re-serializes with body == oracle.
    let full = boxed.try_to_bytes().unwrap();
    assert_eq!(&full[8..], &oracle[..], "vvcC box body round-trip");

    // The record alone (skip the 4-byte FullBox header) also round-trips exactly.
    let record = VvcDecoderConfigurationRecord::parse(&oracle[4..]).expect("parse record");
    let reser = record.try_to_bytes().unwrap();
    assert_eq!(&reser[..], &oracle[4..], "record round-trip byte-identical");

    // Sanity: the arrays decoded to typed SPS/PPS NAL types.
    assert!(
        record
            .arrays
            .iter()
            .any(|a| a.kind() == VvcNalUnitType::Sps)
    );
    assert!(
        record
            .arrays
            .iter()
            .any(|a| a.kind() == VvcNalUnitType::Pps)
    );

    // Not a raw passthrough: mutate a decoded field → serialized bytes change,
    // and the change is exactly at the max_picture_width field (parses back).
    let mut mutated = record.clone();
    mutated.max_picture_width = 1920;
    let mutated_bytes = mutated.try_to_bytes().unwrap();
    assert_ne!(
        mutated_bytes, reser,
        "mutating a decoded field must change the serialized record"
    );
    let reparsed = VvcDecoderConfigurationRecord::parse(&mutated_bytes).expect("reparse mutated");
    assert_eq!(reparsed.max_picture_width, 1920);
    assert_eq!(reparsed.arrays.len(), record.arrays.len());
}

#[test]
fn vvc_rfc6381_smoke() {
    let mp4 = read_fixture();
    let oracle = oracle_vvcc_body(&mp4);
    let record = VvcDecoderConfigurationRecord::parse(&oracle[4..]).unwrap();
    let s = record.rfc6381();
    eprintln!("RFC6381 = {s}");
    assert!(s.starts_with("vvc1."), "got {s}");
}

fn record_with_arrays(
    length_size_minus_one: u8,
    arrays: Vec<VvcNalArray>,
) -> VvcDecoderConfigurationRecord {
    VvcDecoderConfigurationRecord {
        length_size_minus_one,
        ptl_present: false,
        ols_idx: 0,
        num_sublayers: 0,
        constant_frame_rate: 0,
        chroma_format_idc: 0,
        bit_depth_minus8: 0,
        ptl: None,
        max_picture_width: 0,
        max_picture_height: 0,
        avg_frame_rate: 0,
        arrays,
    }
}

fn ptl_with_sub_profiles(sub_profiles: Vec<u32>) -> VvcPtlRecord {
    VvcPtlRecord {
        num_bytes_constraint_info: 0,
        general_profile_idc: 1,
        general_tier_flag: false,
        general_level_idc: 51,
        ptl_frame_only_constraint_flag: true,
        ptl_multilayer_enabled_flag: false,
        general_constraint_info: Vec::new(),
        sublayer_level_present: Vec::new(),
        sublayer_level_idc: Vec::new(),
        sub_profile_idc: sub_profiles,
    }
}

#[test]
fn vvc_nal_unit_type_codes_round_trip_and_labels_are_pinned() {
    let types = [
        (VvcNalUnitType::OPI, VvcNalUnitType::Opi, "OPI_NUT"),
        (VvcNalUnitType::DCI, VvcNalUnitType::Dci, "DCI_NUT"),
        (VvcNalUnitType::VPS, VvcNalUnitType::Vps, "VPS_NUT"),
        (VvcNalUnitType::SPS, VvcNalUnitType::Sps, "SPS_NUT"),
        (VvcNalUnitType::PPS, VvcNalUnitType::Pps, "PPS_NUT"),
        (
            VvcNalUnitType::PREFIX_APS,
            VvcNalUnitType::PrefixAps,
            "PREFIX_APS_NUT",
        ),
    ];
    for (code, kind, name) in types {
        assert_eq!(kind.to_u8(), code);
        assert_eq!(VvcNalUnitType::from_u8(code), kind);
        assert_eq!(kind.name(), name);
        assert_eq!(kind.to_string(), name);
    }
    assert_eq!(VvcNalUnitType::from_u8(20), VvcNalUnitType::Other(20));
    assert_eq!(VvcNalUnitType::Other(20).to_u8(), 20);
    assert_eq!(VvcNalUnitType::Other(20).name(), "reserved");
    assert_eq!(VvcNalUnitType::Other(20).to_string(), "reserved(0x14)");

    assert!(!VvcNalUnitType::Dci.has_num_nalus_field());
    assert!(!VvcNalUnitType::Opi.has_num_nalus_field());
    assert!(VvcNalUnitType::Vps.has_num_nalus_field());
}

#[test]
fn vvc_dci_and_opi_arrays_omit_the_num_nalus_field() {
    let record = record_with_arrays(
        3,
        vec![
            VvcNalArray::new(true, VvcNalUnitType::Dci.to_u8(), vec![vec![0x71, 0x01]]),
            VvcNalArray::new(true, VvcNalUnitType::Opi.to_u8(), vec![vec![0x61, 0x02]]),
        ],
    );
    // reserved(5) '11111' + LengthSizeMinusOne(2) '11' + ptl_present(1) '0'
    // = 0xFE, then num_of_arrays = 2. Each array is completeness+reserved+
    // type, then (no num_nalus for DCI/OPI) a 16-bit nalUnitLength + the NAL.
    let expected = [
        0xFE, 0x02, // record prefix + num_of_arrays
        0x8D, 0x00, 0x02, 0x71, 0x01, // DCI array
        0x8C, 0x00, 0x02, 0x61, 0x02, // OPI array
    ];
    let bytes = record.try_to_bytes().expect("serialize DCI/OPI record");
    assert_eq!(bytes[..], expected);
    assert_eq!(bytes.len(), record.serialized_len());
    let parsed = VvcDecoderConfigurationRecord::parse(&bytes).expect("parse DCI/OPI record");
    assert_eq!(parsed, record);
    assert_eq!(parsed.arrays[0].nalus, vec![vec![0x71, 0x01]]);
    assert_eq!(parsed.arrays[1].nalus, vec![vec![0x61, 0x02]]);
}

#[test]
fn vvc_rejects_length_size_minus_one_two() {
    // reserved(5) '11111' + LengthSizeMinusOne(2) '10' + ptl_present(1) '0',
    // then num_of_arrays = 0.
    match VvcDecoderConfigurationRecord::parse(&[0xFC, 0x00]) {
        Err(isobmff::Error::InvalidValue(e)) => {
            assert_eq!(e.field, "LengthSizeMinusOne");
            assert_eq!(e.value, 2);
        }
        other => panic!("expected InvalidValue for LengthSizeMinusOne = 2, got {other:?}"),
    }
}

#[test]
fn vvc_length_size_zero_and_one_round_trip() {
    for length_size_minus_one in [0u8, 1] {
        let record = record_with_arrays(length_size_minus_one, Vec::new());
        let bytes = record.try_to_bytes().expect("serialize record");
        assert_eq!(bytes.len(), record.serialized_len());
        let parsed = VvcDecoderConfigurationRecord::parse(&bytes).expect("parse record");
        assert_eq!(parsed.length_size_minus_one, length_size_minus_one);
        assert_eq!(parsed, record);
    }
}

#[test]
fn vvc_configuration_box_reports_the_bytes_it_writes() {
    let record = record_with_arrays(
        3,
        vec![VvcNalArray::new(
            true,
            VvcNalUnitType::Sps.to_u8(),
            vec![vec![0x00, 0x01]],
        )],
    );
    let boxed = VvcConfigurationBox::new(record);
    let need = boxed.serialized_len();
    let bytes = boxed.try_to_bytes().expect("serialize vvcC box");
    assert_eq!(bytes.len(), need);
    let mut buf = vec![0u8; need];
    assert_eq!(
        boxed
            .serialize_into(&mut buf)
            .expect("serialize into buffer"),
        need,
        "serialize_into must return the box length, not a product"
    );
    assert_eq!(buf, bytes);
}

#[test]
fn vvc_record_display_is_pinned() {
    let no_ptl = record_with_arrays(0, Vec::new());
    assert_eq!(no_ptl.to_string(), "VVC(no PTL, lenSize=1, arrays=0)");

    let mut with_ptl = record_with_arrays(3, Vec::new());
    with_ptl.ptl_present = true;
    with_ptl.num_sublayers = 1;
    with_ptl.ptl = Some(ptl_with_sub_profiles(Vec::new()));
    assert_eq!(
        with_ptl.to_string(),
        "VVC(profile_idc=1, tier=main, level=51, lenSize=4, arrays=0)"
    );
}

#[test]
fn vvc_ptl_with_zero_and_two_sub_profiles_round_trips() {
    for count in [0u32, 2] {
        let profiles: Vec<u32> = (0..count).map(|i| 0x0100_0000 + i).collect();
        let mut record = record_with_arrays(3, Vec::new());
        record.ptl_present = true;
        record.num_sublayers = 1;
        record.ptl = Some(ptl_with_sub_profiles(profiles.clone()));
        let bytes = record.try_to_bytes().expect("serialize PTL record");
        assert_eq!(bytes.len(), record.serialized_len(), "count {count}");
        let parsed = VvcDecoderConfigurationRecord::parse(&bytes).expect("parse PTL record");
        assert_eq!(
            parsed.ptl.as_ref().expect("PTL present").sub_profile_idc,
            profiles
        );
        assert_eq!(parsed, record);
    }
}

/// `sublayer_level_present` is caller-supplied, and the writer emits one flag
/// per entry plus padding to a byte. The announced length must count each
/// written flag, not a fixed one byte's worth.
#[test]
fn vvc_ptl_length_counts_every_written_sublayer_flag() {
    let mut record = record_with_arrays(3, Vec::new());
    record.ptl_present = true;
    record.num_sublayers = 7;
    record.ptl = Some(VvcPtlRecord {
        num_bytes_constraint_info: 0,
        general_profile_idc: 1,
        general_tier_flag: false,
        general_level_idc: 51,
        ptl_frame_only_constraint_flag: true,
        ptl_multilayer_enabled_flag: false,
        general_constraint_info: Vec::new(),
        sublayer_level_present: vec![true; 9],
        sublayer_level_idc: vec![51; 9],
        sub_profile_idc: Vec::new(),
    });
    let bytes = record
        .try_to_bytes()
        .expect("serialize a record with two bytes of sublayer flags");
    assert_eq!(bytes.len(), record.serialized_len());
}
