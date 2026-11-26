//! Codec-seam spike: OxideAV audio decoders behind
//! [`broadcast_codec::Decoder`].
//!
//! The seam crate holds no codec. This adapter is the second real proof that
//! its traits can carry one, after the symphonia spike: packets go in, decoded
//! audio comes out, and no OxideAV type appears in the seam's API.
//!
//! OxideAV is MIT throughout (the review in
//! `docs/broadcast-codec/OXIDEAV-REVIEW.md`), so unlike the symphonia spike
//! this adapter's licence is compatible with the workspace. It still lives in
//! its own Cargo workspace: it is a spike, and a codec dependency in the root
//! lock file would make the workspace depend on an unpublished adapter.
//!
//! The adapter wraps four decoders, each with a committed fixture:
//! `oxideav-aac` (AAC-LC/HE/HE v2), `oxideav-mp3` (Layer III),
//! `oxideav-opus` (SILK/CELT/hybrid) and `oxideav-basic` (linear PCM). Each is
//! obtained through [`OxideavProvider`], which a
//! [`Registry`](broadcast_codec::Registry) turns into a `Box<dyn Decoder>`, so
//! the tests reach OxideAV without naming it.

use std::num::NonZeroU16;
use std::num::NonZeroU32;

use broadcast_codec::{
    AudioFrame, AudioSamples, ChannelLayout, CodecDescriptor, CodecError, CodecId, CodecParameters,
    CodecProfile, CodecProvider, Decoder, Direction, Encoder, EncoderConfig, Frame, Packet, Probe,
    ProviderInfo, SampleFormat, SampleLayout,
};
use oxideav_core as oxideav;

/// The provider's name, in the preference text form.
const PROVIDER: &str = "oxideav";
/// The AAC implementation's name.
const IMPLEMENTATION_AAC: &str = "aac";
/// The MP3 implementation's name.
const IMPLEMENTATION_MP3: &str = "mp3";
/// The Opus implementation's name.
const IMPLEMENTATION_OPUS: &str = "opus";
/// The linear PCM implementation's name.
const IMPLEMENTATION_PCM: &str = "pcm";
/// The underlying library, as its manifests name it.
const LIBRARY: &str = "oxideav";
/// The licence every wrapped OxideAV crate's manifest states.
const LIBRARY_LICENCE: &str = "MIT";
/// The `oxideav-core` version the adapter is built against.
const CORE_VERSION: &str = "0.1.43";
/// The `oxideav-aac` version the adapter is built against.
const AAC_VERSION: &str = "0.1.7";
/// The `oxideav-mp3` version the adapter is built against.
const MP3_VERSION: &str = "0.1.4";
/// The `oxideav-opus` version the adapter is built against.
const OPUS_VERSION: &str = "0.0.15";
/// The `oxideav-basic` version the adapter is built against.
const BASIC_VERSION: &str = "0.0.12";
/// The OxideAV codec id for AAC.
const OXIDEAV_AAC: &str = "aac";
/// The OxideAV codec id for MPEG audio.
const OXIDEAV_MP3: &str = "mp3";
/// The OxideAV codec id for Opus.
const OXIDEAV_OPUS: &str = "opus";
/// The OxideAV codec id for signed 16-bit little-endian PCM.
const OXIDEAV_PCM_S16LE: &str = "pcm_s16le";
/// The AAC profiles the `oxideav-aac` README states its decoder handles.
const AAC_PROFILES: [CodecProfile; 3] = [
    CodecProfile::AacLc,
    CodecProfile::AacHe,
    CodecProfile::AacHeV2,
];
/// Channels in a mono stream.
const MONO_CHANNELS: usize = 1;
/// Channels in a stereo stream.
const STEREO_CHANNELS: usize = 2;
/// Channels in a 5.1 stream.
const SURROUND_5_1_CHANNELS: usize = 6;
/// Channels in a 7.1 stream.
const SURROUND_7_1_CHANNELS: usize = 8;
/// Bytes in one signed 16-bit sample.
const S16_BYTES: usize = 2;
/// Bytes in one signed 32-bit sample.
const S32_BYTES: usize = 4;
/// Bytes in one 32-bit float sample.
const F32_BYTES: usize = 4;
/// Bytes in one 64-bit float sample.
const F64_BYTES: usize = 8;
/// Bytes in one unsigned 8-bit sample.
const U8_BYTES: usize = 1;
/// The stream index an OxideAV packet carries; the seam has one stream.
const STREAM_INDEX: u32 = 0;

/// The adapter's [`CodecProvider`]: four OxideAV decoders.
pub struct OxideavProvider;

impl CodecProvider for OxideavProvider {
    fn name(&self) -> &str {
        PROVIDER
    }

    fn info(&self) -> ProviderInfo {
        ProviderInfo::new(PROVIDER, env!("CARGO_PKG_VERSION"))
            .with_library(format!(
                "{LIBRARY}-core {CORE_VERSION}; \
                 {LIBRARY}-aac {AAC_VERSION}; \
                 {LIBRARY}-mp3 {MP3_VERSION}; \
                 {LIBRARY}-opus {OPUS_VERSION}; \
                 {LIBRARY}-basic {BASIC_VERSION}"
            ))
            .with_licence(LIBRARY_LICENCE)
    }

    fn descriptors(&self) -> Vec<CodecDescriptor> {
        vec![
            aac_descriptor(),
            simple_descriptor(IMPLEMENTATION_MP3, CodecId::MpegAudio),
            simple_descriptor(IMPLEMENTATION_OPUS, CodecId::Opus),
            simple_descriptor(IMPLEMENTATION_PCM, CodecId::Pcm).with_lossless(true),
        ]
    }

    fn probe(&self, implementation: &str, _direction: Direction, _codec: CodecId) -> Probe {
        if matches!(
            implementation,
            IMPLEMENTATION_AAC | IMPLEMENTATION_MP3 | IMPLEMENTATION_OPUS | IMPLEMENTATION_PCM
        ) {
            Probe::available()
        } else {
            Probe::unavailable(format!("{PROVIDER} implementation `{implementation}`"))
        }
    }

    fn make_decoder(
        &self,
        implementation: &str,
        parameters: &CodecParameters,
    ) -> Result<Box<dyn Decoder>, CodecError> {
        let inner = match implementation {
            IMPLEMENTATION_AAC => {
                let translated = oxideav_parameters(parameters, CodecId::Aac, OXIDEAV_AAC)?;
                oxideav_aac::codec_decoder::make_decoder(&translated).map_err(map_error)?
            }
            IMPLEMENTATION_MP3 => {
                let translated = oxideav_parameters(parameters, CodecId::MpegAudio, OXIDEAV_MP3)?;
                oxideav_mp3::make_decoder(&translated).map_err(map_error)?
            }
            IMPLEMENTATION_OPUS => {
                let translated = oxideav_parameters(parameters, CodecId::Opus, OXIDEAV_OPUS)?;
                oxideav_opus::registry::make_decoder(&translated).map_err(map_error)?
            }
            IMPLEMENTATION_PCM => {
                let translated = oxideav_parameters(parameters, CodecId::Pcm, OXIDEAV_PCM_S16LE)?;
                let mut registry = oxideav::CodecRegistry::new();
                oxideav_basic::register_codecs(&mut registry);
                registry.first_decoder(&translated).map_err(map_error)?
            }
            other => {
                return Err(CodecError::unsupported(format!(
                    "{PROVIDER} implementation `{other}`"
                )));
            }
        };
        Ok(Box::new(OxideavDecoder::new(parameters.clone(), inner)))
    }

    fn make_encoder(
        &self,
        implementation: &str,
        _config: &EncoderConfig,
    ) -> Result<Box<dyn Encoder>, CodecError> {
        Err(CodecError::unsupported(format!(
            "{PROVIDER} implementation `{implementation}` has no encoder in this spike"
        )))
    }
}

/// The AAC descriptor, stating the three profiles its README documents.
fn aac_descriptor() -> CodecDescriptor {
    let descriptor = CodecDescriptor::new(
        PROVIDER,
        IMPLEMENTATION_AAC,
        CodecId::Aac,
        Direction::Decode,
    )
    .with_sample_formats(vec![SampleFormat::S16]);
    match descriptor.clone().with_profiles(AAC_PROFILES) {
        Ok(stated) => stated,
        Err(_) => descriptor,
    }
}

/// A descriptor that states its sample format and no profiles.
fn simple_descriptor(implementation: &str, codec: CodecId) -> CodecDescriptor {
    CodecDescriptor::new(PROVIDER, implementation, codec, Direction::Decode)
        .with_sample_formats(vec![SampleFormat::S16])
}

/// Translate the seam's parameters into OxideAV's.
fn oxideav_parameters(
    parameters: &CodecParameters,
    expected: CodecId,
    id: &str,
) -> Result<oxideav::CodecParameters, CodecError> {
    if parameters.codec() != expected {
        return Err(CodecError::unsupported(format!(
            "{PROVIDER} `{id}` cannot decode {}",
            parameters.codec()
        )));
    }
    let audio = parameters.audio().ok_or_else(|| {
        CodecError::unsupported(format!("{PROVIDER} `{id}` needs audio parameters"))
    })?;
    let channels = u16::try_from(audio.channels().count())
        .map_err(|_| CodecError::refused("channel count"))?;
    let mut translated = oxideav::CodecParameters::audio(oxideav::CodecId::new(id));
    translated.sample_rate = Some(audio.sample_rate().get());
    translated.channels = Some(channels);
    translated.sample_format = Some(oxideav::SampleFormat::S16);
    translated.extradata = parameters.extradata().to_vec();
    Ok(translated)
}

/// Map an OxideAV failure onto the seam's one error type.
fn map_error(error: oxideav::Error) -> CodecError {
    match error {
        oxideav::Error::Unsupported(what)
        | oxideav::Error::CodecNotFound(what)
        | oxideav::Error::FormatNotFound(what)
        | oxideav::Error::ResourceExhausted(what) => CodecError::unsupported(what),
        oxideav::Error::InvalidData(what) | oxideav::Error::Other(what) => {
            CodecError::refused(what)
        }
        oxideav::Error::NeedMore | oxideav::Error::Eof => CodecError::refused("end of input"),
        oxideav::Error::Io(error) => CodecError::failed(error.to_string()),
    }
}

/// One OxideAV decoder behind the seam's [`Decoder`].
struct OxideavDecoder {
    parameters: CodecParameters,
    inner: Box<dyn oxideav::Decoder>,
    timestamp: i64,
    reset_failure: Option<CodecError>,
}

impl OxideavDecoder {
    /// Wrap `inner`, which was built for `parameters`.
    fn new(parameters: CodecParameters, inner: Box<dyn oxideav::Decoder>) -> Self {
        Self {
            parameters,
            inner,
            timestamp: 0,
            reset_failure: None,
        }
    }

    /// Convert one OxideAV audio frame into the seam's frame.
    fn convert(&mut self, audio: oxideav::AudioFrame) -> Result<Frame, CodecError> {
        let output = self.inner.output_params();
        let hinted = self.parameters.audio();
        let sample_rate = output
            .as_ref()
            .and_then(|parameters| parameters.sample_rate)
            .or_else(|| hinted.map(|audio| audio.sample_rate().get()))
            .and_then(NonZeroU32::new)
            .ok_or_else(|| CodecError::failed("OxideAV output sample rate"))?;
        let channel_count = output
            .as_ref()
            .and_then(|parameters| parameters.channels)
            .or_else(|| hinted.and_then(|audio| u16::try_from(audio.channels().count()).ok()))
            .and_then(NonZeroU16::new)
            .ok_or_else(|| CodecError::failed("OxideAV output channels"))?;
        let channels = channel_layout(channel_count.get())?;
        let format = output
            .as_ref()
            .and_then(|parameters| parameters.sample_format)
            .unwrap_or(oxideav::SampleFormat::S16);
        let (samples, layout) = decode_samples(format, audio.data)?;
        let pts = audio.pts.unwrap_or(self.timestamp);
        self.timestamp = pts.saturating_add(i64::from(audio.samples));
        let frame = AudioFrame::new(sample_rate, channels, layout, samples)
            .ok_or_else(|| CodecError::failed("OxideAV decoded sample count"))?;
        Ok(Frame::Audio(frame.with_pts(pts)))
    }
}

impl Decoder for OxideavDecoder {
    fn parameters(&self) -> &CodecParameters {
        &self.parameters
    }

    fn send(&mut self, packet: Packet<'_>) -> Result<(), CodecError> {
        if let Some(error) = &self.reset_failure {
            return Err(error.clone());
        }
        let pts = packet.pts().unwrap_or(self.timestamp);
        let time_base = oxideav::TimeBase::from_rate(self.parameters.time_base().get());
        let mut translated =
            oxideav::Packet::new(STREAM_INDEX, time_base, packet.data().to_vec()).with_pts(pts);
        if let Some(dts) = packet.dts() {
            translated = translated.with_dts(dts);
        }
        translated = translated.with_keyframe(packet.key());
        self.inner.send_packet(&translated).map_err(map_error)
    }

    fn receive(&mut self) -> Result<Option<Frame>, CodecError> {
        if let Some(error) = &self.reset_failure {
            return Err(error.clone());
        }
        loop {
            match self.inner.receive_frame() {
                Ok(oxideav::Frame::Audio(audio)) => return self.convert(audio).map(Some),
                Ok(oxideav::Frame::StreamParams(_)) => continue,
                Ok(_) => return Err(CodecError::unsupported("non-audio frame")),
                Err(oxideav::Error::NeedMore | oxideav::Error::Eof) => return Ok(None),
                Err(error) => return Err(map_error(error)),
            }
        }
    }

    fn finish(&mut self) -> Result<(), CodecError> {
        if let Some(error) = &self.reset_failure {
            return Err(error.clone());
        }
        self.inner.flush().map_err(map_error)
    }

    fn reset(&mut self) {
        self.timestamp = 0;
        self.reset_failure = self.inner.reset().err().map(map_error);
    }
}

/// The seam's channel layout from an OxideAV channel count.
fn channel_layout(count: u16) -> Result<ChannelLayout, CodecError> {
    match usize::from(count) {
        MONO_CHANNELS => Ok(ChannelLayout::Mono),
        STEREO_CHANNELS => Ok(ChannelLayout::Stereo),
        SURROUND_5_1_CHANNELS => Ok(ChannelLayout::Surround5_1),
        SURROUND_7_1_CHANNELS => Ok(ChannelLayout::Surround7_1),
        _ => NonZeroU16::new(count)
            .map(ChannelLayout::Discrete)
            .ok_or_else(|| CodecError::failed("OxideAV zero channels")),
    }
}

/// Decode OxideAV's sample planes into the seam's typed buffer.
fn decode_samples(
    format: oxideav::SampleFormat,
    planes: Vec<Vec<u8>>,
) -> Result<(AudioSamples, SampleLayout), CodecError> {
    let layout = if format.is_planar() {
        SampleLayout::Planar
    } else {
        SampleLayout::Interleaved
    };
    let samples = match format {
        oxideav::SampleFormat::S16 | oxideav::SampleFormat::S16P => {
            let bytes = join_planes(planes, S16_BYTES)?;
            AudioSamples::S16(
                bytes
                    .chunks_exact(S16_BYTES)
                    .map(|sample| i16::from_le_bytes([sample[0], sample[1]]))
                    .collect(),
            )
        }
        oxideav::SampleFormat::S32 | oxideav::SampleFormat::S32P => {
            let bytes = join_planes(planes, S32_BYTES)?;
            AudioSamples::S32(
                bytes
                    .chunks_exact(S32_BYTES)
                    .map(|sample| i32::from_le_bytes([sample[0], sample[1], sample[2], sample[3]]))
                    .collect(),
            )
        }
        oxideav::SampleFormat::F32 | oxideav::SampleFormat::F32P => {
            let bytes = join_planes(planes, F32_BYTES)?;
            AudioSamples::F32(
                bytes
                    .chunks_exact(F32_BYTES)
                    .map(|sample| f32::from_le_bytes([sample[0], sample[1], sample[2], sample[3]]))
                    .collect(),
            )
        }
        oxideav::SampleFormat::F64 | oxideav::SampleFormat::F64P => {
            let bytes = join_planes(planes, F64_BYTES)?;
            AudioSamples::F64(
                bytes
                    .chunks_exact(F64_BYTES)
                    .map(|sample| {
                        f64::from_le_bytes([
                            sample[0], sample[1], sample[2], sample[3], sample[4], sample[5],
                            sample[6], sample[7],
                        ])
                    })
                    .collect(),
            )
        }
        oxideav::SampleFormat::U8 | oxideav::SampleFormat::U8P => {
            AudioSamples::U8(join_planes(planes, U8_BYTES)?)
        }
        _ => return Err(CodecError::unsupported("OxideAV sample format")),
    };
    Ok((samples, layout))
}

/// Concatenate planes in channel order, refusing a partial sample.
fn join_planes(planes: Vec<Vec<u8>>, width: usize) -> Result<Vec<u8>, CodecError> {
    let mut bytes: Vec<u8> = Vec::new();
    for plane in planes {
        bytes.extend_from_slice(&plane);
    }
    if bytes.len().is_multiple_of(width) {
        Ok(bytes)
    } else {
        Err(CodecError::failed("OxideAV partial sample"))
    }
}
