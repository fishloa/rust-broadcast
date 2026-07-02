//! Real-fixture tests for VVC / H.266 (`vvc1` / `vvcC`) in the transmux hub.
//!
//! Oracle: `fixtures/mp4/frag/vvc.frag.mp4` — a real vvenc H.266 bitstream muxed
//! by ffmpeg (`-tag:v vvc1`), moof-framed, 320x240, carrying VPS/SPS/PPS in a
//! `vvcC` box. The `vvcC` box body (FullBox header + `VvcDecoderConfigurationRecord`)
//! is the byte-exact oracle. Every test walks the box tree — no hardcoded offsets.

use broadcast_common::{Package, Parse, Serialize, Unpackage};
use transmux::media::{CmafMux, Fmp4Demux};
use transmux::pipeline::CodecConfig;
use transmux::vvc_config::{VvcConfigurationBox, VvcDecoderConfigurationRecord, VvcNalUnitType};

const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../fixtures/mp4/frag/vvc.frag.mp4"
);

fn read_fixture() -> Vec<u8> {
    std::fs::read(FIXTURE).expect("read vvc.frag.mp4 fixture")
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

    let mut demux = Fmp4Demux::new();
    let media = demux.unpackage(&mp4).expect("demux vvc fixture");
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
    let full = config.to_bytes();
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
    let mut demux = Fmp4Demux::new();
    let media = demux.unpackage(&mp4).expect("demux");

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
// Test 3 — sample fidelity: demux → IR → CmafMux → re-demux, bytes identical
// ---------------------------------------------------------------------------

#[test]
fn vvc_sample_round_trip_through_cmaf() {
    let mp4 = read_fixture();
    let mut demux = Fmp4Demux::new();
    let media = demux.unpackage(&mp4).expect("demux");

    let orig_samples: Vec<Vec<u8>> = media.tracks[0]
        .samples
        .iter()
        .map(|s| s.data.clone())
        .collect();
    assert!(!orig_samples.is_empty(), "expected coded samples");

    let mut mux = CmafMux::default();
    let remuxed = mux.package(&media).expect("mux back to CMAF");

    let mut demux2 = Fmp4Demux::new();
    let media2 = demux2.unpackage(&remuxed).expect("re-demux");
    let round_samples: Vec<Vec<u8>> = media2.tracks[0]
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

    let mut demux = Fmp4Demux::new();
    let media = demux.unpackage(&mp4).expect("demux");

    let mut mux = CmafMux::default();
    let remuxed = mux.package(&media).expect("mux");

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
    let full = boxed.to_bytes();
    assert_eq!(&full[8..], &oracle[..], "vvcC box body round-trip");

    // The record alone (skip the 4-byte FullBox header) also round-trips exactly.
    let record = VvcDecoderConfigurationRecord::parse(&oracle[4..]).expect("parse record");
    let reser = record.to_bytes();
    assert_eq!(&reser[..], &oracle[4..], "record round-trip byte-identical");

    // Sanity: the arrays decoded to typed SPS/PPS NAL types.
    assert!(record
        .arrays
        .iter()
        .any(|a| a.kind() == VvcNalUnitType::Sps));
    assert!(record
        .arrays
        .iter()
        .any(|a| a.kind() == VvcNalUnitType::Pps));

    // Not a raw passthrough: mutate a decoded field → serialized bytes change,
    // and the change is exactly at the max_picture_width field (parses back).
    let mut mutated = record.clone();
    mutated.max_picture_width = 1920;
    let mutated_bytes = mutated.to_bytes();
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
