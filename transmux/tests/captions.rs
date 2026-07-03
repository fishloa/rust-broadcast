//! Integration tests for ISO/IEC 14496-30 subtitle sample entries (stpp + wvtt).
//!
//! EXIT CRITERION 1 (stpp):
//!   Parse the stpp sample entry from `fixtures/mp4/stpp.mp4` (ffmpeg TTML output),
//!   verify namespace is non-empty, byte-exact round-trip, and mutation changes bytes.
//!
//! EXIT CRITERION 2 (wvtt):
//!   Build a WVTTSampleEntry from a spec vector (vttC "WEBVTT" + a vttc/payl cue),
//!   round-trip serialize→parse→serialize and assert byte-identical.

use broadcast_common::{Parse, Serialize};
use transmux::{
    CuePayloadBox, SampleDescriptionBox, SampleEntryVariant, VttCueBox, VttEmptyCueBox,
    WebVttConfigurationBox, WvttSampleEntry, XmlSubtitleSampleEntry,
};

// ---------------------------------------------------------------------------
// Helper: walk boxes and find first box with the given four-CC.
// Returns the full box bytes (header + body).
// ---------------------------------------------------------------------------

fn find_box_recursive<'a>(data: &'a [u8], target: &[u8; 4]) -> Option<&'a [u8]> {
    let mut pos = 0usize;
    while pos + 8 <= data.len() {
        let sz =
            u32::from_be_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]) as usize;
        if sz < 8 {
            break;
        }
        let end = (pos + sz).min(data.len());
        let fourcc = &data[pos + 4..pos + 8];
        if fourcc == target {
            return Some(&data[pos..end]);
        }
        // Recurse into container boxes
        match fourcc {
            b"moov" | b"trak" | b"mdia" | b"minf" | b"stbl" | b"udta" => {
                if let Some(found) = find_box_recursive(&data[pos + 8..end], target) {
                    return Some(found);
                }
            }
            _ => {}
        }
        pos += sz;
    }
    None
}

// ---------------------------------------------------------------------------
// stpp real-fixture test
// ---------------------------------------------------------------------------

#[test]
fn stpp_parse_and_round_trip() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/mp4/stpp.mp4");
    let data = std::fs::read(path).expect("stpp.mp4 fixture must exist");

    // Navigate to the stsd box
    let stsd_box = find_box_recursive(&data, b"stsd").expect("stsd box must exist in stpp.mp4");

    // Parse via SampleDescriptionBox
    let stsd = SampleDescriptionBox::parse(stsd_box).expect("stsd must parse");
    assert_eq!(stsd.entries.len(), 1, "stsd should have exactly 1 entry");

    // Verify the first entry is Stpp
    let entry = match &stsd.entries[0] {
        SampleEntryVariant::Stpp(s) => s,
        other => panic!("expected SampleEntryVariant::Stpp, got {other:?}"),
    };

    // Namespace must be non-empty (ffmpeg encodes http://www.w3.org/ns/ttml)
    assert!(
        !entry.namespace.is_empty(),
        "namespace must be non-empty, got {:?}",
        entry.namespace
    );

    // Byte-exact round-trip: parse the raw stpp box bytes and serialize back
    // The stpp box starts at offset 16 into the stsd box (stsd header=8 + full-box=4 + count=4)
    let stpp_box = &stsd_box[16..];
    let sz = u32::from_be_bytes([stpp_box[0], stpp_box[1], stpp_box[2], stpp_box[3]]) as usize;
    let stpp_bytes = &stpp_box[..sz];
    assert_eq!(&stpp_bytes[4..8], b"stpp", "stpp fourcc must match");

    let parsed = XmlSubtitleSampleEntry::bare_parse(stpp_bytes)
        .expect("XmlSubtitleSampleEntry::bare_parse must succeed");

    // Namespace must match what we parsed through SampleDescriptionBox
    assert_eq!(parsed.namespace, entry.namespace);

    // Byte-exact round-trip
    let serialized = parsed.to_bytes();
    assert_eq!(
        serialized.as_slice(),
        stpp_bytes,
        "round-trip must be byte-identical"
    );

    // Mutation proof: changing namespace changes the serialized bytes
    let mut mutated = parsed.clone();
    mutated.namespace = String::from("http://example.com/mutated");
    let mutated_bytes = mutated.to_bytes();
    assert_ne!(
        mutated_bytes.as_slice(),
        stpp_bytes,
        "mutating namespace must change serialized bytes"
    );
}

// ---------------------------------------------------------------------------
// stpp direct parse test (validates namespace value from fixture)
// ---------------------------------------------------------------------------

#[test]
fn stpp_namespace_is_ttml() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/mp4/stpp.mp4");
    let data = std::fs::read(path).expect("stpp.mp4 fixture must exist");
    let stsd_box = find_box_recursive(&data, b"stsd").expect("stsd");
    let stsd = SampleDescriptionBox::parse(stsd_box).unwrap();

    let entry = match &stsd.entries[0] {
        SampleEntryVariant::Stpp(s) => s,
        _ => panic!("expected Stpp"),
    };

    // ffmpeg uses http://www.w3.org/ns/ttml as namespace
    assert!(
        entry.namespace.contains("ttml"),
        "namespace should reference TTML: {:?}",
        entry.namespace
    );
}

// ---------------------------------------------------------------------------
// wvtt spec-vector test (no ffmpeg fixture available)
// ---------------------------------------------------------------------------

#[test]
fn wvtt_spec_vector_round_trip() {
    // Build a WVTTSampleEntry from scratch following ISO/IEC 14496-30 §9.2
    let entry = WvttSampleEntry::new("WEBVTT");

    // Serialize and verify box structure
    let bytes = entry.to_bytes();
    assert_eq!(&bytes[4..8], b"wvtt", "fourcc must be wvtt");

    // Parse back and assert equality
    let parsed = WvttSampleEntry::bare_parse(&bytes).expect("wvtt must parse");
    assert_eq!(parsed.config.config, "WEBVTT");
    assert_eq!(parsed.data_reference_index, 1);
    assert!(parsed.extra_boxes.is_empty());

    // Byte-exact round-trip
    let reser = parsed.to_bytes();
    assert_eq!(
        reser.as_slice(),
        bytes.as_slice(),
        "wvtt round-trip must be byte-identical"
    );

    // Mutation proof: changing config changes bytes
    let mut mutated = parsed.clone();
    mutated.config.config = String::from("WEBVTT\n\nNOTE mutated");
    let mutated_bytes = mutated.to_bytes();
    assert_ne!(
        mutated_bytes.as_slice(),
        bytes.as_slice(),
        "mutating vttC config must change serialized bytes"
    );
}

// ---------------------------------------------------------------------------
// wvtt: vttc cue box spec vector (sample payload, not sample entry)
// ---------------------------------------------------------------------------

#[test]
fn vttc_payl_spec_vector_round_trip() {
    // Build a vttc cue box: vttC "WEBVTT" in sample entry, cue in vttc box
    let cue = VttCueBox::new("Hello, World!");
    let bytes = cue.to_bytes();
    assert_eq!(&bytes[4..8], b"vttc", "fourcc must be vttc");

    let parsed = VttCueBox::bare_parse(&bytes).expect("vttc must parse");
    assert_eq!(parsed.payload.cue_text, "Hello, World!");
    assert!(parsed.settings.is_none());
    assert!(parsed.cue_id.is_none());

    let reser = parsed.to_bytes();
    assert_eq!(
        reser.as_slice(),
        bytes.as_slice(),
        "vttc round-trip must be byte-identical"
    );
}

// ---------------------------------------------------------------------------
// wvtt: vtte empty cue box spec vector
// ---------------------------------------------------------------------------

#[test]
fn vtte_spec_vector_round_trip() {
    let vtte = VttEmptyCueBox;
    let bytes = vtte.to_bytes();
    assert_eq!(bytes.len(), 8, "vtte must be exactly 8 bytes");
    assert_eq!(&bytes[4..8], b"vtte", "fourcc must be vtte");

    // Parse back
    let parsed = VttEmptyCueBox::bare_parse(&bytes).expect("vtte must parse");
    let reser = parsed.to_bytes();
    assert_eq!(
        reser.as_slice(),
        bytes.as_slice(),
        "vtte round-trip must be byte-identical"
    );
}

// ---------------------------------------------------------------------------
// wvtt: complete spec vector with vttC in sample entry + vttc/payl in sample
// ---------------------------------------------------------------------------

#[test]
fn wvtt_complete_spec_vector() {
    // Build a complete wvtt sample entry (the init-segment part)
    let mut entry = WvttSampleEntry::new("WEBVTT");
    entry.data_reference_index = 1;
    let se_bytes = entry.to_bytes();

    // Verify wvtt structure:
    //   [0..4]  size
    //   [4..8]  'wvtt'
    //   [8..14] reserved (6 zeros)
    //   [14..16] data_reference_index = 1
    //   [16..]  vttC box
    assert_eq!(&se_bytes[4..8], b"wvtt");
    assert_eq!(&se_bytes[8..14], &[0u8; 6], "reserved must be zeros");
    let dri = u16::from_be_bytes([se_bytes[14], se_bytes[15]]);
    assert_eq!(dri, 1, "data_reference_index must be 1");
    // vttC box starts at offset 16
    assert_eq!(&se_bytes[20..24], b"vttC", "vttC fourcc at offset 20");

    // Now build a sample payload: a vttc box with a payl child
    let cue = VttCueBox::new("00:00:01.000 --> 00:00:03.000\nHello WebVTT!");
    let sample_bytes = cue.to_bytes();

    // Parse the sample payload back
    let parsed_cue = VttCueBox::bare_parse(&sample_bytes).unwrap();
    assert!(parsed_cue.payload.cue_text.contains("Hello WebVTT!"));

    // Byte-exact round-trip of sample payload
    assert_eq!(parsed_cue.to_bytes().as_slice(), sample_bytes.as_slice());

    // Parse the sample entry back
    let parsed_se = WvttSampleEntry::bare_parse(&se_bytes).unwrap();
    assert_eq!(parsed_se.config.config, "WEBVTT");
    assert_eq!(parsed_se.to_bytes().as_slice(), se_bytes.as_slice());
}

// ---------------------------------------------------------------------------
// Verify vttC WebVttConfigurationBox parse + round-trip
// ---------------------------------------------------------------------------

#[test]
fn vttc_config_box_round_trip() {
    let cfg = WebVttConfigurationBox::new("WEBVTT");
    let bytes = cfg.to_bytes();
    assert_eq!(&bytes[4..8], b"vttC");
    // body is "WEBVTT" (6 bytes), so total = 8 + 6 = 14
    assert_eq!(bytes.len(), 14);

    let parsed = WebVttConfigurationBox::bare_parse(&bytes).unwrap();
    assert_eq!(parsed.config, "WEBVTT");
    assert_eq!(parsed.to_bytes().as_slice(), bytes.as_slice());
}

// ---------------------------------------------------------------------------
// Verify CuePayloadBox parse + round-trip
// ---------------------------------------------------------------------------

#[test]
fn cue_payload_box_round_trip() {
    let payl = CuePayloadBox::new("Test subtitle text");
    let bytes = payl.to_bytes();
    assert_eq!(&bytes[4..8], b"payl");

    let parsed = CuePayloadBox::bare_parse(&bytes).unwrap();
    assert_eq!(parsed.cue_text, "Test subtitle text");
    assert_eq!(parsed.to_bytes().as_slice(), bytes.as_slice());
}
