#![cfg(feature = "std")]
//! Audit 22 finding 57 (issue #3025): the origin's codec signalling —
//! Dolby Vision `CODECS`/`SUPPLEMENTAL-CODECS`/`VIDEO-RANGE`, HDR10+,
//! `CHANNELS`, measured `FRAME-RATE`, the instance-token epoch and
//! `EXT-X-RENDITION-REPORT`'s relative URI.
//!
//! Spec: `private/specs-verified/dolby/vision/dolby-vision-streams-in-hls-v2.0-2018-11.md`,
//! `private/specs-verified/hls/apple/hls-authoring-spec.md` ("The
//! SUPPLEMENTAL-CODECS attribute"), RFC 8216bis §4.4.5.4, §4.3.4.2.

use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Duration;

use broadcast_common::Timestamp;
use broadcast_hls::MasterPlaylist;
use bytes::Bytes;
use hls_runtime::server::{
    BlockingQuery, DEFAULT_TRACK_ID, HlsBody, HlsOrigin, HlsOriginConfig, HlsRequest,
    RenditionSibling, playlist_reload_timeout,
};
use media_plane::egress::{AwaitPolicy, EgressResponse, ServedEgress};
use media_plane::ids::SegmentSeq;
use media_plane::trunk::{SegmentEntry, SegmentWriter, Trunk, TrunkConfig};
use transmux::TrackSpec;
use transmux::ir::CodecConfig;

fn nz(n: usize) -> NonZeroUsize {
    NonZeroUsize::new(n).expect("non-zero")
}

fn new_trunk(capacity: usize) -> (Arc<Trunk>, SegmentWriter) {
    let trunk = Trunk::new(
        TrunkConfig::default()
            .with_timed_capacity(nz(64))
            .with_sparse_capacity(nz(8))
            .with_segment_capacity(nz(capacity))
            .with_event_capacity(nz(8))
            .with_part_capacity(nz(64)),
    );
    let writer = trunk.segment_writer().expect("segment writer");
    (trunk, writer)
}

fn origin(trunk: &Arc<Trunk>, capacity: usize) -> HlsOrigin {
    HlsOrigin::new(
        HlsOriginConfig::new(Arc::clone(trunk), Duration::from_secs(4), nz(capacity))
            .with_instance(7)
            .with_declared_bandwidth_bps(1_000_000),
    )
    .expect("origin builds")
}

fn master(origin: &HlsOrigin) -> String {
    origin
        .master_playlist("media.m3u8")
        .expect("master playlist")
}

fn set_init(origin: &HlsOrigin) {
    origin.set_init(Bytes::from(test_fixtures::read("hls/cmaf-fmp4/init.mp4")));
}

fn publish(writer: &SegmentWriter, seq: u32, bytes: &[u8]) {
    writer
        .publish_segment(SegmentEntry::new(
            Bytes::copy_from_slice(bytes),
            SegmentSeq::new(seq),
            Duration::from_secs(1),
            Timestamp::from_nanos(0),
            false,
        ))
        .expect("monotonic sequence");
}

fn video_specs(fixture: &str) -> Option<Vec<TrackSpec>> {
    let data = test_fixtures::require_bytes(fixture, "origin_codecs")?;
    let media = test_harness::collect::media_from_mp4(
        &data[..],
        test_harness::collect::CollectMode::Seekable,
    )
    .ok()?;
    Some(
        media
            .tracks
            .into_iter()
            .filter(|t| t.spec.config.is_video())
            .map(|t| t.spec)
            .collect(),
    )
}

fn dv_master(fixture: &str) -> Option<String> {
    let specs = video_specs(fixture)?;
    let (trunk, _writer) = new_trunk(8);
    let origin = origin(&trunk, 8);
    origin.set_track_specs(&specs).expect("specs");
    Some(master(&origin))
}

fn record(
    profile: u8,
    el_present: bool,
    ccid: isobmff::DoviBlSignalCompatibilityId,
) -> isobmff::DoviDecoderConfigurationRecord {
    isobmff::DoviDecoderConfigurationRecord {
        version_major: 1,
        version_minor: 0,
        profile,
        level: 6,
        rpu_present: true,
        el_present,
        bl_present: true,
        bl_signal_compatibility_id: ccid,
        reserved: isobmff::dovi::DOVI_RESERVED_ZERO,
    }
}

fn hevc_spec() -> TrackSpec {
    let specs = video_specs("mp4/hevc-main.mp4").expect("HEVC fixture");
    specs.into_iter().next().expect("a video track")
}

fn avc_spec() -> TrackSpec {
    let data =
        test_fixtures::require_bytes("ts/h264-aac.ts", "origin_codecs").expect("h264 fixture");
    let media = test_harness::collect::media_from_ts(&data).expect("TS demuxes");
    media
        .tracks
        .into_iter()
        .find(|t| t.spec.config.is_video())
        .expect("a video track")
        .spec
}

#[test]
fn a_profile_5_dolby_vision_stream_is_announced_as_dvh1_in_codecs() {
    let Some(body) = dv_master("video/dolby-vision/sollevante-dv-p5-1080p24-10s.mp4") else {
        return;
    };
    assert!(body.contains("CODECS=\"dvh1."), "{body}");
    assert!(body.contains("VIDEO-RANGE=PQ"), "{body}");
    assert!(!body.contains("SUPPLEMENTAL-CODECS"), "{body}");
}

#[test]
fn an_hdr10_compatible_dolby_vision_stream_names_the_db1p_brand() {
    let Some(body) = dv_master("video/dolby-vision/sollevante-dv-p8.1-1080p24-10s.mp4") else {
        return;
    };
    assert!(body.contains("SUPPLEMENTAL-CODECS=\""), "{body}");
    assert!(body.contains("/db1p\""), "{body}");
    assert!(body.contains("VIDEO-RANGE=PQ"), "{body}");
}

#[test]
fn an_hlg_compatible_dolby_vision_stream_names_the_db4h_brand() {
    let Some(body) = dv_master("video/dolby-vision/iphone18promax-dv-p8.4-2160p30-7s.mp4") else {
        return;
    };
    assert!(body.contains("SUPPLEMENTAL-CODECS=\""), "{body}");
    assert!(body.contains("/db4h\""), "{body}");
    assert!(body.contains("VIDEO-RANGE=HLG"), "{body}");
}

#[test]
fn a_non_single_layer_dolby_vision_record_is_not_signalled() {
    let (trunk, _writer) = new_trunk(8);
    let origin = origin(&trunk, 8);
    let mut spec = hevc_spec();
    spec.dovi = Some(record(8, true, isobmff::DoviBlSignalCompatibilityId::Hdr10));
    origin.set_track_specs(&[spec]).expect("specs");
    let body = master(&origin);
    assert!(!body.contains("dvh1"), "{body}");
    assert!(!body.contains("SUPPLEMENTAL-CODECS"), "{body}");
}

#[test]
fn an_avc_profile_dolby_vision_record_is_not_signalled() {
    let (trunk, _writer) = new_trunk(8);
    let origin = origin(&trunk, 8);
    let mut spec = avc_spec();
    spec.dovi = Some(record(
        9,
        false,
        isobmff::DoviBlSignalCompatibilityId::Hdr10,
    ));
    origin.set_track_specs(&[spec]).expect("specs");
    let body = master(&origin);
    assert!(!body.contains("dvh1"), "{body}");
    assert!(!body.contains("dva1"), "{body}");
    assert!(!body.contains("SUPPLEMENTAL-CODECS"), "{body}");
}

#[test]
fn an_hdr10plus_brand_needs_the_flag_and_a_pq_base() {
    let (trunk, _writer) = new_trunk(8);
    let origin = origin(&trunk, 8);
    let mut spec = hevc_spec();
    let mut display = transmux::ir::DisplayGeometry::new(1920, 1080, None);
    display.video_signal = Some(transmux::ir::VideoSignalType::new(
        transmux::ir::VideoFormat::Component,
        false,
        Some(transmux::ir::ColourDescription::from_wire(9, 16, 9)),
    ));
    spec = spec.with_display(Some(display));
    spec.hdr10plus = false;
    origin.set_track_specs(&[spec]).expect("specs");
    let body = master(&origin);
    assert!(!body.contains("cdm4"), "{body}");
    assert!(!body.contains("SUPPLEMENTAL-CODECS"), "{body}");
}

#[test]
fn duplicate_caption_rendition_names_are_disambiguated() {
    let (trunk, _writer) = new_trunk(8);
    let decl = |id: &str| hls_runtime::server::ClosedCaptionDecl {
        instream_id: id.into(),
        name: "Caption".into(),
        language: String::new(),
    };
    let origin = HlsOrigin::new(
        HlsOriginConfig::new(Arc::clone(&trunk), Duration::from_secs(4), nz(8))
            .with_instance(7)
            .with_declared_bandwidth_bps(1_000_000)
            .with_closed_caption_overrides(vec![decl("CC1"), decl("CC2")]),
    )
    .expect("origin builds");
    origin.set_detected_closed_captions(vec![
        broadcast_hls::InstreamId::Cc(1),
        broadcast_hls::InstreamId::Cc(2),
    ]);
    let body = master(&origin);
    assert!(body.contains("NAME=\"Caption\""), "{body}");
    assert!(body.contains("NAME=\"Caption (2)\""), "{body}");
}

#[test]
fn a_segment_named_with_another_instance_token_is_not_found() {
    let (trunk, writer) = new_trunk(8);
    let origin = origin(&trunk, 8);
    publish(&writer, 1, &[1; 8]);
    let (now, policy) = (
        Timestamp::from_nanos(0),
        AwaitPolicy::new(Timestamp::from_nanos(60_000_000_000)),
    );
    let get = |name: &str| {
        origin.resolve(
            HlsRequest::Resource {
                name: name.to_string(),
            },
            now,
            policy,
        )
    };
    assert_eq!(get("seg-1-8-1.m4s"), EgressResponse::NotFound);
    assert!(matches!(get("seg-1-7-1.m4s"), EgressResponse::Ready { .. }));
}

#[test]
fn the_rendition_report_uri_is_relative_to_a_nested_playlist() {
    let (own_trunk, own_writer) = new_trunk(8);
    let (sibling_trunk, _sibling_writer) = new_trunk(8);
    let own = HlsOrigin::new(
        HlsOriginConfig::new(Arc::clone(&own_trunk), Duration::from_secs(4), nz(8))
            .with_instance(7)
            .with_low_latency(Duration::from_millis(500))
            .with_playlist_uri("live/main.m3u8"),
    )
    .expect("origin builds");
    let sibling = Arc::new(
        HlsOrigin::new(
            HlsOriginConfig::new(Arc::clone(&sibling_trunk), Duration::from_secs(4), nz(8))
                .with_instance(8)
                .with_low_latency(Duration::from_millis(500))
                .with_playlist_uri("live/audio/a0.m3u8"),
        )
        .expect("sibling builds"),
    );
    own.set_rendition_siblings(vec![RenditionSibling {
        origin: Arc::downgrade(&sibling),
        playlist_uri: "live/audio/a0.m3u8".to_string(),
    }]);
    publish(&own_writer, 1, &[1; 8]);
    let (now, policy) = (
        Timestamp::from_nanos(0),
        AwaitPolicy::new(Timestamp::from_nanos(60_000_000_000)),
    );
    let body = match own.resolve(
        HlsRequest::Playlist {
            track_id: DEFAULT_TRACK_ID,
            query: BlockingQuery::default(),
        },
        now,
        policy,
    ) {
        EgressResponse::Ready {
            body: HlsBody::Playlist(body),
            ..
        } => body,
        other => panic!("expected a playlist, got {other:?}"),
    };
    assert!(
        body.contains("#EXT-X-RENDITION-REPORT:URI=\"audio/a0.m3u8\""),
        "same directory: {body}"
    );
}

#[test]
fn the_rendition_report_uri_climbs_out_of_an_absolute_directory() {
    let (own_trunk, own_writer) = new_trunk(8);
    let (sibling_trunk, _sibling_writer) = new_trunk(8);
    let own = HlsOrigin::new(
        HlsOriginConfig::new(Arc::clone(&own_trunk), Duration::from_secs(4), nz(8))
            .with_instance(7)
            .with_low_latency(Duration::from_millis(500))
            .with_playlist_uri("/live/main.m3u8"),
    )
    .expect("origin builds");
    let sibling = Arc::new(
        HlsOrigin::new(
            HlsOriginConfig::new(Arc::clone(&sibling_trunk), Duration::from_secs(4), nz(8))
                .with_instance(8)
                .with_low_latency(Duration::from_millis(500))
                .with_playlist_uri("/audio/a0.m3u8"),
        )
        .expect("sibling builds"),
    );
    own.set_rendition_siblings(vec![RenditionSibling {
        origin: Arc::downgrade(&sibling),
        playlist_uri: "/audio/a0.m3u8".to_string(),
    }]);
    publish(&own_writer, 1, &[1; 8]);
    let (now, policy) = (
        Timestamp::from_nanos(0),
        AwaitPolicy::new(Timestamp::from_nanos(60_000_000_000)),
    );
    let body = match own.resolve(
        HlsRequest::Playlist {
            track_id: DEFAULT_TRACK_ID,
            query: BlockingQuery::default(),
        },
        now,
        policy,
    ) {
        EgressResponse::Ready {
            body: HlsBody::Playlist(body),
            ..
        } => body,
        other => panic!("expected a playlist, got {other:?}"),
    };
    assert!(
        body.contains("#EXT-X-RENDITION-REPORT:URI=\"../audio/a0.m3u8\""),
        "{body}"
    );
}

#[test]
fn the_blocking_reload_timeout_is_three_target_durations_with_a_floor() {
    let (trunk, writer) = new_trunk(8);
    let origin = HlsOrigin::new(
        HlsOriginConfig::new(Arc::clone(&trunk), Duration::from_secs(6), nz(8)).with_instance(7),
    )
    .expect("origin builds");
    assert_eq!(playlist_reload_timeout(&origin), Duration::from_secs(18));
    let (cold_trunk, _cold_writer) = new_trunk(8);
    let cold = HlsOrigin::new(
        HlsOriginConfig::new(Arc::clone(&cold_trunk), Duration::from_secs(1), nz(8))
            .with_instance(8),
    )
    .expect("cold origin builds");
    assert_eq!(
        playlist_reload_timeout(&cold),
        hls_runtime::server::BLOCKING_RELOAD_TIMEOUT
    );
    publish(&writer, 1, &[1; 8]);
}

#[test]
fn an_hevc_coded_size_is_used_for_the_resolution() {
    let (trunk, _writer) = new_trunk(8);
    let origin = origin(&trunk, 8);
    let spec = hevc_spec();
    let (w, h) = match &spec.config {
        CodecConfig::Hevc { width, height, .. } => (*width, *height),
        other => panic!("not HEVC: {other:?}"),
    };
    assert_eq!((w, h), (320, 240));
    origin.set_track_specs(&[spec]).expect("specs");
    let body = master(&origin);
    assert!(body.contains("RESOLUTION=320x240"), "{body}");
}

#[test]
fn a_measured_fmp4_frame_rate_appears_in_the_variant() {
    let (trunk, writer) = new_trunk(8);
    let origin = origin(&trunk, 8);
    set_init(&origin);
    // A spec with no SPS VUI timing forces the per-segment measurement, so
    // the rate comes from the fragment's `trun` durations. The 90 009
    // timescale makes the division non-integral, so the half-tick rounding
    // decides the last millihertz.
    let spec = TrackSpec::new(
        transmux::TrackId::new(1),
        broadcast_core::timescale::Timescale::try_new(90_009).expect("timescale"),
        CodecConfig::Avc {
            config: crate::dummy_avc_config::dummy_avc_config(),
            width: 320,
            height: 240,
        },
    );
    origin.set_track_specs(&[spec]).expect("specs");
    let segment = test_fixtures::read("hls/cmaf-fmp4/index0.m4s");
    publish(&writer, 1, &segment);
    let body = master(&origin);
    let parsed = MasterPlaylist::parse(&body).expect("parse master");
    assert_eq!(
        parsed.variants[0]
            .frame_rate
            .map(broadcast_hls::FrameRate::millihertz),
        Some(25_003),
        "{body}"
    );
}

#[test]
fn a_later_dolby_vision_track_keeps_its_own_base_codec() {
    let (trunk, _writer) = new_trunk(8);
    let origin = origin(&trunk, 8);
    let mut first = hevc_spec();
    first.track_id = transmux::TrackId::new(1);
    first.dovi = None;
    let mut second = hevc_spec();
    second.track_id = transmux::TrackId::new(2);
    second.dovi = Some(record(5, false, isobmff::DoviBlSignalCompatibilityId::None));
    origin.set_track_specs(&[first, second]).expect("specs");
    let body = master(&origin);
    assert!(
        !body.contains("dvh1"),
        "only the first video track decides the Dolby Vision signalling: {body}"
    );
}

#[test]
fn a_rendition_report_uri_stops_when_the_target_runs_out_of_segments() {
    let (own_trunk, own_writer) = new_trunk(8);
    let (sibling_trunk, _sibling_writer) = new_trunk(8);
    let own = HlsOrigin::new(
        HlsOriginConfig::new(Arc::clone(&own_trunk), Duration::from_secs(4), nz(8))
            .with_instance(7)
            .with_low_latency(Duration::from_millis(500))
            .with_playlist_uri("a/b/c/main.m3u8"),
    )
    .expect("origin builds");
    let sibling = Arc::new(
        HlsOrigin::new(
            HlsOriginConfig::new(Arc::clone(&sibling_trunk), Duration::from_secs(4), nz(8))
                .with_instance(8)
                .with_low_latency(Duration::from_millis(500))
                .with_playlist_uri("a/b/"),
        )
        .expect("sibling builds"),
    );
    own.set_rendition_siblings(vec![RenditionSibling {
        origin: Arc::downgrade(&sibling),
        playlist_uri: "a/b/".to_string(),
    }]);
    publish(&own_writer, 1, &[1; 8]);
    let (now, policy) = (
        Timestamp::from_nanos(0),
        AwaitPolicy::new(Timestamp::from_nanos(60_000_000_000)),
    );
    let body = match own.resolve(
        HlsRequest::Playlist {
            track_id: DEFAULT_TRACK_ID,
            query: BlockingQuery::default(),
        },
        now,
        policy,
    ) {
        EgressResponse::Ready {
            body: HlsBody::Playlist(body),
            ..
        } => body,
        other => panic!("expected a playlist, got {other:?}"),
    };
    assert!(
        body.contains("#EXT-X-RENDITION-REPORT:URI=\"..\""),
        "{body}"
    );
}
