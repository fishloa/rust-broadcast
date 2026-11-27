//! `Decoder::reconfigure`: the default refusal, the wrapper's forward, and the
//! pending-output rule.

use std::collections::VecDeque;
use std::num::NonZeroU32;

use broadcast_codec::{
    AudioFrame, AudioParameters, AudioSamples, ChannelLayout, CodecError, CodecId, CodecParameters,
    Decoder, Frame, Packet, SampleLayout, Timescale,
};

const RATE: u32 = 48_000;
const OTHER_RATE: u32 = 24_000;

fn rate(value: u32) -> NonZeroU32 {
    NonZeroU32::new(value).expect("a non-zero rate")
}

fn parameters(value: u32, initial: u32) -> CodecParameters {
    CodecParameters::new(CodecId::Pcm, Timescale::from_nonzero(rate(value)))
        .with_audio(AudioParameters::new(rate(value), ChannelLayout::Mono))
        .with_initial_padding(initial)
        .expect("audio parameters take padding")
}

/// A decoder that adopts new parameters unless it still holds a frame.
struct Adopting {
    parameters: CodecParameters,
    ready: VecDeque<Frame>,
}

impl Decoder for Adopting {
    fn parameters(&self) -> &CodecParameters {
        &self.parameters
    }

    fn send(&mut self, _packet: Packet<'_>) -> Result<(), CodecError> {
        let frame = AudioFrame::new(
            self.parameters
                .audio()
                .ok_or_else(|| CodecError::failed("audio parameters"))?
                .sample_rate(),
            ChannelLayout::Mono,
            SampleLayout::Interleaved,
            AudioSamples::S16(vec![1, 2, 3, 4]),
        )
        .ok_or(CodecError::failed("frame"))?;
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

    fn reconfigure(&mut self, parameters: CodecParameters) -> Result<(), CodecError> {
        if !self.ready.is_empty() {
            return Err(CodecError::Full);
        }
        self.parameters = parameters;
        Ok(())
    }
}

/// A decoder that keeps the defaults: it cannot reconfigure.
struct Fixed {
    parameters: CodecParameters,
}

impl Decoder for Fixed {
    fn parameters(&self) -> &CodecParameters {
        &self.parameters
    }

    fn send(&mut self, _packet: Packet<'_>) -> Result<(), CodecError> {
        Ok(())
    }

    fn receive(&mut self) -> Result<Option<Frame>, CodecError> {
        Ok(None)
    }

    fn finish(&mut self) -> Result<(), CodecError> {
        Ok(())
    }

    fn reset(&mut self) {}
}

#[test]
fn the_default_reconfiguration_is_a_typed_refusal() {
    let mut decoder = Fixed {
        parameters: parameters(RATE, 0),
    };
    let err = decoder
        .reconfigure(parameters(OTHER_RATE, 0))
        .expect_err("the default keeps its parameters");
    assert!(
        matches!(err, CodecError::CannotReconfigure { .. }),
        "the refusal is typed, got {err:?}"
    );
}

#[test]
fn a_decoder_with_a_frame_ready_refuses_reconfiguration() {
    let mut decoder = Adopting {
        parameters: parameters(RATE, 0),
        ready: VecDeque::new(),
    };
    decoder.send(Packet::borrowed(&[])).expect("send");
    let err = decoder
        .reconfigure(parameters(OTHER_RATE, 0))
        .expect_err("a frame is pending");
    assert_eq!(err, CodecError::Full, "the pending frame is not lost");
    let _ = decoder.receive().expect("drain").expect("the frame");
    decoder
        .reconfigure(parameters(OTHER_RATE, 0))
        .expect("nothing is pending now");
    assert_eq!(
        decoder.parameters().audio().expect("audio").sample_rate(),
        rate(OTHER_RATE)
    );
}
