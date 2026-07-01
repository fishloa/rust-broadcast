//! Spec-vector tests for the MPEG-H 3D Audio (`mha1`/`mhm1` + `mhaC`) path.
//!
//! No real MPEG-H fixture is vendored because:
//! - No local encoder is available.
//! - Fraunhofer test content is Git-LFS + license-restricted.
//! - DASH-IF MCA assets require live network access.
//!
//! The tests here use **hand-computed spec vectors** derived directly from the
//! `MHADecoderConfigurationRecord` wire layout (ISO/IEC 23008-3 §20) and are
//! therefore ungameable by a raw-passthrough serializer.
//!
//! ## Deferred real-fixture gate
//!
//! A byte-identical fixture gate against a real `mha1`/`mhm1` MP4 is deferred.
//! To run it, fetch a Fraunhofer-IIS sample (e.g. from the DASH-IF MCA test
//! vectors at `https://dash.akamaized.net/dash264/TestCasesMCA/fraunhofer/`) and
//! extract the `mhaC` box body, then add a test that parses the extracted bytes
//! and asserts byte-equality with the serialized form.

extern crate alloc;

use broadcast_common::{Parse, Serialize};
use transmux::{
    build_init_segment, CodecConfig, MHADecoderConfigurationRecord, MhaSampleEntry,
    SampleDescriptionBox, SampleEntryVariant, TrackSpec, MHAC_CONFIGURATION_VERSION, MHAC_FOURCC,
    MHAC_RECORD_FIXED_LEN,
};

// Profile-level constants used across tests (ATSC A/342-3 §5.2.2.1).
const LC_PROFILE_LEVEL_3: u8 = 0x0D;
const CICP_CH_5_1: u8 = 6; // CICP ChannelConfiguration 5.1

// ---------------------------------------------------------------------------
// Helper: build a non-trivial mpegh3daConfig blob (opaque to the container).
// In production this is the mpegh3daConfig() bitstream from the MHAS packet;
// here we use a deterministic byte pattern to verify round-trip fidelity.
// ---------------------------------------------------------------------------
fn test_config_blob() -> alloc::vec::Vec<u8> {
    // 8-byte pattern: alternating 0xA5/0x5A + a length-field-sized value
    alloc::vec![0xA5, 0x5A, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06]
}

// ---------------------------------------------------------------------------
// Gate 1: serialize → hand-computed expected bytes → byte-identical.
//
// Wire layout (ISO/IEC 23008-3 §20):
//   byte 0: configurationVersion           = 0x01
//   byte 1: mpegh3daProfileLevelIndication = 0x0D  (LC L3, ATSC A/342-3 §5.2.2.1)
//   byte 2: referenceChannelLayout         = 0x06  (CICP 5.1)
//   bytes 3-4: mpegh3daConfigLength        = 0x00 0x08 (8 bytes, big-endian)
//   bytes 5-12: mpegh3daConfig             = [0xA5,0x5A,0x01,0x02,0x03,0x04,0x05,0x06]
// ---------------------------------------------------------------------------
#[test]
fn record_serialize_byte_identical() {
    let blob = test_config_blob();
    let rec = MHADecoderConfigurationRecord::new(LC_PROFILE_LEVEL_3, CICP_CH_5_1, blob.clone());

    #[rustfmt::skip]
    let expected: [u8; 13] = [
        0x01,               // configurationVersion = 1
        0x0D,               // mpegh3daProfileLevelIndication = LC L3
        0x06,               // referenceChannelLayout = CICP 5.1
        0x00, 0x08,         // mpegh3daConfigLength = 8 (big-endian)
        0xA5, 0x5A, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06,  // mpegh3daConfig
    ];

    assert_eq!(
        rec.serialized_len(),
        MHAC_RECORD_FIXED_LEN + blob.len(),
        "serialized_len() must be fixed-header + blob length"
    );

    let mut buf = alloc::vec![0u8; rec.serialized_len()];
    let n = rec.serialize_into(&mut buf).expect("serialize_into failed");
    assert_eq!(n, expected.len(), "serialize returned wrong byte count");
    assert_eq!(
        buf.as_slice(),
        &expected[..],
        "serialized bytes must be byte-identical to spec vector"
    );
}

// ---------------------------------------------------------------------------
// Gate 2: serialize → re-parse → field-equal (Parse/Serialize symmetry).
// ---------------------------------------------------------------------------
#[test]
fn record_round_trip_equal() {
    let blob = test_config_blob();
    let original = MHADecoderConfigurationRecord::new(LC_PROFILE_LEVEL_3, CICP_CH_5_1, blob);

    let mut buf = alloc::vec![0u8; original.serialized_len()];
    original
        .serialize_into(&mut buf)
        .expect("serialize_into failed");

    let reparsed =
        MHADecoderConfigurationRecord::parse(&buf).expect("re-parse of serialized bytes failed");

    assert_eq!(
        reparsed.configuration_version, MHAC_CONFIGURATION_VERSION,
        "configurationVersion must survive round-trip"
    );
    assert_eq!(
        reparsed.mpegh3da_profile_level_indication, LC_PROFILE_LEVEL_3,
        "mpegh3daProfileLevelIndication must survive round-trip"
    );
    assert_eq!(
        reparsed.reference_channel_layout, CICP_CH_5_1,
        "referenceChannelLayout must survive round-trip"
    );
    assert_eq!(
        reparsed.mpegh3da_config, original.mpegh3da_config,
        "mpegh3daConfig blob must survive round-trip"
    );
    // Full structural equality
    assert_eq!(
        reparsed, original,
        "round-tripped record must be field-equal to original"
    );
}

// ---------------------------------------------------------------------------
// Gate 3: mutation → serialized bytes change (no raw passthrough).
//
// Verifies the serializer reads from struct fields rather than caching the
// original bytes (passthrough serialize would produce identical bytes even
// after mutation).
// ---------------------------------------------------------------------------
#[test]
fn mutation_changes_serialized_bytes() {
    let blob = test_config_blob();
    let original = MHADecoderConfigurationRecord::new(LC_PROFILE_LEVEL_3, CICP_CH_5_1, blob);

    let mut original_bytes = alloc::vec![0u8; original.serialized_len()];
    original
        .serialize_into(&mut original_bytes)
        .expect("serialize original");

    // Mutate the profile-level field.
    let mut mutated_profile = original.clone();
    mutated_profile.mpegh3da_profile_level_indication = 0x0B; // LC L1
    let mut profile_bytes = alloc::vec![0u8; mutated_profile.serialized_len()];
    mutated_profile
        .serialize_into(&mut profile_bytes)
        .expect("serialize mutated profile");
    assert_ne!(
        original_bytes, profile_bytes,
        "changing mpegh3daProfileLevelIndication must change serialized bytes"
    );

    // Mutate the config blob.
    let mut mutated_blob = original.clone();
    mutated_blob.mpegh3da_config = alloc::vec![0xFF, 0xFE];
    let mut blob_bytes = alloc::vec![0u8; mutated_blob.serialized_len()];
    mutated_blob
        .serialize_into(&mut blob_bytes)
        .expect("serialize mutated blob");
    assert_ne!(
        original_bytes, blob_bytes,
        "changing mpegh3daConfig blob must change serialized bytes"
    );
}

// ---------------------------------------------------------------------------
// Gate 4: build_init_segment for a CodecConfig::MpegH track → parse moov →
// locate the mha1 sample entry → confirm its mhaC child round-trips.
// ---------------------------------------------------------------------------
#[test]
fn build_init_segment_mha1_roundtrip() {
    let blob = test_config_blob();
    let config = MHADecoderConfigurationRecord::new(LC_PROFILE_LEVEL_3, CICP_CH_5_1, blob.clone());

    let tracks = [TrackSpec {
        track_id: 1,
        timescale: 48000,
        config: CodecConfig::MpegH {
            config: config.clone(),
            channel_count: 6,
            sample_rate: 48000,
            sample_size: 16,
        },
    }];
    let init = build_init_segment(&tracks, 90000).expect("build_init_segment failed");

    // Walk the moov box to find the stsd.
    // Layout: ftyp | moov [ mvhd | trak [ tkhd | mdia [ mdhd | hdlr | minf [ smhd | dinf | stbl [ stsd | ... ] ] ] ] | mvex ]
    let (stsd_bytes, stsd_offset) = find_stsd_in_init(&init).expect("stsd not found in init");
    let stsd = SampleDescriptionBox::parse(stsd_bytes).expect("parse stsd failed");
    assert_eq!(stsd.entries.len(), 1, "stsd should have exactly 1 entry");

    let entry = &stsd.entries[0];
    let mha_entry = match entry {
        SampleEntryVariant::Mha(e) => e,
        other => panic!(
            "expected SampleEntryVariant::Mha, found {:?} at stsd offset {}",
            other, stsd_offset
        ),
    };

    assert_eq!(&mha_entry.codec_type, b"mha1", "codec_type must be 'mha1'");
    assert_eq!(
        mha_entry.channelcount, 6,
        "channelcount must survive init build"
    );
    assert_eq!(
        mha_entry.samplesize, 16,
        "samplesize must survive init build"
    );

    // Find the mhaC child in config_boxes.
    let mhac_box = mha_entry
        .config_boxes
        .iter()
        .find(|b| b.box_type == MHAC_FOURCC)
        .expect("mhaC box not found in mha1 config_boxes");

    // Parse the mhaC body as an MHADecoderConfigurationRecord.
    let reparsed = MHADecoderConfigurationRecord::parse(&mhac_box.data)
        .expect("parse mhaC box body as MHADecoderConfigurationRecord failed");

    assert_eq!(
        reparsed, config,
        "mhaC content must round-trip through build_init_segment"
    );
    assert_eq!(
        reparsed.mpegh3da_config, blob,
        "mpegh3daConfig blob must survive init-segment serialization"
    );
}

// ---------------------------------------------------------------------------
// Gate 5: rfc6381() returns "mhm1.0x0D" for LC profile-level 3.
// ---------------------------------------------------------------------------
#[test]
fn rfc6381_format() {
    let rec =
        MHADecoderConfigurationRecord::new(LC_PROFILE_LEVEL_3, CICP_CH_5_1, test_config_blob());
    assert_eq!(
        rec.rfc6381(),
        "mhm1.0x0D",
        "rfc6381() must return 'mhm1.0xNN' with profile-level as two upper-hex digits"
    );
}

// ---------------------------------------------------------------------------
// Additional coverage: boundary / error cases.
// ---------------------------------------------------------------------------

/// Buffer too short for the fixed header.
#[test]
fn parse_error_too_short() {
    let too_short = [0x01u8, 0x0D]; // only 2 bytes — need 5
    let err = MHADecoderConfigurationRecord::parse(&too_short);
    assert!(
        err.is_err(),
        "parse must fail on buffer shorter than MHAC_RECORD_FIXED_LEN"
    );
}

/// configurationVersion != 1 must be rejected.
#[test]
fn parse_error_bad_version() {
    let bad_version = [0x02u8, 0x0D, 0x06, 0x00, 0x00]; // version=2, no config bytes
    let err = MHADecoderConfigurationRecord::parse(&bad_version);
    assert!(
        err.is_err(),
        "parse must reject configurationVersion != 1 (ISO/IEC 23008-3 §20)"
    );
}

/// config_length claims more bytes than available.
#[test]
fn parse_error_config_truncated() {
    // configurationVersion=1, profileLevel=0x0D, rcl=6, configLength=10, only 3 config bytes
    let truncated = [0x01u8, 0x0D, 0x06, 0x00, 0x0A, 0xA5, 0x5A, 0x01];
    let err = MHADecoderConfigurationRecord::parse(&truncated);
    assert!(
        err.is_err(),
        "parse must fail when config blob is truncated"
    );
}

/// Zero-length mpegh3daConfig is valid per the spec.
#[test]
fn zero_length_config_round_trips() {
    let rec = MHADecoderConfigurationRecord::new(LC_PROFILE_LEVEL_3, CICP_CH_5_1, alloc::vec![]);
    assert_eq!(rec.serialized_len(), MHAC_RECORD_FIXED_LEN);

    let mut buf = alloc::vec![0u8; rec.serialized_len()];
    rec.serialize_into(&mut buf).expect("serialize empty blob");

    let reparsed = MHADecoderConfigurationRecord::parse(&buf).expect("parse zero-length config");
    assert_eq!(reparsed, rec);
}

/// MhaSampleEntry can represent mhm1 (in-band MHAS) by setting codec_type.
#[test]
fn mha_sample_entry_mhm1_codec_type() {
    use transmux::MHM1_FOURCC;
    let blob = test_config_blob();
    let config = MHADecoderConfigurationRecord::new(LC_PROFILE_LEVEL_3, CICP_CH_5_1, blob.clone());

    // Serialize the record to get the mhaC body.
    let mut body = alloc::vec![0u8; config.serialized_len()];
    config.serialize_into(&mut body).unwrap();

    let mhac_opaque = transmux::OpaqueBox::new(MHAC_FOURCC, body);

    let entry = MhaSampleEntry {
        codec_type: MHM1_FOURCC,
        data_reference_index: 1,
        channelcount: 2,
        samplesize: 16,
        samplerate: 48000 << 16,
        config_boxes: alloc::vec![mhac_opaque],
    };

    // Serialize then parse to verify the FourCC survives.
    let mut buf = alloc::vec![0u8; entry.serialized_len()];
    entry.serialize_into(&mut buf).unwrap();

    let reparsed = MhaSampleEntry::parse(&buf).expect("parse mhm1 entry");
    assert_eq!(
        &reparsed.codec_type, b"mhm1",
        "mhm1 codec_type must survive serialize/parse"
    );
}

// ---------------------------------------------------------------------------
// Test helper: walk an init segment's raw bytes to find the stsd box.
// Returns the stsd slice and its byte offset in the init buffer.
// ---------------------------------------------------------------------------
fn find_stsd_in_init(init: &[u8]) -> Option<(&[u8], usize)> {
    // The init segment is a flat sequence of top-level boxes (ftyp, moov, …).
    let moov = walk_boxes(init, b"moov")?;
    let trak = walk_boxes(box_children(moov), b"trak")?;
    let mdia = walk_boxes(box_children(trak), b"mdia")?;
    let minf = walk_boxes(box_children(mdia), b"minf")?;
    let stbl = walk_boxes(box_children(minf), b"stbl")?;
    let stsd = walk_boxes(box_children(stbl), b"stsd")?;
    let offset = stsd.as_ptr() as usize - init.as_ptr() as usize;
    Some((stsd, offset))
}

/// Return the children slice of a container box (skipping the 8-byte header).
fn box_children(box_bytes: &[u8]) -> &[u8] {
    if box_bytes.len() >= 8 {
        &box_bytes[8..]
    } else {
        &[]
    }
}

/// Walk a flat sequence of ISOBMFF boxes and return the first with the given
/// FourCC (full box bytes, header included).
fn walk_boxes<'a>(data: &'a [u8], fourcc: &[u8; 4]) -> Option<&'a [u8]> {
    let mut off = 0usize;
    while off + 8 <= data.len() {
        let sz =
            u32::from_be_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]]) as usize;
        if sz < 8 {
            break;
        }
        let end = (off + sz).min(data.len());
        if &data[off + 4..off + 8] == fourcc.as_ref() {
            return Some(&data[off..end]);
        }
        off += sz;
    }
    None
}
