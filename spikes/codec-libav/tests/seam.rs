//! The libav adapter's tests, all through a `Registry`.

use std::num::{NonZeroU16, NonZeroU32};
use std::process::Command;
use std::time::Duration;

use broadcast_codec::{
    AudioParameters, AudioSamples, ChannelLayout, CodecError, CodecId, CodecParameters, Decoder,
    Direction, Encoder, Frame, Packet, PixelFormat, Registry, SampleFormat, SampleLayout,
    VideoFrame, VideoStorage,
};
use broadcast_core::Serialize;
use broadcast_loudness::LoudnessMeter;
use codec_libav::LibavProvider;
use test_fixtures::process::{output_bounded, tool_available};
use transmux::{CodecConfig, TrackSpec};

/// The committed TS fixture with one H.264 and one AAC track.
const TS_FIXTURE: &str = "ts/h264-aac.ts";
/// The committed AAC-LC fixture (mono, 48 kHz).
const ADTS_FIXTURE: &str = "audio/aac.adts";
/// The video frames `ffprobe -count_frames` reports for the TS fixture.
const TS_VIDEO_FRAMES: usize = 75;
/// The audio frames `ffprobe -count_frames` reports for the TS fixture.
const TS_AUDIO_FRAMES: usize = 131;
/// The coded picture size of the TS fixture.
const TS_WIDTH: u32 = 320;
/// The coded picture height of the TS fixture.
const TS_HEIGHT: u32 = 240;
/// Samples one AAC-LC access unit decodes to.
const AAC_FRAME_SAMPLES: usize = 1024;
/// Integrated loudness the symphonia spike and ffmpeg measure for the ADTS
/// fixture.
const EXPECTED_LUFS: f64 = -21.1;
/// Allowed loudness difference, in LU.
const TOLERANCE_LU: f64 = 0.1;
/// Deadline for one external ffmpeg run.
const FFMPEG_DEADLINE: Duration = Duration::from_secs(120);

/// Bytes per packed 8-bit 4:2:0 pixel: one luma plus a half of each chroma.
const YUV420P_FRAME_BYTES: usize = 3;
/// The sample rate the profile test builds its AAC stream at.
const PROBE_RATE: u32 = 48_000;
/// Extra bytes one synthetic padded plane carries per row.
const PLANE_PADDING: usize = 32;
/// The media timescale the TS fixture states.
const TS_TIMESCALE: u32 = 90_000;
/// The MPEG-2 video bit rate the round trip asks for, in bits per second.
const MPEG2_BIT_RATE: u64 = 5_000_000;
/// The MPEG-2 video group length the round trip asks for, in pictures.
const MPEG2_GOP: u32 = 25;
/// Ticks per second the MPEG-2 encoder is configured for: one per picture.
const MPEG2_TICKS: u32 = 25;
/// The committed PCM fixture (mono, 48 kHz, signed 16-bit).
const PCM_FIXTURE: &str = "audio/pcm-s16le.wav";
/// Its sample rate.
const PCM_SAMPLE_RATE: u32 = 48_000;
/// Samples per frame handed to the FLAC encoder.
const FLAC_BLOCK_SAMPLES: usize = 4_608;
/// Bytes of a RIFF/WAVE header before the first chunk.
const RIFF_HEADER_LEN: usize = 12;
/// Bytes of a chunk id and its size field.
const CHUNK_HEADER_LEN: usize = 8;
/// Bytes of a chunk id.
const CHUNK_ID_LEN: usize = 4;

/// One coded access unit, ready to send.
#[derive(Debug, Clone)]
struct Unit {
    data: Vec<u8>,
    pts: Option<i64>,
    dts: Option<i64>,
    key: bool,
}

/// A registry holding only the libav provider, or `None` when libav offers
/// nothing for H.264 here.
fn registry() -> Option<Registry> {
    let registry = Registry::new()
        .with(Box::new(LibavProvider::new()))
        .expect("the libav provider registers");
    if registry
        .candidates(CodecId::H264, Direction::Decode)
        .is_empty()
    {
        eprintln!("skip: the linked libav offers no H.264 decoder here");
        return None;
    }
    Some(registry)
}

/// The channel layout of `count` channels.
fn layout(count: u16) -> ChannelLayout {
    match count {
        1 => ChannelLayout::Mono,
        2 => ChannelLayout::Stereo,
        6 => ChannelLayout::Surround5_1,
        8 => ChannelLayout::Surround7_1,
        other => match NonZeroU16::new(other) {
            Some(count) => ChannelLayout::Discrete(count),
            None => ChannelLayout::Mono,
        },
    }
}

/// The seam parameters a demuxed track describes.
fn parameters_of(spec: &TrackSpec) -> Option<CodecParameters> {
    match &spec.config {
        CodecConfig::Avc {
            config,
            width,
            height,
        } => {
            let mut extradata = vec![0u8; config.config.serialized_len()];
            config.config.serialize_into(&mut extradata).ok()?;
            Some(
                CodecParameters::new(CodecId::H264, spec.timescale)
                    .with_video(broadcast_codec::VideoParameters::new(
                        NonZeroU32::new(u32::from(*width))?,
                        NonZeroU32::new(u32::from(*height))?,
                    ))
                    .with_extradata(extradata),
            )
        }
        CodecConfig::Aac {
            esds,
            channel_count,
            sample_rate,
            ..
        } => {
            let extradata = esds.decoder_specific_info_data()?.to_vec();
            Some(
                CodecParameters::new(CodecId::Aac, spec.timescale)
                    .with_audio(AudioParameters::new(
                        NonZeroU32::new(*sample_rate)?,
                        layout(*channel_count),
                    ))
                    .with_extradata(extradata),
            )
        }
        _ => None,
    }
}

/// Whether the track is the one wanted: H.264 for `video`, AAC otherwise.
fn wanted(spec: &TrackSpec, video: bool) -> bool {
    matches!(
        (&spec.config, video),
        (CodecConfig::Avc { .. }, true) | (CodecConfig::Aac { .. }, false)
    )
}

/// Demux the TS fixture and return the wanted track's parameters and access
/// units, in decode order.
fn ts_track(video: bool) -> Option<(CodecParameters, Vec<Unit>)> {
    let bytes = test_fixtures::read(TS_FIXTURE);
    let media = test_harness::collect::media_from_ts(&bytes).ok()?;
    let track = media
        .tracks
        .iter()
        .find(|track| wanted(&track.spec, video))?;
    let parameters = parameters_of(&track.spec)?;
    let units = track
        .samples
        .iter()
        .map(|sample| {
            let (dts, pts) = match sample.timing {
                Some(transmux::ir::SampleTiming::Timed { dts, pts, .. }) => (Some(dts), Some(pts)),
                _ => (None, None),
            };
            Unit {
                data: sample.data.to_vec(),
                pts,
                dts,
                key: sample.flags.is_sync,
            }
        })
        .collect();
    Some((parameters, units))
}

/// Drive one decoder over `units` and return every frame it produced.
fn decode(decoder: &mut dyn Decoder, units: &[Unit]) -> Vec<Frame> {
    let mut frames = Vec::new();
    for unit in units {
        let mut packet = Packet::borrowed(&unit.data).with_key(unit.key);
        if let Some(pts) = unit.pts {
            packet = packet.with_pts(pts);
        }
        if let Some(dts) = unit.dts {
            packet = packet.with_dts(dts);
        }
        loop {
            match decoder.send(packet.clone()) {
                Ok(()) => break,
                Err(CodecError::Full) => drain(decoder, &mut frames),
                Err(error) => panic!("send: {error}"),
            }
        }
        drain(decoder, &mut frames);
    }
    decoder.finish().expect("finish");
    drain(decoder, &mut frames);
    frames
}

/// Drain every ready frame.
fn drain(decoder: &mut dyn Decoder, frames: &mut Vec<Frame>) {
    while let Some(frame) = decoder.receive().expect("receive") {
        frames.push(frame);
    }
}

/// The video half of a frame.
fn video(frame: &Frame) -> &VideoFrame {
    match frame {
        Frame::Video(video) => video,
        other => panic!("expected a picture, got {other:?}"),
    }
}

/// The audio half of a frame.
fn audio(frame: &Frame) -> &broadcast_codec::AudioFrame {
    match frame {
        Frame::Audio(audio) => audio,
        other => panic!("expected an audio frame, got {other:?}"),
    }
}

/// FNV-1a offset basis.
const HASH_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
/// FNV-1a prime.
const HASH_PRIME: u64 = 0x0000_0100_0000_01b3;

/// Fold `bytes` into `hash`.
fn hash_bytes(hash: &mut u64, bytes: &[u8]) {
    for byte in bytes {
        *hash ^= u64::from(*byte);
        *hash = hash.wrapping_mul(HASH_PRIME);
    }
}

/// One picture's visible pixels, plane by plane, as a hash.
fn hash_visible(frame: &VideoFrame) -> u64 {
    let format = frame.format();
    let width = usize::try_from(frame.width().get()).expect("width");
    let height = usize::try_from(frame.height().get()).expect("height");
    let VideoStorage::Cpu(planes) = frame.storage() else {
        panic!("hardware storage");
    };
    let mut hash = HASH_OFFSET;
    for (index, plane) in planes.iter().enumerate() {
        let size = format.plane_size(width, height, index).expect("plane size");
        let stride = plane.stride().get();
        let rows = plane.data().len() / stride;
        let row_bytes = size / rows;
        for row in 0..rows {
            let from = row * stride;
            hash_bytes(&mut hash, &plane.data()[from..from + row_bytes]);
        }
    }
    hash
}

/// One tightly packed raw picture's hash, in the same plane order.
fn hash_raw(format: PixelFormat, width: usize, height: usize, bytes: &[u8]) -> u64 {
    let mut hash = HASH_OFFSET;
    let mut offset = 0;
    let mut index = 0;
    while let Some(size) = format.plane_size(width, height, index) {
        hash_bytes(&mut hash, &bytes[offset..offset + size]);
        offset += size;
        index += 1;
    }
    hash
}

/// The pictures `ffmpeg` itself decodes from the fixture, tightly packed.
fn ffmpeg_raw_pictures(width: usize, height: usize) -> Option<Vec<Vec<u8>>> {
    if !tool_available("ffmpeg", "-version") {
        return None;
    }
    let mut command = Command::new("ffmpeg");
    command
        .args(["-v", "error", "-i"])
        .arg(test_fixtures::path(TS_FIXTURE))
        .args(["-map", "0:v", "-f", "rawvideo", "-pix_fmt", "yuv420p", "-"]);
    let output = output_bounded(&mut command, FFMPEG_DEADLINE).ok()?;
    assert!(
        output.status.success(),
        "ffmpeg failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let frame_bytes = width * height * YUV420P_FRAME_BYTES / 2;
    Some(
        output
            .stdout
            .chunks_exact(frame_bytes)
            .map(<[u8]>::to_vec)
            .collect(),
    )
}

#[test]
fn t1_h264_decode_produces_every_picture_in_presentation_order() {
    let Some(registry) = registry() else {
        return;
    };
    let Some((parameters, units)) = ts_track(true) else {
        panic!("the fixture must carry a H.264 track");
    };
    let mut decoder = registry.decoder(&parameters).expect("an H.264 decoder");
    let frames = decode(&mut *decoder, &units);

    assert_eq!(frames.len(), TS_VIDEO_FRAMES, "decoded pictures");
    for frame in &frames {
        let picture = video(frame);
        assert_eq!(picture.width().get(), TS_WIDTH);
        assert_eq!(picture.height().get(), TS_HEIGHT);
    }
    let mut input: Vec<i64> = units.iter().filter_map(|unit| unit.pts).collect();
    let mut output: Vec<i64> = frames
        .iter()
        .filter_map(|frame| video(frame).pts())
        .collect();
    assert_eq!(input.len(), output.len(), "every picture keeps a timestamp");
    input.sort_unstable();
    let presented = output.clone();
    output.sort_unstable();
    assert_eq!(input, output, "presentation timestamps are the input's");
    assert!(
        presented.windows(2).all(|pair| pair[0] < pair[1]),
        "frames must come out in presentation order"
    );
}

#[test]
fn t2_visible_pixels_match_ffmpeg() {
    let Some(registry) = registry() else {
        return;
    };
    let Some((parameters, units)) = ts_track(true) else {
        panic!("the fixture must carry a H.264 track");
    };
    let Some(raw) = ffmpeg_raw_pictures(TS_WIDTH as usize, TS_HEIGHT as usize) else {
        panic!("ffmpeg must be on PATH for the pixel oracle");
    };
    let mut decoder = registry.decoder(&parameters).expect("an H.264 decoder");
    let frames = decode(&mut *decoder, &units);

    assert_eq!(frames.len(), raw.len(), "picture count against ffmpeg");
    let width = TS_WIDTH as usize;
    let height = TS_HEIGHT as usize;
    let mut matched = 0usize;
    for (index, (frame, raw)) in frames.iter().zip(&raw).enumerate() {
        let ours = hash_visible(video(frame));
        let theirs = hash_raw(PixelFormat::Yuv420p8, width, height, raw);
        assert_eq!(ours, theirs, "picture {index} differs from ffmpeg's");
        matched += 1;
    }
    println!("{matched} of {} frame hashes matched", raw.len());
}

#[test]
fn t3_aac_decode_and_loudness() {
    let Some(registry) = registry() else {
        return;
    };
    let Some((parameters, units)) = ts_track(false) else {
        panic!("the fixture must carry an AAC track");
    };
    let mut decoder = registry.decoder(&parameters).expect("an AAC decoder");
    let frames = decode(&mut *decoder, &units);
    assert_eq!(frames.len(), TS_AUDIO_FRAMES, "decoded audio frames");
    let samples: usize = frames.iter().map(|frame| audio(frame).frames()).sum();
    assert_eq!(samples, TS_AUDIO_FRAMES * AAC_FRAME_SAMPLES);

    measure_adts(registry);
}

/// Decode `audio/aac.adts` and measure it, the same reference the symphonia
/// spike uses.
fn measure_adts(registry: Registry) -> (f64, usize) {
    let data = test_fixtures::read(ADTS_FIXTURE);
    let packets: Vec<_> = codec_symphonia::adts::AdtsFrames::new(&data).collect();
    let first = packets.first().expect("the fixture carries ADTS frames");
    let rate = NonZeroU32::new(first.sample_rate()).expect("a sample rate");
    let channels = first.channels().expect("a channel layout");
    let parameters =
        CodecParameters::new(CodecId::Aac, broadcast_codec::Timescale::from_nonzero(rate))
            .with_audio(AudioParameters::new(rate, channels));
    let mut decoder = registry.decoder(&parameters).expect("an AAC decoder");
    let units: Vec<Unit> = packets
        .iter()
        .map(|packet| Unit {
            data: packet.bytes().to_vec(),
            pts: None,
            dts: None,
            key: true,
        })
        .collect();
    let frames = decode(&mut *decoder, &units);
    let meter_layout = match channels {
        ChannelLayout::Mono => broadcast_loudness::ChannelLayout::Mono,
        ChannelLayout::Stereo => broadcast_loudness::ChannelLayout::Stereo,
        ChannelLayout::Surround5_1 => broadcast_loudness::ChannelLayout::Surround51,
        other => panic!("no loudness layout for {other:?}"),
    };
    let mut meter = LoudnessMeter::new(rate.get(), meter_layout).expect("a meter");
    let mut decoded = 0usize;
    for frame in &frames {
        let audio = audio(frame);
        let AudioSamples::F32(values) = audio.samples() else {
            panic!("libav decoded AAC to a non-float format");
        };
        if audio.layout() != SampleLayout::Planar {
            panic!("libav decoded AAC to a non-planar layout");
        }
        let channel_count = audio.channels().count();
        let per_channel = audio.frames();
        for index in 0..per_channel {
            let mut sample = Vec::with_capacity(channel_count);
            for channel in 0..channel_count {
                sample.push(values[channel * per_channel + index]);
            }
            meter.push_f32(&sample).expect("a sample");
        }
        decoded += per_channel;
    }
    meter.finish();
    let measured = meter.integrated_lufs();
    println!("libav measured {measured:.3} LUFS over {decoded} samples");
    assert!(
        (measured - EXPECTED_LUFS).abs() <= TOLERANCE_LU,
        "integrated {measured:.3} LUFS, expected {EXPECTED_LUFS} +/- {TOLERANCE_LU}"
    );
    (measured, decoded)
}

#[test]
fn t3_adts_loudness_matches_the_pinned_value() {
    let Some(registry) = registry() else {
        return;
    };
    let (measured, decoded) = measure_adts(registry);
    assert!(decoded > 0);
    assert!((measured - EXPECTED_LUFS).abs() <= TOLERANCE_LU);
}

/// Drive one encoder over `frames` and return every packet it produced.
fn encode(encoder: &mut dyn Encoder, frames: Vec<Frame>) -> Vec<Packet<'static>> {
    let mut packets = Vec::new();
    for frame in frames {
        loop {
            match encoder.send(frame.clone()) {
                Ok(()) => break,
                Err(CodecError::Full) => drain_encoded(encoder, &mut packets),
                Err(error) => panic!("send: {error}"),
            }
        }
        drain_encoded(encoder, &mut packets);
    }
    encoder.finish().expect("finish");
    drain_encoded(encoder, &mut packets);
    packets
}

/// Drain every ready coded packet.
fn drain_encoded(encoder: &mut dyn Encoder, packets: &mut Vec<Packet<'static>>) {
    while let Some(packet) = encoder.receive().expect("receive") {
        packets.push(packet);
    }
}

#[test]
fn t4_video_encode_round_trip() {
    let Some(registry) = registry() else {
        return;
    };
    let Some((parameters, units)) = ts_track(true) else {
        panic!("the fixture must carry a H.264 track");
    };
    let mut decoder = registry.decoder(&parameters).expect("an H.264 decoder");
    let frames = decode(&mut *decoder, &units);
    assert_eq!(frames.len(), TS_VIDEO_FRAMES);

    let config = broadcast_codec::EncoderConfig::new(
        CodecId::Mpeg2Video,
        broadcast_codec::Timescale::from_nonzero(
            NonZeroU32::new(MPEG2_TICKS).expect("a non-zero timescale"),
        ),
    )
    .with_video(broadcast_codec::VideoParameters::new(
        NonZeroU32::new(TS_WIDTH).expect("a width"),
        NonZeroU32::new(TS_HEIGHT).expect("a height"),
    ))
    .with_pixel_format(PixelFormat::Yuv420p8)
    .with_bit_rate(std::num::NonZeroU64::new(MPEG2_BIT_RATE).expect("a bit rate"))
    .with_gop(NonZeroU32::new(MPEG2_GOP).expect("a group length"));
    let mut encoder = registry.encoder(&config).expect("an MPEG-2 video encoder");
    let sent = frames.len();
    let source = broadcast_codec::Timescale::from_nonzero(
        NonZeroU32::new(TS_TIMESCALE).expect("a non-zero timescale"),
    );
    let target = broadcast_codec::Timescale::from_nonzero(
        NonZeroU32::new(MPEG2_TICKS).expect("a non-zero timescale"),
    );
    let frames: Vec<Frame> = frames
        .into_iter()
        .map(|frame| {
            let Frame::Video(video) = frame else {
                panic!("a picture from the H.264 decoder");
            };
            match video.pts() {
                Some(pts) => {
                    let ticks = broadcast_core::timescale::rescale_i64(
                        pts,
                        source,
                        target,
                        broadcast_core::timescale::Rounding::Nearest,
                    );
                    Frame::Video(video.with_pts(ticks))
                }
                None => Frame::Video(video),
            }
        })
        .collect();
    let packets = encode(&mut *encoder, frames);
    assert!(!packets.is_empty(), "the encoder produced no packets");

    let reported = encoder.parameters().expect("parameters once encoding");
    assert_eq!(reported.codec(), CodecId::Mpeg2Video);
    let video_parameters = reported.video().expect("the video half");
    assert_eq!(video_parameters.coded_width().get(), TS_WIDTH);
    assert_eq!(video_parameters.coded_height().get(), TS_HEIGHT);

    let mut decoder = registry
        .decoder(reported)
        .expect("a decoder for what the encoder reports");
    let units: Vec<Unit> = packets
        .iter()
        .map(|packet| Unit {
            data: packet.data().to_vec(),
            pts: packet.pts(),
            dts: packet.dts(),
            key: packet.key(),
        })
        .collect();
    let round_tripped = decode(&mut *decoder, &units);
    assert_eq!(round_tripped.len(), sent, "frames in equal frames out");
    assert_eq!(round_tripped.len(), TS_VIDEO_FRAMES);
    for frame in &round_tripped {
        let picture = video(frame);
        assert_eq!(picture.width().get(), TS_WIDTH);
        assert_eq!(picture.height().get(), TS_HEIGHT);
    }
}

#[test]
fn t5_audio_encode_round_trip() {
    let Some(registry) = registry() else {
        return;
    };
    let samples = wav_samples(PCM_FIXTURE);
    let rate = NonZeroU32::new(PCM_SAMPLE_RATE).expect("a sample rate");
    let config = broadcast_codec::EncoderConfig::new(
        CodecId::Flac,
        broadcast_codec::Timescale::from_nonzero(rate),
    )
    .with_audio(AudioParameters::new(rate, ChannelLayout::Mono))
    .with_sample_format(SampleFormat::S16, SampleLayout::Interleaved);
    let mut encoder = registry.encoder(&config).expect("a FLAC encoder");

    let mut frames = Vec::new();
    for chunk in samples.chunks(FLAC_BLOCK_SAMPLES) {
        let frame = broadcast_codec::AudioFrame::new(
            rate,
            ChannelLayout::Mono,
            SampleLayout::Interleaved,
            AudioSamples::S16(chunk.to_vec()),
        )
        .expect("a whole sample count");
        frames.push(Frame::Audio(frame));
    }
    let packets = encode(&mut *encoder, frames);
    assert!(!packets.is_empty());

    let reported = encoder.parameters().expect("parameters once encoding");
    assert_eq!(reported.codec(), CodecId::Flac);
    let mut decoder = registry
        .decoder(reported)
        .expect("a FLAC decoder for what the encoder reports");
    let units: Vec<Unit> = packets
        .iter()
        .map(|packet| Unit {
            data: packet.data().to_vec(),
            pts: packet.pts(),
            dts: packet.dts(),
            key: packet.key(),
        })
        .collect();
    let decoded = decode(&mut *decoder, &units);
    let mut round_tripped = Vec::new();
    for frame in &decoded {
        let audio = audio(frame);
        let AudioSamples::S16(values) = audio.samples() else {
            panic!("libav decoded FLAC to a non-16-bit format");
        };
        round_tripped.extend_from_slice(values);
    }
    assert_eq!(round_tripped, samples, "FLAC is lossless");
}

/// The signed 16-bit samples of the committed WAV fixture.
fn wav_samples(fixture: &str) -> Vec<i16> {
    let bytes = test_fixtures::read(fixture);
    let data = wav_data(&bytes).expect("a WAV data chunk");
    data.chunks_exact(2)
        .map(|pair| i16::from_le_bytes([pair[0], pair[1]]))
        .collect()
}

/// The `data` chunk's payload of a RIFF/WAVE file.
fn wav_data(bytes: &[u8]) -> Option<&[u8]> {
    let mut offset = RIFF_HEADER_LEN;
    while offset + CHUNK_HEADER_LEN <= bytes.len() {
        let id = bytes.get(offset..offset + CHUNK_ID_LEN)?;
        let size = u32::from_le_bytes(
            bytes
                .get(offset + CHUNK_ID_LEN..offset + CHUNK_HEADER_LEN)?
                .try_into()
                .ok()?,
        );
        let start = offset + CHUNK_HEADER_LEN;
        let end = start.checked_add(usize::try_from(size).ok()?)?;
        if id == b"data" {
            return bytes.get(start..end);
        }
        offset = end + (size % 2) as usize;
    }
    None
}

/// A registry holding both the libav and the symphonia provider.
fn both_providers() -> Option<Registry> {
    let registry = Registry::new()
        .with(Box::new(LibavProvider::new()))
        .expect("the libav provider registers")
        .with(Box::new(codec_symphonia::SymphoniaProvider))
        .expect("the symphonia provider registers");
    if registry
        .candidates(CodecId::Aac, Direction::Decode)
        .is_empty()
    {
        eprintln!("skip: no AAC decoder is registered here");
        return None;
    }
    Some(registry)
}

/// Decode `audio/aac.adts` through `registry`, returning the interleaved
/// samples, the integrated loudness and the sample count.
fn adts_through(registry: &Registry) -> (Vec<f32>, f64, usize) {
    let data = test_fixtures::read(ADTS_FIXTURE);
    let packets: Vec<_> = codec_symphonia::adts::AdtsFrames::new(&data).collect();
    let first = packets.first().expect("the fixture carries ADTS frames");
    let rate = NonZeroU32::new(first.sample_rate()).expect("a sample rate");
    let channels = first.channels().expect("a channel layout");
    let parameters =
        CodecParameters::new(CodecId::Aac, broadcast_codec::Timescale::from_nonzero(rate))
            .with_audio(AudioParameters::new(rate, channels));
    let mut decoder = registry.decoder(&parameters).expect("an AAC decoder");
    let units: Vec<Unit> = packets
        .iter()
        .map(|packet| Unit {
            data: packet.bytes().to_vec(),
            pts: None,
            dts: None,
            key: true,
        })
        .collect();
    let frames = decode(&mut *decoder, &units);
    let meter_layout = match channels {
        ChannelLayout::Mono => broadcast_loudness::ChannelLayout::Mono,
        ChannelLayout::Stereo => broadcast_loudness::ChannelLayout::Stereo,
        ChannelLayout::Surround5_1 => broadcast_loudness::ChannelLayout::Surround51,
        other => panic!("no loudness layout for {other:?}"),
    };
    let mut meter = LoudnessMeter::new(rate.get(), meter_layout).expect("a meter");
    let mut samples = Vec::new();
    let mut decoded = 0usize;
    for frame in &frames {
        let audio = audio(frame);
        let AudioSamples::F32(values) = audio.samples() else {
            panic!("an F32 decode was expected");
        };
        let channel_count = audio.channels().count();
        let per_channel = audio.frames();
        match audio.layout() {
            SampleLayout::Planar => {
                for index in 0..per_channel {
                    let mut one = Vec::with_capacity(channel_count);
                    for channel in 0..channel_count {
                        one.push(values[channel * per_channel + index]);
                    }
                    meter.push_f32(&one).expect("a sample");
                    samples.extend_from_slice(&one);
                }
            }
            SampleLayout::Interleaved => {
                for one in values.chunks(channel_count) {
                    meter.push_f32(one).expect("a sample");
                    samples.extend_from_slice(one);
                }
            }
            other => panic!("unexpected sample layout {other:?}"),
        }
        decoded += per_channel;
    }
    meter.finish();
    (samples, meter.integrated_lufs(), decoded)
}

#[test]
fn t7_the_two_audio_adapters_agree() {
    if both_providers().is_none() {
        return;
    }
    let mut measurements = Vec::new();
    for (wanted, text) in [
        ("symphonia", "dec:aac=symphonia"),
        ("libav", "dec:aac=libav"),
    ] {
        let Some(mut registry) = both_providers() else {
            return;
        };
        let preferences = broadcast_codec::parse_preferences(text).expect("a preference");
        for preference in preferences {
            registry = registry.prefer(preference);
        }
        let candidates = registry.candidates(CodecId::Aac, Direction::Decode);
        assert_eq!(
            candidates.first().map(|descriptor| descriptor.provider()),
            Some(wanted),
            "the preference must put `{wanted}` first"
        );
        let (samples, lufs, decoded) = adts_through(&registry);
        println!("{wanted}: {decoded} samples, {lufs:.3} LUFS");
        assert!(
            (lufs - EXPECTED_LUFS).abs() <= TOLERANCE_LU,
            "{wanted} integrated {lufs:.3} LUFS"
        );
        measurements.push((wanted, samples, decoded));
    }
    let (first, second) = (&measurements[0], &measurements[1]);
    assert_eq!(first.2, second.2, "both decoders produce the same count");
    assert_eq!(first.1.len(), second.1.len());
    let largest = first
        .1
        .iter()
        .zip(&second.1)
        .map(|(a, b)| (f64::from(*a) - f64::from(*b)).abs())
        .fold(0.0f64, f64::max);
    println!(
        "largest absolute sample difference between {} and {}: {largest:e}",
        first.0, second.0
    );
    assert!(largest.is_finite());
}

#[test]
fn t8_lists_what_libav_offers() {
    let Some(registry) = registry() else {
        return;
    };
    let list = registry.list();
    let decoders = list
        .iter()
        .filter(|descriptor| descriptor.direction() == Direction::Decode)
        .count();
    let encoders = list.len() - decoders;
    println!("libav offers {decoders} decoders and {encoders} encoders here");
    for descriptor in &list {
        println!(
            "{:12} {:8} {:28} hw={}",
            descriptor.codec().name(),
            descriptor.direction().name(),
            descriptor.implementation(),
            descriptor.hardware()
        );
    }
    assert!(decoders > 0 && encoders > 0);
    assert!(list.iter().any(|d| {
        d.codec() == CodecId::H264
            && d.direction() == Direction::Decode
            && d.implementation() == "h264"
    }));
    assert!(list.iter().any(|d| {
        d.codec() == CodecId::Aac
            && d.direction() == Direction::Decode
            && d.implementation() == "aac"
    }));
}

#[test]
fn t9_videotoolbox_encodes_what_the_software_decoder_reads() {
    let Some(registry) = registry() else {
        return;
    };
    let implementation = "h264_videotoolbox";
    let listed = registry
        .candidates(CodecId::H264, Direction::Encode)
        .iter()
        .any(|descriptor| descriptor.implementation() == implementation);
    if !listed {
        eprintln!("skip: libav registers no `{implementation}` encoder here");
        return;
    }
    let Some((parameters, units)) = ts_track(true) else {
        panic!("the fixture must carry a H.264 track");
    };
    let mut decoder = registry.decoder(&parameters).expect("an H.264 decoder");
    let frames = decode(&mut *decoder, &units);
    assert_eq!(frames.len(), TS_VIDEO_FRAMES);

    let mut registry = registry;
    for preference in broadcast_codec::parse_preferences("enc:h264=libav:h264_videotoolbox")
        .expect("a preference")
    {
        registry = registry.prefer(preference);
    }
    let config = broadcast_codec::EncoderConfig::new(
        CodecId::H264,
        broadcast_codec::Timescale::from_nonzero(
            NonZeroU32::new(MPEG2_TICKS).expect("a non-zero timescale"),
        ),
    )
    .with_video(broadcast_codec::VideoParameters::new(
        NonZeroU32::new(TS_WIDTH).expect("a width"),
        NonZeroU32::new(TS_HEIGHT).expect("a height"),
    ))
    .with_pixel_format(PixelFormat::Yuv420p8)
    .with_bit_rate(std::num::NonZeroU64::new(MPEG2_BIT_RATE).expect("a bit rate"))
    .with_gop(NonZeroU32::new(MPEG2_GOP).expect("a group length"));
    let mut encoder = registry
        .encoder(&config)
        .expect("the VideoToolbox H.264 encoder");
    let source = broadcast_codec::Timescale::from_nonzero(
        NonZeroU32::new(TS_TIMESCALE).expect("a non-zero timescale"),
    );
    let target = broadcast_codec::Timescale::from_nonzero(
        NonZeroU32::new(MPEG2_TICKS).expect("a non-zero timescale"),
    );
    let frames: Vec<Frame> = frames
        .into_iter()
        .map(|frame| match frame {
            Frame::Video(video) => match video.pts() {
                Some(pts) => Frame::Video(video.with_pts(broadcast_core::timescale::rescale_i64(
                    pts,
                    source,
                    target,
                    broadcast_core::timescale::Rounding::Nearest,
                ))),
                None => Frame::Video(video),
            },
            other => other,
        })
        .collect();
    let packets = encode(&mut *encoder, frames);
    assert!(!packets.is_empty());

    let reported = encoder.parameters().expect("parameters once encoding");
    let units: Vec<Unit> = packets
        .iter()
        .map(|packet| Unit {
            data: packet.data().to_vec(),
            pts: packet.pts(),
            dts: packet.dts(),
            key: packet.key(),
        })
        .collect();
    let mut software = registry
        .decoder(reported)
        .expect("the software H.264 decoder");
    let decoded = decode(&mut *software, &units);
    assert_eq!(
        decoded.len(),
        TS_VIDEO_FRAMES,
        "pictures out of the round trip"
    );
    for frame in &decoded {
        assert_eq!(video(frame).width().get(), TS_WIDTH);
        assert_eq!(video(frame).height().get(), TS_HEIGHT);
    }
}

#[test]
fn t10_a_candidate_that_cannot_serve_falls_back() {
    let Some((parameters, units)) = ts_track(true) else {
        panic!("the fixture must carry a H.264 track");
    };
    let Some(mut strict) = registry() else {
        return;
    };
    for preference in
        broadcast_codec::parse_preferences("dec:h264=libav:h264_cuvid").expect("a preference")
    {
        strict = strict.prefer(preference);
    }
    let Err(error) = strict.decoder(&parameters) else {
        panic!("an implementation this libav does not list must not serve");
    };
    println!("strict refusal: {error}");
    assert!(error.to_string().contains("h264_cuvid"), "{error}");

    let config = broadcast_codec::EncoderConfig::new(
        CodecId::H264,
        broadcast_codec::Timescale::from_nonzero(
            NonZeroU32::new(MPEG2_TICKS).expect("a non-zero timescale"),
        ),
    )
    .with_video(broadcast_codec::VideoParameters::new(
        NonZeroU32::new(TS_WIDTH).expect("a width"),
        NonZeroU32::new(TS_HEIGHT).expect("a height"),
    ))
    .with_bit_rate(std::num::NonZeroU64::new(MPEG2_BIT_RATE).expect("a bit rate"))
    .with_gop(NonZeroU32::new(MPEG2_GOP).expect("a group length"));

    let Some(mut preferring) = registry() else {
        return;
    };
    for preference in
        broadcast_codec::parse_preferences("enc:h264=libav:libx264rgb").expect("a preference")
    {
        preferring = preferring.prefer(preference);
    }
    let candidates = preferring.candidates(CodecId::H264, Direction::Encode);
    let first = candidates.first().expect("an encoder candidate");
    let second = candidates.get(1).expect("a second encoder candidate");
    assert_eq!(first.implementation(), "libx264rgb");
    assert_eq!(second.implementation(), "libx264");
    let Err(strict_error) = preferring.encoder(&config) else {
        panic!("`libx264rgb` does not take an 8-bit 4:2:0 source");
    };
    println!("strict refusal: {strict_error}");
    assert!(
        strict_error.to_string().contains("libx264rgb"),
        "{strict_error}"
    );

    let Some(next) = registry() else {
        return;
    };
    let next = next
        .fallback(broadcast_codec::Fallback::NextCandidate)
        .prefer(
            broadcast_codec::Preference::new(CodecId::H264, "libav")
                .with_implementation("libx264rgb"),
        );
    let mut encoder = next.encoder(&config).expect("the next candidate serves");
    let Some(decoding) = registry() else {
        return;
    };
    let mut decoder = decoding.decoder(&parameters).expect("an H.264 decoder");
    let frames = decode(&mut *decoder, &units);
    let source = broadcast_codec::Timescale::from_nonzero(
        NonZeroU32::new(TS_TIMESCALE).expect("a non-zero timescale"),
    );
    let target = broadcast_codec::Timescale::from_nonzero(
        NonZeroU32::new(MPEG2_TICKS).expect("a non-zero timescale"),
    );
    let frames: Vec<Frame> = frames
        .into_iter()
        .map(|frame| match frame {
            Frame::Video(video) => match video.pts() {
                Some(pts) => Frame::Video(video.with_pts(broadcast_core::timescale::rescale_i64(
                    pts,
                    source,
                    target,
                    broadcast_core::timescale::Rounding::Nearest,
                ))),
                None => Frame::Video(video),
            },
            other => other,
        })
        .collect();
    let packets = encode(&mut *encoder, frames);
    assert!(!packets.is_empty(), "the second candidate encoded nothing");
}

#[test]
fn t11_a_profile_the_implementation_does_not_state_is_not_offered() {
    let Some(registry) = registry() else {
        return;
    };
    let rate = NonZeroU32::new(PROBE_RATE).expect("a sample rate");
    let base = CodecParameters::new(CodecId::Aac, broadcast_codec::Timescale::from_nonzero(rate))
        .with_audio(AudioParameters::new(rate, ChannelLayout::Mono));
    let high_efficiency = base
        .clone()
        .with_profile(broadcast_codec::CodecProfile::AacXhe)
        .expect("an AAC profile");
    let rejected = registry.rejected_for(&high_efficiency);
    println!("rejected for aac/xhe: {rejected:?}");
    assert!(
        rejected
            .iter()
            .any(|rejection| rejection.descriptor().implementation() == "aac"),
        "libav's AAC decoder states no extended-HE profile"
    );
    let Err(error) = registry.decoder(&high_efficiency) else {
        panic!("no implementation states aac/xhe");
    };
    assert!(error.to_string().contains("aac/xhe"), "{error}");

    let low_complexity = base
        .with_profile(broadcast_codec::CodecProfile::AacLc)
        .expect("an AAC profile");
    assert!(
        registry
            .candidates_for(&low_complexity)
            .iter()
            .any(|descriptor| descriptor.implementation() == "aac"),
        "the decoder states Low Complexity"
    );
}

#[test]
fn t12_renders_the_inventory() {
    let Some(registry) = registry() else {
        return;
    };
    let entries = registry.inventory(true);
    assert_eq!(entries.len(), registry.list().len());
    for entry in &entries {
        assert_ne!(
            entry.probe().availability(),
            broadcast_codec::Availability::Unknown,
            "{} was not probed",
            entry.descriptor().implementation()
        );
    }
    let table = broadcast_codec::render_inventory(&entries);
    println!("{table}");
    assert_eq!(table.lines().count(), entries.len() + 1);
    let provider = &registry.providers()[0];
    println!(
        "{} {} library={:?} licence={:?}",
        provider.name(),
        provider.version(),
        provider.library(),
        provider.licence()
    );
    assert!(
        provider
            .library()
            .is_some_and(|library| library.starts_with("libavcodec "))
    );
    assert!(
        provider
            .licence()
            .is_some_and(|licence| !licence.is_empty())
    );
}

/// A copy of `frame` whose planes are padded to a wider line than the visible
/// row, as a decoder may hand back.
fn padded(frame: &VideoFrame) -> VideoFrame {
    let format = frame.format();
    let width = usize::try_from(frame.width().get()).expect("width");
    let height = usize::try_from(frame.height().get()).expect("height");
    let VideoStorage::Cpu(planes) = frame.storage() else {
        panic!("hardware storage");
    };
    let mut padded = Vec::with_capacity(planes.len());
    for (index, plane) in planes.iter().enumerate() {
        let size = format.plane_size(width, height, index).expect("plane size");
        let stride = plane.stride().get();
        let rows = plane.data().len() / stride;
        let row_bytes = size / rows;
        let padded_stride = stride + PLANE_PADDING;
        let mut data = vec![0u8; padded_stride * rows];
        for row in 0..rows {
            let from = row * stride;
            let to = row * padded_stride;
            data[to..to + row_bytes].copy_from_slice(&plane.data()[from..from + row_bytes]);
        }
        padded.push(broadcast_codec::VideoPlane::new(
            data,
            std::num::NonZeroUsize::new(padded_stride).expect("a stride"),
        ));
    }
    VideoFrame::new(
        frame.width(),
        frame.height(),
        format,
        VideoStorage::Cpu(padded),
    )
    .expect("a padded picture")
}

#[test]
fn a_padded_source_plane_is_copied_by_its_stride() {
    let Some(registry) = registry() else {
        return;
    };
    let Some((parameters, units)) = ts_track(true) else {
        panic!("the fixture must carry a H.264 track");
    };
    let mut decoder = registry.decoder(&parameters).expect("an H.264 decoder");
    let Some(Frame::Video(original)) = decode(&mut *decoder, &units).into_iter().next() else {
        panic!("one picture");
    };
    let padded = padded(&original);

    let config = broadcast_codec::EncoderConfig::new(
        CodecId::Mpeg2Video,
        broadcast_codec::Timescale::from_nonzero(
            NonZeroU32::new(MPEG2_TICKS).expect("a non-zero timescale"),
        ),
    )
    .with_video(broadcast_codec::VideoParameters::new(
        NonZeroU32::new(TS_WIDTH).expect("a width"),
        NonZeroU32::new(TS_HEIGHT).expect("a height"),
    ))
    .with_pixel_format(PixelFormat::Yuv420p8)
    .with_bit_rate(std::num::NonZeroU64::new(MPEG2_BIT_RATE).expect("a bit rate"))
    .with_gop(NonZeroU32::new(MPEG2_GOP).expect("a group length"));

    let encode_one = |frame: VideoFrame| {
        let mut encoder = registry.encoder(&config).expect("an MPEG-2 encoder");
        let frames = encode(&mut *encoder, vec![Frame::Video(frame)]);
        let reported = encoder.parameters().expect("parameters").clone();
        let units: Vec<Unit> = frames
            .iter()
            .map(|packet| Unit {
                data: packet.data().to_vec(),
                pts: packet.pts(),
                dts: packet.dts(),
                key: packet.key(),
            })
            .collect();
        let mut decoder = registry.decoder(&reported).expect("a decoder");
        decode(&mut *decoder, &units)
    };
    let plain = encode_one(original);
    let padded = encode_one(padded);
    assert_eq!(plain.len(), 1);
    assert_eq!(padded.len(), 1);
    assert_eq!(
        hash_visible(video(&plain[0])),
        hash_visible(video(&padded[0])),
        "padding must not reach the encoded picture"
    );
}

/// Bytes in one f32 sample.
const F32_BYTES: usize = 4;
/// The sample rate both audio fixtures are decoded at.
const FIXTURE_RATE: u32 = 48_000;

/// One fixture's packets through libav's own demuxer, in decode order.
fn libav_stream(fixture: &str, codec: CodecId) -> Option<(CodecParameters, Vec<Unit>)> {
    let mut input = ffmpeg_next::format::input(&test_fixtures::path(fixture)).ok()?;
    let stream = input.streams().best(ffmpeg_next::media::Type::Audio)?;
    let index = stream.index();
    let denominator = u32::try_from(stream.time_base().denominator()).ok()?;
    let time_base = broadcast_codec::Timescale::new(denominator)?;
    let rate = NonZeroU32::new(FIXTURE_RATE)?;
    let parameters = CodecParameters::new(codec, time_base)
        .with_audio(AudioParameters::new(rate, ChannelLayout::Mono));
    let mut units = Vec::new();
    for (stream, packet) in input.packets() {
        if stream.index() != index {
            continue;
        }
        let Some(data) = packet.data() else {
            continue;
        };
        units.push(Unit {
            data: data.to_vec(),
            pts: packet.pts(),
            dts: packet.dts(),
            key: packet.is_key(),
        });
    }
    Some((parameters, units))
}

/// The per-channel sample count ffmpeg itself decodes from `fixture`.
fn ffmpeg_audio_samples(fixture: &str) -> Option<usize> {
    if !tool_available("ffmpeg", "-version") {
        eprintln!("skip: ffmpeg is not on PATH, so the external decode did NOT run");
        return None;
    }
    let mut command = Command::new("ffmpeg");
    command
        .arg("-v")
        .arg("error")
        .arg("-i")
        .arg(test_fixtures::path(fixture))
        .arg("-f")
        .arg("f32le")
        .arg("-ar")
        .arg(FIXTURE_RATE.to_string())
        .arg("-ac")
        .arg("1")
        .arg("-");
    let output = output_bounded(&mut command, FFMPEG_DEADLINE).expect("ffmpeg runs");
    assert!(
        output.status.success(),
        "ffmpeg failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Some(output.stdout.len() / F32_BYTES)
}

#[test]
fn t13_the_adapter_does_not_apply_the_container_trim() {
    for (name, fixture, codec) in [
        ("mp3", "audio/audio.mp3", CodecId::MpegAudio),
        ("opus", "audio/opus.ogg", CodecId::Opus),
    ] {
        let Some(reference) = ffmpeg_audio_samples(fixture) else {
            return;
        };
        let Some((parameters, units)) = libav_stream(fixture, codec) else {
            eprintln!("skip: the linked libav has no {name} demuxer here");
            continue;
        };
        let registry = Registry::new()
            .with(Box::new(LibavProvider::new()))
            .expect("the libav provider registers");
        let Some(descriptor) = registry
            .candidates(codec, Direction::Decode)
            .into_iter()
            .next()
        else {
            eprintln!("skip: the linked libav offers no {name} decoder here");
            continue;
        };
        let Ok(mut decoder) = registry.decoder(&parameters) else {
            eprintln!("skip: {name}: the linked libav will not open its decoder here");
            continue;
        };
        let frames = decode(&mut *decoder, &units);
        let count: usize = frames.iter().map(|frame| audio(frame).frames()).sum();
        println!(
            "{name}: libav adapter {count} samples, ffmpeg {reference}; applies_trim {}",
            descriptor.applies_trim()
        );
        assert_eq!(
            descriptor.applies_trim(),
            count == reference,
            "{name}: the descriptor must state whether libav applied the container trim"
        );
    }
}
