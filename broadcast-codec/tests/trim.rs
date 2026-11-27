//! The gapless trim: initial, per-frame, trailing and the held bound.

use std::num::NonZeroU32;

use broadcast_codec::{
    AudioFrame, AudioParameters, AudioSamples, ChannelLayout, CodecId, CodecParameters,
    PaddingError, SampleLayout, Timescale, TrimStage, VideoParameters,
};

/// The sample rate every frame uses.
const RATE: u32 = 48_000;
/// The stream time base, deliberately not the sample rate.
const STREAM_BASE: u32 = 90_000;

fn rate() -> NonZeroU32 {
    NonZeroU32::new(RATE).expect("a non-zero sample rate")
}

fn base() -> CodecParameters {
    CodecParameters::new(
        CodecId::MpegAudio,
        Timescale::new(STREAM_BASE).expect("non-zero"),
    )
    .with_audio(AudioParameters::new(rate(), ChannelLayout::Mono))
}

fn parameters(initial: u32, trailing: Option<u32>) -> CodecParameters {
    let parameters = base()
        .with_initial_padding(initial)
        .expect("audio parameters take padding");
    match trailing {
        Some(samples) => parameters
            .with_trailing_padding(samples)
            .expect("audio parameters take padding"),
        None => parameters,
    }
}

fn mono(values: Vec<i16>, pts: i64) -> AudioFrame {
    AudioFrame::new(
        rate(),
        ChannelLayout::Mono,
        SampleLayout::Interleaved,
        AudioSamples::S16(values),
    )
    .expect("a whole number of frames")
    .with_pts(pts)
}

fn values(frame: &AudioFrame) -> Vec<i16> {
    match frame.samples() {
        AudioSamples::S16(values) => values.clone(),
        other => panic!("expected S16, got {other:?}"),
    }
}

/// A frame's two-channel planar samples, channel by channel.
fn planar_values(frame: &AudioFrame) -> Vec<i16> {
    match frame.samples() {
        AudioSamples::S16(values) => values.clone(),
        other => panic!("expected S16, got {other:?}"),
    }
}

#[test]
fn no_initial_padding_emits_every_sample() {
    let mut stage = TrimStage::new(&parameters(0, None));
    let out = stage.push(mono(vec![1, 2, 3, 4], 0));
    assert_eq!(out.len(), 1);
    assert_eq!(values(&out[0]), [1, 2, 3, 4]);
    assert_eq!(out[0].pts(), Some(0));
}

#[test]
fn initial_padding_cuts_a_frame_and_moves_its_timestamp() {
    // 2 samples at 48 kHz in a 90 kHz time base: 2 * 90 000 / 48 000 = 3.75,
    // truncated toward zero to 3 ticks.
    let mut stage = TrimStage::new(&parameters(2, None));
    let out = stage.push(mono(vec![1, 2, 3, 4], 0));
    assert_eq!(out.len(), 1);
    assert_eq!(values(&out[0]), [3, 4]);
    assert_eq!(out[0].pts(), Some(3));
}

#[test]
fn initial_padding_spans_one_and_several_frames() {
    let mut stage = TrimStage::new(&parameters(6, None));
    assert!(stage.push(mono(vec![1, 2, 3, 4], 0)).is_empty());
    let out = stage.push(mono(vec![5, 6, 7, 8], 7));
    assert_eq!(out.len(), 1);
    assert_eq!(values(&out[0]), [7, 8]);
    // The second frame keeps its own 7 ticks and moves by 2 samples.
    assert_eq!(out[0].pts(), Some(10));
}

#[test]
fn trailing_padding_is_held_back_then_discarded_at_finish() {
    let mut stage = TrimStage::new(&parameters(0, Some(3)));
    let first = stage.push(mono(vec![1, 2, 3, 4], 0));
    assert_eq!(first.len(), 1);
    assert_eq!(values(&first[0]), [1]);
    assert_eq!(stage.held_samples(), 3);
    let second = stage.push(mono(vec![5, 6, 7, 8], 4));
    let emitted: Vec<i16> = second.iter().flat_map(values).collect();
    assert_eq!(emitted, [2, 3, 4, 5]);
    assert_eq!(stage.held_samples(), 3);
    assert!(stage.finish().is_empty());
    assert_eq!(stage.held_samples(), 0);
    assert_eq!(stage.held_frames(), 0);
}

#[test]
fn trailing_padding_larger_than_the_last_frame_holds_several() {
    let mut stage = TrimStage::new(&parameters(0, Some(6)));
    assert!(stage.push(mono(vec![1, 2, 3, 4], 0)).is_empty());
    let second = stage.push(mono(vec![5, 6, 7, 8], 4));
    assert_eq!(values(&second[0]), [1, 2]);
    assert_eq!(stage.held_samples(), 6);
    assert_eq!(stage.held_frames(), 2);
    assert!(stage.finish().is_empty());
    assert_eq!(stage.held_samples(), 0);
}

#[test]
fn absent_trailing_padding_emits_everything() {
    let mut stage = TrimStage::new(&parameters(0, None));
    let first = stage.push(mono(vec![1, 2, 3, 4], 0));
    let second = stage.push(mono(vec![5, 6, 7, 8], 4));
    assert_eq!(values(&first[0]), [1, 2, 3, 4]);
    assert_eq!(values(&second[0]), [5, 6, 7, 8]);
    assert_eq!(stage.held_samples(), 0);
    assert!(stage.finish().is_empty());
}

#[test]
fn a_frame_carries_its_own_trim_and_the_stage_clears_it() {
    let mut stage = TrimStage::new(&parameters(0, None));
    let frame = mono(vec![1, 2, 3, 4, 5, 6], 0)
        .with_trim_start(2)
        .with_trim_end(1);
    let out = stage.push(frame);
    assert_eq!(out.len(), 1);
    assert_eq!(values(&out[0]), [3, 4, 5]);
    // 2 samples removed from the front: 2 * 90 000 / 48 000 = 3 ticks.
    assert_eq!(out[0].pts(), Some(3));
    assert_eq!(out[0].trim_start(), 0);
    assert_eq!(out[0].trim_end(), 0);
}

#[test]
fn a_per_frame_trim_may_leave_nothing() {
    let mut stage = TrimStage::new(&parameters(0, None));
    let frame = mono(vec![1, 2, 3, 4], 0)
        .with_trim_start(2)
        .with_trim_end(2);
    assert!(stage.push(frame).is_empty());
}

#[test]
fn planar_and_interleaved_layouts_trim_the_same_frames() {
    let planar = AudioFrame::new(
        rate(),
        ChannelLayout::Stereo,
        SampleLayout::Planar,
        AudioSamples::S16(vec![1, 2, 3, 11, 12, 13]),
    )
    .expect("whole frames")
    .with_pts(0);
    let mut stage = TrimStage::new(&parameters(1, None));
    let out = stage.push(planar);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].layout(), SampleLayout::Planar);
    assert_eq!(planar_values(&out[0]), [2, 3, 12, 13]);
    assert_eq!(out[0].pts(), Some(1));

    let interleaved = AudioFrame::new(
        rate(),
        ChannelLayout::Stereo,
        SampleLayout::Interleaved,
        AudioSamples::S16(vec![1, 11, 2, 12, 3, 13]),
    )
    .expect("whole frames")
    .with_pts(0);
    let mut stage = TrimStage::new(&parameters(1, None));
    let out = stage.push(interleaved);
    assert_eq!(out[0].layout(), SampleLayout::Interleaved);
    assert_eq!(planar_values(&out[0]), [2, 12, 3, 13]);
}

#[test]
fn every_sample_variant_is_trimmed() {
    let cases = [
        AudioSamples::U8(vec![1, 2, 3, 4]),
        AudioSamples::S16(vec![1, 2, 3, 4]),
        AudioSamples::S32(vec![1, 2, 3, 4]),
        AudioSamples::F32(vec![1.0, 2.0, 3.0, 4.0]),
        AudioSamples::F64(vec![1.0, 2.0, 3.0, 4.0]),
    ];
    for samples in cases {
        let frame = AudioFrame::new(
            rate(),
            ChannelLayout::Mono,
            SampleLayout::Interleaved,
            samples,
        )
        .expect("whole frames")
        .with_pts(0);
        let mut stage = TrimStage::new(&parameters(1, None));
        let out = stage.push(frame);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].frames(), 3);
        assert_eq!(out[0].pts(), Some(1));
    }
}

#[test]
fn a_stream_shorter_than_its_padding_emits_nothing() {
    let mut stage = TrimStage::new(&parameters(10, None));
    assert!(stage.push(mono(vec![1, 2, 3, 4], 0)).is_empty());
    assert!(stage.finish().is_empty());
    assert_eq!(stage.held_samples(), 0);
}

#[test]
fn the_stage_never_holds_more_than_the_trailing_padding() {
    let mut stage = TrimStage::new(&parameters(0, Some(5)));
    for index in 0..4_i16 {
        let frame = mono(vec![index; 1000], 0);
        let _ = stage.push(frame);
        assert!(
            stage.held_samples() <= 5,
            "held {} samples",
            stage.held_samples()
        );
    }
}

#[test]
fn reset_restores_the_initial_padding() {
    let mut stage = TrimStage::new(&parameters(2, Some(1)));
    let out = stage.push(mono(vec![1, 2, 3, 4], 0));
    assert_eq!(values(&out[0]), [3]);
    assert!(stage.finish().is_empty());
    stage.reset();
    let out = stage.push(mono(vec![1, 2, 3, 4], 0));
    assert_eq!(values(&out[0]), [3]);
}

#[test]
fn a_video_stream_refuses_padding() {
    let video = CodecParameters::new(
        CodecId::H264,
        Timescale::new(STREAM_BASE).expect("non-zero"),
    )
    .with_video(VideoParameters::new(
        NonZeroU32::new(320).expect("non-zero"),
        NonZeroU32::new(240).expect("non-zero"),
    ));
    assert_eq!(
        video.with_initial_padding(1).map(|_| ()),
        Err(PaddingError::Video)
    );
    let video = CodecParameters::new(
        CodecId::H264,
        Timescale::new(STREAM_BASE).expect("non-zero"),
    )
    .with_video(VideoParameters::new(
        NonZeroU32::new(320).expect("non-zero"),
        NonZeroU32::new(240).expect("non-zero"),
    ));
    assert_eq!(
        video.with_trailing_padding(1).map(|_| ()),
        Err(PaddingError::Video)
    );
}

#[test]
fn audio_parameters_read_their_padding_back() {
    let padded = parameters(7, Some(9));
    assert_eq!(padded.initial_padding(), 7);
    assert_eq!(padded.trailing_padding(), Some(9));
    let none = parameters(0, None);
    assert_eq!(none.initial_padding(), 0);
    assert_eq!(none.trailing_padding(), None);
}

#[test]
fn a_decoder_delay_cuts_the_front_and_leaves_an_unstated_tail_alone() {
    let mut stage = TrimStage::new(&parameters(0, None)).with_decoder_delay(2);
    assert_eq!(stage.decoder_delay(), 2);
    let out = stage.push(mono(vec![1, 2, 3, 4], 0));
    assert_eq!(values(&out[0]), [3, 4]);
    assert_eq!(out[0].pts(), Some(3), "two samples move the timestamp");
    assert!(stage.finish().is_empty());
}

#[test]
fn a_decoder_delay_composes_with_the_container_trim() {
    // Container: discard 1 at the front, hold 4 at the end. Delay: 2.
    // Front becomes 3; the tail shrinks to 2.
    let mut stage = TrimStage::new(&parameters(1, Some(4))).with_decoder_delay(2);
    let first = stage.push(mono(vec![1, 2, 3, 4], 0));
    assert!(
        first.is_empty(),
        "the held tail withholds the one sample left"
    );
    let second = stage.push(mono(vec![1, 2, 3, 4], 4));
    let emitted: Vec<i16> = second.iter().flat_map(values).collect();
    assert_eq!(emitted, [4, 1, 2]);
    assert_eq!(
        stage.held_samples(),
        2,
        "4 container samples minus the delay"
    );
    assert!(stage.finish().is_empty());
}

#[test]
fn a_delay_larger_than_the_first_frame_spans_frames() {
    let mut stage = TrimStage::new(&parameters(0, None)).with_decoder_delay(6);
    assert!(stage.push(mono(vec![1, 2, 3, 4], 0)).is_empty());
    let out = stage.push(mono(vec![5, 6, 7, 8], 7));
    assert_eq!(values(&out[0]), [7, 8]);
    assert_eq!(
        out[0].pts(),
        Some(10),
        "the second frame keeps its own 7 ticks plus 2 samples"
    );
}

#[test]
fn an_undeclared_delay_applies_the_container_values_alone() {
    let stage = TrimStage::new(&parameters(2, None));
    assert_eq!(stage.decoder_delay(), 0);
    let mut stage = stage;
    let out = stage.push(mono(vec![1, 2, 3, 4], 0));
    assert_eq!(values(&out[0]), [3, 4]);
    assert_eq!(stage.held_samples(), 0);
}

#[test]
fn a_pre_roll_is_not_a_trim() {
    let padded = parameters(0, None).with_pre_roll(8);
    let mut stage = TrimStage::new(&padded);
    let out = stage.push(mono(vec![1, 2, 3, 4], 0));
    assert_eq!(
        values(&out[0]),
        [1, 2, 3, 4],
        "a decoder requirement is never discarded as padding"
    );
}

#[test]
fn a_delay_larger_than_the_container_tail_holds_nothing() {
    let mut stage = TrimStage::new(&parameters(0, Some(2))).with_decoder_delay(5);
    let out = stage.push(mono(vec![1, 2, 3, 4, 5, 6, 7, 8], 0));
    assert_eq!(values(&out[0]), [6, 7, 8]);
    assert_eq!(
        stage.held_samples(),
        0,
        "the container's tail is smaller than the delay, so it shrinks to zero"
    );
}
