//! The two conversions across the codec seam: this crate's [`TrackSpec`] and
//! `broadcast-codec`'s `CodecParameters`.
//!
//! One function each way, and both total: every input yields either a value or
//! a typed [`Error::TranscodeUnrepresentable`], never a guess. A track
//! description this crate carries but the seam does not (an MPEG audio layer,
//! a DTS sample entry, a WebM-native VP8/Vorbis codec, a subtitle or an opaque
//! data track) says exactly which fact is missing.
//!
//! `extradata` is the codec's out-of-band configuration in the form the codec's
//! own container record carries it: an `avcC`/`hvcC`/`vvcC` decoder
//! configuration record body, an `av1C`/`vpcC`/`dac3`/`dec3`/`dac4`/`dOps`/
//! `dfLa`/`ddts`/`mhaC` payload, an AAC `AudioSpecificConfig`, or an `esds`
//! body for the MPEG-2 video and MPEG audio configurations that carry one.
//! Each is what the matching `isobmff` type parses, so a record written here
//! is the record read back.
//!
//! Neither direction carries a codec *profile*: the seam's `CodecProfile` is a
//! typed variant key the caller states, and a `TrackSpec` does not hold one.
//! The gapless trim the seam expresses is audio-only and this crate's IR has
//! no field for it, so a track states none.

use alloc::vec::Vec;
use core::num::{NonZeroU16, NonZeroU32};

use broadcast_codec::{AudioParameters, ChannelLayout, CodecId, CodecParameters, VideoParameters};
use broadcast_core::{Parse, Serialize};
use isobmff::ac3::{Ac3SpecificBox, Ec3SpecificBox};
use isobmff::ac4::Ac4SpecificBox;
use isobmff::av1::Av1ConfigurationBox;
use isobmff::avc_config::AVCDecoderConfigurationRecord;
use isobmff::flac::FlacSpecificBox;
use isobmff::hevc_config::HEVCDecoderConfigurationRecord;
use isobmff::mp4esds::{
    DecoderConfigDescriptor, DecoderSpecificInfo, ESDescriptor, EsdsBox, SLConfigDescriptor,
    STREAM_TYPE_AUDIO,
};
use isobmff::mpegh::MHADecoderConfigurationRecord;
use isobmff::opus::OpusSpecificBox;
use isobmff::vp9::Vp9ConfigurationBox;
use isobmff::vvc_config::VvcDecoderConfigurationRecord;

use crate::TrackId;
use crate::error::{Error, Result};
use crate::ir::{CodecConfig, TrackSpec};

/// The `esds` object type indication for MPEG-4 audio (ISO/IEC 14496-1 Table 5).
const OTI_MPEG4_AUDIO: u8 = 0x40;
/// The `esds` object type indication for MPEG-2 Main Visual (Table 5).
const OTI_MPEG2_VIDEO_MAIN: u8 = 0x61;
/// The `esds` `streamType` for a VisualStream (ISO/IEC 14496-1 Table 6).
const STREAM_TYPE_VISUAL: u8 = 0x04;
/// The `ES_ID` this crate assigns to a single audio elementary stream.
const AUDIO_ES_ID: u16 = 1;
/// The `ES_ID` this crate assigns to a single video elementary stream.
const VIDEO_ES_ID: u16 = 2;
/// `esds` FullBox header bytes (`version` + `flags`, ISO/IEC 14496-12:2026
/// §4.2) that precede the `ES_Descriptor` in an `esds` body.
const ESDS_FULLBOX_LEN: usize = 4;
/// `sample_size` of the audio configurations that store a constant 16
/// (ISO/IEC 14496-12:2026 §12.2.3.2); the seam carries no sample width.
const AUDIO_SAMPLE_SIZE_BITS: u16 = 16;

/// The codec parameters the seam needs to decode `spec`.
///
/// # Errors
/// [`Error::TranscodeUnrepresentable`] when the track's description states
/// something the seam has no place for: a codec it does not name (VP8,
/// Vorbis), a non-media track (subtitle, opaque data), a zero sample rate or
/// coded dimension, or a dimension wider than a 32-bit field.
pub fn codec_parameters_from_track(spec: &TrackSpec) -> Result<CodecParameters> {
    let parameters = CodecParameters::new(codec_id(&spec.config)?, spec.timescale);
    let parameters = match &spec.config {
        CodecConfig::Avc {
            config,
            width,
            height,
        } => parameters
            .with_extradata(record_bytes(&config.config)?)
            .with_video(video(*width, *height)?),
        CodecConfig::Hevc {
            config,
            width,
            height,
        } => parameters
            .with_extradata(record_bytes(&config.config)?)
            .with_video(video(*width, *height)?),
        CodecConfig::Vvc {
            config,
            width,
            height,
        } => parameters
            .with_extradata(record_bytes(&config.config)?)
            .with_video(video(*width, *height)?),
        CodecConfig::Av1 { config, .. } => parameters.with_extradata(record_bytes(config)?),
        CodecConfig::Vp9 { config, .. } => parameters.with_extradata(record_bytes(config)?),
        CodecConfig::Aac {
            esds,
            channel_count,
            sample_rate,
            ..
        } => parameters
            .with_extradata(
                esds.decoder_specific_info_data()
                    .map_or_else(Vec::new, <[u8]>::to_vec),
            )
            .with_audio(audio(*channel_count, *sample_rate)?),
        CodecConfig::Ac3 {
            config,
            channel_count,
            sample_rate,
            ..
        } => parameters
            .with_extradata(record_bytes(config)?)
            .with_audio(audio(*channel_count, *sample_rate)?),
        CodecConfig::Eac3 {
            config,
            channel_count,
            sample_rate,
            ..
        } => parameters
            .with_extradata(record_bytes(config)?)
            .with_audio(audio(*channel_count, *sample_rate)?),
        CodecConfig::Ac4 {
            config,
            channel_count,
            sample_rate,
            ..
        } => parameters
            .with_extradata(record_bytes(config)?)
            .with_audio(audio(*channel_count, *sample_rate)?),
        CodecConfig::Opus {
            config,
            channel_count,
            sample_rate,
            ..
        } => parameters
            .with_extradata(record_bytes(config)?)
            .with_audio(audio(*channel_count, *sample_rate)?),
        CodecConfig::Flac {
            config,
            channel_count,
            sample_rate,
            ..
        } => parameters
            .with_extradata(record_bytes(config)?)
            .with_audio(audio(*channel_count, *sample_rate)?),
        CodecConfig::Dts {
            config,
            channel_count,
            sample_rate,
            ..
        } => parameters
            .with_extradata(record_bytes(config)?)
            .with_audio(audio(*channel_count, *sample_rate)?),
        CodecConfig::MpegH {
            config,
            channel_count,
            sample_rate,
            ..
        } => parameters
            .with_extradata(record_bytes(config)?)
            .with_audio(audio(*channel_count, *sample_rate)?),
        CodecConfig::Mpeg2Video {
            esds,
            width,
            height,
        } => parameters
            .with_extradata(esds_body(esds)?)
            .with_video(video(*width, *height)?),
        CodecConfig::MpegAudio {
            esds,
            channel_count,
            sample_rate,
            ..
        } => parameters
            .with_extradata(esds_body(esds)?)
            .with_audio(audio(*channel_count, *sample_rate)?),
        CodecConfig::Vp8 { .. } => {
            return Err(unrepresentable("the WebM-native VP8 codec identity"));
        }
        CodecConfig::Vorbis { .. } => {
            return Err(unrepresentable("the WebM-native Vorbis codec identity"));
        }
        CodecConfig::Subtitle { .. } => {
            return Err(unrepresentable("a subtitle track"));
        }
        CodecConfig::Data { .. } => {
            return Err(unrepresentable("an opaque data track"));
        }
    };
    Ok(parameters)
}

/// The [`TrackSpec`] `parameters` describe, for `track_id`.
///
/// # Errors
/// [`Error::TranscodeUnrepresentable`] when the seam's parameters say
/// something this crate's track description cannot carry: a codec it has no
/// configuration for (including PCM and an unnamed [`CodecId::Other`]), an
/// MPEG audio layer, a DTS sample-entry type, a coded dimension wider than 16
/// bits, or a configuration record that does not parse.
pub fn track_spec_from_codec_parameters(
    track_id: TrackId,
    parameters: &CodecParameters,
) -> Result<TrackSpec> {
    let config = track_config(parameters)?;
    Ok(TrackSpec::new(track_id, parameters.time_base(), config))
}

/// The codec identity of `config`, or the fact the seam cannot name.
fn codec_id(config: &CodecConfig) -> Result<CodecId> {
    let codec = match config {
        CodecConfig::Avc { .. } => CodecId::H264,
        CodecConfig::Hevc { .. } => CodecId::Hevc,
        CodecConfig::Vvc { .. } => CodecId::Vvc,
        CodecConfig::Av1 { .. } => CodecId::Av1,
        CodecConfig::Vp9 { .. } => CodecId::Vp9,
        CodecConfig::Mpeg2Video { .. } => CodecId::Mpeg2Video,
        CodecConfig::Aac { .. } => CodecId::Aac,
        CodecConfig::Ac3 { .. } => CodecId::Ac3,
        CodecConfig::Eac3 { .. } => CodecId::Eac3,
        CodecConfig::Ac4 { .. } => CodecId::Ac4,
        CodecConfig::Dts { .. } => CodecId::Dts,
        CodecConfig::Opus { .. } => CodecId::Opus,
        CodecConfig::Flac { .. } => CodecId::Flac,
        CodecConfig::MpegAudio { .. } => CodecId::MpegAudio,
        CodecConfig::MpegH { .. } => CodecId::MpegH3dAudio,
        CodecConfig::Vp8 { .. } => return Err(unrepresentable("the VP8 codec identity")),
        CodecConfig::Vorbis { .. } => return Err(unrepresentable("the Vorbis codec identity")),
        CodecConfig::Subtitle { .. } => return Err(unrepresentable("a subtitle track")),
        CodecConfig::Data { .. } => return Err(unrepresentable("an opaque data track")),
    };
    Ok(codec)
}

/// The configuration `parameters` describe.
fn track_config(parameters: &CodecParameters) -> Result<CodecConfig> {
    let extradata = parameters.extradata();
    match parameters.codec() {
        CodecId::H264 => {
            let (width, height) = dimensions(parameters)?;
            Ok(CodecConfig::Avc {
                config: isobmff::avc_config::AVCConfigurationBox::new(
                    AVCDecoderConfigurationRecord::parse(extradata)?,
                ),
                width,
                height,
            })
        }
        CodecId::Hevc => {
            let (width, height) = dimensions(parameters)?;
            Ok(CodecConfig::Hevc {
                config: isobmff::hevc_config::HEVCConfigurationBox::new(
                    HEVCDecoderConfigurationRecord::parse(extradata)?,
                ),
                width,
                height,
            })
        }
        CodecId::Vvc => {
            let (width, height) = dimensions(parameters)?;
            Ok(CodecConfig::Vvc {
                config: isobmff::vvc_config::VvcConfigurationBox::new(
                    VvcDecoderConfigurationRecord::parse(extradata)?,
                ),
                width,
                height,
            })
        }
        CodecId::Av1 => {
            let (width, height) = dimensions(parameters)?;
            Ok(CodecConfig::Av1 {
                config: Av1ConfigurationBox::parse(extradata)?,
                width,
                height,
            })
        }
        CodecId::Vp9 => {
            let (width, height) = dimensions(parameters)?;
            Ok(CodecConfig::Vp9 {
                config: Vp9ConfigurationBox::parse(extradata)?,
                width,
                height,
            })
        }
        CodecId::Mpeg2Video => {
            let (width, height) = dimensions(parameters)?;
            Ok(CodecConfig::Mpeg2Video {
                esds: esds_from_body(
                    extradata,
                    VIDEO_ES_ID,
                    STREAM_TYPE_VISUAL,
                    OTI_MPEG2_VIDEO_MAIN,
                )?,
                width,
                height,
            })
        }
        CodecId::Aac => {
            let (channel_count, sample_rate) = audio_fields(parameters)?;
            Ok(CodecConfig::Aac {
                esds: aac_esds(extradata),
                channel_count,
                sample_rate,
                sample_size: AUDIO_SAMPLE_SIZE_BITS,
            })
        }
        CodecId::Ac3 => {
            let (channel_count, sample_rate) = audio_fields(parameters)?;
            Ok(CodecConfig::Ac3 {
                config: Ac3SpecificBox::parse(extradata)?,
                channel_count,
                sample_rate,
                sample_size: AUDIO_SAMPLE_SIZE_BITS,
            })
        }
        CodecId::Eac3 => {
            let (channel_count, sample_rate) = audio_fields(parameters)?;
            Ok(CodecConfig::Eac3 {
                config: Ec3SpecificBox::parse(extradata)?,
                channel_count,
                sample_rate,
                sample_size: AUDIO_SAMPLE_SIZE_BITS,
            })
        }
        CodecId::Ac4 => {
            let (channel_count, sample_rate) = audio_fields(parameters)?;
            Ok(CodecConfig::Ac4 {
                config: Ac4SpecificBox::parse(extradata)?,
                channel_count,
                sample_rate,
                sample_size: AUDIO_SAMPLE_SIZE_BITS,
            })
        }
        CodecId::Opus => {
            let (channel_count, sample_rate) = audio_fields(parameters)?;
            Ok(CodecConfig::Opus {
                config: OpusSpecificBox::parse(extradata)?,
                channel_count,
                sample_rate,
                sample_size: AUDIO_SAMPLE_SIZE_BITS,
            })
        }
        CodecId::Flac => {
            let (channel_count, sample_rate) = audio_fields(parameters)?;
            Ok(CodecConfig::Flac {
                config: FlacSpecificBox::parse(extradata)?,
                channel_count,
                sample_rate,
                sample_size: AUDIO_SAMPLE_SIZE_BITS,
            })
        }
        CodecId::MpegH3dAudio => {
            let (channel_count, sample_rate) = audio_fields(parameters)?;
            Ok(CodecConfig::MpegH {
                config: MHADecoderConfigurationRecord::parse(extradata)?,
                channel_count,
                sample_rate,
                sample_size: AUDIO_SAMPLE_SIZE_BITS,
            })
        }
        CodecId::Dts => Err(unrepresentable(
            "the DTS sample-entry type (`dtsc`/`dtsh`/`dtsl`/`dtse`)",
        )),
        CodecId::MpegAudio => Err(unrepresentable("the MPEG audio layer (I, II or III)")),
        CodecId::Pcm => Err(unrepresentable("linear PCM")),
        _ => Err(unrepresentable("this codec identity")),
    }
}

/// `avc1`/`av01`/… coded dimensions, as the two 16-bit fields the container
/// stores.
fn dimensions(parameters: &CodecParameters) -> Result<(u16, u16)> {
    let video = parameters
        .video()
        .ok_or_else(|| unrepresentable("a video codec with no coded dimensions"))?;
    let width = u16::try_from(video.coded_width().get())
        .map_err(|_| unrepresentable("a coded width wider than 16 bits"))?;
    let height = u16::try_from(video.coded_height().get())
        .map_err(|_| unrepresentable("a coded height taller than 16 bits"))?;
    Ok((width, height))
}

/// A coded dimension pair, or the fact the seam cannot state it.
fn video(width: u16, height: u16) -> Result<VideoParameters> {
    let width =
        NonZeroU32::new(u32::from(width)).ok_or_else(|| unrepresentable("a zero coded width"))?;
    let height =
        NonZeroU32::new(u32::from(height)).ok_or_else(|| unrepresentable("a zero coded height"))?;
    Ok(VideoParameters::new(width, height))
}

/// The audio half's channel count and sample rate.
fn audio_fields(parameters: &CodecParameters) -> Result<(u16, u32)> {
    let audio = parameters
        .audio()
        .ok_or_else(|| unrepresentable("an audio codec with no sample rate"))?;
    let count = u16::try_from(audio.channels().count())
        .map_err(|_| unrepresentable("more than 65535 channels"))?;
    Ok((count, audio.sample_rate().get()))
}

/// An audio sample rate and channel count, or the fact the seam cannot state
/// them.
fn audio(channel_count: u16, sample_rate: u32) -> Result<AudioParameters> {
    let rate = NonZeroU32::new(sample_rate).ok_or_else(|| unrepresentable("a zero sample rate"))?;
    Ok(AudioParameters::new(rate, channel_layout(channel_count)?))
}

/// The named layout for the channel counts the workspace's configurations use,
/// a `Discrete` layout otherwise (§ no order is claimed for an unnamed count).
fn channel_layout(channel_count: u16) -> Result<ChannelLayout> {
    let layout = match channel_count {
        1 => ChannelLayout::Mono,
        2 => ChannelLayout::Stereo,
        6 => ChannelLayout::Surround5_1,
        8 => ChannelLayout::Surround7_1,
        count => ChannelLayout::Discrete(
            NonZeroU16::new(count).ok_or_else(|| unrepresentable("a track with no channels"))?,
        ),
    };
    Ok(layout)
}

/// The bytes of a configuration record, body only (no box header).
fn record_bytes<T>(record: &T) -> Result<Vec<u8>>
where
    T: Serialize<Error = isobmff::Error>,
{
    Ok(record.try_to_bytes()?)
}

/// An `esds` body: the FullBox header and the `ES_Descriptor`.
fn esds_body(esds: &EsdsBox) -> Result<Vec<u8>> {
    let descriptor = esds.es_descriptor.try_to_bytes()?;
    let mut body = alloc::vec![0u8; ESDS_FULLBOX_LEN];
    body.extend_from_slice(&descriptor);
    Ok(body)
}

/// An `esds` from its body, or the canonical one `es_id`/`stream_type`/`oti`
/// describe when the body is empty (the seam carries no `esds` framing for
/// these codecs, so a caller may leave the descriptor out).
fn esds_from_body(body: &[u8], es_id: u16, stream_type: u8, oti: u8) -> Result<EsdsBox> {
    if body.is_empty() {
        return Ok(EsdsBox::new(ESDescriptor::new(
            es_id,
            0,
            Some(DecoderConfigDescriptor::new(
                oti,
                stream_type,
                false,
                0,
                0,
                0,
                None,
            )),
            Some(SLConfigDescriptor::predefined_two()),
        )));
    }
    Ok(EsdsBox::parse_body(body)?)
}

/// An AAC `esds` whose DecoderSpecificInfo is `asc` (ISO/IEC 14496-3 §1.6.2.1).
fn aac_esds(asc: &[u8]) -> EsdsBox {
    EsdsBox::new(ESDescriptor::new(
        AUDIO_ES_ID,
        0,
        Some(DecoderConfigDescriptor::new(
            OTI_MPEG4_AUDIO,
            STREAM_TYPE_AUDIO,
            false,
            0,
            0,
            0,
            Some(DecoderSpecificInfo::new(asc.to_vec())),
        )),
        Some(SLConfigDescriptor::predefined_two()),
    ))
}

fn unrepresentable(what: &'static str) -> Error {
    Error::TranscodeUnrepresentable { what }
}
