//! The adapter's public face: every libav implementation the machine reports.

use std::collections::BTreeSet;
use std::string::{String, ToString};
use std::vec::Vec;

use broadcast_codec::{
    ChannelLayout, CodecDescriptor, CodecError, CodecId, CodecParameters, CodecProfile,
    CodecProvider, Decoder, Direction, Encoder, EncoderConfig, Probe, ProviderInfo, Timescale,
};
use ffmpeg_next::codec;

use crate::decoder::LibavDecoder;
use crate::encoder::LibavEncoder;
use crate::map;

/// The provider name a preference and the inventory use.
const PROVIDER: &str = "libav";

/// Version component shifts of libav's packed version number.
const VERSION_MAJOR_SHIFT: u32 = 16;
/// Shift of the minor component.
const VERSION_MINOR_SHIFT: u32 = 8;
/// Mask of one version component.
const VERSION_COMPONENT_MASK: u32 = 0xff;

/// The picture size a probe asks a video encoder to open for.
const PROBE_WIDTH: u32 = 320;
/// The picture height a probe asks a video encoder to open for.
const PROBE_HEIGHT: u32 = 240;
/// The tick rate a probe opens an encoder with.
const PROBE_VIDEO_TICKS: u32 = 25;
/// The sample rate a probe asks an audio encoder to open for.
const PROBE_SAMPLE_RATE: u32 = 48_000;

/// Every decoder and encoder the linked libav reports under a codec this
/// adapter maps.
///
/// One [`CodecId`] can have several implementations: libav's default first,
/// then the hardware codecs and alternate libraries it registers under a name
/// of their own, each listed only when this machine's libav has it.
#[derive(Debug, Default, Clone, Copy)]
pub struct LibavProvider;

impl LibavProvider {
    /// The provider.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl CodecProvider for LibavProvider {
    fn name(&self) -> &str {
        PROVIDER
    }

    fn info(&self) -> ProviderInfo {
        ProviderInfo::new(PROVIDER, env!("CARGO_PKG_VERSION"))
            .with_library(format!("libavcodec {}", version()))
            .with_licence(codec::license().to_string())
            .with_notes(codec::configuration().to_string())
    }

    fn descriptors(&self) -> Vec<CodecDescriptor> {
        let mut descriptors = Vec::new();
        for (codec, id) in map::CODECS {
            for (direction, found, named) in [
                (
                    Direction::Decode,
                    codec::decoder::find(*id),
                    map::NAMED_DECODERS,
                ),
                (
                    Direction::Encode,
                    codec::encoder::find(*id),
                    map::NAMED_ENCODERS,
                ),
            ] {
                let mut seen: BTreeSet<String> = BTreeSet::new();
                if let Some(found) = found {
                    let name = found.name().to_string();
                    descriptors.push(descriptor(*codec, direction, &name, &found));
                    seen.insert(name);
                }
                for (_, name) in named.iter().filter(|(mapped, _)| mapped == codec) {
                    let name = *name;
                    if seen.contains(name) {
                        continue;
                    }
                    let found = match direction {
                        Direction::Decode => codec::decoder::find_by_name(name),
                        Direction::Encode => codec::encoder::find_by_name(name),
                        _ => None,
                    };
                    if let Some(found) = found {
                        descriptors.push(descriptor(*codec, direction, name, &found));
                        seen.insert(name.to_string());
                    }
                }
            }
        }
        descriptors
    }

    fn probe(&self, implementation: &str, direction: Direction, codec: CodecId) -> Probe {
        match direction {
            Direction::Decode => {
                let Some(codec) = codec::decoder::find_by_name(implementation) else {
                    return Probe::unavailable(format!("libav has no decoder `{implementation}`"));
                };
                let context = codec::Context::new_with_codec(codec);
                match context.decoder().open_as(codec) {
                    Ok(_) => Probe::available(),
                    Err(error) => Probe::unavailable(error.to_string()),
                }
            }
            Direction::Encode => match probe_encoder(implementation, codec) {
                Ok(()) => Probe::available(),
                Err(error) => Probe::unavailable(error.to_string()),
            },
            _ => Probe::unknown(),
        }
    }

    fn make_decoder(
        &self,
        implementation: &str,
        parameters: &CodecParameters,
    ) -> Result<Box<dyn Decoder>, CodecError> {
        Ok(Box::new(LibavDecoder::open(implementation, parameters)?))
    }

    fn make_encoder(
        &self,
        implementation: &str,
        config: &EncoderConfig,
    ) -> Result<Box<dyn Encoder>, CodecError> {
        Ok(Box::new(LibavEncoder::open(implementation, config)?))
    }
}

/// One descriptor, with libav's own profile list for the codec.
fn descriptor(
    codec: CodecId,
    direction: Direction,
    implementation: &str,
    found: &ffmpeg_next::Codec,
) -> CodecDescriptor {
    let profiles: Vec<CodecProfile> = found
        .profiles()
        .into_iter()
        .flatten()
        .filter_map(map::codec_profile)
        .filter(|profile| profile.codec() == codec)
        .fold(Vec::new(), |mut unique, profile| {
            if !unique.contains(&profile) {
                unique.push(profile);
            }
            unique
        });
    let descriptor = CodecDescriptor::new(PROVIDER, implementation, codec, direction)
        .with_hardware(map::hardware(implementation))
        .with_lossless(map::lossless(codec));
    match descriptor.with_profiles(profiles) {
        Ok(descriptor) => descriptor,
        Err(_) => CodecDescriptor::new(PROVIDER, implementation, codec, direction)
            .with_hardware(map::hardware(implementation))
            .with_lossless(map::lossless(codec)),
    }
}

/// Whether an encoder called `implementation` opens with a minimal
/// configuration for its own medium.
fn probe_encoder(implementation: &str, codec_id: CodecId) -> Result<(), CodecError> {
    let Some(codec) = codec::encoder::find_by_name(implementation) else {
        return Err(CodecError::unsupported(format!(
            "libav has no encoder `{implementation}`"
        )));
    };
    let config = match codec.medium() {
        ffmpeg_next::media::Type::Audio => EncoderConfig::new(
            codec_id,
            Timescale::from_nonzero(nonzero(PROBE_SAMPLE_RATE)?),
        )
        .with_audio(broadcast_codec::AudioParameters::new(
            nonzero(PROBE_SAMPLE_RATE)?,
            ChannelLayout::Stereo,
        )),
        ffmpeg_next::media::Type::Video => EncoderConfig::new(
            codec_id,
            Timescale::from_nonzero(nonzero(PROBE_VIDEO_TICKS)?),
        )
        .with_video(broadcast_codec::VideoParameters::new(
            nonzero(PROBE_WIDTH)?,
            nonzero(PROBE_HEIGHT)?,
        )),
        _ => return Err(CodecError::unsupported("an encoder of another medium")),
    };
    LibavEncoder::open(implementation, &config).map(|_| ())
}

/// A non-zero value for the probe's settings.
fn nonzero(value: u32) -> Result<std::num::NonZeroU32, CodecError> {
    std::num::NonZeroU32::new(value).ok_or_else(|| CodecError::failed("a zero probe size"))
}

/// libav's version as `major.minor.micro`.
fn version() -> String {
    let packed = codec::version();
    format!(
        "{}.{}.{}",
        packed >> VERSION_MAJOR_SHIFT,
        (packed >> VERSION_MINOR_SHIFT) & VERSION_COMPONENT_MASK,
        packed & VERSION_COMPONENT_MASK
    )
}
