//! One libav encoder behind [`broadcast_codec::Encoder`].

use std::string::String;
use std::vec::Vec;

use broadcast_codec::{
    AudioFrame, AudioSamples, CodecError, CodecId, CodecParameters, Encoder, EncoderConfig, Frame,
    Packet, SampleFormat, SampleLayout, VideoFrame,
};
use ffmpeg_next::codec::encoder::audio::{Audio as AudioBuilder, Encoder as AudioEncoder};
use ffmpeg_next::codec::encoder::video::{Encoder as VideoEncoder, Video as VideoBuilder};
use ffmpeg_next::format::{Pixel, Sample};
use ffmpeg_next::{Dictionary, Rational};

use crate::decoder::{drained, try_again};
use crate::map;

/// The pixel format every video encoder this adapter builds is configured for.
///
/// [`EncoderConfig`] states no source format, so the adapter states one: an
/// 8-bit 4:2:0 picture. A frame in another format is refused rather than
/// silently converted.
const VIDEO_PIXEL_FORMAT: Pixel = Pixel::YUV420P;

/// The codec option a profile is carried in.
const PROFILE_OPTION: &str = "profile";

/// The libav encoder context this adapter drives.
enum Opened {
    /// A video encoder.
    Video(VideoEncoder),
    /// An audio encoder.
    Audio(AudioEncoder),
}

/// An encoder over one libav codec context.
///
/// The settings are the seam's [`EncoderConfig`]: the coded format, the time
/// base its packets use, the profile to produce, the source parameters, a bit
/// rate and a group length, plus provider options forwarded to libav. The
/// adapter configures an 8-bit 4:2:0 video encoder, or an audio encoder in the
/// storage format [`audio_encoder_format`] names for the codec.
pub struct LibavEncoder {
    implementation: String,
    opened: Opened,
    parameters: CodecParameters,
    draining: bool,
}

impl LibavEncoder {
    /// Open libav's encoder called `implementation` for `config`.
    ///
    /// # Errors
    /// [`CodecError::Unsupported`] when the codec has no libav mapping, when
    /// `config` describes neither audio nor video, or when no encoder is
    /// registered under `implementation`; [`CodecError::Refused`] when the
    /// profile cannot be requested or libav will not open the encoder, naming
    /// the option key that made it refuse when one does.
    pub fn open(implementation: &str, config: &EncoderConfig) -> Result<Self, CodecError> {
        if map::libav_id(config.codec()).is_none() {
            return Err(CodecError::unsupported(format!(
                "codec `{}`",
                config.codec().name()
            )));
        }
        let codec = ffmpeg_next::codec::encoder::find_by_name(implementation)
            .ok_or_else(|| CodecError::unsupported(format!("libav encoder `{implementation}`")))?;
        let mut options: Vec<(&str, String)> = config
            .options()
            .iter()
            .map(|(key, value)| (key.as_str(), value.clone()))
            .collect();
        if let Some(profile) = config.profile() {
            let libav = map::libav_profile(profile)
                .ok_or_else(|| CodecError::refused(format!("profile `{}`", profile.token())))?;
            options.push((PROFILE_OPTION, i32::from(libav).to_string()));
        }
        let time_base = config.time_base().get();
        let denominator = i32::try_from(time_base).map_err(|_| {
            CodecError::unsupported(format!("time base `{time_base}` as a libav time base"))
        })?;
        let opened = match (config.video(), config.audio()) {
            (Some(video), None) => {
                let mut encoder = ffmpeg_next::codec::Context::new_with_codec(codec)
                    .encoder()
                    .video()
                    .map_err(|error| refused(implementation, &error))?;
                encoder.set_width(video.coded_width().get());
                encoder.set_height(video.coded_height().get());
                encoder.set_format(VIDEO_PIXEL_FORMAT);
                encoder.set_time_base(Rational::new(1, denominator));
                if let Some(bit_rate) = config.bit_rate() {
                    encoder.set_bit_rate(usize::try_from(bit_rate.get()).unwrap_or(usize::MAX));
                }
                if let Some(gop) = config.gop() {
                    encoder.set_gop(gop.get());
                }
                open_video(encoder, &options).map(Opened::Video)?
            }
            (None, Some(audio)) => {
                let mut encoder = ffmpeg_next::codec::Context::new_with_codec(codec)
                    .encoder()
                    .audio()
                    .map_err(|error| refused(implementation, &error))?;
                let rate = i32::try_from(audio.sample_rate().get())
                    .map_err(|_| CodecError::unsupported("sample rate as a libav rate"))?;
                encoder.set_rate(rate);
                encoder.set_channel_layout(map::libav_channel_layout(audio.channels()));
                encoder.set_format(audio_encoder_format(config.codec()));
                encoder.set_time_base(Rational::new(1, denominator));
                if let Some(bit_rate) = config.bit_rate() {
                    encoder.set_bit_rate(usize::try_from(bit_rate.get()).unwrap_or(usize::MAX));
                }
                open_audio(encoder, &options).map(Opened::Audio)?
            }
            _ => {
                return Err(CodecError::unsupported(
                    "encoder settings that are not exactly one of audio and video",
                ));
            }
        };
        let mut parameters = CodecParameters::new(config.codec(), config.time_base());
        if let Some(profile) = config.profile() {
            parameters = parameters
                .with_profile(profile)
                .map_err(|_| CodecError::refused("a profile of another codec"))?;
        }
        parameters = match (config.video(), config.audio()) {
            (Some(video), None) => parameters.with_video(video),
            (None, Some(audio)) => parameters.with_audio(audio),
            _ => parameters,
        };
        Ok(Self {
            implementation: implementation.to_string(),
            opened,
            parameters,
            draining: false,
        })
    }

    /// The libav implementation this encoder runs.
    #[must_use]
    pub fn implementation(&self) -> &str {
        &self.implementation
    }
}

/// The sample format this adapter configures `codec`'s encoder for.
fn audio_encoder_format(codec: CodecId) -> Sample {
    match codec {
        CodecId::Flac | CodecId::Opus => {
            map::libav_sample(SampleFormat::S16, SampleLayout::Interleaved)
        }
        _ => map::libav_sample(SampleFormat::F32, SampleLayout::Planar),
    }
}

/// Build the options dictionary and open.
fn open_video(
    encoder: VideoBuilder,
    options: &[(&str, String)],
) -> Result<VideoEncoder, CodecError> {
    let mut dictionary = Dictionary::new();
    for (key, value) in options {
        dictionary.set(key, value);
    }
    encoder
        .open_with(dictionary)
        .map_err(|error| refused_option(options, &error))
}

/// Build the options dictionary and open.
fn open_audio(
    encoder: AudioBuilder,
    options: &[(&str, String)],
) -> Result<AudioEncoder, CodecError> {
    let mut dictionary = Dictionary::new();
    for (key, value) in options {
        dictionary.set(key, value);
    }
    encoder
        .open_with(dictionary)
        .map_err(|error| refused_option(options, &error))
}

/// A refusal naming the option libav's own message names, when it names one.
fn refused_option(options: &[(&str, String)], error: &ffmpeg_next::Error) -> CodecError {
    let message = error.to_string();
    match options.iter().find(|(key, _)| message.contains(key)) {
        Some((key, _)) => CodecError::refused(format!("option `{key}`: {message}")),
        None => refused("this implementation", error),
    }
}

/// A refusal carrying libav's own text.
fn refused(implementation: &str, error: &ffmpeg_next::Error) -> CodecError {
    CodecError::refused(format!("libav encoder `{implementation}`: {error}"))
}

/// Queue one decoded frame.
fn send_video(
    encoder: &mut VideoEncoder,
    implementation: &str,
    frame: VideoFrame,
) -> Result<(), CodecError> {
    let format = map::libav_pixel(frame.format());
    if format != VIDEO_PIXEL_FORMAT {
        return Err(CodecError::refused(format!(
            "a source picture in `{:?}`; this encoder takes `{VIDEO_PIXEL_FORMAT:?}`",
            frame.format()
        )));
    }
    let mut coded =
        ffmpeg_next::frame::Video::new(format, frame.width().get(), frame.height().get());
    match frame.storage() {
        broadcast_codec::VideoStorage::Cpu(planes) => {
            copy_planes(
                planes,
                &mut coded,
                frame.width().get(),
                frame.height().get(),
            )?;
        }
        _ => return Err(CodecError::unsupported("a picture in hardware storage")),
    }
    coded.set_pts(frame.pts());
    match encoder.send_frame(&coded) {
        Ok(()) => Ok(()),
        Err(error) if try_again(&error) => Err(CodecError::Full),
        Err(error) => Err(CodecError::failed(format!(
            "libav encoder `{implementation}` send: {error}"
        ))),
    }
}

/// Queue one decoded frame.
fn send_audio(
    encoder: &mut AudioEncoder,
    implementation: &str,
    codec: CodecId,
    frame: AudioFrame,
) -> Result<(), CodecError> {
    let expected = audio_encoder_format(codec);
    let (storage, layout) = map::sample_format(expected)
        .ok_or_else(|| CodecError::unsupported("an encoder sample format"))?;
    if frame.layout() != layout {
        return Err(CodecError::refused(format!(
            "a source frame in `{}` samples; this encoder takes `{}`",
            frame.layout().name(),
            layout.name()
        )));
    }
    let mut coded = ffmpeg_next::frame::Audio::new(
        expected,
        frame.frames(),
        map::libav_channel_layout(frame.channels()),
    );
    coded.set_rate(frame.sample_rate().get());
    coded.set_pts(frame.pts());
    copy_audio(
        &mut coded,
        frame.samples(),
        storage,
        layout,
        frame.frames(),
        frame.channels().count(),
    )?;
    match encoder.send_frame(&coded) {
        Ok(()) => Ok(()),
        Err(error) if try_again(&error) => Err(CodecError::Full),
        Err(error) => Err(CodecError::failed(format!(
            "libav encoder `{implementation}` send: {error}"
        ))),
    }
}

/// Copy every plane of `planes` into `coded`, honouring libav's line size.
fn copy_planes(
    planes: &[broadcast_codec::VideoPlane],
    coded: &mut ffmpeg_next::frame::Video,
    width: u32,
    height: u32,
) -> Result<(), CodecError> {
    let format = map::pixel_format(coded.format())
        .ok_or_else(|| CodecError::unsupported("a source pixel format"))?;
    if planes.len() != format.plane_count() {
        return Err(CodecError::refused("a source picture's plane count"));
    }
    for (index, plane) in planes.iter().enumerate() {
        let rows = usize::try_from(coded.plane_height(index))
            .map_err(|_| CodecError::failed("a source picture's plane height"))?;
        let visible = format
            .plane_size(
                usize::try_from(width).unwrap_or(usize::MAX),
                usize::try_from(height).unwrap_or(usize::MAX),
                index,
            )
            .ok_or_else(|| CodecError::failed("a source picture's plane size"))?;
        if rows == 0 || !visible.is_multiple_of(rows) {
            return Err(CodecError::failed("a source picture's plane geometry"));
        }
        let row_bytes = visible / rows;
        let stride = coded.stride(index);
        let source_stride = plane.stride().get();
        let source = plane.data();
        let destination = coded.data_mut(index);
        for row in 0..rows {
            let from = row * source_stride;
            let to = row * stride;
            let Some(bytes) = source.get(from..from + row_bytes) else {
                return Err(CodecError::refused("a source picture's plane data"));
            };
            let Some(slot) = destination.get_mut(to..to + row_bytes) else {
                return Err(CodecError::failed("a destination picture's plane data"));
            };
            slot.copy_from_slice(bytes);
        }
    }
    Ok(())
}

/// Copy `samples` into `coded` in the storage and layout it is in.
fn copy_audio(
    coded: &mut ffmpeg_next::frame::Audio,
    samples: &AudioSamples,
    storage: SampleFormat,
    layout: SampleLayout,
    per_channel: usize,
    channels: usize,
) -> Result<(), CodecError> {
    let (width, bytes) = match (storage, samples) {
        (SampleFormat::U8, AudioSamples::U8(values)) => (1, values.clone()),
        (SampleFormat::S16, AudioSamples::S16(values)) => (2, to_bytes(values)),
        (SampleFormat::S32, AudioSamples::S32(values)) => (4, to_bytes_i32(values)),
        (SampleFormat::F32, AudioSamples::F32(values)) => (4, to_bytes_f32(values)),
        (SampleFormat::F64, AudioSamples::F64(values)) => (8, to_bytes_f64(values)),
        _ => {
            return Err(CodecError::refused(format!(
                "a source frame in another sample format; this encoder takes `{}`",
                storage.name()
            )));
        }
    };
    if layout == SampleLayout::Planar {
        let per_plane = per_channel
            .checked_mul(width)
            .ok_or_else(|| CodecError::failed("a source frame's channel size"))?;
        for channel in 0..channels {
            let Some(source) = bytes.get(channel * per_plane..(channel + 1) * per_plane) else {
                return Err(CodecError::refused("a source frame's channel data"));
            };
            let destination = coded.data_mut(channel);
            let Some(slot) = destination.get_mut(..per_plane) else {
                return Err(CodecError::failed("a destination frame's channel data"));
            };
            slot.copy_from_slice(source);
        }
    } else {
        let needed = per_channel
            .checked_mul(channels)
            .and_then(|total| total.checked_mul(width))
            .ok_or_else(|| CodecError::failed("a source frame's sample count"))?;
        let Some(source) = bytes.get(..needed) else {
            return Err(CodecError::refused("a source frame's sample count"));
        };
        let destination = coded.data_mut(0);
        let Some(slot) = destination.get_mut(..needed) else {
            return Err(CodecError::failed("a destination frame's sample buffer"));
        };
        slot.copy_from_slice(source);
    }
    Ok(())
}

/// The native-endian bytes of one signed 16-bit sample buffer.
fn to_bytes(values: &[i16]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_ne_bytes())
        .collect()
}

/// The native-endian bytes of one signed 32-bit sample buffer.
fn to_bytes_i32(values: &[i32]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_ne_bytes())
        .collect()
}

/// The native-endian bytes of one 32-bit float sample buffer.
fn to_bytes_f32(values: &[f32]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_ne_bytes())
        .collect()
}

/// The native-endian bytes of one 64-bit float sample buffer.
fn to_bytes_f64(values: &[f64]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_ne_bytes())
        .collect()
}

impl Encoder for LibavEncoder {
    fn send(&mut self, frame: Frame) -> Result<(), CodecError> {
        if self.draining {
            return Err(CodecError::refused("send after finish"));
        }
        let implementation = self.implementation.as_str();
        let codec = self.parameters.codec();
        match (&mut self.opened, frame) {
            (Opened::Video(encoder), Frame::Video(video)) => {
                send_video(encoder, implementation, video)
            }
            (Opened::Audio(encoder), Frame::Audio(audio)) => {
                send_audio(encoder, implementation, codec, audio)
            }
            (Opened::Video(_), _) => Err(CodecError::refused("an audio frame for a video encoder")),
            (Opened::Audio(_), _) => Err(CodecError::refused("a video frame for an audio encoder")),
        }
    }

    fn receive(&mut self) -> Result<Option<Packet<'static>>, CodecError> {
        loop {
            let mut coded = ffmpeg_next::Packet::empty();
            let result = match &mut self.opened {
                Opened::Video(encoder) => encoder.receive_packet(&mut coded),
                Opened::Audio(encoder) => encoder.receive_packet(&mut coded),
            };
            match result {
                Ok(()) => {}
                Err(error) if try_again(&error) || drained(&error) => return Ok(None),
                Err(error) => {
                    return Err(CodecError::failed(format!(
                        "libav encoder `{}` receive: {error}",
                        self.implementation
                    )));
                }
            }
            // A zero-length packet carries no coded bytes (the FLAC encoder
            // emits one at flush); the seam's Packet is an access unit.
            let Some(data) = coded.data().filter(|data| !data.is_empty()) else {
                continue;
            };
            let mut packet = Packet::owned(data.to_vec()).with_key(coded.is_key());
            if let Some(pts) = coded.pts() {
                packet = packet.with_pts(pts);
            }
            if let Some(dts) = coded.dts() {
                packet = packet.with_dts(dts);
            }
            return Ok(Some(packet));
        }
    }

    fn finish(&mut self) -> Result<(), CodecError> {
        if self.draining {
            return Ok(());
        }
        self.draining = true;
        let result = match &mut self.opened {
            Opened::Video(encoder) => encoder.send_eof(),
            Opened::Audio(encoder) => encoder.send_eof(),
        };
        match result {
            Ok(()) => Ok(()),
            Err(error) if try_again(&error) => Ok(()),
            Err(error) => Err(CodecError::failed(format!(
                "libav encoder `{}` finish: {error}",
                self.implementation
            ))),
        }
    }

    fn parameters(&self) -> Option<&CodecParameters> {
        Some(&self.parameters)
    }
}
