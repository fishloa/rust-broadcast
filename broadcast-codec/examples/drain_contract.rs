//! Show the drain contract: a decoder may hold every packet and only emit
//! frames after `finish`.

use std::collections::VecDeque;
use std::error::Error;
use std::num::NonZeroU32;

use broadcast_codec::{
    AudioFrame, AudioParameters, AudioSamples, ChannelLayout, CodecError, CodecId, CodecParameters,
    Decoder, Frame, Packet, SampleLayout,
};
use broadcast_core::timescale::Timescale;

/// Samples per second of the fake stream.
const SAMPLE_RATE: u32 = 48_000;
/// Packets the fake decoder holds before `finish`.
const HELD_PACKETS: usize = 3;

/// A decoder that holds every packet until `finish`.
struct DelayingDecoder {
    params: CodecParameters,
    sample_rate: NonZeroU32,
    held: VecDeque<Frame>,
    finished: bool,
}

impl DelayingDecoder {
    fn new(sample_rate: NonZeroU32) -> Self {
        let params = CodecParameters::new(CodecId::Pcm, Timescale::from_nonzero(sample_rate))
            .with_audio(AudioParameters::new(sample_rate, ChannelLayout::Mono));
        Self {
            params,
            sample_rate,
            held: VecDeque::new(),
            finished: false,
        }
    }
}

impl Decoder for DelayingDecoder {
    fn parameters(&self) -> &CodecParameters {
        &self.params
    }

    fn send(&mut self, packet: Packet<'_>) -> Result<(), CodecError> {
        let sample = packet.data().first().copied().unwrap_or(0);
        let frame = AudioFrame::new(
            self.sample_rate,
            ChannelLayout::Mono,
            SampleLayout::Interleaved,
            AudioSamples::S16(vec![i16::from(sample)]),
        )
        .ok_or(CodecError::refused(
            "sample count not divisible by channels",
        ))?;
        self.held.push_back(Frame::Audio(frame));
        Ok(())
    }

    fn receive(&mut self) -> Result<Option<Frame>, CodecError> {
        if !self.finished {
            return Ok(None);
        }
        Ok(self.held.pop_front())
    }

    fn finish(&mut self) -> Result<(), CodecError> {
        self.finished = true;
        Ok(())
    }

    fn reset(&mut self) {
        self.held.clear();
        self.finished = false;
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let sample_rate = NonZeroU32::new(SAMPLE_RATE).ok_or("sample rate must be non-zero")?;
    let mut decoder = DelayingDecoder::new(sample_rate);

    for _ in 0..HELD_PACKETS {
        decoder.send(Packet::borrowed(&[0]))?;
    }
    if decoder.receive()?.is_some() {
        return Err("decoder emitted before finish".into());
    }

    decoder.finish()?;
    let mut drained = 0;
    while decoder.receive()?.is_some() {
        drained += 1;
    }
    if drained != HELD_PACKETS {
        return Err(format!("drained {drained} frames, expected {HELD_PACKETS}").into());
    }
    println!("{drained} frames arrived after finish");
    Ok(())
}
