#![cfg(feature = "std")]
//! I-frame master playlist advertising (P2b).

fn timescale(value: u32) -> Timescale {
    Timescale::try_new(value).expect("a non-zero timescale")
}

use broadcast_common::timescale::Timescale;
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Duration;

use broadcast_common::Timestamp;
use broadcast_hls::MasterPlaylist;
use bytes::Bytes;
use hls_runtime::server::{HlsOrigin, HlsOriginConfig};
use media_plane::ids::SegmentSeq;
use media_plane::trunk::{SegmentEntry, SegmentWriter, Trunk, TrunkConfig};
use transmux::SegmentMeta;
use transmux::TrackId;
use transmux::pipeline::{
    CodecConfig, FragmentTrackData, TrackSpec, build_init_segment, build_media_segment,
};
use transmux::ts_demux::TsDemux;

fn nz(n: usize) -> NonZeroUsize {
    NonZeroUsize::new(n).expect("non-zero")
}

fn trunk_with(cap: usize) -> (Arc<Trunk>, SegmentWriter) {
    let trunk = Trunk::new(
        TrunkConfig::default()
            .with_timed_capacity(nz(64))
            .with_sparse_capacity(nz(8))
            .with_segment_capacity(nz(64))
            .with_event_capacity(nz(8))
            .with_part_capacity(nz(cap)),
    );
    let writer = trunk.segment_writer().expect("writer");
    (trunk, writer)
}

fn origin_fmp4(cap: usize) -> (Arc<Trunk>, HlsOrigin, SegmentWriter) {
    let (trunk, writer) = trunk_with(cap);
    let origin = HlsOrigin::new(
        HlsOriginConfig::new(Arc::clone(&trunk), 4.0, nz(3))
            .with_low_latency(Duration::from_millis(500))
            .with_instance(7)
            .with_declared_bandwidth_bps(5_000_000),
    )
    .expect("origin");
    let init = test_fixtures::read("hls/cmaf-fmp4/init.mp4");
    origin.set_init(Bytes::from(init));
    (trunk, origin, writer)
}

fn fixture_bytes(name: &str) -> Vec<u8> {
    test_fixtures::read(&format!("hls/cmaf-fmp4/{name}"))
}

fn compute_iframe_len(segment: &[u8], video_track_id: TrackId) -> Option<u64> {
    let found =
        transmux::box_types::clamped_boxes(segment).find(|b| b.header.box_type.is(b"moof"))?;
    let moof_off = found.start;
    let body = &segment[found.payload_start()..found.end];
    let moof = transmux::movie_fragment::MovieFragmentBox::parse_body(body).ok()?;

    let mdats = transmux::frag_offsets::mdat_ranges(segment);
    let samples =
        transmux::frag_offsets::sample_ranges_in(segment, &mdats, moof_off, &moof, video_track_id)
            .ok()?;
    if samples.is_empty() {
        return None;
    }
    let first = samples[0];
    if first.flags & (1 << 16) != 0 {
        return None;
    }
    let mdat_payload_start = transmux::box_types::clamped_boxes(segment)
        .filter(|b| b.header.box_type.is(b"mdat"))
        .find(|b| (b.payload_start()..b.end).contains(&first.start))?
        .payload_start();
    if first.start != mdat_payload_start {
        return None;
    }
    u64::try_from(first.end).ok()
}

fn independent_peak(entries: &[(Vec<u8>, Duration)]) -> u64 {
    entries
        .iter()
        .map(|(b, d)| {
            let len = compute_iframe_len(b, TrackId::new(1)).expect("iframe len");
            let nanos = d.as_nanos();
            let bps = (u128::from(len) * 8 * 1_000_000_000).div_ceil(nanos);
            u64::try_from(bps).expect("bps")
        })
        .max()
        .expect("max")
}

fn independent_mean(entries: &[(Vec<u8>, Duration)]) -> u32 {
    let total_bytes: u64 = entries
        .iter()
        .map(|(b, _)| compute_iframe_len(b, TrackId::new(1)).expect("len"))
        .sum();
    let total_nanos: u128 = entries.iter().map(|(_, d)| d.as_nanos()).sum();
    let mean = (u128::from(total_bytes) * 8 * 1_000_000_000 + total_nanos / 2) / total_nanos;
    u32::try_from(mean).expect("mean")
}

#[test]
fn iframe_master_typed_fields() {
    let (_trunk, origin, writer) = origin_fmp4(64);
    let names = ["index0.m4s", "index1.m4s", "index2.m4s"];
    let mut entries = Vec::new();
    for (i, name) in names.iter().enumerate() {
        let bytes = fixture_bytes(name);
        entries.push((bytes.clone(), Duration::from_secs(1)));
        writer
            .publish_segment(SegmentEntry::new(
                Bytes::from(bytes),
                SegmentSeq::new(i as u32 + 1),
                Duration::from_secs(1),
                Timestamp::from_nanos(0),
                SegmentMeta {
                    discontinuous: false,
                },
            ))
            .expect("publish");
    }

    let text = origin
        .master_playlist_with_iframes("media.m3u8", "iframe.m3u8")
        .expect("master");
    let parsed = MasterPlaylist::parse(&text).expect("parse master");
    assert_eq!(parsed.iframe_variants.len(), 1);
    let iv = &parsed.iframe_variants[0];
    assert_eq!(iv.bandwidth, independent_peak(&entries) as u32);
    assert_eq!(iv.average_bandwidth, Some(independent_mean(&entries)));
    assert_eq!(iv.codecs.as_deref(), Some("avc1.4D400D"));
    assert!(iv.codecs.as_deref().unwrap().contains("avc1"));
    assert!(!iv.codecs.as_deref().unwrap().contains("mp4a"));
    // resolution matches the variant's (media variant resolution)
    assert_eq!(iv.resolution, Some((320, 240)));
    assert_eq!(iv.uri, "iframe.m3u8");
    // no FRAME-RATE on the I-frame line
    for line in text.lines() {
        if line.contains("I-FRAME-STREAM-INF") {
            assert!(!line.contains("FRAME-RATE"));
        }
    }
    // plain master unchanged (no iframe variant)
    let plain = origin.master_playlist("media.m3u8").expect("plain");
    assert!(!plain.contains("I-FRAME-STREAM-INF"));
}

#[test]
fn iframe_master_omitted_before_first_segment() {
    let (_trunk, origin, _writer) = origin_fmp4(64);
    let text = origin
        .master_playlist_with_iframes("media.m3u8", "iframe.m3u8")
        .expect("master");
    assert!(!text.contains("I-FRAME-STREAM-INF"));
}

#[test]
fn iframe_master_audio_only_omitted() {
    let (trunk, writer) = trunk_with(64);
    let origin = HlsOrigin::new(
        HlsOriginConfig::new(Arc::clone(&trunk), 4.0, nz(3))
            .with_instance(7)
            .with_declared_bandwidth_bps(5_000_000),
    )
    .expect("origin");
    origin.set_track_specs(&[]).expect("specs");
    let text = origin
        .master_playlist_with_iframes("media.m3u8", "iframe.m3u8")
        .expect("master");
    assert!(!text.contains("I-FRAME-STREAM-INF"));
    let _ = writer;
}

#[test]
fn iframe_master_d3prime_unavailable_omitted() {
    // Build an audio-first segment that follows an entry.
    let Some(data) = test_fixtures::require_bytes("ts/h264-aac.ts", "iframe") else {
        return;
    };
    let media = TsDemux::new().demux(&data).expect("demux");
    let specs: Vec<TrackSpec> = media.tracks.iter().map(|t| t.spec.clone()).collect();
    let init = build_init_segment(&specs, media.movie_timescale).expect("init");
    let audio = media
        .tracks
        .iter()
        .find(|t| matches!(t.config(), CodecConfig::Aac { .. }))
        .expect("audio");
    let video = media
        .tracks
        .iter()
        .find(|t| matches!(t.config(), CodecConfig::Avc { .. }))
        .expect("video");
    let samples_n = 5usize.min(video.samples.len()).min(audio.samples.len());
    let frags = vec![
        FragmentTrackData::new(
            audio.track_id(),
            audio.start_decode_time,
            &audio.samples[..samples_n],
        ),
        FragmentTrackData::new(
            video.track_id(),
            video.start_decode_time,
            &video.samples[..samples_n],
        ),
    ];
    let seg_bytes = build_media_segment(1, &frags).expect("segment");

    let (_trunk, origin, writer) = origin_fmp4(64);
    origin.set_init(Bytes::from(init));
    // entry first
    let normal = fixture_bytes("index0.m4s");
    writer
        .publish_segment(SegmentEntry::new(
            Bytes::from(normal),
            SegmentSeq::new(1),
            Duration::from_secs(1),
            Timestamp::from_nanos(0),
            SegmentMeta {
                discontinuous: false,
            },
        ))
        .expect("publish");
    // audio-first after entry -> unavailable
    writer
        .publish_segment(SegmentEntry::new(
            Bytes::from(seg_bytes),
            SegmentSeq::new(2),
            Duration::from_secs(1),
            Timestamp::from_nanos(0),
            SegmentMeta {
                discontinuous: false,
            },
        ))
        .expect("publish");
    let text = origin
        .master_playlist_with_iframes("media.m3u8", "iframe.m3u8")
        .expect("master");
    assert!(!text.contains("I-FRAME-STREAM-INF"));
}

#[test]
fn iframe_master_survives_eviction() {
    let (_trunk, origin, writer) = origin_fmp4(3);
    let names = ["index0.m4s", "index1.m4s", "index2.m4s", "index0.m4s"];
    let mut entries = Vec::new();
    for (i, name) in names.iter().enumerate() {
        let bytes = fixture_bytes(name);
        entries.push((bytes.clone(), Duration::from_secs(1)));
        writer
            .publish_segment(SegmentEntry::new(
                Bytes::from(bytes),
                SegmentSeq::new(i as u32 + 1),
                Duration::from_secs(1),
                Timestamp::from_nanos(0),
                SegmentMeta {
                    discontinuous: false,
                },
            ))
            .expect("publish");
    }
    let text = origin
        .master_playlist_with_iframes("media.m3u8", "iframe.m3u8")
        .expect("master");
    let parsed = MasterPlaylist::parse(&text).expect("parse");
    let iv = &parsed.iframe_variants[0];
    assert_eq!(iv.bandwidth, independent_peak(&entries) as u32);
    assert_eq!(iv.average_bandwidth, Some(independent_mean(&entries)));
}

#[test]
fn iframe_master_resets_on_init_change() {
    let (_trunk, origin, writer) = origin_fmp4(64);
    let bytes = fixture_bytes("index0.m4s");
    writer
        .publish_segment(SegmentEntry::new(
            Bytes::from(bytes),
            SegmentSeq::new(1),
            Duration::from_secs(1),
            Timestamp::from_nanos(0),
            SegmentMeta {
                discontinuous: false,
            },
        ))
        .expect("publish");
    // unparseable init resets the snapshot
    origin.set_init(Bytes::from(vec![0u8; 8]));
    let text = origin
        .master_playlist_with_iframes("media.m3u8", "iframe.m3u8")
        .expect("master");
    assert!(!text.contains("I-FRAME-STREAM-INF"));
    let plain = origin.master_playlist("media.m3u8").expect("plain");
    assert!(!plain.contains("I-FRAME-STREAM-INF"));
}

#[test]
fn iframe_master_plain_byte_identical() {
    let (_trunk, origin, writer) = origin_fmp4(64);
    let bytes = fixture_bytes("index0.m4s");
    writer
        .publish_segment(SegmentEntry::new(
            Bytes::from(bytes),
            SegmentSeq::new(1),
            Duration::from_secs(1),
            Timestamp::from_nanos(0),
            SegmentMeta {
                discontinuous: false,
            },
        ))
        .expect("publish");
    let a = origin.master_playlist("media.m3u8").expect("a");
    let b = origin.master_playlist("media.m3u8").expect("b");
    assert_eq!(a, b);
    assert!(!a.contains("I-FRAME-STREAM-INF"));
}

// ----- VIDEO-RANGE on the I-frame variant (Apple 9.16 covers variants AND I-frame variants) -----

fn hdr_spec(transfer_code: u8) -> TrackSpec {
    use transmux::ir::{ColourDescription, DisplayGeometry, VideoFormat, VideoSignalType};
    use transmux::{AVCConfigurationBox, AVCDecoderConfigurationRecord, AvcPps, AvcSps};
    let config = AVCConfigurationBox::new(AVCDecoderConfigurationRecord {
        configuration_version: 1,
        profile_indication: 66,
        profile_compatibility: 0,
        level_indication: 30,
        length_size_minus_one: 3,
        sps: vec![AvcSps(vec![0x67, 66, 0, 30, 0x00])],
        pps: vec![AvcPps(vec![0x68, 0xCE, 0x3C, 0x80])],
        chroma_format: None,
        bit_depth_luma_minus8: None,
        bit_depth_chroma_minus8: None,
        sps_ext: vec![],
    });
    let spec = TrackSpec::new(
        TrackId::new(1),
        timescale(90_000),
        CodecConfig::Avc {
            config,
            width: 320,
            height: 240,
        },
    );
    let mut g = DisplayGeometry::new(320, 240, None);
    g.video_signal = Some(VideoSignalType::new(
        VideoFormat::Component,
        false,
        Some(ColourDescription::from_wire(1, transfer_code, 1)),
    ));
    spec.with_display(Some(g))
}

fn iframe_variant_line(transfer_code: u8) -> String {
    let (_trunk, origin, writer) = origin_fmp4(64);
    origin
        .set_track_specs(&[hdr_spec(transfer_code)])
        .expect("specs");
    for (i, name) in ["index0.m4s", "index1.m4s"].iter().enumerate() {
        writer
            .publish_segment(SegmentEntry::new(
                Bytes::from(fixture_bytes(name)),
                SegmentSeq::new(i as u32 + 1),
                Duration::from_secs(1),
                Timestamp::from_nanos(0),
                SegmentMeta {
                    discontinuous: false,
                },
            ))
            .expect("publish");
    }
    let text = origin
        .master_playlist_with_iframes("media.m3u8", "iframe.m3u8")
        .expect("master");
    text.lines()
        .find(|l| l.starts_with("#EXT-X-I-FRAME-STREAM-INF"))
        .unwrap_or_else(|| panic!("no I-frame variant in:\n{text}"))
        .to_string()
}

#[test]
fn iframe_variant_carries_video_range_for_hdr() {
    // transfer 16 = PQ (SMPTE ST 2084), 18 = HLG (ARIB STD-B67): HLS VIDEO-RANGE values.
    assert!(iframe_variant_line(16).contains("VIDEO-RANGE=PQ"), "PQ");
    assert!(iframe_variant_line(18).contains("VIDEO-RANGE=HLG"), "HLG");
    // SDR (transfer 1): the attribute is omitted (Apple 9.16 requires it only for non-SDR).
    assert!(!iframe_variant_line(1).contains("VIDEO-RANGE"), "SDR");
}
