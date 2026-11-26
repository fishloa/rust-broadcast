//! A tiny 16-bit PCM decoder/encoder pair used by the passthrough example.

use std::collections::VecDeque;
use std::num::NonZeroU32;

use broadcast_codec::{
    AudioFrame, AudioParameters, AudioSamples, ChannelLayout, CodecError, CodecId, CodecParameters,
    Decoder, Encoder, Frame, Packet, SampleLayout,
};
use broadcast_core::timescale::Timescale;

/// Bytes in one signed 16-bit little-endian sample.
const BYTES_PER_SAMPLE: usize = 2;

/// Interleaved signed 16-bit little-endian PCM.
pub struct PcmDecoder {
    params: CodecParameters,
    sample_rate: NonZeroU32,
    channels: ChannelLayout,
    ready: VecDeque<Frame>,
}

impl PcmDecoder {
    /// A decoder for `channels` at `sample_rate`.
    pub fn new(sample_rate: NonZeroU32, channels: ChannelLayout) -> Self {
        let params = CodecParameters::new(CodecId::Pcm, Timescale::from_nonzero(sample_rate))
            .with_audio(AudioParameters::new(sample_rate, channels));
        Self {
            params,
            sample_rate,
            channels,
            ready: VecDeque::new(),
        }
    }
}

impl Decoder for PcmDecoder {
    fn parameters(&self) -> &CodecParameters {
        &self.params
    }

    fn send(&mut self, packet: Packet<'_>) -> Result<(), CodecError> {
        if !packet.data().len().is_multiple_of(BYTES_PER_SAMPLE) {
            return Err(CodecError::refused("odd PCM byte count"));
        }
        let mut samples = Vec::with_capacity(packet.data().len() / BYTES_PER_SAMPLE);
        for chunk in packet.data().chunks_exact(BYTES_PER_SAMPLE) {
            let mut bytes = [0u8; BYTES_PER_SAMPLE];
            bytes.copy_from_slice(chunk);
            samples.push(i16::from_le_bytes(bytes));
        }
        let mut frame = AudioFrame::new(
            self.sample_rate,
            self.channels,
            SampleLayout::Interleaved,
            AudioSamples::S16(samples),
        )
        .ok_or(CodecError::refused(
            "sample count not divisible by channels",
        ))?;
        if let Some(pts) = packet.pts() {
            frame = frame.with_pts(pts);
        }
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

/// The inverse of [`PcmDecoder`].
pub struct PcmEncoder {
    params: Option<CodecParameters>,
    sample_rate: NonZeroU32,
    channels: ChannelLayout,
    ready: VecDeque<Packet<'static>>,
}

impl PcmEncoder {
    /// An encoder for `channels` at `sample_rate`.
    pub fn new(sample_rate: NonZeroU32, channels: ChannelLayout) -> Self {
        Self {
            params: None,
            sample_rate,
            channels,
            ready: VecDeque::new(),
        }
    }
}

impl Encoder for PcmEncoder {
    fn send(&mut self, frame: Frame) -> Result<(), CodecError> {
        let Frame::Audio(audio) = frame else {
            return Err(CodecError::refused("non-audio frame"));
        };
        let AudioSamples::S16(samples) = audio.samples() else {
            return Err(CodecError::refused("non-16-bit samples"));
        };
        let mut bytes = Vec::with_capacity(samples.len() * BYTES_PER_SAMPLE);
        for sample in samples {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        let mut packet = Packet::owned(bytes);
        if let Some(pts) = audio.pts() {
            packet = packet.with_pts(pts);
        }
        if self.params.is_none() {
            self.params = Some(
                CodecParameters::new(CodecId::Pcm, Timescale::from_nonzero(self.sample_rate))
                    .with_audio(AudioParameters::new(self.sample_rate, self.channels)),
            );
        }
        self.ready.push_back(packet);
        Ok(())
    }

    fn receive(&mut self) -> Result<Option<Packet<'static>>, CodecError> {
        Ok(self.ready.pop_front())
    }

    fn finish(&mut self) -> Result<(), CodecError> {
        Ok(())
    }

    fn parameters(&self) -> Option<&CodecParameters> {
        self.params.as_ref()
    }
}
