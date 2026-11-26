//! `Registry::trimmed_decoder`: the wrapper, the pass-through and the rule.

use std::collections::VecDeque;
use std::num::NonZeroU32;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use broadcast_codec::{
    AudioFrame, AudioParameters, AudioSamples, ChannelLayout, CodecDescriptor, CodecError, CodecId,
    CodecParameters, CodecProvider, Decoder, Direction, Encoder, EncoderConfig, Frame, Packet,
    ProviderInfo, Registry, SampleLayout, Timescale,
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
    trims: bool,
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
                .with_trims(self.trims),
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
}

impl Decoder for FakeDecoder {
    fn parameters(&self) -> &CodecParameters {
        &self.parameters
    }

    fn send(&mut self, packet: Packet<'_>) -> Result<(), CodecError> {
        let frame = AudioFrame::new(
            rate(),
            ChannelLayout::Mono,
            SampleLayout::Interleaved,
            AudioSamples::S16(vec![1, 2, 3, 4]),
        )
        .ok_or(CodecError::failed("fake sample count"))?
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
    }
}

fn registry(trims: bool) -> (Registry, Arc<AtomicUsize>) {
    let made = Arc::new(AtomicUsize::new(0));
    let provider = FakeProvider {
        trims,
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
    let (registry, made) = registry(false);
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
    let (registry, made) = registry(true);
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
    let (registry, _) = registry(false);
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
fn the_wrapper_applies_both_paddings() {
    let (registry, _) = registry(false);
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
    let (registry, _) = registry(false);
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
fn a_descriptor_states_whether_it_trims() {
    let descriptor = CodecDescriptor::new("fake", "pcm", CodecId::Pcm, Direction::Decode);
    assert!(!descriptor.applies_trim());
    assert!(descriptor.with_trims(true).applies_trim());
}
