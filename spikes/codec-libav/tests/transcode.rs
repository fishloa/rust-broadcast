//! The transcode stage driven by libav's own decoder and encoder.
//!
//! This is the pipeline proof the seam exists for: a real fixture's track goes
//! through a real decoder, a real encoder and `transmux::TranscodeStage`, is
//! muxed, demuxed back, and decoded again for comparison. Nothing is faked.
//!
//! # The audio half
//!
//! `ffmpeg-next` 8.1 exposes no extradata getter, so the adapter reads
//! `AVCodecContext.extradata` and the encoder's `initial_padding`/`trailing_padding`
//! through the binding's raw pointer (`src/sys.rs`, one `SAFETY` argument). A
//! transcoded AAC track therefore announces its `AudioSpecificConfig` and the
//! encoder's priming, is muxed and demuxed, and decodes again.
//! [`the_stage_transcodes_the_aac_track_with_libavs_decoder_and_encoder`]
//! checks the packet count, the announced record, the sample count after the
//! seam's own trim, and integrated loudness against the direct decode. The
//! fragmented-MP4 init segment still has no edit list to re-state that trim, so
//! the value is taken from the announced track rather than the re-demux.
//!
//! The video half completes the whole loop too: an MPEG-2 video configuration
//! is describable without a decoder configuration record (its `esds` is the
//! canonical one for MPEG-2 Main Visual), so libav's `mpeg2video` encoder can
//! be announced and its output decoded again.

use std::num::{NonZeroU32, NonZeroU64};
use std::time::Duration;

use broadcast_codec::{
    AudioSamples, CodecId, CodecParameters, Direction, EncoderConfig, Frame, Registry,
    SampleLayout, Timescale, VideoParameters,
};
use broadcast_common::stage::{Stage, Timestamp};
use broadcast_loudness::LoudnessMeter;
use codec_libav::LibavProvider;
use test_harness::collect::{CollectMode, media_from_mp4, media_from_ts};
use transmux::mux_stage::{MuxInput, MuxOutput};
use transmux::{
    CmafMuxStage, CodecConfig, FragmentCut, Sample, TrackId, TrackSpec, TranscodeStage, TsMuxStage,
    codec_parameters_from_track,
};

/// The committed TS fixture with one H.264 and one AAC track.
const TS_FIXTURE: &str = "ts/h264-aac.ts";
/// The audio access units `ffprobe -count_frames` reports for it.
const TS_AUDIO_FRAMES: usize = 131;
/// The video frames `ffprobe -count_frames` reports for it.
const TS_VIDEO_FRAMES: usize = 75;
/// Allowed loudness difference, in LU - the tolerance the spikes already use.
const TOLERANCE_LU: f64 = 0.1;
/// The encoder delay libav's AAC encoder reports, in samples (a frame).
const AAC_ENCODER_DELAY: u32 = 1024;
/// The AAC bit rate the encoder is asked for, in bits per second.
const AAC_BIT_RATE: u64 = 128_000;
/// The MPEG-2 video bit rate the encoder is asked for, in bits per second.
const MPEG2_BIT_RATE: u64 = 5_000_000;
/// The MPEG-2 group length, in pictures.
const MPEG2_GOP: u32 = 25;
/// Ticks per second the MPEG-2 encoder is configured for: one per picture.
const MPEG2_TICKS: u32 = 25;
/// The coded picture size of the fixture.
const TS_WIDTH: u32 = 320;
/// The coded picture height of the fixture.
const TS_HEIGHT: u32 = 240;

fn registry() -> Option<Registry> {
    let registry = Registry::new()
        .with(Box::new(LibavProvider::new()))
        .expect("the libav provider registers");
    if registry
        .candidates(CodecId::Aac, Direction::Decode)
        .is_empty()
    {
        eprintln!("skip: the linked libav offers no AAC decoder here");
        return None;
    }
    Some(registry)
}

fn timescale(value: u32) -> Timescale {
    Timescale::from_nonzero(NonZeroU32::new(value).expect("a non-zero timescale"))
}

/// The fixture's tracks, demuxed.
fn fixture() -> (Vec<TrackSpec>, Vec<(TrackId, Sample)>) {
    let bytes = test_fixtures::read(TS_FIXTURE);
    let media = media_from_ts(&bytes).expect("demux the fixture");
    let specs = media.tracks.iter().map(|t| t.spec.clone()).collect();
    let mut samples = Vec::new();
    for track in &media.tracks {
        for sample in &track.samples {
            samples.push((track.spec.track_id, sample.clone()));
        }
    }
    (specs, samples)
}

fn spec_of<'a>(specs: &'a [TrackSpec], codec: &str) -> &'a TrackSpec {
    specs
        .iter()
        .find(|spec| match (codec, &spec.config) {
            ("aac", CodecConfig::Aac { .. }) | ("avc", CodecConfig::Avc { .. }) => true,
            _ => false,
        })
        .expect("the fixture carries the track")
}

fn samples_of(samples: &[(TrackId, Sample)], track: TrackId) -> Vec<Sample> {
    samples
        .iter()
        .filter(|(id, _)| *id == track)
        .map(|(_, sample)| sample.clone())
        .collect()
}

/// Drive the stage over announcements then samples, returning its output.
fn drive(
    stage: &mut TranscodeStage,
    specs: &[TrackSpec],
    samples: &[(TrackId, Sample)],
) -> transmux::Result<Vec<MuxInput>> {
    let mut outputs = Vec::new();
    for spec in specs {
        stage.feed(MuxInput::Track(Box::new(spec.clone())), Timestamp::ZERO)?;
        while let Some(output) = stage.poll() {
            outputs.push(output);
        }
    }
    for (track, sample) in samples {
        stage.feed(
            MuxInput::Sample {
                track: *track,
                sample: sample.clone(),
            },
            Timestamp::ZERO,
        )?;
        while let Some(output) = stage.poll() {
            outputs.push(output);
        }
    }
    stage.finish()?;
    while let Some(output) = stage.poll() {
        outputs.push(output);
    }
    Ok(outputs)
}

/// Track announcements first, then everything else in order.
fn tracks_then_rest(outputs: Vec<MuxInput>) -> (Vec<MuxInput>, Vec<MuxInput>) {
    let mut tracks = Vec::new();
    let mut rest = Vec::new();
    for output in outputs {
        match output {
            MuxInput::Track(spec) => tracks.push(MuxInput::Track(spec)),
            other => rest.push(other),
        }
    }
    (tracks, rest)
}

fn announced(outputs: &[MuxInput], track: TrackId) -> TrackSpec {
    outputs
        .iter()
        .find_map(|output| match output {
            MuxInput::Track(spec) if spec.track_id == track => Some((**spec).clone()),
            _ => None,
        })
        .expect("the stage announced the track")
}

fn packets_for(outputs: &[MuxInput], track: TrackId) -> Vec<Sample> {
    outputs
        .iter()
        .filter_map(|output| match output {
            MuxInput::Sample {
                track: id, sample, ..
            } if *id == track => Some(sample.clone()),
            _ => None,
        })
        .collect()
}

/// Decode a track's coded samples through `registry`.
fn decode(registry: &Registry, parameters: &CodecParameters, samples: &[Sample]) -> Vec<Frame> {
    let mut decoder = registry.decoder(parameters).expect("a libav decoder");
    let mut frames = Vec::new();
    for sample in samples {
        let mut packet =
            broadcast_codec::Packet::borrowed(&sample.data).with_key(sample.flags.is_sync);
        if let Some(timing) = sample.timing {
            if let Some(dts) = timing.dts() {
                packet = packet.with_dts(dts);
            }
            if let Some(pts) = timing.pts() {
                packet = packet.with_pts(pts);
            }
        }
        decoder.send(packet).expect("send a packet");
        while let Some(frame) = decoder.receive().expect("receive") {
            frames.push(frame);
        }
    }
    decoder.finish().expect("finish the decoder");
    while let Some(frame) = decoder.receive().expect("drain") {
        frames.push(frame);
    }
    frames
}

/// Decode a track's coded samples through `registry`, applying the container's
/// gapless trim exactly as a real caller does.
fn decode_trimmed(
    registry: &Registry,
    parameters: &CodecParameters,
    samples: &[Sample],
) -> Vec<Frame> {
    let mut decoder = registry
        .trimmed_decoder(parameters)
        .expect("a trimmed libav decoder");
    let mut frames = Vec::new();
    for sample in samples {
        let mut packet =
            broadcast_codec::Packet::borrowed(&sample.data).with_key(sample.flags.is_sync);
        if let Some(timing) = sample.timing {
            if let Some(dts) = timing.dts() {
                packet = packet.with_dts(dts);
            }
            if let Some(pts) = timing.pts() {
                packet = packet.with_pts(pts);
            }
        }
        decoder.send(packet).expect("send a packet");
        while let Some(frame) = decoder.receive().expect("receive") {
            frames.push(frame);
        }
    }
    decoder.finish().expect("finish the decoder");
    while let Some(frame) = decoder.receive().expect("drain") {
        frames.push(frame);
    }
    frames
}

/// Total decoded samples across the channels.
fn decoded_samples(frames: &[Frame]) -> usize {
    frames
        .iter()
        .map(|frame| match frame {
            Frame::Audio(audio) => audio.samples().len(),
            _ => 0,
        })
        .sum()
}

/// Integrated loudness of a decoded track, measured the way the other spikes
/// measure the same fixture.
fn integrated_lufs(parameters: &CodecParameters, frames: &[Frame]) -> f64 {
    let audio_parameters = parameters.audio().expect("audio parameters");
    let layout = match audio_parameters.channels().count() {
        1 => broadcast_loudness::ChannelLayout::Mono,
        2 => broadcast_loudness::ChannelLayout::Stereo,
        6 => broadcast_loudness::ChannelLayout::Surround51,
        other => panic!("no loudness layout for {other} channels"),
    };
    let mut meter =
        LoudnessMeter::new(audio_parameters.sample_rate().get(), layout).expect("a meter");
    for frame in frames {
        let Frame::Audio(audio) = frame else {
            continue;
        };
        let AudioSamples::F32(values) = audio.samples() else {
            panic!("libav decoded AAC to a non-float format");
        };
        assert_eq!(
            audio.layout(),
            SampleLayout::Planar,
            "libav decoded AAC to a planar layout"
        );
        let channels = audio.channels().count();
        let per_channel = audio.frames();
        for index in 0..per_channel {
            let mut sample = Vec::with_capacity(channels);
            for channel in 0..channels {
                sample.push(values[channel * per_channel + index]);
            }
            meter.push_f32(&sample).expect("a sample");
        }
    }
    meter.finish();
    meter.integrated_lufs()
}

/// Mux one track set and its samples to CMAF, returning the bytes.
fn mux_cmaf(inputs: Vec<MuxInput>, anchor: TrackId) -> Vec<u8> {
    let (tracks, rest) = tracks_then_rest(inputs);
    let mut mux = CmafMuxStage::new(
        timescale(1_000),
        FragmentCut::AnchorKeyframe {
            track: anchor,
            target: Duration::from_secs(1),
        },
        1,
    );
    let mut bytes = Vec::new();
    for track in tracks {
        mux.feed(track, Timestamp::ZERO).expect("announce");
    }
    for input in rest {
        mux.feed(input, Timestamp::ZERO).expect("mux");
        drain_cmaf(&mut mux, &mut bytes);
    }
    mux.finish().expect("finish");
    drain_cmaf(&mut mux, &mut bytes);
    bytes
}

fn drain_cmaf(mux: &mut CmafMuxStage, bytes: &mut Vec<u8>) {
    while let Some(output) = mux.poll() {
        match output {
            MuxOutput::Init(data) | MuxOutput::Bytes(data) => bytes.extend_from_slice(&data),
            MuxOutput::Segment(segment) => bytes.extend_from_slice(&segment.bytes),
            _ => {}
        }
    }
}

/// Mux one track set and its samples to MPEG-2 TS, returning the bytes.
fn mux_ts(inputs: Vec<MuxInput>) -> Vec<u8> {
    let (tracks, rest) = tracks_then_rest(inputs);
    let mut mux = TsMuxStage::new(None);
    let mut bytes = Vec::new();
    for track in tracks {
        mux.feed(track, Timestamp::ZERO).expect("announce");
    }
    for input in rest {
        mux.feed(input, Timestamp::ZERO).expect("mux");
        while let Some(output) = mux.poll() {
            if let MuxOutput::Bytes(data) = output {
                bytes.extend_from_slice(&data);
            }
        }
    }
    mux.finish().expect("finish");
    while let Some(output) = mux.poll() {
        if let MuxOutput::Bytes(data) = output {
            bytes.extend_from_slice(&data);
        }
    }
    bytes
}

/// The audio track through libav's AAC decoder and encoder.
///
/// Everything the stage and the container can express is checked; the decoded
/// comparison needs the encoder's `AudioSpecificConfig`, which this binding
/// cannot report (see the module docs).
#[test]
fn the_stage_transcodes_the_aac_track_with_libavs_decoder_and_encoder() {
    let Some(registry) = registry() else {
        return;
    };
    let (specs, samples) = fixture();
    let audio = spec_of(&specs, "aac").clone();
    let source = codec_parameters_from_track(&audio).expect("the audio track converts");
    let audio_parameters = source.audio().expect("audio parameters");
    let decoder = registry.decoder(&source).expect("a libav AAC decoder");
    let encoder = registry
        .encoder(
            &EncoderConfig::new(CodecId::Aac, source.time_base())
                .with_audio(audio_parameters)
                .with_bit_rate(NonZeroU64::new(AAC_BIT_RATE).expect("a bit rate")),
        )
        .expect("a libav AAC encoder");
    let mut stage = TranscodeStage::new(audio.track_id, decoder, encoder);

    let outputs = drive(&mut stage, &specs, &samples).expect("the stage transcodes");
    let packets = packets_for(&outputs, audio.track_id);
    assert!(
        packets.len() + 1 >= TS_AUDIO_FRAMES,
        "every access unit reached the encoder: {} packets for {TS_AUDIO_FRAMES} frames",
        packets.len()
    );
    let track = announced(&outputs, audio.track_id);
    let CodecConfig::Aac {
        sample_rate,
        channel_count,
        ..
    } = track.config
    else {
        panic!("the encoder reports AAC")
    };
    assert_eq!(sample_rate, audio_parameters.sample_rate().get());
    assert_eq!(
        usize::from(channel_count),
        audio_parameters.channels().count()
    );
    let back = media_from_mp4(
        &mux_cmaf(outputs, spec_of(&specs, "avc").track_id),
        CollectMode::Seekable,
    )
    .expect("re-demux the CMAF output");
    let output_track = back
        .tracks
        .iter()
        .find(|candidate| matches!(candidate.spec.config, CodecConfig::Aac { .. }))
        .expect("the output carries the audio track");
    assert_eq!(
        output_track.samples.len(),
        packets.len(),
        "every packet the encoder emitted is in the output"
    );

    // The encoder's own record and priming: an `AudioSpecificConfig` read from
    // libav's extradata and the AAC encoder delay it reports, both carried on
    // the track the stage announced. A fragmented-MP4 init segment has no edit
    // list, so a re-demux cannot state the trim again; that is the muxer's
    // documented gap, not the stage's.
    let output_parameters =
        codec_parameters_from_track(&track).expect("the announced track converts");
    assert!(
        !output_parameters.extradata().is_empty(),
        "the encoder's AudioSpecificConfig reaches the announced track"
    );
    assert_eq!(
        output_parameters.initial_padding(),
        AAC_ENCODER_DELAY,
        "the encoder's priming is on the announced track"
    );
    let output_frames = decode_trimmed(&registry, &output_parameters, &output_track.samples);
    let direct_frames = decode(&registry, &source, &samples_of(&samples, audio.track_id));
    assert_eq!(
        decoded_samples(&output_frames),
        decoded_samples(&direct_frames),
        "the decoded output matches the direct decode once the stated gapless trim is applied"
    );
    let source_lufs = integrated_lufs(&source, &direct_frames);
    let output_lufs = integrated_lufs(&output_parameters, &output_frames);
    eprintln!(
        "libav AAC round trip: packets={} decoded_out={} decoded_direct={} lufs_out={output_lufs:.3} lufs_direct={source_lufs:.3}",
        packets.len(),
        decoded_samples(&output_frames),
        decoded_samples(&direct_frames),
    );
    assert!(
        (source_lufs - output_lufs).abs() <= TOLERANCE_LU,
        "the transcode keeps loudness: {source_lufs:.3} LUFS in, {output_lufs:.3} LUFS out"
    );
}

/// The video track through libav's H.264 decoder and MPEG-2 encoder, out to
/// TS and back into libav's MPEG-2 decoder.
#[test]
fn the_stage_transcodes_the_video_track_to_mpeg2_and_the_output_decodes() {
    let Some(registry) = registry() else {
        return;
    };
    if registry
        .candidates(CodecId::Mpeg2Video, Direction::Encode)
        .is_empty()
    {
        eprintln!("skip: the linked libav offers no MPEG-2 video encoder here");
        return;
    }
    let (specs, samples) = fixture();
    let video = spec_of(&specs, "avc").clone();
    let source = codec_parameters_from_track(&video).expect("the video track converts");
    let decoder = registry.decoder(&source).expect("a libav H.264 decoder");
    let encoder = registry
        .encoder(
            &EncoderConfig::new(CodecId::Mpeg2Video, timescale(MPEG2_TICKS))
                .with_video(VideoParameters::new(
                    NonZeroU32::new(TS_WIDTH).expect("a width"),
                    NonZeroU32::new(TS_HEIGHT).expect("a height"),
                ))
                .with_bit_rate(NonZeroU64::new(MPEG2_BIT_RATE).expect("a bit rate"))
                .with_gop(NonZeroU32::new(MPEG2_GOP).expect("a group length")),
        )
        .expect("a libav MPEG-2 encoder");
    let mut stage = TranscodeStage::new(video.track_id, decoder, encoder);

    let outputs = drive(&mut stage, &specs, &samples).expect("the stage transcodes");
    let track = announced(&outputs, video.track_id);
    let CodecConfig::Mpeg2Video {
        width,
        height,
        ..
    } = track.config
    else {
        panic!("the encoder reports MPEG-2 video, got {:?}", track.config)
    };
    assert_eq!((u32::from(width), u32::from(height)), (TS_WIDTH, TS_HEIGHT));
    let packets = packets_for(&outputs, video.track_id);
    assert!(
        packets.len() >= TS_VIDEO_FRAMES - 1,
        "every input picture reached the encoder: {} packets",
        packets.len()
    );

    let back = media_from_ts(&mux_ts(outputs)).expect("re-demux the TS output");
    let output_track = back
        .tracks
        .iter()
        .find(|candidate| matches!(candidate.spec.config, CodecConfig::Mpeg2Video { .. }))
        .expect("the output carries the MPEG-2 track");
    let parameters =
        codec_parameters_from_track(&output_track.spec).expect("the output track converts");
    let frames = decode(&registry, &parameters, &output_track.samples);
    assert_eq!(
        frames.len(),
        TS_VIDEO_FRAMES,
        "the re-demuxed stream decodes to the fixture's own picture count"
    );
    let times: Vec<i64> = output_track
        .samples
        .iter()
        .filter_map(|sample| sample.timing.and_then(|timing| timing.dts()))
        .collect();
    assert!(
        times.windows(2).all(|pair| pair[0] <= pair[1]),
        "decode timestamps stay monotonic through the stage"
    );
}
