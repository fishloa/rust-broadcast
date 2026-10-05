//! Real-fixture round-trip tests for the MPEG-4 `esds` box.
//!
//! These tests navigate the container hierarchy in a real fragmented MP4 file
//! and verify that parsing an existing `esds` box and serializing it back produces
//! byte-identical output.
//!
//! The fixture `h264_aac_frag.mp4` contains an AAC audio track whose `mp4a` sample
//! entry carries an `esds` box with a full ES_Descriptor chain (ES_Descriptor,
//! DecoderConfigDescriptor, DecoderSpecificInfo for AAC, SLConfigDescriptor).

use broadcast_common::{Parse, Serialize};
use transmux::{ESDescriptor, EsdsBox, ObjectTypeIndication, StreamType};

/// Find the first `esds` box in the byte stream by scanning for the four-CC.
fn find_esds_box(data: &[u8]) -> &[u8] {
    let pos = data
        .windows(4)
        .position(|w| w == b"esds")
        .expect("esds four-CC must be present");
    let start = pos - 4;
    let size = u32::from_be_bytes([
        data[start],
        data[start + 1],
        data[start + 2],
        data[start + 3],
    ]) as usize;
    &data[start..start + size]
}

#[test]
fn esds_real_fixture_round_trip() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/transmux/h264_aac_frag.mp4"
    );
    let data = std::fs::read(path).expect("fixture file must exist");
    let esds_box_bytes = find_esds_box(&data);

    // Verify the esds box has expected size (54 bytes for this fixture)
    assert_eq!(esds_box_bytes.len(), 54, "esds box size in fixture");

    // Parse the esds box
    let esds = EsdsBox::parse_box(esds_box_bytes).expect("should parse esds box");
    let desc = &esds.es_descriptor;

    // Verify key ES_Descriptor fields (14496-14 §3.1.2: ES_ID=0 for MP4)
    assert_eq!(desc.es_id, 2, "ES_ID should be 2");
    assert!(
        !desc.stream_dependence_flag,
        "streamDependenceFlag should be false"
    );
    assert!(!desc.url_flag, "URL_Flag should be false");
    assert!(!desc.ocr_stream_flag, "OCRstreamFlag should be false");
    assert_eq!(desc.stream_priority, 0, "streamPriority should be 0");
    assert!(
        desc.depends_on_es_id.is_none(),
        "dependsOn_ES_ID should be absent"
    );
    assert!(desc.url.is_none(), "URL should be absent");
    assert!(desc.ocr_es_id.is_none(), "OCR_ES_Id should be absent");

    // Verify DecoderConfigDescriptor
    let dc = desc
        .decoder_config
        .as_ref()
        .expect("should have DecoderConfigDescriptor");
    assert_eq!(
        dc.object_type_indication,
        ObjectTypeIndication(0x40),
        "OTI should be 0x40 (AAC)"
    );
    assert_eq!(
        dc.stream_type,
        StreamType(0x05),
        "streamType should be 5 (AudioStream)"
    );
    assert!(!dc.up_stream, "upStream should be false");
    assert_eq!(dc.buffer_size_db, 0, "bufferSizeDB should be 0");
    assert_eq!(
        dc.max_bitrate, 0x0001_7700,
        "maxBitrate should be 0x00017700 = 95488"
    );
    assert_eq!(
        dc.avg_bitrate, 0x0001_7700,
        "avgBitrate should be 0x00017700 = 95488"
    );

    // Verify DecoderSpecificInfo (opaque AAC AudioSpecificConfig bytes)
    let dsi = dc
        .decoder_specific_info
        .as_ref()
        .expect("should have DecoderSpecificInfo");
    assert_eq!(dsi.data, vec![0x12, 0x08, 0x56, 0xe5, 0x00]);

    // Verify SLConfigDescriptor (predefined=2 for MP4)
    let sl = desc
        .sl_config
        .as_ref()
        .expect("should have SLConfigDescriptor");
    assert_eq!(sl.body, vec![0x02], "SLConfig predefined=2");

    // Round-trip: serialize the parsed box
    let mut out = vec![0u8; esds.serialized_len()];
    let n = esds
        .serialize_into(&mut out)
        .expect("should serialize esds");

    // Must be byte-identical to the original esds box (including 4-byte expanded varints)
    assert_eq!(
        &out[..n],
        esds_box_bytes,
        "esds round-trip must be byte-identical (including 4-byte size varints)"
    );
}

#[test]
fn esds_oti_mutation_changes_bytes() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/transmux/h264_aac_frag.mp4"
    );
    let data = std::fs::read(path).expect("fixture file must exist");
    let esds_box_bytes = find_esds_box(&data);

    let mut esds = EsdsBox::parse_box(esds_box_bytes).expect("should parse");
    let original = {
        let mut buf = vec![0u8; esds.serialized_len()];
        esds.serialize_into(&mut buf).unwrap();
        buf
    };

    // Mutate objectTypeIndication
    let dc = esds
        .es_descriptor
        .decoder_config
        .as_mut()
        .expect("should have DecoderConfigDescriptor");
    dc.object_type_indication = ObjectTypeIndication(0x21); // AVC

    let mutated = esds.to_bytes();
    assert_ne!(
        mutated, original,
        "mutating OTI must change serialized bytes"
    );
}

/// #1148 (finding C): byte-exact round trips must hold for `esds` boxes as
/// they appear in committed real fixtures, not only hand-authored bytes —
/// through `EsdsBox` *and* through the public `ESDescriptor::parse`, which
/// reads the framed bytes (tag + expandable size + body) directly.
///
/// No committed fixture carries `URL_Flag` (real muxers essentially never
/// emit a `URLstring`); no URL-carrying real `esds` is fabricated here.
#[test]
fn esds_real_fixtures_round_trip_box_and_public_parse() {
    const BOX_HEADER: usize = 8;
    const FULLBOX_EXTRA: usize = 4;
    let fixtures = [
        "transmux/h264_aac_frag.mp4",
        "ts/demux-oracle/h264_aac.ref.mp4",
        "mp4/aac_sgpd.mp4",
        "mp4/progressive/av_prog.mp4",
    ];
    let mut seen: Vec<Vec<u8>> = Vec::new();

    for rel in fixtures {
        let path = format!("{}/../fixtures/{}", env!("CARGO_MANIFEST_DIR"), rel);
        let data = std::fs::read(&path).unwrap_or_else(|e| panic!("fixture {path}: {e}"));
        let box_bytes = find_esds_box(&data);

        // Box path: parse_box -> serialize must be byte-identical.
        let esds = EsdsBox::parse_box(box_bytes)
            .unwrap_or_else(|e| panic!("{rel}: parse_box failed: {e:?}"));
        let mut out = vec![0u8; esds.serialized_len()];
        let n = esds.serialize_into(&mut out).expect("serialize");
        assert_eq!(&out[..n], box_bytes, "{rel}: esds box not byte-identical");

        // Public Parse path: same bytes after the box header + FullBox
        // header (what ESDescriptor::serialize_into writes).
        let payload = &box_bytes[BOX_HEADER + FULLBOX_EXTRA..];
        let desc = ESDescriptor::parse(payload)
            .unwrap_or_else(|e| panic!("{rel}: public ESDescriptor::parse failed: {e:?}"));
        let mut desc_out = vec![0u8; desc.serialized_len()];
        let dn = desc.serialize_into(&mut desc_out).expect("serialize");
        assert_eq!(
            &desc_out[..dn],
            payload,
            "{rel}: public ESDescriptor round trip not byte-identical"
        );
        assert_eq!(
            desc, esds.es_descriptor,
            "{rel}: public parse must agree with the box parse"
        );
        assert!(!desc.url_flag, "{rel}: unexpected URL_Flag in real fixture");

        seen.push(payload.to_vec());
    }

    // At least two of the fixtures must carry genuinely different payloads
    // (some committed files share an esds byte-for-byte).
    let mut distinct = 0usize;
    for i in 0..seen.len() {
        if !seen[..i].contains(&seen[i]) {
            distinct += 1;
        }
    }
    assert!(
        distinct >= 2,
        "round-tripped {distinct} distinct esds payloads; want >= 2"
    );
}
