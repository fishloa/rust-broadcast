#![cfg(feature = "std")]
//! WebVTT subtitle renditions in the origin engine (issue #1428): the
//! multivariant `EXT-X-MEDIA TYPE=SUBTITLES` group and a `Container::WebVtt`
//! origin's media playlist. Content-type of served segments is decided in
//! `multimux`, not here, so there is no `text/vtt` test in this crate.

use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Duration;

use broadcast_common::Timestamp;
use broadcast_hls::{MasterPlaylist, MediaType};
use bytes::Bytes;
use hls_runtime::server::{
    BlockingQuery, Container, DEFAULT_TRACK_ID, HlsBody, HlsOrigin, HlsOriginBuildError,
    HlsOriginBuilder, HlsRequest, SUBTITLES_GROUP_ID, SubtitleRenditionDecl,
};
use media_doctor::{Report, check_hls_playlist};
use media_plane::ServedEgress;
use media_plane::egress::{AwaitPolicy, EgressResponse};
use media_plane::ids::SegmentSeq;
use media_plane::trunk::{SegmentEntry, SegmentWriter, Trunk, TrunkConfig};
use transmux::{
    AVCConfigurationBox, AVCDecoderConfigurationRecord, AvcPps, AvcSps, CodecConfig, SegmentMeta,
    TrackId, TrackSpec,
};

const TARGET_SECS: f64 = 4.0;
const HEARING_IMPAIRED: &str = "CHARACTERISTICS=\"public.accessibility.transcribes-spoken-dialog,public.accessibility.describes-music-and-sound\"";

fn nz(n: usize) -> NonZeroUsize {
    NonZeroUsize::new(n).expect("non-zero")
}

fn new_trunk() -> Arc<Trunk> {
    Trunk::new(TrunkConfig::new(nz(64), nz(8), nz(8), nz(8), nz(64)))
}

fn avc_spec() -> TrackSpec {
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
    TrackSpec::new(
        TrackId::new(1),
        90_000,
        CodecConfig::Avc {
            config,
            width: 320,
            height: 240,
        },
    )
}

fn primary_builder() -> HlsOriginBuilder {
    HlsOrigin::builder(new_trunk())
        .container(Container::MpegTs)
        .target_duration_secs(TARGET_SECS)
        .window_segments(nz(4))
        .instance(7)
        .declared_bandwidth_bps(5_000_000)
}

fn primary_with(decls: Vec<SubtitleRenditionDecl>) -> HlsOrigin {
    let origin = primary_builder()
        .subtitle_renditions(decls)
        .build()
        .expect("primary builds");
    origin.set_track_specs(&[avc_spec()]).expect("track specs");
    origin
}

fn english() -> SubtitleRenditionDecl {
    SubtitleRenditionDecl::new("English", "en", "subs/en.m3u8")
}

fn master_of(origin: &HlsOrigin) -> String {
    origin.master_playlist("media.m3u8").expect("master")
}

fn media_lines(master: &str) -> Vec<&str> {
    master
        .lines()
        .filter(|l| l.starts_with("#EXT-X-MEDIA:"))
        .collect()
}

fn webvtt_origin(trunk: &Arc<Trunk>, target_secs: f64) -> HlsOrigin {
    HlsOrigin::builder(Arc::clone(trunk))
        .container(Container::WebVtt)
        .target_duration_secs(target_secs)
        .window_segments(nz(4))
        .instance(9)
        .build()
        .expect("webvtt origin builds")
}

fn publish_vtt(w: &SegmentWriter, seq: u32) {
    w.publish_segment(SegmentEntry::new(
        Bytes::from(format!(
            "WEBVTT\n\n00:00:00.000 --> 00:00:01.000\ncue {seq}\n"
        )),
        SegmentSeq::new(seq),
        Duration::from_secs(4),
        Timestamp::from_nanos(0),
        SegmentMeta {
            discontinuous: false,
        },
    ))
    .expect("monotonic sequence number");
}

fn playlist_of(origin: &HlsOrigin, query: BlockingQuery) -> EgressResponse<HlsBody> {
    origin.resolve(
        HlsRequest::Playlist {
            track_id: DEFAULT_TRACK_ID,
            query,
        },
        Timestamp::from_nanos(1),
        AwaitPolicy::new(Timestamp::from_nanos(60_000_000_000)),
    )
}

fn playlist_text(origin: &HlsOrigin) -> String {
    match playlist_of(origin, BlockingQuery::default()) {
        EgressResponse::Ready {
            body: HlsBody::Playlist(text),
            ..
        } => text,
        other => panic!("expected a playlist, got {other:?}"),
    }
}

fn target_duration(text: &str) -> u32 {
    text.lines()
        .find_map(|l| l.strip_prefix("#EXT-X-TARGETDURATION:"))
        .expect("target duration")
        .parse()
        .expect("integer")
}

fn assert_checker_clean(text: &str) {
    let mut report = Report::new();
    check_hls_playlist(text, &mut report);
    assert!(
        report.is_empty(),
        "check_hls_playlist findings: {:?}\n{text}",
        report.findings()
    );
}

#[test]
fn master_declares_subtitles_group_with_language() {
    let origin = primary_with(vec![english()]);
    let master = master_of(&origin);

    let media = media_lines(&master);
    assert_eq!(media.len(), 1, "{master}");
    let line = media[0];
    for token in [
        "TYPE=SUBTITLES",
        "GROUP-ID=\"subs\"",
        "NAME=\"English\"",
        "LANGUAGE=\"en\"",
        "URI=\"subs/en.m3u8\"",
    ] {
        assert!(line.contains(token), "{token} missing in {line}");
    }
    let stream_infs: Vec<_> = master
        .lines()
        .filter(|l| l.starts_with("#EXT-X-STREAM-INF:"))
        .collect();
    assert!(!stream_infs.is_empty());
    for l in stream_infs {
        assert!(l.contains("SUBTITLES=\"subs\""), "{l}");
    }

    let parsed = MasterPlaylist::parse(&master).expect("parse");
    assert_eq!(parsed.media.len(), 1);
    let m = &parsed.media[0];
    assert_eq!(m.media_type, MediaType::Subtitles);
    assert_eq!(m.group_id, SUBTITLES_GROUP_ID);
    assert_eq!(m.name, "English");
    assert_eq!(m.language.as_deref(), Some("en"));
    assert_eq!(m.uri.as_deref(), Some("subs/en.m3u8"));
    assert!(
        parsed
            .variants
            .iter()
            .all(|v| v.subtitles.as_deref() == Some(SUBTITLES_GROUP_ID))
    );
    assert_checker_clean(&master);
}

#[test]
fn hearing_impaired_subtitles_carry_apple_characteristics_and_autoselect() {
    let mut sdh = SubtitleRenditionDecl::new("English SDH", "en", "subs/en-sdh.m3u8");
    sdh.autoselect = false;
    sdh.hearing_impaired = true;
    let master = master_of(&primary_with(vec![sdh]));
    let line = media_lines(&master)[0];
    assert!(line.contains(HEARING_IMPAIRED), "{line}");
    assert!(line.contains("AUTOSELECT=YES"), "{line}");
}

#[test]
fn forced_subtitles_are_autoselected() {
    let mut forced = SubtitleRenditionDecl::new("English Forced", "en", "subs/en-forced.m3u8");
    forced.autoselect = false;
    forced.forced = true;
    let master = master_of(&primary_with(vec![forced]));
    let line = media_lines(&master)[0];
    assert!(line.contains("FORCED=YES"), "{line}");
    assert!(line.contains("AUTOSELECT=YES"), "{line}");
}

#[test]
fn variant_codecs_name_wvtt_when_subtitles_exist() {
    let with = master_of(&primary_with(vec![english()]));
    let stream_inf = with
        .lines()
        .find(|l| l.starts_with("#EXT-X-STREAM-INF:"))
        .expect("variant");
    assert!(
        stream_inf.contains("CODECS=\"avc1.42001E,wvtt\""),
        "{stream_inf}"
    );

    let without = master_of(&primary_with(Vec::new()));
    assert!(!without.contains("wvtt"), "{without}");
    assert!(without.contains("CODECS=\"avc1.42001E\""), "{without}");
}

#[test]
fn master_without_subtitles_is_unchanged() {
    // `audio_renditions.rs` pins the audio masters byte for byte
    // (`two_group_two_language_master_matches_golden`,
    // `no_declarations_and_flag_off_keeps_plain_master`); this pins the
    // codecs-known variant.
    let master = master_of(&primary_with(Vec::new()));
    assert_eq!(
        master,
        "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=5000000,CODECS=\"avc1.42001E\",RESOLUTION=320x240\nmedia.m3u8\n"
    );
}

#[test]
fn subtitle_playlist_has_no_map_and_matches_primary_target_duration() {
    let primary_trunk = new_trunk();
    let primary = HlsOrigin::builder(Arc::clone(&primary_trunk))
        .container(Container::MpegTs)
        .target_duration_secs(TARGET_SECS)
        .window_segments(nz(4))
        .instance(7)
        .build()
        .expect("primary");
    let primary_writer = primary_trunk.segment_writer().expect("writer");
    for seq in 1..=3 {
        w_ts(&primary_writer, seq);
    }

    let sub_trunk = new_trunk();
    let sub = webvtt_origin(&sub_trunk, TARGET_SECS);
    let sub_writer = sub_trunk.segment_writer().expect("writer");
    for seq in 1..=3 {
        publish_vtt(&sub_writer, seq);
    }

    let text = playlist_text(&sub);
    assert!(!text.contains("EXT-X-MAP"), "{text}");
    assert!(!text.contains("EXT-X-PART"), "{text}");
    let uris: Vec<_> = text.lines().filter(|l| !l.starts_with('#')).collect();
    assert_eq!(uris.len(), 3, "{text}");
    assert!(uris.iter().all(|u| u.ends_with(".vtt")), "{text}");
    assert_eq!(
        target_duration(&text),
        target_duration(&playlist_text(&primary))
    );
    assert_checker_clean(&text);
}

fn w_ts(w: &SegmentWriter, seq: u32) {
    w.publish_segment(SegmentEntry::new(
        Bytes::from(vec![0x47; 188]),
        SegmentSeq::new(seq),
        Duration::from_secs(4),
        Timestamp::from_nanos(0),
        SegmentMeta {
            discontinuous: false,
        },
    ))
    .expect("monotonic sequence number");
}

#[test]
fn low_latency_webvtt_origin_has_no_parts_but_blocks_on_media_sequence() {
    let trunk = new_trunk();
    let origin = HlsOrigin::builder(Arc::clone(&trunk))
        .container(Container::WebVtt)
        .target_duration_secs(TARGET_SECS)
        .window_segments(nz(4))
        .low_latency(500)
        .instance(9)
        .build()
        .expect("builds");
    let writer = trunk.segment_writer().expect("writer");
    publish_vtt(&writer, 1);
    let text = playlist_text(&origin);
    assert!(!text.contains("#EXT-X-PART:"), "{text}");
    assert!(!text.contains("EXT-X-PRELOAD-HINT"), "{text}");
    assert!(text.contains("CAN-BLOCK-RELOAD=YES"), "{text}");

    let query = BlockingQuery {
        hls_msn: Some(2),
        ..Default::default()
    };
    assert!(matches!(
        playlist_of(&origin, query.clone()),
        EgressResponse::Await { .. }
    ));
    publish_vtt(&writer, 2);
    assert!(matches!(
        playlist_of(&origin, query.clone()),
        EgressResponse::Ready { .. }
    ));
}

fn ll_origin(container: Container, trunk: &Arc<Trunk>) -> HlsOrigin {
    let origin = HlsOrigin::builder(Arc::clone(trunk))
        .container(container)
        .target_duration_secs(TARGET_SECS)
        .window_segments(nz(8))
        .low_latency(500)
        .instance(9)
        .build()
        .expect("builds");
    if container != Container::WebVtt {
        origin.set_track_specs(&[avc_spec()]).expect("track specs");
    }
    origin
}

fn server_control_of(text: &str) -> broadcast_hls::LowLatencyConfig {
    broadcast_hls::MediaPlaylist::parse(text)
        .unwrap_or_else(|e| panic!("{e}: {text}"))
        .low_latency
        .unwrap_or_else(|| panic!("no low-latency tags: {text}"))
}

#[test]
fn webvtt_server_control_equals_primary_server_control() {
    let primary_trunk = new_trunk();
    let primary = ll_origin(Container::MpegTs, &primary_trunk);
    let vtt_trunk = new_trunk();
    let vtt = ll_origin(Container::WebVtt, &vtt_trunk);
    let primary_writer = primary_trunk.segment_writer().expect("writer");
    let vtt_writer = vtt_trunk.segment_writer().expect("writer");
    for seq in 1..=8 {
        publish_vtt(&primary_writer, seq);
        publish_vtt(&vtt_writer, seq);
    }

    let primary_text = playlist_text(&primary);
    let vtt_text = playlist_text(&vtt);
    let a = server_control_of(&primary_text);
    let b = server_control_of(&vtt_text);
    assert!(a.can_block_reload && a.can_skip_until.is_some() && a.part_hold_back.is_some());
    assert_eq!(a.can_block_reload, b.can_block_reload);
    assert_eq!(a.part_hold_back, b.part_hold_back);
    assert_eq!(a.hold_back, b.hold_back);
    assert_eq!(a.can_skip_until, b.can_skip_until);
    assert_eq!(a.can_skip_dateranges, b.can_skip_dateranges);
    assert_eq!(a.sc_extra_attrs, b.sc_extra_attrs);
    assert_eq!(a.part_target, b.part_target);

    assert!(!vtt_text.contains("#EXT-X-PART:"), "{vtt_text}");
    assert!(!vtt_text.contains("EXT-X-PRELOAD-HINT"), "{vtt_text}");
}

#[test]
fn webvtt_playlist_honours_skip_and_treats_a_part_index_as_the_next_segment() {
    let trunk = new_trunk();
    let origin = ll_origin(Container::WebVtt, &trunk);
    let writer = trunk.segment_writer().expect("writer");
    for seq in 1..=8 {
        publish_vtt(&writer, seq);
    }
    let skipped = match playlist_of(
        &origin,
        BlockingQuery {
            hls_skip: Some(hls_runtime::server::SkipDirective::Yes),
            ..Default::default()
        },
    ) {
        EgressResponse::Ready {
            body: HlsBody::Playlist(text),
            ..
        } => text,
        other => panic!("expected a playlist, got {other:?}"),
    };
    assert!(
        skipped.contains("#EXT-X-SKIP:SKIPPED-SEGMENTS="),
        "{skipped}"
    );

    let beyond = BlockingQuery {
        hls_msn: Some(8),
        hls_part: Some(0),
        ..Default::default()
    };
    assert!(matches!(
        playlist_of(&origin, beyond.clone()),
        EgressResponse::Await { .. }
    ));
    publish_vtt(&writer, 9);
    assert!(matches!(
        playlist_of(&origin, beyond),
        EgressResponse::Ready { .. }
    ));
}

#[test]
fn sibling_origins_must_be_webvtt_with_the_primary_target_duration() {
    let sub_trunk = new_trunk();
    let good = Arc::new(webvtt_origin(&sub_trunk, TARGET_SECS));
    assert!(
        primary_builder()
            .subtitle_renditions(vec![english()])
            .subtitle_sibling_origins(vec![good])
            .build()
            .is_ok()
    );

    let slow = Arc::new(webvtt_origin(&new_trunk(), 6.0));
    assert_eq!(
        build_err(
            primary_builder()
                .subtitle_renditions(vec![english()])
                .subtitle_sibling_origins(vec![slow])
        ),
        HlsOriginBuildError::SubtitleSiblingTargetDurationMismatch
    );

    let ts = Arc::new(
        HlsOrigin::builder(new_trunk())
            .container(Container::MpegTs)
            .target_duration_secs(TARGET_SECS)
            .window_segments(nz(4))
            .build()
            .expect("ts"),
    );
    assert_eq!(
        build_err(
            primary_builder()
                .subtitle_renditions(vec![english()])
                .subtitle_sibling_origins(vec![ts])
        ),
        HlsOriginBuildError::SubtitleSiblingNotWebVtt
    );

    let a = Arc::new(webvtt_origin(&new_trunk(), TARGET_SECS));
    let b = Arc::new(webvtt_origin(&new_trunk(), TARGET_SECS));
    assert_eq!(
        build_err(
            primary_builder()
                .subtitle_renditions(vec![english()])
                .subtitle_sibling_origins(vec![a, b])
        ),
        HlsOriginBuildError::SubtitleSiblingCountMismatch
    );
}

fn build_err(builder: HlsOriginBuilder) -> HlsOriginBuildError {
    match builder.build() {
        Ok(_) => panic!("build unexpectedly succeeded"),
        Err(e) => e,
    }
}

fn err_for(decls: Vec<SubtitleRenditionDecl>) -> HlsOriginBuildError {
    build_err(primary_builder().subtitle_renditions(decls))
}

#[test]
fn invalid_subtitle_declarations_are_rejected() {
    let mut no_lang = english();
    no_lang.language = String::new();
    assert_eq!(
        err_for(vec![no_lang]),
        HlsOriginBuildError::EmptySubtitleRenditionLanguage
    );

    let dup_name = SubtitleRenditionDecl::new("English", "de", "subs/de.m3u8");
    assert_eq!(
        err_for(vec![english(), dup_name]),
        HlsOriginBuildError::DuplicateSubtitleRenditionName
    );

    let dup_uri = SubtitleRenditionDecl::new("Deutsch", "de", "subs/en.m3u8");
    assert_eq!(
        err_for(vec![english(), dup_uri]),
        HlsOriginBuildError::DuplicateSubtitleRenditionUri
    );

    let mut default_no_auto = english();
    default_no_auto.default = true;
    default_no_auto.autoselect = false;
    assert_eq!(
        err_for(vec![default_no_auto]),
        HlsOriginBuildError::SubtitleRenditionDefaultNotAutoselect
    );

    let mut d1 = english();
    d1.default = true;
    let mut d2 = SubtitleRenditionDecl::new("Deutsch", "de", "subs/de.m3u8");
    d2.default = true;
    assert_eq!(
        err_for(vec![d1, d2]),
        HlsOriginBuildError::MultipleDefaultSubtitleRenditions
    );
}
