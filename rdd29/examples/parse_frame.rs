//! Wrap the committed real-fixture E-AC-3 frame
//! (`tests/fixtures/eac3_frame0.bin`) as an opaque `AudioDataDLC` payload
//! inside a hand-built `ATMOSFrame`, parse it back, and confirm the payload
//! is byte-identical to the real capture — SMPTE RDD 29:2019 §2.4/§4.5.
//!
//! Run with `cargo run -p rdd29 --example parse_frame`.

use broadcast_common::{Parse, Serialize};
use rdd29::{
    AnyElement, AtmosFrame, AudioDataDlc, BedChannel, BedDefinition1, BitDepth, ChannelId,
    FrameRate, SampleRate,
};

fn main() {
    // See st337/docs/st337-PROVENANCE.md for how this fixture was extracted
    // (a real E-AC-3 syncframe from fixtures/ts/dolby/eac3.ts). RDD 29 never
    // decodes audio essence -- this crate treats it as an opaque
    // AudioDataDLC payload, exactly like st337 treats its burst_payload.
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/eac3_frame0.bin"
    );
    let real_audio = std::fs::read(path).expect("fixture must exist");

    let bed = BedDefinition1::new(
        1,
        vec![BedChannel {
            channel_id: ChannelId::LeftScreen,
            audio_data_id: 10,
        }],
    );
    let dlc = AudioDataDlc::new(10, &real_audio).expect("build AudioDataDLC");

    let frame = AtmosFrame::new(
        SampleRate::Hz48000,
        BitDepth::Bits24,
        FrameRate::Fps24,
        1,
        vec![
            AnyElement::BedDefinition1(bed),
            AnyElement::AudioDataDlc(dlc),
        ],
    );
    let bytes = frame.to_bytes();

    let parsed = AtmosFrame::parse(&bytes).expect("parse ATMOSFrame");
    println!("version: {}", parsed.version);
    println!("sample_rate: {}", parsed.sample_rate);
    println!("frame_rate: {}", parsed.frame_rate);
    println!("sub-elements: {}", parsed.elements.len());

    let AnyElement::AudioDataDlc(parsed_dlc) = &parsed.elements[1] else {
        panic!("expected AudioDataDLC as the second element");
    };
    assert_eq!(
        parsed_dlc.payload, real_audio,
        "byte-identical real E-AC-3 payload carried as opaque AudioDataDLC bytes"
    );
    println!(
        "AudioDataDLC payload ({} bytes) matches the real E-AC-3 fixture byte-for-byte.",
        parsed_dlc.payload.len()
    );
}
