//! `Registry::trimmed_decoder`: the wrapper, the pass-through and the rule.

use std::collections::VecDeque;
use std::num::NonZeroU32;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use broadcast_codec::{
    AudioFrame, AudioParameters, AudioSamples, ChannelLayout, CodecDescriptor, CodecError, CodecId,
    CodecParameters, CodecProvider, Decoder, DecoderDelay, DecoderTrim, Direction, Encoder,
    EncoderConfig, Frame, Packet, ProviderInfo, Registry, SampleLayout, Timescale,
};

const RATE: u32 = 48_000;

fn rate() -> NonZeroU32 {
    NonZeroU32::new(RATE).expect("non-zero")
}

fn parameters(initial: u32, trailing: Option<u32>) -> CodecParameters {
    let parameters = CodecParameters::new(CodecId::Pcm, Timescale::from_nonzero(rate()))
        .with_audio(AudioParameters::new(rate(), ChannelLayout::Mono))
        .with_initial_padding(initial)
        .expect("audio parameters take padding");
    match trailing {
        Some(samples) => parameters
            .with_trailing_padding(samples)
            .expect("audio parameters take padding"),
        None => parameters,
    }
}

struct FakeProvider {
    trim: DecoderTrim,
    made: Arc<AtomicUsize>,
}

impl CodecProvider for FakeProvider {
    fn name(&self) -> &str {
        "fake"
    }

    fn info(&self) -> ProviderInfo {
        ProviderInfo::new("fake", "0.0.0")
    }

    fn descriptors(&self) -> Vec<CodecDescriptor> {
        vec![
            CodecDescriptor::new("fake", "pcm", CodecId::Pcm, Direction::Decode)
                .with_trim(self.trim),
        ]
    }

    fn make_decoder(
        &self,
        _implementation: &str,
        parameters: &CodecParameters,
    ) -> Result<Box<dyn Decoder>, CodecError> {
        self.made.fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(FakeDecoder {
            parameters: parameters.clone(),
            ready: VecDeque::new(),
            next_pts: 0,
        }))
    }

    fn make_encoder(
        &self,
        _implementation: &str,
        _config: &EncoderConfig,
    ) -> Result<Box<dyn Encoder>, CodecError> {
        Err(CodecError::unsupported("no encoder"))
    }
}

struct FakeDecoder {
    parameters: CodecParameters,
    ready: VecDeque<Frame>,
    next_pts: i64,
}

impl Decoder for FakeDecoder {
    fn parameters(&self) -> &CodecParameters {
        &self.parameters
    }

    fn send(&mut self, packet: Packet<'_>) -> Result<(), CodecError> {
        let pts = self.next_pts;
        self.next_pts = self.next_pts.saturating_add(4);
        let frame = AudioFrame::new(
            rate(),
            ChannelLayout::Mono,
            SampleLayout::Interleaved,
            AudioSamples::S16(vec![1, 2, 3, 4]),
        )
        .ok_or(CodecError::failed("fake sample count"))?
        .with_pts(pts)
        .with_trim_start(packet.trim_start())
        .with_trim_end(packet.trim_end());
        self.ready.push_back(Frame::Audio(frame));
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
        self.next_pts = 0;
    }

    fn reconfigure(&mut self, parameters: CodecParameters) -> Result<(), CodecError> {
        if !self.ready.is_empty() {
            return Err(CodecError::Full);
        }
        self.parameters = parameters;
        Ok(())
    }

    fn output_delay(&self) -> Option<u32> {
        Some(2)
    }
}

fn registry(trim: DecoderTrim) -> (Registry, Arc<AtomicUsize>) {
    let made = Arc::new(AtomicUsize::new(0));
    let provider = FakeProvider {
        trim,
        made: Arc::clone(&made),
    };
    let registry = Registry::new()
        .with(Box::new(provider))
        .expect("the provider registers");
    (registry, made)
}

fn samples(frame: &Frame) -> Vec<i16> {
    let Frame::Audio(audio) = frame else {
        panic!("expected audio");
    };
    match audio.samples() {
        AudioSamples::S16(values) => values.clone(),
        other => panic!("expected S16, got {other:?}"),
    }
}

fn audio(frame: &Frame) -> &AudioFrame {
    let Frame::Audio(audio) = frame else {
        panic!("expected audio");
    };
    audio
}

#[test]
fn a_non_trimming_decoder_is_wrapped() {
    let (registry, made) = registry(unknown());
    let parameters = parameters(0, None);
    let mut decoder = registry.trimmed_decoder(&parameters).expect("a decoder");
    decoder
        .send(Packet::borrowed(&[]).with_trim_start(1))
        .expect("send");
    let frame = decoder.receive().expect("receive").expect("a frame");
    assert_eq!(samples(&frame), [2, 3, 4]);
    assert_eq!(audio(&frame).trim_start(), 0);
    assert_eq!(made.load(Ordering::SeqCst), 1);
}

#[test]
fn a_trimming_decoder_passes_through_untouched() {
    let (registry, made) = registry(DecoderTrim::Implementation);
    let parameters = parameters(0, None);
    let mut decoder = registry.trimmed_decoder(&parameters).expect("a decoder");
    decoder
        .send(Packet::borrowed(&[]).with_trim_start(1))
        .expect("send");
    let frame = decoder.receive().expect("receive").expect("a frame");
    assert_eq!(samples(&frame), [1, 2, 3, 4]);
    assert_eq!(audio(&frame).trim_start(), 1);
    assert_eq!(made.load(Ordering::SeqCst), 1);
}

#[test]
fn decoder_alone_stays_untrimmed() {
    let (registry, _) = registry(unknown());
    let parameters = parameters(0, None);
    let mut decoder = registry.decoder(&parameters).expect("a decoder");
    decoder
        .send(Packet::borrowed(&[]).with_trim_start(1))
        .expect("send");
    let frame = decoder.receive().expect("receive").expect("a frame");
    assert_eq!(samples(&frame), [1, 2, 3, 4]);
    assert_eq!(audio(&frame).trim_start(), 1);
}

#[test]
fn the_wrapper_forwards_reconfiguration_and_applies_the_new_padding() {
    let (registry, _) = registry(unknown());
    let source = parameters(0, None);
    let mut decoder = registry.trimmed_decoder(&source).expect("a decoder");
    decoder
        .reconfigure(parameters(2, None))
        .expect("the inner decoder adopts the new parameters");
    assert_eq!(
        decoder.parameters().initial_padding(),
        2,
        "the wrapper reports the inner decoder's new parameters"
    );
    decoder.send(Packet::borrowed(&[])).expect("send");
    let frame = decoder.receive().expect("receive").expect("a frame");
    assert_eq!(samples(&frame), [3, 4], "the new padding is applied");
}

#[test]
fn the_wrapper_applies_both_paddings() {
    let (registry, _) = registry(unknown());
    let parameters = parameters(1, Some(1));
    let mut decoder = registry.trimmed_decoder(&parameters).expect("a decoder");
    let mut decoded = Vec::new();
    for _ in 0..2 {
        decoder.send(Packet::borrowed(&[])).expect("send");
        while let Some(frame) = decoder.receive().expect("receive") {
            decoded.extend(samples(&frame));
        }
    }
    decoder.finish().expect("finish");
    while let Some(frame) = decoder.receive().expect("receive") {
        decoded.extend(samples(&frame));
    }
    assert_eq!(decoded, [2, 3, 4, 1, 2, 3]);
}

#[test]
fn reset_drops_the_held_tail_and_restarts() {
    let (registry, _) = registry(unknown());
    let parameters = parameters(0, Some(2));
    let mut decoder = registry.trimmed_decoder(&parameters).expect("a decoder");
    decoder.send(Packet::borrowed(&[])).expect("send");
    let frame = decoder.receive().expect("receive").expect("a frame");
    assert_eq!(samples(&frame), [1, 2]);
    decoder.reset();
    decoder.send(Packet::borrowed(&[])).expect("send");
    let frame = decoder.receive().expect("receive").expect("a frame");
    assert_eq!(samples(&frame), [1, 2]);
}

#[test]
fn a_descriptor_states_who_trims_and_its_delay() {
    let descriptor = CodecDescriptor::new("fake", "pcm", CodecId::Pcm, Direction::Decode);
    assert_eq!(
        descriptor.trim(),
        DecoderTrim::Caller(DecoderDelay::Unknown),
        "the default states neither a delay nor a trim"
    );
    assert_eq!(
        descriptor
            .with_trim(DecoderTrim::Caller(DecoderDelay::Declared(529)))
            .trim(),
        DecoderTrim::Caller(DecoderDelay::Declared(529))
    );
    assert!(
        CodecDescriptor::new("fake", "pcm", CodecId::Pcm, Direction::Decode)
            .with_trim(DecoderTrim::Implementation)
            .trim()
            .applies_trim()
    );
}

/// A descriptor that states neither a delay nor a trim.
fn unknown() -> DecoderTrim {
    DecoderTrim::Caller(DecoderDelay::Unknown)
}

fn declared(samples: u32) -> DecoderTrim {
    DecoderTrim::Caller(DecoderDelay::Declared(samples))
}

#[test]
fn a_declared_delay_is_discarded_before_the_container_padding_and_shortens_the_tail() {
    let (registry, _) = registry(declared(2));
    let parameters = parameters(1, Some(3));
    let mut decoder = registry.trimmed_decoder(&parameters).expect("a decoder");
    let mut decoded = Vec::new();
    for _ in 0..2 {
        decoder.send(Packet::borrowed(&[])).expect("send");
        while let Some(frame) = decoder.receive().expect("receive") {
            decoded.extend(samples(&frame));
        }
    }
    decoder.finish().expect("finish");
    assert_eq!(
        decoded,
        [4, 1, 2, 3],
        "three samples go at the front (one container plus a two-sample delay) and the tail shrinks from three to one"
    );
}

#[test]
fn a_per_stream_delay_comes_from_the_decoder_instance() {
    let (registry, _) = registry(DecoderTrim::Caller(DecoderDelay::PerStream));
    let parameters = parameters(0, None);
    let mut decoder = registry.trimmed_decoder(&parameters).expect("a decoder");
    assert_eq!(
        decoder.output_delay(),
        Some(2),
        "the wrapper reports the instance's delay"
    );
    decoder.send(Packet::borrowed(&[])).expect("send");
    let frame = decoder.receive().expect("receive").expect("a frame");
    assert_eq!(samples(&frame), [3, 4]);
}

#[test]
fn an_unknown_delay_applies_the_container_values_alone() {
    let (registry, _) = registry(unknown());
    let parameters = parameters(1, Some(1));
    let mut decoder = registry.trimmed_decoder(&parameters).expect("a decoder");
    assert_eq!(decoder.output_delay(), None);
    decoder.send(Packet::borrowed(&[])).expect("send");
    let frame = decoder.receive().expect("receive").expect("a frame");
    assert_eq!(samples(&frame), [2, 3]);
}

#[test]
fn packet_and_frame_trim_default_to_zero_and_keep_their_values() {
    let packet = Packet::borrowed(&[]);
    assert_eq!(packet.trim_start(), 0);
    assert_eq!(packet.trim_end(), 0);
    let packet = Packet::owned(Vec::new())
        .with_trim_start(2)
        .with_trim_end(3)
        .into_owned();
    assert_eq!(packet.trim_start(), 2);
    assert_eq!(packet.trim_end(), 3);

    let frame = AudioFrame::new(
        rate(),
        ChannelLayout::Mono,
        SampleLayout::Interleaved,
        AudioSamples::S16(vec![1, 2]),
    )
    .expect("whole frames");
    assert_eq!(frame.trim_start(), 0);
    assert_eq!(frame.trim_end(), 0);
    let frame = frame.with_trim_start(2).with_trim_end(3);
    assert_eq!(frame.trim_start(), 2);
    assert_eq!(frame.trim_end(), 3);
}

fn pre_roll(samples: u32) -> CodecParameters {
    parameters(0, None).with_pre_roll(samples)
}

#[test]
fn pre_roll_is_not_discarded_at_the_start_of_a_stream() {
    let (registry, _) = registry(unknown());
    let mut decoder = registry.trimmed_decoder(&pre_roll(4)).expect("a decoder");
    decoder.send(Packet::borrowed(&[])).expect("send");
    let frame = decoder.receive().expect("receive").expect("a frame");
    assert_eq!(samples(&frame), [1, 2, 3, 4]);
}

#[test]
fn pre_roll_drops_the_settling_output_after_a_discontinuity() {
    let (registry, _) = registry(unknown());
    let mut decoder = registry.trimmed_decoder(&pre_roll(4)).expect("a decoder");
    decoder.discontinuity();
    decoder.send(Packet::borrowed(&[])).expect("send");
    assert!(
        decoder.receive().expect("receive").is_none(),
        "the first pre-roll frame is not for output"
    );
    decoder.send(Packet::borrowed(&[])).expect("send");
    let frame = decoder.receive().expect("receive").expect("a frame");
    assert_eq!(samples(&frame), [1, 2, 3, 4]);
    assert_eq!(
        audio(&frame).pts(),
        Some(4),
        "the first sample after the pre-roll keeps the stream's timeline"
    );
}

#[test]
fn pre_roll_is_rearmed_by_reset_and_reconfigure() {
    let (registry, _) = registry(unknown());
    let mut decoder = registry.trimmed_decoder(&pre_roll(4)).expect("a decoder");
    decoder.send(Packet::borrowed(&[])).expect("send");
    assert!(decoder.receive().expect("receive").is_some());
    decoder.reset();
    decoder.send(Packet::borrowed(&[])).expect("send");
    assert!(
        decoder.receive().expect("receive").is_none(),
        "reset starts a cold decoder"
    );
    decoder.reconfigure(pre_roll(4)).expect("reconfigure");
    decoder.send(Packet::borrowed(&[])).expect("send");
    assert!(
        decoder.receive().expect("receive").is_none(),
        "a reconfiguration starts a cold decoder"
    );
}

#[test]
fn a_partial_pre_roll_frame_keeps_only_the_samples_past_the_requirement() {
    let (registry, _) = registry(unknown());
    let mut decoder = registry.trimmed_decoder(&pre_roll(2)).expect("a decoder");
    decoder.discontinuity();
    decoder.send(Packet::borrowed(&[])).expect("send");
    let frame = decoder.receive().expect("receive").expect("a frame");
    assert_eq!(samples(&frame), [3, 4]);
    assert_eq!(audio(&frame).pts(), Some(2));
}

#[test]
fn pre_roll_dropping_leaves_no_gap_in_the_timeline() {
    let (registry, _) = registry(unknown());
    let mut decoder = registry.trimmed_decoder(&pre_roll(8)).expect("a decoder");
    decoder.discontinuity();
    decoder.send(Packet::borrowed(&[])).expect("send");
    assert!(decoder.receive().expect("receive").is_none());
    decoder.send(Packet::borrowed(&[])).expect("send");
    assert!(decoder.receive().expect("receive").is_none());
    decoder.send(Packet::borrowed(&[])).expect("send");
    let frame = decoder.receive().expect("receive").expect("a frame");
    assert_eq!(samples(&frame), [1, 2, 3, 4]);
    assert_eq!(
        audio(&frame).pts(),
        Some(8),
        "frames cover samples 0..4, 4..8 and 8..12 with no gap"
    );
}
