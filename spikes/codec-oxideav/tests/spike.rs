//! The spike's acceptance tests: loudness, cross-check, other decoders and
//! the inventory.

mod support;

use std::sync::atomic::Ordering;

use broadcast_codec::{
    Availability, CodecId, CodecProfile, Frame, Packet, Registry, parse_preferences,
    render_inventory,
};
use broadcast_loudness::LoudnessMeter;
use codec_oxideav::OxideavProvider;
use codec_symphonia::SymphoniaProvider;

/// Integrated loudness measured with ffmpeg 8.1.2 `ebur128` on the same file.
const EXPECTED_LUFS: f64 = -21.1;
/// Allowed difference from the pinned value, in LU.
const TOLERANCE_LU: f64 = 0.1;
/// The RMS bound for a decoder compared with ffmpeg.
const RMS_TOLERANCE: f32 = 1e-4;

#[test]
fn t1_aac_loudness_matches_the_pinned_value() {
    let registry = Registry::new()
        .with(Box::new(OxideavProvider))
        .expect("the provider registers");
    let parameters = support::mono_parameters(CodecId::Aac);
    let packets = support::aac_packets();
    let decoded = support::decode_mono(&registry, &parameters, &packets);

    let mut meter = LoudnessMeter::new(
        support::SAMPLE_RATE,
        broadcast_loudness::ChannelLayout::Mono,
    )
    .expect("the meter accepts 48 kHz mono");
    for sample in &decoded {
        meter
            .push_f32(std::slice::from_ref(sample))
            .expect("the sample is finite");
    }
    meter.finish();
    let measured = meter.integrated_lufs();
    println!(
        "oxideav AAC: {measured:.3} LUFS over {} samples ({} packets, {} per packet)",
        decoded.len(),
        packets.len(),
        support::AAC_FRAME_SAMPLES
    );

    assert!(!decoded.is_empty(), "the decoder produced no audio");
    assert_eq!(
        decoded.len(),
        packets.len() * support::AAC_FRAME_SAMPLES,
        "the decoder lost or invented audio frames"
    );
    assert!(
        (measured - EXPECTED_LUFS).abs() <= TOLERANCE_LU,
        "integrated {measured:.3} LUFS, expected {EXPECTED_LUFS} +/- {TOLERANCE_LU}"
    );
}

#[test]
fn t1_timestamps_cross_unchanged() {
    let registry = Registry::new()
        .with(Box::new(OxideavProvider))
        .expect("the provider registers");
    let parameters = support::mono_parameters(CodecId::Aac);
    let packets = support::aac_packets();
    let mut decoder = registry.decoder(&parameters).expect("an AAC decoder");
    for (index, packet) in packets.iter().take(2).enumerate() {
        let pts = (index * support::AAC_FRAME_SAMPLES) as i64;
        decoder
            .send(Packet::borrowed(packet).with_pts(pts))
            .expect("the packet is accepted");
        let Some(Frame::Audio(audio)) = decoder.receive().expect("receive succeeds") else {
            panic!("no decoded frame");
        };
        assert_eq!(audio.pts(), Some(pts), "the timestamp changed");
    }
}

#[test]
fn t2_registry_selects_each_named_provider() {
    let parameters = support::mono_parameters(CodecId::Aac);
    let packets = support::aac_packets();

    let (registry, oxideav_calls, symphonia_calls) = two_provider_registry();
    let preference = parse_preferences("dec:aac=oxideav")
        .expect("a valid preference")
        .remove(0);
    let registry = registry.prefer(preference);
    let oxideav = support::decode_mono(&registry, &parameters, &packets);
    assert_eq!(
        oxideav_calls.load(Ordering::SeqCst),
        1,
        "oxideav was not served"
    );
    assert_eq!(
        symphonia_calls.load(Ordering::SeqCst),
        0,
        "symphonia was served"
    );

    let (registry, oxideav_calls, symphonia_calls) = two_provider_registry();
    let preference = parse_preferences("dec:aac=symphonia")
        .expect("a valid preference")
        .remove(0);
    let registry = registry.prefer(preference);
    let symphonia = support::decode_mono(&registry, &parameters, &packets);
    assert_eq!(
        oxideav_calls.load(Ordering::SeqCst),
        0,
        "oxideav was served"
    );
    assert_eq!(
        symphonia_calls.load(Ordering::SeqCst),
        1,
        "symphonia was not served"
    );

    assert!(!oxideav.is_empty());
    assert!(!symphonia.is_empty());
    let (offset, overlap, largest, rms) = support::align(&oxideav, &symphonia);
    println!(
        "oxideav {} samples, symphonia {} samples; offset {offset} samples \
         (minimum RMS over +/-{}), overlap {overlap}, max |diff| {largest:.9}, RMS {rms:.9}",
        oxideav.len(),
        symphonia.len(),
        4096,
    );
}

#[test]
fn t3_aac_sample_count_matches_ffmpeg() {
    let parameters = support::mono_parameters(CodecId::Aac);
    let packets = support::aac_packets();
    check_against_ffmpeg("aac", "audio/aac.adts", &parameters, &packets, true);
}

#[test]
fn t3_mp3_sample_count_matches_ffmpeg() {
    let (parameters, packets) = support::mp3_stream();
    check_against_ffmpeg("mp3", "audio/audio.mp3", &parameters, &packets, true);
}

#[test]
fn t3_mp3_aligned_audio_matches_ffmpeg() {
    let (parameters, packets) = support::mp3_stream();
    check_aligned_with_ffmpeg("mp3", "audio/audio.mp3", &parameters, &packets);
}

#[test]
fn t3_opus_sample_count_matches_ffmpeg() {
    let (parameters, packets) = support::opus_stream();
    check_against_ffmpeg("opus", "audio/opus.ogg", &parameters, &packets, true);
}

#[test]
fn t3_opus_aligned_audio_matches_ffmpeg() {
    let (parameters, packets) = support::opus_stream();
    check_aligned_with_ffmpeg("opus", "audio/opus.ogg", &parameters, &packets);
}

#[test]
fn t3_pcm_matches_ffmpeg() {
    let bytes = support::pcm_bytes();
    let packets = support::pcm_packets(&bytes);
    let parameters = support::mono_parameters(CodecId::Pcm);
    check_against_ffmpeg("pcm", "audio/pcm-s16le.wav", &parameters, &packets, false);
}

#[test]
fn t4_inventory_is_the_run_time_table() {
    let registry = Registry::new()
        .with(Box::new(OxideavProvider))
        .expect("the provider registers");
    let entries = registry.inventory(true);
    let table = render_inventory(&entries);
    println!("{table}");

    assert_eq!(entries.len(), 4);
    assert!(
        entries
            .iter()
            .all(|entry| entry.probe().availability() == Availability::Available)
    );
    assert_eq!(
        entries[0].descriptor().profiles(),
        [
            CodecProfile::AacLc,
            CodecProfile::AacHe,
            CodecProfile::AacHeV2
        ]
    );
    let providers = registry.providers();
    assert_eq!(providers.len(), 1);
    assert_eq!(providers[0].licence(), Some("MIT"));
    let library = providers[0].library().expect("a library is named");
    assert!(library.contains("oxideav-aac 0.1.7"), "{library}");
    assert!(library.contains("oxideav-opus 0.0.15"), "{library}");
    assert!(
        table.contains("529"),
        "the MPEG audio implementation's measured delay is shown:\n{table}"
    );
}

/// Build a registry holding both providers, with fresh call counters.
fn two_provider_registry() -> (
    Registry,
    std::sync::Arc<std::sync::atomic::AtomicUsize>,
    std::sync::Arc<std::sync::atomic::AtomicUsize>,
) {
    let (oxideav, oxideav_calls) = support::CountingProvider::new(Box::new(OxideavProvider));
    let (symphonia, symphonia_calls) = support::CountingProvider::new(Box::new(SymphoniaProvider));
    let registry = Registry::new()
        .with(Box::new(oxideav))
        .expect("oxideav registers")
        .with(Box::new(symphonia))
        .expect("symphonia registers");
    (registry, oxideav_calls, symphonia_calls)
}

/// The numbers from comparing one decoded stream with ffmpeg's.
struct Comparison {
    decoded: usize,
    reference: usize,
    offset: i64,
    overlap: usize,
    largest: f32,
    rms: f32,
}

/// Decode through OxideAV, compare with ffmpeg, and report the numbers.
fn compare_with_ffmpeg(
    name: &str,
    fixture: &str,
    parameters: &broadcast_codec::CodecParameters,
    packets: &[Vec<u8>],
    trimmed: bool,
) -> Option<Comparison> {
    let registry = Registry::new()
        .with(Box::new(OxideavProvider))
        .expect("the provider registers");
    let decoded = if trimmed {
        support::decode_mono_trimmed(&registry, parameters, packets)
    } else {
        support::decode_mono(&registry, parameters, packets)
    };
    let reference = support::ffmpeg_f32(fixture)?;
    let (offset, overlap, largest, rms) = support::align(&decoded, &reference);
    println!(
        "{name}: oxideav {} samples, ffmpeg {} samples; offset {offset} samples, \
         overlap {overlap}, max |diff| {largest:.9}, RMS {rms:.9}",
        decoded.len(),
        reference.len(),
    );
    assert!(!decoded.is_empty(), "the decoder produced no audio");
    Some(Comparison {
        decoded: decoded.len(),
        reference: reference.len(),
        offset,
        overlap,
        largest,
        rms,
    })
}

/// Decode through OxideAV, compare with ffmpeg, and require equal counts.
fn check_against_ffmpeg(
    name: &str,
    fixture: &str,
    parameters: &broadcast_codec::CodecParameters,
    packets: &[Vec<u8>],
    require_alignment: bool,
) {
    let Some(comparison) = compare_with_ffmpeg(name, fixture, parameters, packets, true) else {
        return;
    };
    assert_eq!(
        comparison.decoded, comparison.reference,
        "{name}: decoded sample count differs from ffmpeg"
    );
    assert!(
        comparison.rms <= RMS_TOLERANCE,
        "{name}: RMS difference {:.9} exceeds {RMS_TOLERANCE}",
        comparison.rms
    );
    if require_alignment {
        assert_eq!(
            comparison.offset, 0,
            "{name}: the trimmed stream must start at ffmpeg's first sample"
        );
    }
}

/// Decode through OxideAV, compare with ffmpeg over the aligned content, and
/// assert the audio itself matches even when one side carries lead-in/tail.
fn check_aligned_with_ffmpeg(
    name: &str,
    fixture: &str,
    parameters: &broadcast_codec::CodecParameters,
    packets: &[Vec<u8>],
) {
    let Some(comparison) = compare_with_ffmpeg(name, fixture, parameters, packets, false) else {
        return;
    };
    assert_eq!(
        comparison.overlap, comparison.reference,
        "{name}: the aligned overlap does not cover ffmpeg's whole decode"
    );
    assert!(
        comparison.rms <= RMS_TOLERANCE,
        "{name}: RMS difference {:.9} exceeds {RMS_TOLERANCE}",
        comparison.rms
    );
    assert!(
        comparison.largest <= 1e-3,
        "{name}: largest difference {:.9} exceeds 1e-3",
        comparison.largest
    );
    println!(
        "{name}: aligned audio matches at offset {} over {} samples",
        comparison.offset, comparison.overlap
    );
}

// ---------------------------------------------------------------------------
// The container gapless signals, decoded by OxideAV and trimmed by the seam.
// ---------------------------------------------------------------------------

/// Decode every packet through a plain decoder and return the raw frames.
fn decode_audio_frames(
    registry: &Registry,
    parameters: &broadcast_codec::CodecParameters,
    packets: &[Vec<u8>],
) -> Vec<broadcast_codec::AudioFrame> {
    let mut decoder = registry.decoder(parameters).expect("a decoder");
    let mut frames = Vec::new();
    for packet in packets {
        decoder
            .send(Packet::borrowed(packet))
            .expect("the packet is accepted");
        while let Some(frame) = decoder.receive().expect("receive succeeds") {
            let Frame::Audio(audio) = frame else {
                panic!("an audio decoder emitted a non-audio frame");
            };
            frames.push(audio);
        }
    }
    decoder.finish().expect("end of input is accepted");
    while let Some(frame) = decoder.receive().expect("receive succeeds") {
        let Frame::Audio(audio) = frame else {
            panic!("an audio decoder emitted a non-audio frame");
        };
        frames.push(audio);
    }
    frames
}

fn audio_collected_track(
    media: &test_harness::collect::Collected,
) -> &test_harness::collect::CollectedTrack {
    media
        .tracks
        .iter()
        .find(|track| track.spec.config.audio_sample_rate().is_some())
        .expect("an audio track")
}

fn oxideav_registry() -> Registry {
    Registry::new()
        .with(Box::new(OxideavProvider))
        .expect("the provider registers")
}

#[test]
fn matroska_discard_padding_trimmed_count_matches_ffmpeg() {
    use broadcast_codec::TrimStage;
    use test_harness::collect::{CollectMode, media_from_matroska};

    let Some(reference) = support::ffmpeg_f32("mkv/vp9-opus.webm") else {
        return;
    };
    let bytes = test_fixtures::read("mkv/vp9-opus.webm");
    let media = media_from_matroska(&bytes, CollectMode::Seekable).expect("demux");
    let track = audio_collected_track(&media);
    let trim = track.samples.last().expect("a last sample").trim;
    assert_eq!(trim.end(), 648, "the WebM states 13.5 ms of end padding");
    let parameters = transmux::codec_parameters_from_track(&track.spec).expect("converts");
    // The seam's extradata for an Opus track is the `dOps` record; OxideAV's
    // decoder takes the raw `OpusHead`, with the pre-skip cleared because it
    // applies that itself. The container's trim is the stage's job (the same
    // arrangement as `support::opus_stream`).
    let transmux::ir::CodecConfig::Opus { config, .. } = &track.spec.config else {
        panic!("an Opus track");
    };
    let mut decoder_head = Vec::new();
    decoder_head.extend_from_slice(b"OpusHead");
    decoder_head.push(config.version);
    decoder_head.push(config.output_channel_count);
    decoder_head.extend_from_slice(&0u16.to_le_bytes());
    decoder_head.extend_from_slice(&config.input_sample_rate.to_le_bytes());
    decoder_head.extend_from_slice(&config.output_gain.to_le_bytes());
    decoder_head.push(config.channel_mapping_family);
    if let Some(map) = &config.channel_mapping {
        decoder_head.push(map.stream_count);
        decoder_head.push(map.coupled_count);
        decoder_head.extend_from_slice(&map.channel_mapping);
    }
    let raw = parameters
        .clone()
        .with_extradata(decoder_head)
        .with_initial_padding(0)
        .expect("audio parameters take padding")
        .with_trailing_padding(0)
        .expect("audio parameters take padding");
    let packets: Vec<Vec<u8>> = track.samples.iter().map(|s| s.data.to_vec()).collect();
    let frames = decode_audio_frames(&oxideav_registry(), &raw, &packets);
    let trimmed = parameters
        .clone()
        .with_initial_padding(track.spec.padding.initial())
        .expect("audio parameters take padding")
        .with_trailing_padding(0)
        .expect("audio parameters take padding");
    let mut stage = TrimStage::new(&trimmed);
    let mut produced = 0usize;
    let last = frames.len().saturating_sub(1);
    for (index, frame) in frames.into_iter().enumerate() {
        let frame = if index == last {
            frame.with_trim_end(trim.end())
        } else {
            frame
        };
        for frame in stage.push(frame) {
            produced += frame.frames();
        }
    }
    for frame in stage.finish() {
        produced += frame.frames();
    }
    println!(
        "matroska DiscardPadding: {produced} trimmed samples, ffmpeg {}",
        reference.len()
    );
    assert_eq!(
        produced,
        reference.len(),
        "the stage's trimmed count must equal ffmpeg's decoded count"
    );
}
