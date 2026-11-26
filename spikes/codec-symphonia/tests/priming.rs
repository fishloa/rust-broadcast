//! What symphonia does with AAC priming: it returns every decoded sample.

use std::num::NonZeroU32;
use std::process::Command;
use std::time::Duration;

use broadcast_codec::{
    AudioParameters, ChannelLayout, CodecId, CodecParameters, Decoder, Direction, Frame, Packet,
    Registry, Timescale,
};
use codec_symphonia::SymphoniaProvider;
use codec_symphonia::adts::AdtsFrames;
use test_fixtures::process::{output_bounded, tool_available};

/// The committed AAC-LC fixture.
const ADTS_FIXTURE: &str = "audio/aac.adts";
/// Its sample rate.
const SAMPLE_RATE: u32 = 48_000;
/// Samples one AAC access unit decodes to.
const AAC_FRAME_SAMPLES: usize = 1024;
/// Bytes in one f32 sample.
const F32_BYTES: usize = 4;
/// Deadline for one external ffmpeg run.
const FFMPEG_DEADLINE: Duration = Duration::from_secs(60);

fn rate() -> NonZeroU32 {
    NonZeroU32::new(SAMPLE_RATE).expect("non-zero")
}

fn parameters() -> CodecParameters {
    CodecParameters::new(CodecId::Aac, Timescale::from_nonzero(rate())).with_audio(
        AudioParameters::new(rate(), ChannelLayout::Mono),
    )
}

/// The per-channel sample count ffmpeg itself decodes from the fixture.
fn ffmpeg_samples() -> Option<usize> {
    if !tool_available("ffmpeg", "-version") {
        eprintln!("skip: ffmpeg is not on PATH, so the external decode did NOT run");
        return None;
    }
    let mut command = Command::new("ffmpeg");
    command
        .arg("-v")
        .arg("error")
        .arg("-i")
        .arg(test_fixtures::path(ADTS_FIXTURE))
        .arg("-f")
        .arg("f32le")
        .arg("-ar")
        .arg(SAMPLE_RATE.to_string())
        .arg("-ac")
        .arg("1")
        .arg("-");
    let output = output_bounded(&mut command, FFMPEG_DEADLINE).expect("ffmpeg runs");
    assert!(
        output.status.success(),
        "ffmpeg failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Some(output.stdout.len() / F32_BYTES)
}

#[test]
fn symphonia_applies_no_aac_priming() {
    let registry = Registry::new()
        .with(Box::new(SymphoniaProvider))
        .expect("the provider registers");
    let descriptor = registry
        .candidates(CodecId::Aac, Direction::Decode)
        .into_iter()
        .next()
        .expect("an AAC decoder");
    assert!(!descriptor.applies_trim());

    let data = test_fixtures::read(ADTS_FIXTURE);
    let packets: Vec<Vec<u8>> = AdtsFrames::new(&data).map(|frame| frame.bytes().to_vec()).collect();
    let mut decoder = registry.decoder(&parameters()).expect("an AAC decoder");
    let mut decoded = 0;
    for packet in &packets {
        decoder
            .send(Packet::borrowed(packet))
            .expect("the packet is accepted");
        drain(&mut *decoder, &mut decoded);
    }
    decoder.finish().expect("end of input is accepted");
    drain(&mut *decoder, &mut decoded);

    let reference = ffmpeg_samples();
    println!(
        "symphonia {decoded} samples over {} access units; ffmpeg {}",
        packets.len(),
        reference.map_or_else(|| "not run".to_string(), |samples| samples.to_string())
    );
    assert_eq!(decoded, packets.len() * AAC_FRAME_SAMPLES);
    if let Some(reference) = reference {
        assert_eq!(decoded, reference);
    }
}

fn drain(decoder: &mut dyn Decoder, decoded: &mut usize) {
    while let Some(frame) = decoder.receive().expect("receive succeeds") {
        let Frame::Audio(audio) = frame else {
            panic!("an audio decoder emitted a non-audio frame");
        };
        *decoded += audio.frames();
    }
}
