//! `TranscodeStage` and the two conversions across the codec seam, driven by a
//! fake codec pair.
//!
//! The fake pair is the point: it holds three inputs before emitting, emits two
//! outputs for one batch, reports parameters different from the source's, and
//! counts what it was given and what it emitted. No codec is decoded or
//! encoded, and this workspace gains no codec dependency from it.
//!
//! The pipeline tests drive real fixtures through demux → `TranscodeStage` →
//! mux, for the MPEG-2 TS muxer, the CMAF writer and `LlHlsSegmenter`, and
//! assert the output describes the *encoder's* parameters (re-parsed with this
//! crate's own demuxers), that timestamps stay monotonic, that every input
//! given to the decoder is accounted for at `finish`, and that the untouched
//! track is byte-identical to a run without the stage.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::num::NonZeroU32;
use std::rc::Rc;
use std::time::Duration;

use broadcast_codec::{
    AudioFrame, AudioParameters, AudioSamples, ChannelLayout, CodecError, CodecId, CodecParameters,
    Decoder, Encoder, Frame, Packet, SampleLayout, Timescale,
};
use broadcast_common::stage::{Stage, Timestamp};
use broadcast_core::Serialize;
use test_harness::collect::{CollectMode, Collected, media_from_mp4, media_from_ts};
use transmux::ll_hls::{LlHlsSegmenter, LlHlsStageOutput};
use transmux::mux_stage::{MuxInput, MuxOutput};
use transmux::{
    CmafMuxStage, CodecConfig, DecoderPreRoll, FragmentCut, Sample, SampleTrim, TrackId, TrackSpec,
    TranscodeStage, TsMuxStage, codec_parameters_from_track, track_spec_from_codec_parameters,
};

/// Inputs the fake decoder holds before it emits a frame.
const HOLD: usize = 3;
/// Samples per fake decoded frame.
const FRAME_SAMPLES: usize = 1024;
/// The sample rate the fake decoder reports.
const SOURCE_RATE: u32 = 44_100;
/// The sample rate the fake encoder reports: a different stream entirely.
const OUTPUT_RATE: u32 = 48_000;
/// The media timescale the fake encoder reports, in ticks per second.
const OUTPUT_TIMESCALE: u32 = 1_000;
/// `AudioSpecificConfig` for AAC LC, 48 kHz, mono (ISO/IEC 14496-3 §1.6.2.1).
const OUTPUT_ASC: [u8; 2] = [0x11, 0x88];

fn timescale(value: u32) -> Timescale {
    Timescale::try_new(value).expect("a non-zero timescale")
}

fn rate(value: u32) -> NonZeroU32 {
    NonZeroU32::new(value).expect("a non-zero rate")
}

fn output_parameters() -> CodecParameters {
    CodecParameters::new(CodecId::Aac, timescale(OUTPUT_TIMESCALE))
        .with_extradata(OUTPUT_ASC.to_vec())
        .with_audio(AudioParameters::new(rate(OUTPUT_RATE), ChannelLayout::Mono))
}

/// What the fake pair was given and what it emitted.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct Counts {
    packets_in: usize,
    frames_out: usize,
    frames_in: usize,
    packets_out: usize,
    discontinuities: usize,
}

/// A decoder that holds [`HOLD`] packets, then emits one frame — two for the
/// first batch — and drains its remainder as one last frame at `finish`, so
/// `frames = ceil(packets / HOLD) + 1` for every input it is given.
struct FakeDecoder {
    parameters: CodecParameters,
    queued: VecDeque<Packet<'static>>,
    frames: VecDeque<Frame>,
    batches: usize,
    finished: bool,
    counts: Rc<RefCell<Counts>>,
}

impl FakeDecoder {
    fn new(parameters: CodecParameters, counts: Rc<RefCell<Counts>>) -> Self {
        Self {
            parameters,
            queued: VecDeque::new(),
            frames: VecDeque::new(),
            batches: 0,
            finished: false,
            counts,
        }
    }

    fn frame(&self, count: usize) -> Frame {
        let frame = AudioFrame::new(
            rate(SOURCE_RATE),
            ChannelLayout::Mono,
            SampleLayout::Interleaved,
            AudioSamples::S16(vec![count as i16; FRAME_SAMPLES]),
        )
        .expect("a mono frame");
        Frame::Audio(frame.with_pts(self.counts.borrow().frames_out as i64))
    }
}

impl Decoder for FakeDecoder {
    fn parameters(&self) -> &CodecParameters {
        &self.parameters
    }

    fn send(&mut self, packet: Packet<'_>) -> Result<(), CodecError> {
        self.counts.borrow_mut().packets_in += 1;
        self.queued.push_back(packet.into_owned());
        Ok(())
    }

    fn receive(&mut self) -> Result<Option<Frame>, CodecError> {
        if let Some(frame) = self.frames.pop_front() {
            return Ok(Some(frame));
        }
        let ready = self.queued.len() >= HOLD || (self.finished && !self.queued.is_empty());
        if !ready {
            return Ok(None);
        }
        let batch = self.queued.len().min(HOLD);
        for _ in 0..batch {
            self.queued.pop_front();
        }
        // Two frames for the first batch, one for each after it.
        let frames = if self.batches == 0 { 2 } else { 1 };
        self.batches += 1;
        for _ in 0..frames {
            let frame = self.frame(batch);
            self.frames.push_back(frame);
            self.counts.borrow_mut().frames_out += 1;
        }
        Ok(self.frames.pop_front())
    }

    fn finish(&mut self) -> Result<(), CodecError> {
        self.finished = true;
        Ok(())
    }

    fn reset(&mut self) {
        self.queued.clear();
        self.frames.clear();
    }

    fn discontinuity(&mut self) {
        self.counts.borrow_mut().discontinuities += 1;
    }
}

/// An encoder that reports its parameters only after [`HOLD`] frames, emits one
/// packet per frame (two for the first), and counts everything.
struct FakeEncoder {
    pending: VecDeque<Packet<'static>>,
    parameters: Option<CodecParameters>,
    reported: Option<CodecParameters>,
    clock: i64,
    fail_send: bool,
    counts: Rc<RefCell<Counts>>,
}

impl FakeEncoder {
    fn new(counts: Rc<RefCell<Counts>>) -> Self {
        Self {
            pending: VecDeque::new(),
            parameters: None,
            reported: None,
            clock: 0,
            fail_send: false,
            counts,
        }
    }

    fn reporting(parameters: CodecParameters, counts: Rc<RefCell<Counts>>) -> Self {
        Self {
            reported: Some(parameters),
            ..Self::new(counts)
        }
    }
}

impl Encoder for FakeEncoder {
    fn send(&mut self, frame: Frame) -> Result<(), CodecError> {
        if self.fail_send {
            return Err(CodecError::failed("the fake encoder refuses this stream"));
        }
        let Frame::Audio(frame) = frame else {
            return Err(CodecError::unsupported("fake encoder takes audio only"));
        };
        let mut counts = self.counts.borrow_mut();
        counts.frames_in += 1;
        let first = counts.frames_in == 1;
        let packets = if first { 2 } else { 1 };
        let samples = u32::try_from(frame.samples().len()).unwrap_or(u32::MAX);
        for _ in 0..packets {
            let packet = Packet::owned(vec![self.pending.len() as u8; 4])
                .with_dts(self.clock)
                .with_pts(self.clock)
                .with_key(true);
            self.pending.push_back(packet);
            self.clock = self.clock.saturating_add(
                i64::from(samples) * i64::from(OUTPUT_TIMESCALE) / i64::from(OUTPUT_RATE),
            );
        }
        if counts.frames_in >= HOLD {
            self.parameters = Some(self.reported.clone().unwrap_or_else(output_parameters));
        }
        Ok(())
    }

    fn receive(&mut self) -> Result<Option<Packet<'static>>, CodecError> {
        match self.pending.pop_front() {
            Some(packet) => {
                self.counts.borrow_mut().packets_out += 1;
                Ok(Some(packet))
            }
            None => Ok(None),
        }
    }

    fn finish(&mut self) -> Result<(), CodecError> {
        Ok(())
    }

    fn parameters(&self) -> Option<&CodecParameters> {
        self.parameters.as_ref()
    }
}

/// A stage for `spec`, with a decoder built for exactly the source's
/// parameters, plus the counters both fakes write.
fn stage_for(spec: &TrackSpec) -> (TranscodeStage, Rc<RefCell<Counts>>) {
    let counts = Rc::new(RefCell::new(Counts::default()));
    let decoder = FakeDecoder::new(
        codec_parameters_from_track(spec).expect("the source track converts to the seam"),
        Rc::clone(&counts),
    );
    let encoder = FakeEncoder::new(Rc::clone(&counts));
    let stage = TranscodeStage::new(spec.track_id, Box::new(decoder), Box::new(encoder));
    (stage, counts)
}

/// The fixture's tracks, demuxed: `(specs, samples in demux order)`.
fn fixture() -> (Vec<TrackSpec>, Vec<(TrackId, Sample)>) {
    let ts = test_fixtures::read("ts/h264-aac.ts");
    let media = media_from_ts(&ts).expect("demux the fixture");
    let specs: Vec<TrackSpec> = media.tracks.iter().map(|t| t.spec.clone()).collect();
    let mut samples = Vec::new();
    for track in &media.tracks {
        for sample in &track.samples {
            samples.push((track.spec.track_id, sample.clone()));
        }
    }
    (specs, samples)
}

fn audio_spec(specs: &[TrackSpec]) -> TrackSpec {
    specs
        .iter()
        .find(|spec| matches!(spec.config, CodecConfig::Aac { .. }))
        .cloned()
        .expect("the fixture carries an AAC track")
}

fn video_spec(specs: &[TrackSpec]) -> TrackSpec {
    specs
        .iter()
        .find(|spec| matches!(spec.config, CodecConfig::Avc { .. }))
        .cloned()
        .expect("the fixture carries an AVC track")
}

/// Drive the stage over every input, returning its output in order.
fn drive(stage: &mut TranscodeStage, inputs: Vec<MuxInput>) -> transmux::Result<Vec<MuxInput>> {
    let mut outputs = Vec::new();
    for input in inputs {
        stage.feed(input, Timestamp::ZERO)?;
        while let Some(output) = stage.poll() {
            outputs.push(output);
        }
    }
    stage.finish()?;
    while let Some(output) = stage.poll() {
        outputs.push(output);
    }
    Ok(outputs)
}

fn stage_inputs(specs: &[TrackSpec], samples: &[(TrackId, Sample)]) -> Vec<MuxInput> {
    let mut inputs: Vec<MuxInput> = specs
        .iter()
        .map(|spec| MuxInput::Track(Box::new(spec.clone())))
        .collect();
    inputs.extend(samples.iter().map(|(track, sample)| MuxInput::Sample {
        track: *track,
        sample: sample.clone(),
    }));
    inputs
}

/// Track announcements first, then everything else in order — what the
/// `MuxChain` seam releases, and what a segmenter built up front needs.
fn tracks_then_rest(outputs: Vec<MuxInput>) -> (Vec<MuxInput>, Vec<MuxInput>) {
    let mut tracks = Vec::new();
    let mut rest = Vec::new();
    for output in outputs {
        match output {
            MuxInput::Track(spec) => tracks.push(MuxInput::Track(spec)),
            other => rest.push(other),
        }
    }
    (tracks, rest)
}

fn sample_times(samples: &[Sample]) -> Vec<i64> {
    samples
        .iter()
        .map(|sample| sample.timing.and_then(|timing| timing.dts()).unwrap_or(0))
        .collect()
}

fn monotonic(times: &[i64]) -> bool {
    times.windows(2).all(|pair| pair[0] <= pair[1])
}

fn track_of(media: &Collected, id: TrackId) -> &test_harness::collect::CollectedTrack {
    media
        .tracks
        .iter()
        .find(|track| track.spec.track_id == id)
        .expect("the output carries the track")
}

fn audio_of(media: &Collected) -> &test_harness::collect::CollectedTrack {
    media
        .tracks
        .iter()
        .find(|track| matches!(track.spec.config, CodecConfig::Aac { .. }))
        .expect("the output carries an AAC track")
}

// ── The conversions ─────────────────────────────────────────────────────────

#[test]
fn the_track_conversions_round_trip_the_fixtures_codecs() {
    let (specs, _) = fixture();
    for spec in [video_spec(&specs), audio_spec(&specs)] {
        let parameters =
            codec_parameters_from_track(&spec).expect("a fixture track converts to the seam");
        let back = track_spec_from_codec_parameters(spec.track_id, &parameters)
            .expect("the seam's parameters convert back");
        assert_eq!(back.track_id, spec.track_id);
        assert_eq!(back.timescale, spec.timescale, "the time base survives");
        assert_eq!(back.config, spec.config, "the codec configuration survives");
        assert_eq!(
            back.display, spec.display,
            "the display geometry is re-derived"
        );
        assert!(
            back.source_pid.is_none() && back.program_number.is_none(),
            "container provenance is the container's, not the seam's"
        );
    }
}

#[test]
fn a_discontinuity_reaches_the_tracks_decoder_and_is_forwarded() {
    let (specs, _) = fixture();
    let spec = audio_spec(&specs);
    let (mut stage, counts) = stage_for(&spec);
    let mut outputs = Vec::new();
    stage
        .feed(
            MuxInput::Discontinuity {
                track: Some(spec.track_id),
            },
            Timestamp::ZERO,
        )
        .expect("a discontinuity");
    while let Some(output) = stage.poll() {
        outputs.push(output);
    }
    assert_eq!(
        counts.borrow().discontinuities,
        1,
        "the track's decoder learns the timeline broke"
    );
    assert!(
        matches!(outputs.as_slice(), [MuxInput::Discontinuity { track: Some(track) }] if *track == spec.track_id),
        "the mark still reaches downstream"
    );
}

#[test]
fn a_discontinuity_for_another_track_leaves_this_decoder_alone() {
    let (specs, _) = fixture();
    let spec = audio_spec(&specs);
    let (mut stage, counts) = stage_for(&spec);
    stage
        .feed(
            MuxInput::Discontinuity {
                track: Some(spec.track_id.next()),
            },
            Timestamp::ZERO,
        )
        .expect("a discontinuity");
    assert_eq!(counts.borrow().discontinuities, 0);
}

#[test]
fn the_track_conversions_round_trip_the_decoder_pre_roll() {
    let (specs, _) = fixture();
    let pre_roll = DecoderPreRoll::from_samples(3840).expect("a non-zero pre-roll");
    let spec = audio_spec(&specs).with_pre_roll(Some(pre_roll));
    let parameters =
        codec_parameters_from_track(&spec).expect("the source track converts to the seam");
    assert_eq!(
        parameters.pre_roll(),
        Some(pre_roll.samples()),
        "the pre-roll crosses to the seam as samples"
    );
    let back = track_spec_from_codec_parameters(spec.track_id, &parameters)
        .expect("the seam's parameters convert back");
    assert_eq!(
        back.pre_roll,
        Some(pre_roll),
        "and back as a typed pre-roll"
    );
}

#[test]
fn a_track_the_seam_cannot_name_is_a_typed_error() {
    let (specs, _) = fixture();
    let mut spec = video_spec(&specs);
    spec.config = CodecConfig::Vp8 {
        width: 640,
        height: 360,
    };
    let err = codec_parameters_from_track(&spec).expect_err("VP8 has no codec identity");
    assert!(
        matches!(err, transmux::Error::TranscodeUnrepresentable { .. }),
        "a WebM-native codec is a typed refusal, got {err:?}"
    );
}

#[test]
fn an_mpeg_audio_layer_cannot_be_announced_from_the_seam() {
    let parameters = CodecParameters::new(CodecId::MpegAudio, timescale(48_000))
        .with_audio(AudioParameters::new(rate(48_000), ChannelLayout::Stereo));
    let err = track_spec_from_codec_parameters(TrackId::new(1), &parameters)
        .expect_err("the seam does not carry the MPEG audio layer");
    assert!(
        matches!(err, transmux::Error::TranscodeUnrepresentable { .. }),
        "the missing fact is named, got {err:?}"
    );
}

// ── The stage on its own ────────────────────────────────────────────────────

#[test]
fn the_announcement_waits_for_the_encoder_and_precedes_the_first_sample() {
    let (specs, samples) = fixture();
    let spec = audio_spec(&specs);
    let (mut stage, counts) = stage_for(&spec);
    let audio: Vec<Sample> = samples
        .iter()
        .filter(|(track, _)| *track == spec.track_id)
        .map(|(_, sample)| sample.clone())
        .take(HOLD * 2)
        .collect();

    let mut outputs = Vec::new();
    stage
        .feed(MuxInput::Track(Box::new(spec.clone())), Timestamp::ZERO)
        .expect("a matching source announcement");
    for (index, sample) in audio.iter().enumerate() {
        stage
            .feed(
                MuxInput::Sample {
                    track: spec.track_id,
                    sample: sample.clone(),
                },
                Timestamp::ZERO,
            )
            .expect("a sample");
        while let Some(output) = stage.poll() {
            outputs.push(output);
        }
        if index + 1 < HOLD {
            assert!(
                outputs.is_empty(),
                "the stage announces nothing before the encoder reports its parameters"
            );
        }
    }
    let announced = outputs
        .iter()
        .position(|output| matches!(output, MuxInput::Track(_)))
        .expect("the track is announced once the encoder knows its output");
    assert_eq!(announced, 0, "the announcement precedes every sample");
    let MuxInput::Track(announced) = &outputs[announced] else {
        unreachable!()
    };
    assert_eq!(
        announced.timescale,
        timescale(OUTPUT_TIMESCALE),
        "the announced track carries the encoder's time base"
    );
    match &announced.config {
        CodecConfig::Aac {
            channel_count,
            sample_rate,
            ..
        } => {
            assert_eq!(*sample_rate, OUTPUT_RATE, "the encoder's rate");
            assert_eq!(*channel_count, 1, "the encoder's channel count");
        }
        other => panic!("the encoder reports AAC, got {other:?}"),
    }
    assert_eq!(
        counts.borrow().packets_in,
        HOLD * 2,
        "the decoder was given exactly what was fed"
    );
}

#[test]
fn a_source_update_that_restates_the_decoder_is_absorbed_and_a_change_is_refused() {
    let (specs, samples) = fixture();
    let spec = audio_spec(&specs);
    let (mut stage, _) = stage_for(&spec);
    stage
        .feed(MuxInput::Track(Box::new(spec.clone())), Timestamp::ZERO)
        .expect("a matching re-announcement");
    let sample = samples
        .iter()
        .find(|(track, _)| *track == spec.track_id)
        .map(|(_, sample)| sample.clone())
        .expect("an audio sample");
    stage
        .feed(
            MuxInput::Sample {
                track: spec.track_id,
                sample,
            },
            Timestamp::ZERO,
        )
        .expect("a sample");

    let CodecConfig::Aac { esds, .. } = &spec.config else {
        unreachable!()
    };
    let mut changed = spec.clone();
    changed.config = CodecConfig::Aac {
        esds: esds.clone(),
        channel_count: 5,
        sample_rate: 32_000,
        sample_size: 16,
    };
    let err = stage
        .feed(MuxInput::Track(Box::new(changed)), Timestamp::ZERO)
        .expect_err("a changed source cannot be decoded by the same decoder");
    assert!(
        matches!(
            err,
            transmux::Error::TranscodeSourceChanged { track } if track == spec.track_id
        ),
        "the refusal names the track, got {err:?}"
    );
}

#[test]
fn a_track_removed_mid_stream_flushes_the_decoder_then_the_encoder() {
    let (specs, samples) = fixture();
    let spec = audio_spec(&specs);
    let (mut stage, counts) = stage_for(&spec);
    let audio: Vec<Sample> = samples
        .iter()
        .filter(|(track, _)| *track == spec.track_id)
        .map(|(_, sample)| sample.clone())
        .collect();
    let fed = audio.len();

    let mut inputs = vec![MuxInput::Track(Box::new(spec.clone()))];
    inputs.extend(audio.into_iter().map(|sample| MuxInput::Sample {
        track: spec.track_id,
        sample,
    }));
    inputs.push(MuxInput::TrackRemoved {
        track: spec.track_id,
    });
    let outputs = drive(&mut stage, inputs).expect("the removal flushes both directions");

    let counts = *counts.borrow();
    assert_eq!(counts.packets_in, fed, "every input reached the decoder");
    assert_eq!(
        counts.frames_out,
        fed.div_ceil(HOLD) + 1,
        "the decoder's remainder is flushed as a frame at finish: nothing held is lost"
    );
    assert_eq!(
        counts.frames_in, counts.frames_out,
        "every decoded frame reached the encoder"
    );
    assert_eq!(
        counts.packets_out,
        fed.div_ceil(HOLD) + 2,
        "the encoder emitted one packet per frame plus its first extra"
    );
    let samples_out = outputs
        .iter()
        .filter(|output| matches!(output, MuxInput::Sample { .. }))
        .count();
    assert_eq!(
        samples_out, counts.packets_out,
        "every coded packet left the stage"
    );
    assert!(
        outputs
            .iter()
            .any(|output| matches!(output, MuxInput::TrackRemoved { .. })),
        "the removal is forwarded downstream"
    );
    let times: Vec<i64> = outputs
        .iter()
        .filter_map(|output| match output {
            MuxInput::Sample { sample, .. } => sample.timing.and_then(|timing| timing.dts()),
            _ => None,
        })
        .collect();
    assert!(monotonic(&times), "the flush keeps the encoder's order");
}

#[test]
fn an_encoder_error_carries_the_track() {
    let (specs, samples) = fixture();
    let spec = audio_spec(&specs);
    let counts = Rc::new(RefCell::new(Counts::default()));
    let decoder = FakeDecoder::new(
        codec_parameters_from_track(&spec).expect("the source track converts to the seam"),
        Rc::clone(&counts),
    );
    let mut encoder = FakeEncoder::new(Rc::clone(&counts));
    encoder.fail_send = true;
    let mut stage = TranscodeStage::new(spec.track_id, Box::new(decoder), Box::new(encoder));
    stage
        .feed(MuxInput::Track(Box::new(spec.clone())), Timestamp::ZERO)
        .expect("a matching source announcement");
    let sample = samples
        .iter()
        .find(|(track, _)| *track == spec.track_id)
        .map(|(_, sample)| sample.clone())
        .expect("an audio sample");
    let mut err = None;
    for index in 0..HOLD {
        let _ = index;
        if let Err(e) = stage.feed(
            MuxInput::Sample {
                track: spec.track_id,
                sample: sample.clone(),
            },
            Timestamp::ZERO,
        ) {
            err = Some(e);
            break;
        }
    }
    let err = err.expect("the encoder refuses every frame");
    let transmux::Error::Transcode { track, source } = err else {
        panic!("a codec error is `Transcode`, got {err:?}");
    };
    assert_eq!(track, spec.track_id, "the error names the track");
    assert!(
        matches!(source, CodecError::Failed { .. }),
        "the seam's own error is carried whole, got {source:?}"
    );
}

#[test]
fn a_parameter_set_a_track_spec_cannot_express_is_a_typed_error() {
    let (specs, samples) = fixture();
    let spec = audio_spec(&specs);
    let counts = Rc::new(RefCell::new(Counts::default()));
    let decoder = FakeDecoder::new(
        codec_parameters_from_track(&spec).expect("the source track converts to the seam"),
        Rc::clone(&counts),
    );
    let parameters = CodecParameters::new(CodecId::Pcm, timescale(OUTPUT_TIMESCALE))
        .with_audio(AudioParameters::new(rate(OUTPUT_RATE), ChannelLayout::Mono));
    let mut stage = TranscodeStage::new(
        spec.track_id,
        Box::new(decoder),
        Box::new(FakeEncoder::reporting(parameters, Rc::clone(&counts))),
    );
    stage
        .feed(MuxInput::Track(Box::new(spec.clone())), Timestamp::ZERO)
        .expect("a matching source announcement");
    let audio: Vec<Sample> = samples
        .iter()
        .filter(|(track, _)| *track == spec.track_id)
        .map(|(_, sample)| sample.clone())
        .take(HOLD * 4)
        .collect();
    let mut err = None;
    for sample in audio {
        if let Err(e) = stage.feed(
            MuxInput::Sample {
                track: spec.track_id,
                sample,
            },
            Timestamp::ZERO,
        ) {
            err = Some(e);
            break;
        }
    }
    let err = err.expect("PCM is not a track description this crate can announce");
    assert!(
        matches!(err, transmux::Error::TranscodeUnrepresentable { .. }),
        "an unexpressible parameter set is a typed error, got {err:?}"
    );
}

#[test]
fn the_pre_announcement_window_is_the_callers_cap() {
    let (specs, samples) = fixture();
    let spec = audio_spec(&specs);
    let counts = Rc::new(RefCell::new(Counts::default()));
    let decoder = FakeDecoder::new(
        codec_parameters_from_track(&spec).expect("the source track converts to the seam"),
        Rc::clone(&counts),
    );
    let mut stage = TranscodeStage::new(
        spec.track_id,
        Box::new(decoder),
        Box::new(FakeEncoder::new(Rc::clone(&counts))),
    )
    .pending_limit(1);
    stage
        .feed(MuxInput::Track(Box::new(spec.clone())), Timestamp::ZERO)
        .expect("a matching source announcement");
    let audio: Vec<Sample> = samples
        .iter()
        .filter(|(track, _)| *track == spec.track_id)
        .map(|(_, sample)| sample.clone())
        .collect();
    let mut err = None;
    for sample in audio {
        if let Err(e) = stage.feed(
            MuxInput::Sample {
                track: spec.track_id,
                sample,
            },
            Timestamp::ZERO,
        ) {
            err = Some(e);
            break;
        }
    }
    let err = err.expect("the cap is hit before the encoder reports parameters");
    assert!(
        matches!(err, transmux::Error::TranscodePendingLimit { limit: 1 }),
        "the cap is the caller's, got {err:?}"
    );
}

// ── The pipeline, over real fixtures ───────────────────────────────────────

/// Mux one track set and its samples to CMAF, returning the contiguous bytes.
fn mux_cmaf(inputs: Vec<MuxInput>, anchor: TrackId) -> Vec<u8> {
    let (tracks, rest) = tracks_then_rest(inputs);
    let mut mux = CmafMuxStage::new(
        timescale(1_000),
        FragmentCut::AnchorKeyframe {
            track: anchor,
            target: Duration::from_secs(1),
        },
        1,
    );
    let mut bytes = Vec::new();
    for track in tracks {
        mux.feed(track, Timestamp::ZERO).expect("announce");
    }
    for input in rest {
        mux.feed(input, Timestamp::ZERO).expect("mux");
        drain_cmaf(&mut mux, &mut bytes);
    }
    mux.finish().expect("finish");
    drain_cmaf(&mut mux, &mut bytes);
    bytes
}

fn drain_cmaf(mux: &mut CmafMuxStage, bytes: &mut Vec<u8>) {
    while let Some(output) = mux.poll() {
        match output {
            MuxOutput::Init(data) | MuxOutput::Bytes(data) => bytes.extend_from_slice(&data),
            MuxOutput::Segment(segment) => bytes.extend_from_slice(&segment.bytes),
            _ => {}
        }
    }
}

/// Mux one track set and its samples to MPEG-2 TS, returning the bytes.
fn mux_ts(inputs: Vec<MuxInput>) -> Vec<u8> {
    let (tracks, rest) = tracks_then_rest(inputs);
    let mut mux = TsMuxStage::new(None);
    let mut bytes = Vec::new();
    for track in tracks {
        mux.feed(track, Timestamp::ZERO).expect("announce");
    }
    for input in rest {
        mux.feed(input, Timestamp::ZERO).expect("mux");
        drain_ts(&mut mux, &mut bytes);
    }
    mux.finish().expect("finish");
    drain_ts(&mut mux, &mut bytes);
    bytes
}

fn drain_ts(mux: &mut TsMuxStage, bytes: &mut Vec<u8>) {
    while let Some(output) = mux.poll() {
        if let MuxOutput::Bytes(data) = output {
            bytes.extend_from_slice(&data);
        }
    }
}

#[test]
fn cmaf_out_describes_the_encoder_and_loses_nothing() {
    let (specs, samples) = fixture();
    let audio = audio_spec(&specs);
    let video = video_spec(&specs);

    let plain = media_from_mp4(
        &mux_cmaf(stage_inputs(&specs, &samples), video.track_id),
        CollectMode::Seekable,
    )
    .expect("re-demux the plain CMAF output");

    let (mut stage, counts) = stage_for(&audio);
    let outputs = drive(&mut stage, stage_inputs(&specs, &samples)).expect("transcode");
    let transformed = media_from_mp4(&mux_cmaf(outputs, video.track_id), CollectMode::Seekable)
        .expect("re-demux the transcoded CMAF output");

    let track = audio_of(&transformed);
    match &track.spec.config {
        CodecConfig::Aac {
            channel_count,
            sample_rate,
            ..
        } => {
            assert_eq!(
                *sample_rate, OUTPUT_RATE,
                "the init segment carries the encoder's rate"
            );
            assert_eq!(
                *channel_count, 1,
                "the init segment carries the encoder's channels"
            );
        }
        other => panic!("the audio track is AAC, got {other:?}"),
    }
    assert_eq!(
        track.spec.timescale,
        timescale(OUTPUT_TIMESCALE),
        "the init segment carries the encoder's time base"
    );
    let counts = *counts.borrow();
    let audio_in = samples
        .iter()
        .filter(|(id, _)| *id == audio.track_id)
        .count();
    assert_eq!(
        counts.frames_out,
        audio_in.div_ceil(HOLD) + 1,
        "the decoder's remainder is flushed as a frame at finish: nothing is lost"
    );
    assert_eq!(
        counts.frames_in, counts.frames_out,
        "every decoded frame reached the encoder"
    );
    assert_eq!(
        track.samples.len(),
        audio_in.div_ceil(HOLD) + 2,
        "every coded packet the encoder emitted is in the output"
    );
    assert_is_monotonic(&track.samples);

    let plain_video = track_of(&plain, video.track_id);
    let transformed_video = track_of(&transformed, video.track_id);
    assert_eq!(
        plain_video.samples.len(),
        transformed_video.samples.len(),
        "the untouched track keeps its sample count"
    );
    for (a, b) in plain_video
        .samples
        .iter()
        .zip(transformed_video.samples.iter())
    {
        assert_eq!(a.data, b.data, "the untouched track is byte-identical");
        assert_eq!(
            a.timing, b.timing,
            "the untouched track's timing is unchanged"
        );
    }
}

fn assert_is_monotonic(samples: &[Sample]) {
    let times = sample_times(samples);
    assert!(
        monotonic(&times),
        "timestamps stay monotonic across the stage: {times:?}"
    );
}

#[test]
fn ts_out_describes_the_encoder_and_loses_nothing() {
    let (specs, samples) = fixture();
    let audio = audio_spec(&specs);
    let video = video_spec(&specs);
    let plain = media_from_ts(&mux_ts(stage_inputs(&specs, &samples))).expect("plain re-demux");
    let (mut stage, counts) = stage_for(&audio);
    let outputs = drive(&mut stage, stage_inputs(&specs, &samples)).expect("transcode");
    let back = media_from_ts(&mux_ts(outputs)).expect("re-demux the TS output");

    let track = audio_of(&back);
    match &track.spec.config {
        CodecConfig::Aac {
            channel_count,
            sample_rate,
            ..
        } => {
            assert_eq!(
                *sample_rate, OUTPUT_RATE,
                "the ADTS the muxer derives is the encoder's"
            );
            assert_eq!(*channel_count, 1);
        }
        other => panic!("the audio track is AAC, got {other:?}"),
    }
    let counts = *counts.borrow();
    let audio_in = samples
        .iter()
        .filter(|(id, _)| *id == audio.track_id)
        .count();
    assert_eq!(
        counts.frames_out,
        audio_in.div_ceil(HOLD) + 1,
        "the decoder's remainder is flushed as a frame at finish"
    );
    assert_eq!(
        track.samples.len(),
        audio_in.div_ceil(HOLD) + 2,
        "every coded packet the encoder emitted is in the TS output"
    );
    assert_is_monotonic(&track.samples);

    let plain_video = track_of(&plain, video.track_id);
    let transformed_video = track_of(&back, video.track_id);
    assert_eq!(
        plain_video.samples.len(),
        transformed_video.samples.len(),
        "the untouched track keeps its sample count"
    );
    for (a, b) in plain_video
        .samples
        .iter()
        .zip(transformed_video.samples.iter())
    {
        assert_eq!(a.data, b.data, "the untouched track is byte-identical");
        assert_eq!(
            a.timing, b.timing,
            "the untouched track's timing is unchanged"
        );
    }
}

#[test]
fn ll_hls_out_takes_the_encoder_track_and_loses_nothing() {
    let (specs, samples) = fixture();
    let audio = audio_spec(&specs);
    let video = video_spec(&specs);
    let (mut stage, counts) = stage_for(&audio);
    let outputs = drive(&mut stage, stage_inputs(&specs, &samples)).expect("transcode");
    let (tracks, rest) = tracks_then_rest(outputs);

    let untouched: Vec<Sample> = rest
        .iter()
        .filter_map(|input| match input {
            MuxInput::Sample { track, sample } if *track == video.track_id => Some(sample.clone()),
            _ => None,
        })
        .collect();
    let source: Vec<Sample> = samples
        .iter()
        .filter(|(track, _)| *track == video.track_id)
        .map(|(_, sample)| sample.clone())
        .collect();
    assert_eq!(
        untouched.len(),
        source.len(),
        "the untouched track is whole"
    );
    for (a, b) in untouched.iter().zip(source.iter()) {
        assert_eq!(a.data, b.data, "the untouched track is byte-identical");
        assert_eq!(
            a.timing, b.timing,
            "the untouched track's timing is unchanged"
        );
    }

    let announced: Vec<TrackSpec> = tracks
        .iter()
        .map(|input| match input {
            MuxInput::Track(spec) => (**spec).clone(),
            _ => unreachable!(),
        })
        .collect();
    assert!(
        announced
            .iter()
            .any(|spec| spec.timescale == timescale(OUTPUT_TIMESCALE)),
        "the segmenter is built from the encoder's track description"
    );
    let mut segmenter = LlHlsSegmenter::with_part_target(
        announced,
        timescale(1_000),
        Duration::from_secs(1),
        Duration::from_millis(250),
    )
    .expect("the announced set builds a segmenter");

    let mut produced = 0usize;
    for input in rest {
        segmenter
            .feed(input, Timestamp::ZERO)
            .expect("the segmenter takes the stage's samples");
        while let Some(output) = segmenter.poll() {
            if matches!(
                output,
                LlHlsStageOutput::Part(_) | LlHlsStageOutput::Segment(_)
            ) {
                produced += 1;
            }
        }
    }
    segmenter.finish().expect("finish");
    while segmenter.poll().is_some() {
        produced += 1;
    }
    assert!(
        produced > 0,
        "the LL-HLS segmenter cuts the transcoded track"
    );
    let counts = *counts.borrow();
    assert_eq!(
        counts.packets_out,
        counts.frames_in + 1,
        "the segmenter was fed every packet the encoder emitted"
    );
}

// ── Reconfiguration ─────────────────────────────────────────────────────────

/// A decoder that adopts new parameters unless a decoded frame is still
/// waiting to be taken.
struct AdoptingDecoder {
    parameters: CodecParameters,
    ready: VecDeque<Frame>,
    adopted: Rc<RefCell<usize>>,
    rate: NonZeroU32,
    counts: Rc<RefCell<Counts>>,
}

impl AdoptingDecoder {
    fn new(
        parameters: CodecParameters,
        adopted: Rc<RefCell<usize>>,
        counts: Rc<RefCell<Counts>>,
    ) -> Self {
        let rate = parameters
            .audio()
            .map(|audio| audio.sample_rate())
            .unwrap_or_else(|| rate(SOURCE_RATE));
        Self {
            parameters,
            ready: VecDeque::new(),
            adopted,
            rate,
            counts,
        }
    }
}

impl Decoder for AdoptingDecoder {
    fn parameters(&self) -> &CodecParameters {
        &self.parameters
    }

    fn send(&mut self, _packet: Packet<'_>) -> Result<(), CodecError> {
        self.counts.borrow_mut().packets_in += 1;
        let frame = AudioFrame::new(
            self.rate,
            ChannelLayout::Mono,
            SampleLayout::Interleaved,
            AudioSamples::S16(vec![1; FRAME_SAMPLES]),
        )
        .ok_or_else(|| CodecError::failed("a fake frame"))?;
        self.ready.push_back(Frame::Audio(
            frame.with_pts(self.counts.borrow().frames_out as i64),
        ));
        self.counts.borrow_mut().frames_out += 1;
        Ok(())
    }

    fn receive(&mut self) -> Result<Option<Frame>, CodecError> {
        Ok(self.ready.pop_front())
    }

    fn finish(&mut self) -> Result<(), CodecError> {
        Ok(())
    }

    fn reset(&mut self) {
        self.ready.clear();
    }

    fn reconfigure(&mut self, parameters: CodecParameters) -> Result<(), CodecError> {
        if !self.ready.is_empty() {
            return Err(CodecError::Full);
        }
        self.rate = parameters
            .audio()
            .map(|audio| audio.sample_rate())
            .unwrap_or(self.rate);
        self.parameters = parameters;
        *self.adopted.borrow_mut() += 1;
        Ok(())
    }
}

/// An encoder that reports `parameters` only after `report_after` packets and
/// emits `packets` in order, one per `receive`.
struct ScriptedEncoder {
    parameters: Option<CodecParameters>,
    packets: VecDeque<Packet<'static>>,
    report_after: usize,
    seen: usize,
}

impl ScriptedEncoder {
    fn new(parameters: CodecParameters, packets: VecDeque<Packet<'static>>) -> Self {
        Self {
            parameters: Some(parameters),
            packets,
            report_after: 1,
            seen: 0,
        }
    }

    fn reporting_late(parameters: CodecParameters, packets: VecDeque<Packet<'static>>) -> Self {
        Self {
            report_after: 2,
            ..Self::new(parameters, packets)
        }
    }
}

impl Encoder for ScriptedEncoder {
    fn send(&mut self, _frame: Frame) -> Result<(), CodecError> {
        Ok(())
    }

    fn receive(&mut self) -> Result<Option<Packet<'static>>, CodecError> {
        let packet = self.packets.pop_front();
        if packet.is_some() {
            self.seen += 1;
        }
        Ok(packet)
    }

    fn finish(&mut self) -> Result<(), CodecError> {
        Ok(())
    }

    fn parameters(&self) -> Option<&CodecParameters> {
        if self.seen >= self.report_after {
            self.parameters.as_ref()
        } else {
            None
        }
    }
}

/// A source spec whose audio rate is `rate` and channel count is `channels`.
fn changed_audio_spec(spec: &TrackSpec, sample_rate: u32, channels: u16) -> TrackSpec {
    let CodecConfig::Aac { esds, .. } = &spec.config else {
        panic!("the fixture's other track is not AAC");
    };
    let mut changed = spec.clone();
    changed.config = CodecConfig::Aac {
        esds: esds.clone(),
        channel_count: channels,
        sample_rate,
        sample_size: 16,
    };
    changed
}

fn audio_samples(spec: &TrackSpec, samples: &[(TrackId, Sample)], take: usize) -> Vec<Sample> {
    samples
        .iter()
        .filter(|(track, _)| *track == spec.track_id)
        .map(|(_, sample)| sample.clone())
        .take(take)
        .collect()
}

#[test]
fn a_source_change_is_drained_reconfigured_and_counted() {
    let (specs, samples) = fixture();
    let spec = audio_spec(&specs);
    let adopted = Rc::new(RefCell::new(0));
    let counts = Rc::new(RefCell::new(Counts::default()));
    let decoder = AdoptingDecoder::new(
        codec_parameters_from_track(&spec).expect("the source converts"),
        Rc::clone(&adopted),
        Rc::clone(&counts),
    );
    let mut stage = TranscodeStage::new(
        spec.track_id,
        Box::new(decoder),
        Box::new(FakeEncoder::new(Rc::clone(&counts))),
    );
    let first = audio_samples(&spec, &samples, 3);
    let changed = changed_audio_spec(&spec, 32_000, 5);
    let second = audio_samples(&spec, &samples, 5)[3..].to_vec();

    stage
        .feed(MuxInput::Track(Box::new(spec.clone())), Timestamp::ZERO)
        .expect("the source announcement");
    for sample in &first {
        stage
            .feed(
                MuxInput::Sample {
                    track: spec.track_id,
                    sample: sample.clone(),
                },
                Timestamp::ZERO,
            )
            .expect("a sample before the change");
    }
    stage
        .feed(MuxInput::Track(Box::new(changed)), Timestamp::ZERO)
        .expect("the decoder adopts the new parameters");
    for sample in &second {
        stage
            .feed(
                MuxInput::Sample {
                    track: spec.track_id,
                    sample: sample.clone(),
                },
                Timestamp::ZERO,
            )
            .expect("a sample after the change");
    }
    stage.finish().expect("finish");

    assert_eq!(
        *adopted.borrow(),
        1,
        "the decoder was reconfigured once, in place"
    );
    let counts = *counts.borrow();
    assert_eq!(
        counts.packets_in,
        first.len() + second.len(),
        "every packet reached the decoder across the change"
    );
    assert_eq!(
        counts.frames_in, counts.frames_out,
        "every decoded frame reached the encoder"
    );
    let mut times = Vec::new();
    while let Some(output) = stage.poll() {
        if let MuxInput::Sample { sample, .. } = output {
            times.push(sample.timing.and_then(|timing| timing.dts()).unwrap_or(0));
        }
    }
    assert!(monotonic(&times), "the change keeps the encoder's order");
}

#[test]
fn a_refusing_decoder_is_rebuilt_through_the_factory_and_loses_nothing() {
    let (specs, samples) = fixture();
    let spec = audio_spec(&specs);
    let counts = Rc::new(RefCell::new(Counts::default()));
    let builds = Rc::new(RefCell::new(0));
    let decoder = FakeDecoder::new(
        codec_parameters_from_track(&spec).expect("the source converts"),
        Rc::clone(&counts),
    );
    let factory_counts = Rc::clone(&counts);
    let factory_builds = Rc::clone(&builds);
    let factory = Box::new(move |parameters: &CodecParameters| {
        *factory_builds.borrow_mut() += 1;
        Ok(Box::new(AdoptingDecoder::new(
            parameters.clone(),
            Rc::new(RefCell::new(0)),
            Rc::clone(&factory_counts),
        )) as Box<dyn Decoder>)
    });
    let mut stage = TranscodeStage::new(
        spec.track_id,
        Box::new(decoder),
        Box::new(FakeEncoder::new(Rc::clone(&counts))),
    )
    .factory(factory);
    let audio = audio_samples(&spec, &samples, 5);
    let changed = changed_audio_spec(&spec, 32_000, 5);

    stage
        .feed(MuxInput::Track(Box::new(spec.clone())), Timestamp::ZERO)
        .expect("the source announcement");
    for sample in &audio[..3] {
        stage
            .feed(
                MuxInput::Sample {
                    track: spec.track_id,
                    sample: sample.clone(),
                },
                Timestamp::ZERO,
            )
            .expect("a sample before the change");
    }
    stage
        .feed(MuxInput::Track(Box::new(changed)), Timestamp::ZERO)
        .expect("the factory builds a fresh decoder");
    for sample in &audio[3..] {
        stage
            .feed(
                MuxInput::Sample {
                    track: spec.track_id,
                    sample: sample.clone(),
                },
                Timestamp::ZERO,
            )
            .expect("a sample after the change");
    }
    stage.finish().expect("finish");

    assert_eq!(*builds.borrow(), 1, "one fresh decoder was built");
    assert_eq!(
        counts.borrow().packets_in,
        audio.len(),
        "no input was lost across the rebuild"
    );
}

// ── In-band configuration records ───────────────────────────────────────────

/// The AAC `AudioSpecificConfig` a track's `esds` carries.
fn asc_of(spec: &TrackSpec) -> Vec<u8> {
    let CodecConfig::Aac { esds, .. } = &spec.config else {
        panic!("not an AAC track");
    };
    esds.decoder_specific_info_data()
        .map(<[u8]>::to_vec)
        .expect("the fixture's esds carries an AudioSpecificConfig")
}

/// An ADTS-wrapped access unit stating the fixture's own AAC configuration.
fn adts_unit(first: &Sample, sfi: u8, channels: u8) -> Vec<u8> {
    let header = broadcast_bitstream::build_adts_header(1, sfi, channels, first.data.len() as u16);
    let mut unit = header.to_vec();
    unit.extend_from_slice(&first.data);
    unit
}

#[test]
fn an_in_band_aac_record_is_derived_from_the_first_access_unit() {
    let (specs, samples) = fixture();
    let spec = audio_spec(&specs);
    let expected = asc_of(&spec);
    let first = audio_samples(&spec, &samples, 1)
        .into_iter()
        .next()
        .expect("an audio sample");
    // 44.1 kHz (index 4), mono (configuration 1), AAC LC (profile 1):
    // ffprobe reports `profile=LC sample_rate=44100 channels=1` for the
    // fixture.
    let first_unit = adts_unit(&first, 4, 1);
    // A second unit stating a different stream: deriving from it would
    // announce 48 kHz and must not.
    let second_unit = adts_unit(&first, 3, 1);

    let parameters = CodecParameters::new(CodecId::Aac, timescale(OUTPUT_TIMESCALE))
        .with_in_band_configuration()
        .with_audio(AudioParameters::new(rate(OUTPUT_RATE), ChannelLayout::Mono));
    let encoder = ScriptedEncoder::reporting_late(
        parameters,
        VecDeque::from([
            Packet::owned(first_unit).with_dts(0).with_pts(0),
            Packet::owned(second_unit).with_dts(1).with_pts(1),
        ]),
    );
    let counts = Rc::new(RefCell::new(Counts::default()));
    let decoder = AdoptingDecoder::new(
        codec_parameters_from_track(&spec).expect("the source converts"),
        Rc::new(RefCell::new(0)),
        Rc::clone(&counts),
    );
    let mut stage = TranscodeStage::new(spec.track_id, Box::new(decoder), Box::new(encoder));
    stage
        .feed(MuxInput::Track(Box::new(spec.clone())), Timestamp::ZERO)
        .expect("the source announcement");
    let sample = first.clone();
    stage
        .feed(
            MuxInput::Sample {
                track: spec.track_id,
                sample,
            },
            Timestamp::ZERO,
        )
        .expect("one input makes the encoder emit both units");
    stage.finish().expect("finish");

    let mut announced = None;
    let mut packets = 0usize;
    while let Some(output) = stage.poll() {
        match output {
            MuxInput::Track(spec) => announced = Some(*spec),
            MuxInput::Sample { .. } => packets += 1,
            _ => {}
        }
    }
    let announced = announced.expect("the stage announced the output track");
    assert_eq!(
        asc_of(&announced),
        expected,
        "the record derived from the first access unit is the one the demuxer reports"
    );
    assert_eq!(
        packets, 2,
        "the first access unit is not consumed by deriving"
    );
}

#[test]
fn an_in_band_h264_record_is_derived_from_the_first_access_unit() {
    let (specs, samples) = fixture();
    let video = video_spec(&specs);
    let CodecConfig::Avc { config, .. } = &video.config else {
        panic!("not an AVC track");
    };
    let record = &config.config;
    let sps = record
        .sps
        .first()
        .expect("the fixture carries an SPS")
        .0
        .clone();
    let pps = record
        .pps
        .first()
        .expect("the fixture carries a PPS")
        .0
        .clone();
    let first_unit =
        broadcast_bitstream::annexb::length_prefix(4, [sps.as_slice(), pps.as_slice()]);
    let parameters = CodecParameters::new(CodecId::H264, timescale(OUTPUT_TIMESCALE))
        .with_in_band_configuration()
        .with_video(broadcast_codec::VideoParameters::new(
            std::num::NonZeroU32::new(320).expect("a width"),
            std::num::NonZeroU32::new(240).expect("a height"),
        ));
    let encoder = ScriptedEncoder::new(
        parameters,
        VecDeque::from([Packet::owned(first_unit).with_dts(0).with_pts(0)]),
    );
    let counts = Rc::new(RefCell::new(Counts::default()));
    let decoder = AdoptingDecoder::new(
        codec_parameters_from_track(&video).expect("the source converts"),
        Rc::new(RefCell::new(0)),
        Rc::clone(&counts),
    );
    let mut stage = TranscodeStage::new(video.track_id, Box::new(decoder), Box::new(encoder));
    stage
        .feed(MuxInput::Track(Box::new(video.clone())), Timestamp::ZERO)
        .expect("the source announcement");
    let sample = samples
        .iter()
        .find(|(track, _)| *track == video.track_id)
        .map(|(_, sample)| sample.clone())
        .expect("a video sample");
    stage
        .feed(
            MuxInput::Sample {
                track: video.track_id,
                sample,
            },
            Timestamp::ZERO,
        )
        .expect("the encoder emits its first access unit");
    stage.finish().expect("finish");

    let mut announced = None;
    while let Some(output) = stage.poll() {
        if let MuxInput::Track(spec) = output {
            announced = Some(*spec);
        }
    }
    let announced = announced.expect("the stage announced the output track");
    let CodecConfig::Avc {
        config: derived, ..
    } = &announced.config
    else {
        panic!("the announced track is AVC");
    };
    assert_eq!(
        derived.config.try_to_bytes().expect("serialize"),
        record.try_to_bytes().expect("serialize"),
        "the derived avcC is the record the demuxer reports for the same stream"
    );
}

#[test]
fn two_real_fixtures_with_different_parameters_concatenate_through_the_stage() {
    let (specs, samples) = fixture();
    let first = audio_spec(&specs);
    let mut other = test_harness::collect::media_from_mp4(
        &test_fixtures::read("mp4/boxes/loudness.m4a"),
        test_harness::collect::CollectMode::Seekable,
    )
    .expect("demux the second fixture")
    .tracks
    .into_iter()
    .map(|track| track.spec)
    .find(|spec| spec.config.audio_sample_rate().is_some())
    .expect("the second fixture carries AAC");
    other.track_id = first.track_id;
    assert_ne!(
        codec_parameters_from_track(&first).expect("converts"),
        codec_parameters_from_track(&other).expect("converts"),
        "the two fixtures really describe different streams"
    );

    let adopted = Rc::new(RefCell::new(0));
    let counts = Rc::new(RefCell::new(Counts::default()));
    let decoder = AdoptingDecoder::new(
        codec_parameters_from_track(&first).expect("the source converts"),
        Rc::clone(&adopted),
        Rc::clone(&counts),
    );
    let mut stage = TranscodeStage::new(
        first.track_id,
        Box::new(decoder),
        Box::new(FakeEncoder::new(Rc::clone(&counts))),
    );
    let before = audio_samples(&first, &samples, 3);

    stage
        .feed(MuxInput::Track(Box::new(first.clone())), Timestamp::ZERO)
        .expect("the first source");
    for sample in &before {
        stage
            .feed(
                MuxInput::Sample {
                    track: first.track_id,
                    sample: sample.clone(),
                },
                Timestamp::ZERO,
            )
            .expect("a sample of the first stream");
    }
    stage
        .feed(MuxInput::Track(Box::new(other.clone())), Timestamp::ZERO)
        .expect("the second stream is adopted");
    for _ in 0..3 {
        let sample = Sample::timed_raw(std::vec![0u8; 8], 0, 0, None);
        stage
            .feed(
                MuxInput::Sample {
                    track: first.track_id,
                    sample,
                },
                Timestamp::ZERO,
            )
            .expect("a sample of the second stream");
    }
    stage.finish().expect("finish");

    assert_eq!(*adopted.borrow(), 1, "one in-place reconfiguration");
    assert_eq!(
        counts.borrow().packets_in,
        before.len() + 3,
        "both streams' samples reached the decoder"
    );
}

#[test]
fn a_samples_own_trim_crosses_the_seam_both_ways() {
    let (specs, samples) = fixture();
    let spec = audio_spec(&specs);
    let source = codec_parameters_from_track(&spec).expect("the source converts");
    let encoder = ScriptedEncoder::new(
        output_parameters(),
        VecDeque::from([Packet::owned(vec![1u8, 2, 3, 4])
            .with_dts(0)
            .with_pts(0)
            .with_key(true)
            .with_trim_start(5)
            .with_trim_end(7)]),
    );
    let decoder = AdoptingDecoder::new(
        source,
        Rc::new(RefCell::new(0)),
        Rc::new(RefCell::new(Counts::default())),
    );
    let mut stage = TranscodeStage::new(spec.track_id, Box::new(decoder), Box::new(encoder));
    stage
        .feed(MuxInput::Track(Box::new(spec.clone())), Timestamp::ZERO)
        .expect("the source announcement");
    let mut sample = samples
        .iter()
        .find(|(track, _)| *track == spec.track_id)
        .map(|(_, sample)| sample.clone())
        .expect("an audio sample");
    sample = sample.with_trim(SampleTrim::new(3, 2));
    stage
        .feed(
            MuxInput::Sample {
                track: spec.track_id,
                sample,
            },
            Timestamp::ZERO,
        )
        .expect("a sample with its own trim");
    stage.finish().expect("finish");
    let mut out = None;
    while let Some(output) = stage.poll() {
        if let MuxInput::Sample { sample, .. } = output {
            out = Some(sample);
        }
    }
    let out = out.expect("one output sample");
    assert_eq!(
        out.trim,
        SampleTrim::new(5, 7),
        "the encoder's per-packet trim is carried onto the output sample"
    );
}
