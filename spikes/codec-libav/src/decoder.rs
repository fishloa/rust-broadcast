//! One libav decoder behind [`broadcast_codec::Decoder`].

use std::num::{NonZeroU32, NonZeroUsize};
use std::string::String;
use std::vec::Vec;

use broadcast_codec::{
    AudioFrame, AudioSamples, CodecError, CodecParameters, Decoder, Frame, Packet, SampleLayout,
    VideoFrame, VideoPlane, VideoStorage,
};
use ffmpeg_next::codec::decoder::Opened;
use ffmpeg_next::{Dictionary, Rational};

use crate::framing::Inbound;
use crate::map;
use crate::sys;

/// A decoder over one libav codec context.
///
/// It is built for a [`CodecParameters`] and an implementation name
/// [`LibavDecoder::open`] or the provider hands it; the seam's `send`/
/// `receive`/`finish`/`reset` map one to one onto libav's own calls. Nothing
/// here holds or reorders frames: libav's decoder delay is libav's.
pub struct LibavDecoder {
    parameters: CodecParameters,
    opened: Opened,
    implementation: String,
    inbound: Inbound,
    first_packet: bool,
    draining: bool,
}

impl LibavDecoder {
    /// Open libav's decoder called `implementation` for `parameters`.
    ///
    /// # Errors
    /// [`CodecError::Unsupported`] when the codec has no libav mapping, when
    /// `parameters` describes neither audio nor video, or when no decoder is
    /// registered under `implementation`; [`CodecError::Refused`] when libav
    /// will not open it, naming its own error text.
    pub fn open(implementation: &str, parameters: &CodecParameters) -> Result<Self, CodecError> {
        if parameters.video().is_none() && parameters.audio().is_none() {
            return Err(CodecError::unsupported(
                "codec parameters that are neither audio nor video",
            ));
        }
        if map::libav_id(parameters.codec()).is_none() {
            return Err(CodecError::unsupported(format!(
                "codec `{}`",
                parameters.codec().name()
            )));
        }
        let codec = ffmpeg_next::codec::decoder::find_by_name(implementation)
            .ok_or_else(|| CodecError::unsupported(format!("libav decoder `{implementation}`")))?;
        let inbound = Inbound::new(parameters.codec(), parameters.extradata());
        let context = ffmpeg_next::codec::Context::new_with_codec(codec);
        let mut decoder = context.decoder();
        let time_base = parameters.time_base().get();
        let denominator = i32::try_from(time_base).map_err(|_| {
            CodecError::unsupported(format!("time base `{time_base}` as a libav time base"))
        })?;
        decoder.set_packet_time_base(Rational::new(1, denominator));
        let mut options = Dictionary::new();
        for (key, value) in inbound.options() {
            options.set(key, &value);
        }
        let opened = decoder.open_as_with(codec, options).map_err(|error| {
            CodecError::refused(format!("libav decoder `{implementation}`: {error}"))
        })?;
        Ok(Self {
            parameters: parameters.clone(),
            opened,
            implementation: implementation.to_string(),
            inbound,
            first_packet: true,
            draining: false,
        })
    }

    /// The libav implementation this decoder runs.
    #[must_use]
    pub fn implementation(&self) -> &str {
        &self.implementation
    }

    /// Copy one received audio frame into the seam's storage.
    fn audio_frame(&self, audio: &ffmpeg_next::frame::Audio) -> Result<AudioFrame, CodecError> {
        let rate = NonZeroU32::new(audio.rate())
            .ok_or_else(|| CodecError::failed("a decoded audio frame without a sample rate"))?;
        let channels = map::channel_layout(audio.channel_layout());
        let sample = audio.format();
        let (storage, layout) = map::sample_format(sample).ok_or_else(|| {
            CodecError::unsupported(format!("libav sample format `{}`", sample.name()))
        })?;
        let per_channel = audio.samples();
        let channel_count = channels.count();
        let samples = read_samples(audio, storage, layout, per_channel, channel_count)?;
        let frame = AudioFrame::new(rate, channels, layout, samples)
            .ok_or_else(|| CodecError::failed("a decoded audio frame's sample count"))?;
        Ok(match audio.pts() {
            Some(pts) => frame.with_pts(pts),
            None => frame,
        })
    }

    /// Copy one received picture plane by plane into the seam's storage.
    fn video_frame(&self, video: &ffmpeg_next::frame::Video) -> Result<VideoFrame, CodecError> {
        let format = map::pixel_format(video.format()).ok_or_else(|| {
            CodecError::unsupported(format!("libav pixel format `{:?}`", video.format()))
        })?;
        let (width, height) = map::coded_size(video.width(), video.height())
            .ok_or_else(|| CodecError::failed("a decoded picture without a size"))?;
        let mut planes = Vec::with_capacity(format.plane_count());
        for index in 0..format.plane_count() {
            let stride = video.stride(index);
            let rows = usize::try_from(video.plane_height(index))
                .map_err(|_| CodecError::failed("a decoded picture's plane height"))?;
            let visible = format
                .plane_size(
                    usize::try_from(width.get()).unwrap_or(usize::MAX),
                    usize::try_from(height.get()).unwrap_or(usize::MAX),
                    index,
                )
                .ok_or_else(|| CodecError::failed("a decoded picture's plane size"))?;
            if rows == 0 || !visible.is_multiple_of(rows) || stride < visible / rows {
                return Err(CodecError::failed("a decoded picture's plane geometry"));
            }
            let row_bytes = visible / rows;
            let plane_bytes = stride
                .checked_mul(rows)
                .ok_or_else(|| CodecError::failed("a decoded picture's plane size"))?;
            let mut data = vec![0u8; plane_bytes];
            let source = video.data(index);
            for row in 0..rows {
                let from = row * stride;
                let to = &mut data[from..from + row_bytes];
                to.copy_from_slice(&source[from..from + row_bytes]);
            }
            let line_size = NonZeroUsize::new(stride)
                .ok_or_else(|| CodecError::failed("a decoded picture's line size"))?;
            planes.push(VideoPlane::new(data, line_size));
        }
        let frame = VideoFrame::new(width, height, format, VideoStorage::Cpu(planes))
            .ok_or_else(|| CodecError::failed("a decoded picture's plane layout"))?;
        let frame = match video.pts() {
            Some(pts) => frame.with_pts(pts),
            None => frame,
        };
        let frame = if video.is_key() {
            frame.with_key(true)
        } else {
            frame
        };
        Ok(match map::colour(video) {
            Some(colour) => frame.with_colour(colour),
            None => frame,
        })
    }
}

/// Read `per_channel` samples across `channels` out of `audio`.
fn read_samples(
    audio: &ffmpeg_next::frame::Audio,
    storage: broadcast_codec::SampleFormat,
    layout: SampleLayout,
    per_channel: usize,
    channels: usize,
) -> Result<AudioSamples, CodecError> {
    use broadcast_codec::SampleFormat;
    let total = per_channel
        .checked_mul(channels)
        .ok_or_else(|| CodecError::failed("a decoded audio frame's sample count"))?;
    let width = match storage {
        SampleFormat::U8 => 1,
        SampleFormat::S16 => 2,
        SampleFormat::S32 | SampleFormat::F32 => 4,
        SampleFormat::F64 => 8,
        _ => return Err(CodecError::unsupported("a decoded audio sample format")),
    };
    let mut bytes: Vec<u8> = Vec::with_capacity(total * width);
    if layout == SampleLayout::Planar {
        let per_plane = per_channel
            .checked_mul(width)
            .ok_or_else(|| CodecError::failed("a decoded audio frame's channel size"))?;
        for index in 0..channels {
            let plane = audio
                .data(index)
                .get(..per_plane)
                .ok_or_else(|| CodecError::failed("a decoded audio frame's channel plane"))?;
            bytes.extend_from_slice(plane);
        }
    } else {
        let needed = total
            .checked_mul(width)
            .ok_or_else(|| CodecError::failed("a decoded audio frame's sample count"))?;
        let plane = audio
            .data(0)
            .get(..needed)
            .ok_or_else(|| CodecError::failed("a decoded audio frame's sample buffer"))?;
        bytes.extend_from_slice(plane);
    }
    Ok(match storage {
        SampleFormat::U8 => AudioSamples::U8(bytes),
        SampleFormat::S16 => AudioSamples::S16(elements(&bytes, i16::from_ne_bytes)),
        SampleFormat::S32 => AudioSamples::S32(elements(&bytes, i32::from_ne_bytes)),
        SampleFormat::F32 => AudioSamples::F32(elements(&bytes, f32::from_ne_bytes)),
        SampleFormat::F64 => AudioSamples::F64(elements(&bytes, f64::from_ne_bytes)),
        _ => return Err(CodecError::unsupported("a decoded audio sample format")),
    })
}

/// Decode `bytes` as consecutive `WIDTH`-byte native-endian values.
fn elements<const WIDTH: usize, T>(bytes: &[u8], from_bytes: impl Fn([u8; WIDTH]) -> T) -> Vec<T> {
    bytes
        .chunks_exact(WIDTH)
        .map(|chunk| {
            let mut value = [0u8; WIDTH];
            value.copy_from_slice(chunk);
            from_bytes(value)
        })
        .collect()
}

impl Decoder for LibavDecoder {
    fn parameters(&self) -> &CodecParameters {
        &self.parameters
    }

    fn send(&mut self, packet: Packet<'_>) -> Result<(), CodecError> {
        if self.draining {
            return Err(CodecError::refused("send after finish; reset first"));
        }
        if packet.data().is_empty() {
            return Err(CodecError::refused("an empty access unit"));
        }
        let with_parameter_sets = self.first_packet || packet.key();
        let data = self.inbound.prepare(packet.data(), with_parameter_sets)?;
        let mut coded = ffmpeg_next::Packet::copy(data.as_ref());
        coded.set_pts(packet.pts());
        coded.set_dts(packet.dts());
        if packet.key() {
            coded.set_flags(ffmpeg_next::packet::Flags::KEY);
        }
        self.first_packet = false;
        match self.opened.send_packet(&coded) {
            Ok(()) => Ok(()),
            Err(error) if try_again(&error) => Err(CodecError::Full),
            Err(error) => Err(CodecError::failed(format!(
                "libav decoder `{}` send: {error}",
                self.implementation
            ))),
        }
    }

    fn receive(&mut self) -> Result<Option<Frame>, CodecError> {
        let mut decoded = sys::empty_frame();
        match self.opened.receive_frame(&mut decoded) {
            Ok(()) => {}
            Err(error) if try_again(&error) || drained(&error) => return Ok(None),
            Err(error) => {
                return Err(CodecError::failed(format!(
                    "libav decoder `{}` receive: {error}",
                    self.implementation
                )));
            }
        }
        let frame = match self.opened.medium() {
            ffmpeg_next::media::Type::Video => {
                let video = ffmpeg_next::frame::Video::from(decoded);
                Frame::Video(self.video_frame(&video)?)
            }
            ffmpeg_next::media::Type::Audio => {
                let audio = ffmpeg_next::frame::Audio::from(decoded);
                Frame::Audio(self.audio_frame(&audio)?)
            }
            _ => {
                return Err(CodecError::unsupported(
                    "a decoded frame of a medium the seam does not carry",
                ));
            }
        };
        Ok(Some(frame))
    }

    fn finish(&mut self) -> Result<(), CodecError> {
        if self.draining {
            return Ok(());
        }
        self.draining = true;
        match self.opened.send_eof() {
            Ok(()) => Ok(()),
            Err(error) if try_again(&error) => Ok(()),
            Err(error) => Err(CodecError::failed(format!(
                "libav decoder `{}` finish: {error}",
                self.implementation
            ))),
        }
    }

    fn reset(&mut self) {
        self.opened.flush();
        self.first_packet = true;
        self.draining = false;
    }
}

/// Whether a libav call wants the caller to try again after more output.
pub(crate) fn try_again(error: &ffmpeg_next::Error) -> bool {
    matches!(error, ffmpeg_next::Error::Other { errno } if *errno == ffmpeg_next::error::EAGAIN)
}

/// Whether a libav call reports the stream as drained.
pub(crate) fn drained(error: &ffmpeg_next::Error) -> bool {
    matches!(error, ffmpeg_next::Error::Eof)
}
