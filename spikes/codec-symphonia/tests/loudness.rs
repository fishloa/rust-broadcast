//! Decode the committed AAC fixture through the seam and measure it.

use std::num::NonZeroU32;
use std::process::Command;
use std::time::Duration;

use broadcast_codec::{
    AudioParameters, AudioSamples, ChannelLayout, CodecId, CodecParameters, Decoder, Frame, Packet,
    Registry, Timescale,
};
use broadcast_loudness::LoudnessMeter;
use codec_symphonia::SymphoniaProvider;
use codec_symphonia::adts::AdtsFrames;
use test_fixtures::process::{output_bounded, tool_available};

/// The committed AAC-LC fixture (mono, 48 kHz, 1.02 s).
const FIXTURE: &str = "audio/aac.adts";
/// Integrated loudness measured with ffmpeg 8.1.2:
/// `ffmpeg -nostats -i fixtures/audio/aac.adts -filter_complex ebur128 -f null -`
const EXPECTED_LUFS: f64 = -21.1;
/// Allowed difference from the pinned ffmpeg value, in LU.
const TOLERANCE_LU: f64 = 0.1;
/// Deadline for the external ffmpeg run.
const FFMPEG_DEADLINE: Duration = Duration::from_secs(60);
/// Samples one ADTS access unit in this fixture decodes to (AAC-LC).
const AAC_FRAME_SAMPLES: u64 = 1024;

fn loudness_layout(layout: ChannelLayout) -> Option<broadcast_loudness::ChannelLayout> {
    match layout {
        ChannelLayout::Mono => Some(broadcast_loudness::ChannelLayout::Mono),
        ChannelLayout::Stereo => Some(broadcast_loudness::ChannelLayout::Stereo),
        ChannelLayout::Surround5_1 => Some(broadcast_loudness::ChannelLayout::Surround51),
        _ => None,
    }
}

fn drain(decoder: &mut dyn Decoder, meter: &mut LoudnessMeter, samples: &mut u64) {
    while let Some(frame) = decoder.receive().unwrap() {
        let Frame::Audio(audio) = frame else {
            continue;
        };
        let AudioSamples::F32(values) = audio.samples() else {
            continue;
        };
        let channels = audio.channels().count();
        for chunk in values.chunks(channels) {
            meter.push_f32(chunk).unwrap();
        }
        *samples += audio.frames() as u64;
    }
}

/// Decode the fixture through a `Box<dyn Decoder>` and measure it.
fn measure() -> (f64, u64) {
    let data = test_fixtures::read(FIXTURE);
    let frames: Vec<_> = AdtsFrames::new(&data).collect();
    let first = frames.first().unwrap();
    let sample_rate = NonZeroU32::new(first.sample_rate()).unwrap();
    let channels = first.channels().unwrap();
    let layout = loudness_layout(channels).unwrap();

    let params = CodecParameters::new(CodecId::Aac, Timescale::from_nonzero(sample_rate))
        .with_audio(AudioParameters::new(sample_rate, channels));
    let registry = Registry::new()
        .with(Box::new(SymphoniaProvider))
        .expect("the symphonia provider registers");
    let mut decoder = registry.decoder(&params).expect("an AAC decoder");
    let mut meter = LoudnessMeter::new(sample_rate.get(), layout).unwrap();
    let mut decoded_samples = 0u64;

    let mut pts = 0i64;
    for frame in &frames {
        decoder
            .send(Packet::borrowed(frame.bytes()).with_pts(pts))
            .unwrap();
        drain(&mut *decoder, &mut meter, &mut decoded_samples);
        pts = pts.saturating_add(i64::from(sample_rate.get()));
    }
    decoder.finish().unwrap();
    drain(&mut *decoder, &mut meter, &mut decoded_samples);

    meter.finish();
    assert_eq!(
        decoded_samples,
        frames.len() as u64 * AAC_FRAME_SAMPLES,
        "decoder lost audio frames"
    );
    (meter.integrated_lufs(), decoded_samples)
}

#[test]
fn integrated_loudness_matches_the_pinned_value() {
    let (measured, samples) = measure();
    println!("measured {measured:.3} LUFS over {samples} samples");
    assert!(samples > 0, "decoder produced no audio");
    assert!(
        (measured - EXPECTED_LUFS).abs() <= TOLERANCE_LU,
        "integrated {measured:.3} LUFS, expected {EXPECTED_LUFS} +/- {TOLERANCE_LU}"
    );
}

#[test]
fn integrated_loudness_matches_a_live_ffmpeg_run() {
    if !tool_available("ffmpeg", "-version") {
        eprintln!("skip: ffmpeg is not on PATH");
        return;
    }
    let (measured, _) = measure();

    let mut command = Command::new("ffmpeg");
    command
        .arg("-nostats")
        .arg("-i")
        .arg(test_fixtures::path(FIXTURE))
        .arg("-filter_complex")
        .arg("ebur128")
        .arg("-f")
        .arg("null")
        .arg("-");
    let output = output_bounded(&mut command, FFMPEG_DEADLINE).unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    let ffmpeg_lufs = stderr
        .lines()
        .filter_map(|line| line.trim().strip_prefix("I:"))
        .filter_map(|value| value.split_whitespace().next())
        .filter_map(|value| value.parse::<f64>().ok())
        .next_back()
        .unwrap_or_else(|| panic!("no `I:` value in ffmpeg output:\n{stderr}"));

    println!("ffmpeg {ffmpeg_lufs:.3} LUFS, seam {measured:.3} LUFS");
    assert!(
        (measured - ffmpeg_lufs).abs() <= TOLERANCE_LU,
        "seam {measured:.3} LUFS vs ffmpeg {ffmpeg_lufs:.3} LUFS"
    );
}
