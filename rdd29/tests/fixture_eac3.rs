//! Real-fixture gate: wrap a real E-AC-3 syncframe in a hand-built
//! `AudioDataDLC` element inside an `ATMOSFrame`, and confirm byte-identical
//! parse/serialize round-tripping.
//!
//! ## Fixture provenance
//!
//! `tests/fixtures/eac3_frame0.bin` is 834 real bytes: the same first
//! E-AC-3 syncframe already extracted, cross-checked, and committed for
//! `st337`'s own real-fixture test (`st337/tests/fixture_eac3.rs`,
//! `st337/docs/st337-PROVENANCE.md`) — itself pulled from this workspace's
//! ffmpeg-encoded capture `fixtures/ts/dolby/eac3.ts` (issue #426). RDD 29
//! carries audio essence in a proprietary lossless codec (DLC), not E-AC-3,
//! and has no capturable file format of its own outside a licensed encoder
//! -- so per this project's real-fixture discipline
//! (`docs/CRATE-ACCEPTANCE.md`) this crate reuses the same real payload
//! bytes as an opaque `AudioDataDLC` blob (exactly the role `st337` gives
//! them as an opaque `burst_payload`), with the RDD 29 framing around them
//! built directly through this crate's own typed API.

use broadcast_common::{Parse, Serialize};
use rdd29::{
    AnyElement, AtmosFrame, AudioDataDlc, BedChannel, BedDefinition1, BitDepth, ChannelId,
    FrameRate, SampleRate,
};

fn real_eac3_frame() -> Vec<u8> {
    std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/eac3_frame0.bin"
    ))
    .expect("eac3_frame0.bin fixture must exist")
}

fn build_frame(payload: &[u8]) -> AtmosFrame<'_> {
    let bed = BedDefinition1::new(
        1,
        vec![BedChannel {
            channel_id: ChannelId::LeftScreen,
            audio_data_id: 10,
        }],
    );
    let dlc = AudioDataDlc::new(10, payload).expect("build AudioDataDLC from real E-AC-3 frame");
    AtmosFrame::new(
        SampleRate::Hz48000,
        BitDepth::Bits24,
        FrameRate::Fps24,
        1,
        vec![
            AnyElement::BedDefinition1(bed),
            AnyElement::AudioDataDlc(dlc),
        ],
    )
}

#[test]
fn wraps_real_eac3_frame_and_parses_back() {
    let payload = real_eac3_frame();
    assert_eq!(
        payload.len(),
        834,
        "real E-AC-3 frame 0 is 834 bytes (ffprobe-confirmed, see st337/docs/st337-PROVENANCE.md)"
    );

    let frame = build_frame(&payload);
    let bytes = frame.to_bytes();

    let parsed = AtmosFrame::parse(&bytes).expect("parse the wrapped real frame");
    assert_eq!(parsed.elements.len(), 2);
    let AnyElement::AudioDataDlc(dlc) = &parsed.elements[1] else {
        panic!("expected AudioDataDLC as the second element");
    };
    assert_eq!(dlc.audio_data_id, 10);
    assert_eq!(
        dlc.payload, payload,
        "payload extracted from the parsed AudioDataDLC must be byte-identical to the real \
         E-AC-3 frame"
    );
}

#[test]
fn byte_identical_round_trip_real_fixture() {
    let payload = real_eac3_frame();
    let frame = build_frame(&payload);

    let mut out = vec![0u8; frame.serialized_len()];
    let n = frame.serialize_into(&mut out).unwrap();
    assert_eq!(n, out.len());

    let reparsed = AtmosFrame::parse(&out).unwrap();
    assert_eq!(reparsed, frame, "parse(serialize(frame)) == frame");

    let mut out2 = vec![0u8; reparsed.serialized_len()];
    reparsed.serialize_into(&mut out2).unwrap();
    assert_eq!(
        out, out2,
        "serialize(parse(serialize(frame))) is byte-identical"
    );
}

#[test]
fn mutating_meta_id_changes_only_bed_definition_bytes() {
    // Proves serialize_into recomputes fields from typed data rather than
    // echoing a stored raw buffer (guards against a `self.raw` passthrough).
    let payload = real_eac3_frame();
    let mut frame = build_frame(&payload);
    let original = frame.to_bytes();

    let AnyElement::BedDefinition1(bed) = &mut frame.elements[0] else {
        panic!("expected BedDefinition1 as the first element");
    };
    bed.meta_id = 200; // still Plex(8)-direct, same wire width as 1

    let mutated = frame.to_bytes();
    assert_ne!(
        original, mutated,
        "changing meta_id must change the wire bytes"
    );
    // Only the BedDefinition1 element's bytes (well before the large
    // AudioDataDLC payload) should differ.
    assert_eq!(
        &original[original.len() - payload.len()..],
        &mutated[mutated.len() - payload.len()..],
        "the real E-AC-3 payload bytes themselves must be untouched"
    );
}
