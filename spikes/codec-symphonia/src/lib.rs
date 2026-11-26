//! Codec-seam spike: a symphonia AAC-LC decoder behind
//! [`broadcast_codec::Decoder`].
//!
//! The seam crate holds no codec. This adapter is the smallest real proof that
//! the traits can carry one: ADTS frames go in as [`Packet`]s, decoded audio
//! comes out as [`AudioFrame`]s, and no symphonia type appears in the seam's
//! API. Symphonia is MPL-2.0 (a different licence from this workspace), which
//! is why the spike lives in its own Cargo workspace outside the root
//! `Cargo.lock` rather than as a workspace member.
//!
//! ADTS framing is the spike's own small parser ([`adts`]) rather than
//! symphonia's reader: the seam's `Decoder` takes exactly one access unit per
//! `send`, so the file is split before it reaches the adapter.
//!
//! The adapter is also a [`CodecProvider`] ([`SymphoniaProvider`]): a
//! [`Registry`](broadcast_codec::Registry) built with it hands the decoder back
//! as a `Box<dyn Decoder>`, so the test that measures loudness reaches symphonia
//! without naming it.

pub mod adts;

use std::collections::VecDeque;
use std::num::NonZeroU32;

use broadcast_codec::{
    AudioFrame, AudioSamples, ChannelLayout, CodecDescriptor, CodecError, CodecId,
    CodecParameters, CodecProvider, Decoder, Direction, Encoder, EncoderConfig, Frame, Packet,
    Probe, ProviderInfo, SampleFormat, SampleLayout,
};
use symphonia::core::audio::{Channels, Position};
use symphonia::core::codecs::audio::well_known::CODEC_ID_AAC;
use symphonia::core::codecs::audio::{AudioCodecParameters, AudioDecoder, AudioDecoderOptions};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::packet::Packet as SymphoniaPacket;
use symphonia::core::units::{Duration, Timestamp};
use symphonia::default::codecs::AacDecoder;

use crate::adts::AdtsFrame;

/// Track id handed to symphonia; the adapter carries one stream.
const TRACK_ID: u32 = 0;
/// Samples a single AAC-LC access unit decodes to.
const AAC_FRAME_SAMPLES: u64 = 1024;
/// The 5.1 channel count.
const SURROUND_5_1_CHANNELS: u16 = 6;
/// The 7.1 channel count.
const SURROUND_7_1_CHANNELS: u16 = 8;
/// The provider's name.
const PROVIDER: &str = "symphonia";
/// The one implementation the provider offers.
const IMPLEMENTATION: &str = "aac";
/// The underlying library, as its manifest names it.
const LIBRARY: &str = "symphonia";
/// The library version the adapter is built against (symphonia's manifest;
/// the spike's `Cargo.lock` pins it).
const LIBRARY_VERSION: &str = "0.6.1";
/// The licence symphonia's manifest states.
const LIBRARY_LICENCE: &str = "MPL-2.0";

/// An AAC-LC decoder implementing the seam's [`Decoder`].
pub struct SymphoniaAacDecoder {
    params: CodecParameters,
    decoder: AacDecoder,
    sample_rate: NonZeroU32,
    channels: ChannelLayout,
    ready: VecDeque<Frame>,
    timestamp: i64,
}

impl SymphoniaAacDecoder {
    /// Build a decoder for `params`, which must describe an AAC stream with
    /// audio parameters.
    ///
    /// # Errors
    /// [`CodecError::Unsupported`] when the codec is not AAC, the audio
    /// parameters are absent, or symphonia refuses the parameters.
    pub fn new(params: CodecParameters) -> Result<Self, CodecError> {
        if params.codec() != CodecId::Aac {
            return Err(CodecError::unsupported("non-AAC codec"));
        }
        let audio = params
            .audio()
            .ok_or(CodecError::unsupported("AAC audio parameters"))?;
        let sample_rate = audio.sample_rate();
        let channels = audio.channels();

        let mut symphonia_params = AudioCodecParameters::new();
        symphonia_params
            .for_codec(CODEC_ID_AAC)
            .with_sample_rate(sample_rate.get())
            .with_channels(symphonia_channels(channels));
        let decoder = AacDecoder::try_new(&symphonia_params, &AudioDecoderOptions::default())
            .map_err(|_| CodecError::unsupported("AAC decoder parameters"))?;

        Ok(Self {
            params,
            decoder,
            sample_rate,
            channels,
            ready: VecDeque::new(),
            timestamp: 0,
        })
    }
}

impl Decoder for SymphoniaAacDecoder {
    fn parameters(&self) -> &CodecParameters {
        &self.params
    }

    fn send(&mut self, packet: Packet<'_>) -> Result<(), CodecError> {
        let frame = AdtsFrame::parse(packet.data()).ok_or(CodecError::refused("ADTS frame"))?;
        if frame.sample_rate() != self.sample_rate.get() {
            return Err(CodecError::refused("ADTS sample rate"));
        }
        if frame.channels() != Some(self.channels) {
            return Err(CodecError::refused("ADTS channel layout"));
        }

        let pts = packet.pts().unwrap_or(self.timestamp);
        let symphonia_packet = SymphoniaPacket::new(
            TRACK_ID,
            Timestamp::new(pts),
            Duration::new(AAC_FRAME_SAMPLES),
            frame.payload(),
        );
        let decoded = self
            .decoder
            .decode(&symphonia_packet)
            .map_err(symphonia_error)?;
        let decoded_frames = decoded.frames();
        let mut samples = vec![0.0f32; decoded_frames * self.channels.count()];
        decoded.copy_to_slice_interleaved(&mut samples[..]);

        let audio = AudioFrame::new(
            self.sample_rate,
            self.channels,
            SampleLayout::Interleaved,
            AudioSamples::F32(samples),
        )
        .ok_or(CodecError::failed("decoded sample count"))?
        .with_pts(pts);
        self.ready.push_back(Frame::Audio(audio));
        self.timestamp = pts.saturating_add(decoded_frames as i64);
        Ok(())
    }

    fn receive(&mut self) -> Result<Option<Frame>, CodecError> {
        Ok(self.ready.pop_front())
    }

    fn finish(&mut self) -> Result<(), CodecError> {
        let _ = self.decoder.finalize();
        Ok(())
    }

    fn reset(&mut self) {
        self.ready.clear();
        self.decoder.reset();
        self.timestamp = 0;
    }
}

/// Map a symphonia decoder failure onto the seam's one error type.
fn symphonia_error(error: SymphoniaError) -> CodecError {
    match error {
        SymphoniaError::DecodeError(_) => CodecError::refused("AAC access unit"),
        SymphoniaError::ResetRequired => CodecError::failed("AAC decoder requires reset"),
        _ => CodecError::failed("AAC decode"),
    }
}

/// The seam layout as symphonia's channel set.
fn symphonia_channels(channels: ChannelLayout) -> Channels {
    match channels {
        ChannelLayout::Mono => Channels::Positioned(Position::FRONT_CENTER),
        ChannelLayout::Stereo => Channels::Positioned(Position::FRONT_LEFT | Position::FRONT_RIGHT),
        ChannelLayout::Surround5_1 => Channels::Discrete(SURROUND_5_1_CHANNELS),
        ChannelLayout::Surround7_1 => Channels::Discrete(SURROUND_7_1_CHANNELS),
        ChannelLayout::Discrete(count) => Channels::Discrete(count.get()),
        _ => Channels::Discrete(channels.count() as u16),
    }
}

/// The spike's [`CodecProvider`]: one AAC-LC decoder.
pub struct SymphoniaProvider;

impl CodecProvider for SymphoniaProvider {
    fn name(&self) -> &str {
        PROVIDER
    }

    fn info(&self) -> ProviderInfo {
        ProviderInfo::new(PROVIDER, env!("CARGO_PKG_VERSION"))
            .with_library(format!("{LIBRARY} {LIBRARY_VERSION}"))
            .with_licence(LIBRARY_LICENCE)
    }

    fn descriptors(&self) -> Vec<CodecDescriptor> {
        vec![
            CodecDescriptor::new(PROVIDER, IMPLEMENTATION, CodecId::Aac, Direction::Decode)
                .with_sample_formats(vec![SampleFormat::F32]),
        ]
    }

    fn probe(
        &self,
        _implementation: &str,
        _direction: Direction,
        _codec: CodecId,
    ) -> Probe {
        Probe::available()
    }

    fn make_decoder(
        &self,
        implementation: &str,
        parameters: &CodecParameters,
    ) -> Result<Box<dyn Decoder>, CodecError> {
        if implementation != IMPLEMENTATION {
            return Err(CodecError::unsupported(format!(
                "{PROVIDER} implementation `{implementation}`"
            )));
        }
        Ok(Box::new(SymphoniaAacDecoder::new(parameters.clone())?))
    }

    fn make_encoder(
        &self,
        implementation: &str,
        _config: &EncoderConfig,
    ) -> Result<Box<dyn Encoder>, CodecError> {
        Err(CodecError::unsupported(format!(
            "{PROVIDER} implementation `{implementation}`"
        )))
    }
}
