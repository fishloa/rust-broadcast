//! The seam contract: object safety, checked constructors, a PCM round trip,
//! the drain rule and reset.

use std::collections::VecDeque;
use std::num::{NonZeroU16, NonZeroU32, NonZeroUsize};

use broadcast_codec::{
    AudioFrame, AudioParameters, AudioSamples, ChannelLayout, CodecError, CodecId, CodecParameters,
    ContentLightLevel, Decoder, Encoder, EncoderConfig, Frame, ItuT35, MasteringDisplay, Packet,
    PixelFormat, SampleFormat, SampleLayout, VideoFrame, VideoPlane, VideoStorage,
};
use broadcast_core::timescale::Timescale;

const BYTES_PER_SAMPLE: usize = 2;

struct PcmDecoder {
    params: CodecParameters,
    sample_rate: NonZeroU32,
    channels: ChannelLayout,
    ready: VecDeque<Frame>,
    finished: bool,
}

impl PcmDecoder {
    fn new(sample_rate: NonZeroU32, channels: ChannelLayout) -> Self {
        let params = CodecParameters::new(CodecId::Pcm, Timescale::from_nonzero(sample_rate))
            .with_audio(AudioParameters::new(sample_rate, channels));
        Self {
            params,
            sample_rate,
            channels,
            ready: VecDeque::new(),
            finished: false,
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
        self.finished = true;
        Ok(())
    }

    fn reset(&mut self) {
        self.ready.clear();
        self.finished = false;
    }
}

struct PcmEncoder {
    params: Option<CodecParameters>,
    sample_rate: NonZeroU32,
    channels: ChannelLayout,
    ready: VecDeque<Packet<'static>>,
}

impl PcmEncoder {
    fn new(sample_rate: NonZeroU32, channels: ChannelLayout) -> Self {
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

/// A decoder that holds up to `capacity` packets before emitting, and holds
/// every remaining one until `finish`.
struct DelayingDecoder {
    params: CodecParameters,
    sample_rate: NonZeroU32,
    held: VecDeque<Frame>,
    capacity: usize,
    finished: bool,
}

impl DelayingDecoder {
    fn new(sample_rate: NonZeroU32, capacity: usize) -> Self {
        let params = CodecParameters::new(CodecId::Pcm, Timescale::from_nonzero(sample_rate))
            .with_audio(AudioParameters::new(sample_rate, ChannelLayout::Mono));
        Self {
            params,
            sample_rate,
            held: VecDeque::new(),
            capacity,
            finished: false,
        }
    }
}

impl Decoder for DelayingDecoder {
    fn parameters(&self) -> &CodecParameters {
        &self.params
    }

    fn send(&mut self, packet: Packet<'_>) -> Result<(), CodecError> {
        if self.held.len() >= self.capacity {
            return Err(CodecError::Full);
        }
        let sample = packet.data().first().copied().unwrap_or(0);
        let mut frame = AudioFrame::new(
            self.sample_rate,
            ChannelLayout::Mono,
            SampleLayout::Interleaved,
            AudioSamples::S16(vec![i16::from(sample)]),
        )
        .ok_or(CodecError::refused(
            "sample count not divisible by channels",
        ))?;
        if let Some(pts) = packet.pts() {
            frame = frame.with_pts(pts);
        }
        self.held.push_back(Frame::Audio(frame));
        Ok(())
    }

    fn receive(&mut self) -> Result<Option<Frame>, CodecError> {
        if self.finished || self.held.len() >= self.capacity {
            return Ok(self.held.pop_front());
        }
        Ok(None)
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

#[test]
fn traits_are_object_safe_and_usable() {
    let sample_rate = NonZeroU32::new(48_000).unwrap();
    let mut decoder: Box<dyn Decoder> = Box::new(PcmDecoder::new(sample_rate, ChannelLayout::Mono));
    let mut encoder: Box<dyn Encoder> = Box::new(PcmEncoder::new(sample_rate, ChannelLayout::Mono));

    decoder.send(Packet::borrowed(&[0, 0, 1, 0])).unwrap();
    let frame = decoder.receive().unwrap().unwrap();
    encoder.send(frame).unwrap();
    let packet = encoder.receive().unwrap().unwrap();
    assert_eq!(packet.data(), &[0, 0, 1, 0]);
    assert!(encoder.parameters().is_some());
}

#[test]
fn audio_constructor_refuses_partial_frames() {
    let sample_rate = NonZeroU32::new(48_000).unwrap();
    let samples = AudioSamples::S16(vec![0, 0, 0]);
    assert!(
        AudioFrame::new(
            sample_rate,
            ChannelLayout::Stereo,
            SampleLayout::Interleaved,
            samples,
        )
        .is_none()
    );

    let frame = AudioFrame::new(
        sample_rate,
        ChannelLayout::Stereo,
        SampleLayout::Planar,
        AudioSamples::S16(vec![0, 0, 0, 0]),
    )
    .unwrap();
    assert_eq!(frame.frames(), 2);
    assert_eq!(frame.samples().len(), 4);
}

#[test]
fn video_constructor_refuses_inconsistent_storage() {
    let width = NonZeroU32::new(16).unwrap();
    let height = NonZeroU32::new(16).unwrap();
    let luma = VideoPlane::new(vec![0; 16 * 16], NonZeroUsize::new(16).unwrap());
    let chroma = VideoPlane::new(vec![0; 8 * 8], NonZeroUsize::new(8).unwrap());

    assert!(
        VideoFrame::new(
            width,
            height,
            PixelFormat::Yuv420p8,
            VideoStorage::Cpu(vec![luma.clone()]),
        )
        .is_none()
    );

    let small = VideoPlane::new(vec![0; 16 * 16 - 1], NonZeroUsize::new(16).unwrap());
    assert!(
        VideoFrame::new(
            width,
            height,
            PixelFormat::Yuv420p8,
            VideoStorage::Cpu(vec![small, chroma.clone(), chroma.clone()]),
        )
        .is_none()
    );

    let narrow = VideoPlane::new(vec![0; 16 * 16], NonZeroUsize::new(8).unwrap());
    assert!(
        VideoFrame::new(
            width,
            height,
            PixelFormat::Yuv420p8,
            VideoStorage::Cpu(vec![narrow, chroma.clone(), chroma.clone()]),
        )
        .is_none()
    );

    let frame = VideoFrame::new(
        width,
        height,
        PixelFormat::Yuv420p8,
        VideoStorage::Cpu(vec![luma, chroma.clone(), chroma]),
    )
    .unwrap();
    assert_eq!(frame.width(), width);
    assert_eq!(frame.format().plane_count(), 3);
}

#[test]
fn zero_sizes_are_unrepresentable() {
    assert!(NonZeroU32::new(0).is_none());
    assert!(NonZeroU16::new(0).is_none());
    assert!(NonZeroUsize::new(0).is_none());
}

#[test]
fn pcm_pair_round_trips() {
    let sample_rate = NonZeroU32::new(48_000).unwrap();
    let mut input = Vec::new();
    for index in 0..64 {
        input.extend_from_slice(&(index as i16).to_le_bytes());
    }
    let mut decoder = PcmDecoder::new(sample_rate, ChannelLayout::Mono);
    let mut encoder = PcmEncoder::new(sample_rate, ChannelLayout::Mono);
    let mut output = Vec::new();

    for chunk in input.chunks(8) {
        decoder.send(Packet::borrowed(chunk)).unwrap();
        while let Some(frame) = decoder.receive().unwrap() {
            encoder.send(frame).unwrap();
            while let Some(packet) = encoder.receive().unwrap() {
                output.extend_from_slice(packet.data());
            }
        }
    }
    decoder.finish().unwrap();
    encoder.finish().unwrap();
    assert_eq!(output, input);
}

#[test]
fn delayed_decoder_loses_nothing_at_finish() {
    let sample_rate = NonZeroU32::new(48_000).unwrap();
    let mut decoder = DelayingDecoder::new(sample_rate, 3);
    for pts in 0..3i64 {
        decoder.send(Packet::borrowed(&[1]).with_pts(pts)).unwrap();
    }
    assert!(decoder.receive().unwrap().is_some());
    assert!(decoder.receive().unwrap().is_none());

    decoder.finish().unwrap();
    let mut frames = 1;
    while decoder.receive().unwrap().is_some() {
        frames += 1;
    }
    assert_eq!(frames, 3);
}

#[test]
fn reset_drops_held_state() {
    let sample_rate = NonZeroU32::new(48_000).unwrap();
    let mut decoder = DelayingDecoder::new(sample_rate, 8);
    decoder.send(Packet::borrowed(&[1])).unwrap();
    decoder.send(Packet::borrowed(&[2])).unwrap();

    decoder.reset();
    decoder.finish().unwrap();
    assert!(decoder.receive().unwrap().is_none());

    decoder.reset();
    decoder.send(Packet::borrowed(&[3])).unwrap();
    decoder.finish().unwrap();
    assert!(decoder.receive().unwrap().is_some());
    assert!(decoder.receive().unwrap().is_none());
}

#[test]
fn high_bit_depth_and_hdr_metadata_are_carried() {
    let width = NonZeroU32::new(16).unwrap();
    let height = NonZeroU32::new(16).unwrap();
    let luma = VideoPlane::new(vec![0; 16 * 16 * 2], NonZeroUsize::new(32).unwrap());
    let chroma = VideoPlane::new(vec![0; 8 * 8 * 2], NonZeroUsize::new(16).unwrap());

    let frame = VideoFrame::new(
        width,
        height,
        PixelFormat::Yuv420p12,
        VideoStorage::Cpu(vec![luma, chroma.clone(), chroma]),
    )
    .unwrap()
    .with_content_light(ContentLightLevel::new(1_000, 400))
    .with_mastering_display(MasteringDisplay::new(
        [[1, 2], [3, 4], [5, 6]],
        [7, 8],
        1_000,
        1,
    ))
    .with_itu_t35(ItuT35::new(0xB5, 0, vec![1, 2, 3]));

    assert_eq!(frame.format(), PixelFormat::Yuv420p12);
    assert_eq!(frame.format().plane_size(16, 16, 0), Some(512));
    assert_eq!(frame.format().plane_size(16, 16, 1), Some(128));
    assert_eq!(PixelFormat::Gray12.plane_size(16, 16, 0), Some(512));
    assert_eq!(
        frame.content_light().unwrap().max_content_light_level(),
        1_000
    );
    assert_eq!(frame.mastering_display().unwrap().max_luminance(), 1_000);
    assert_eq!(frame.itu_t35().len(), 1);
    assert_eq!(frame.itu_t35()[0].payload(), &[1, 2, 3]);
}

#[test]
fn encoder_target_storage_is_carried() {
    let video = EncoderConfig::new(CodecId::H264, Timescale::new(25).unwrap())
        .with_pixel_format(PixelFormat::Yuv420p10);
    assert_eq!(video.pixel_format(), Some(PixelFormat::Yuv420p10));
    assert_eq!(video.sample_format(), None);
    assert_eq!(video.sample_layout(), None);

    let audio = EncoderConfig::new(CodecId::Flac, Timescale::new(48_000).unwrap())
        .with_sample_format(SampleFormat::S16, SampleLayout::Interleaved);
    assert_eq!(audio.sample_format(), Some(SampleFormat::S16));
    assert_eq!(audio.sample_layout(), Some(SampleLayout::Interleaved));
    assert_eq!(audio.pixel_format(), None);
}
