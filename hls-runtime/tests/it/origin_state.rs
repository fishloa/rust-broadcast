#![cfg(feature = "std")]
//! Audit 22 finding 56 (issue #3024): the origin's live-edge and state
//! accessors — `current_live_edge`, `render_epoch`/`bump_epoch`,
//! `init_generation_for`/`current_init_generation`, the media- and
//! discontinuity-sequence offsets, the part-target override and siblings,
//! `window_snapshot` and `target_duration_violations`.
//!
//! Spec: RFC 8216bis §6.2.2, §6.2.5.2, §4.4.3.7, §4.4.5.4.

use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Duration;

use broadcast_common::Timestamp;
use broadcast_hls::PartIndex as HlsPartIndex;
use bytes::Bytes;
use hls_runtime::server::{
    BlockingQuery, DEFAULT_TRACK_ID, Generation, HlsBody, HlsOrigin, HlsOriginConfig, HlsRequest,
    RenditionSibling, SubtitleRenditionDecl,
};
use media_plane::egress::{AwaitPolicy, EgressResponse, ServedEgress};
use media_plane::ids::{PartIndex, SegmentSeq};
use media_plane::trunk::{PartEntry, SegmentEntry, SegmentWriter, Trunk, TrunkConfig};

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

fn origin(trunk: &Arc<Trunk>, capacity: usize, low_latency: bool) -> HlsOrigin {
    let mut config = HlsOriginConfig::new(Arc::clone(trunk), Duration::from_secs(4), nz(capacity))
        .with_instance(7);
    if low_latency {
        config = config.with_low_latency(Duration::from_millis(500));
    }
    HlsOrigin::new(config).expect("origin builds")
}

fn publish(writer: &SegmentWriter, seq: u32, secs: u64, discontinuous: bool) {
    publish_at(writer, seq, secs, discontinuous, 0);
}

fn publish_at(writer: &SegmentWriter, seq: u32, secs: u64, discontinuous: bool, start_ns: u64) {
    writer
        .publish_segment(SegmentEntry::new(
            Bytes::from(vec![u8::try_from(seq).unwrap(); 8]),
            SegmentSeq::new(seq),
            Duration::from_secs(secs),
            Timestamp::from_nanos(start_ns),
            discontinuous,
        ))
        .expect("monotonic sequence");
}

fn publish_part(writer: &SegmentWriter, seq: u32, idx: u32) {
    writer.publish_part(PartEntry::new(
        Bytes::from(vec![u8::try_from(idx).unwrap(); 4]),
        SegmentSeq::new(seq),
        PartIndex::new(idx),
        Duration::from_millis(500),
        idx == 0,
    ));
}

fn playlist(origin: &HlsOrigin) -> String {
    let (now, policy) = (
        Timestamp::from_nanos(0),
        AwaitPolicy::new(Timestamp::from_nanos(60_000_000_000)),
    );
    match origin.resolve(
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
        other => panic!("expected a rendered playlist, got {other:?}"),
    }
}

#[test]
fn a_fresh_origin_current_init_generation_is_one() {
    let (trunk, _writer) = new_trunk(8);
    let o = origin(&trunk, 8, false);
    assert_eq!(o.current_init_generation(), Generation::new(1));
}

#[test]
fn init_generations_switch_at_the_boundary_segment() {
    let (trunk, writer) = new_trunk(8);
    let o = origin(&trunk, 8, false);
    o.set_init(vec![0xA1u8; 8]);
    publish(&writer, 1, 4, false);
    assert_eq!(o.current_init_generation(), Generation::new(1));
    // Closed segment 1 pins the first init generation; the second init starts
    // at the next segment not yet closed.
    o.set_init(vec![0xB2u8; 8]);
    publish(&writer, 2, 4, false);
    assert_eq!(o.current_init_generation(), Generation::new(2));
    assert_eq!(
        o.init_generation_for(SegmentSeq::new(1)),
        Generation::new(1)
    );
    assert_eq!(
        o.init_generation_for(SegmentSeq::new(2)),
        Generation::new(2)
    );
}

#[test]
fn the_sequence_offsets_are_reported_by_their_accessors() {
    let (trunk, writer) = new_trunk(8);
    let o = HlsOrigin::new(
        HlsOriginConfig::new(Arc::clone(&trunk), Duration::from_secs(4), nz(8))
            .with_instance(7)
            .with_media_sequence_offset(1000)
            .with_discontinuity_sequence(7),
    )
    .expect("origin builds");
    assert_eq!(o.media_sequence_offset(), 1000);
    publish(&writer, 1, 4, true);
    publish(&writer, 2, 4, true);
    assert!(
        playlist(&o).contains("#EXT-X-MEDIA-SEQUENCE:1001\n"),
        "the offset numbers the first segment"
    );
    assert_eq!(
        o.discontinuity_sequence_after(),
        9,
        "base 7 plus the two in-window discontinuities"
    );
}

#[test]
fn set_part_target_raises_the_rendered_part_target() {
    let (trunk, writer) = new_trunk(8);
    let o = origin(&trunk, 8, true);
    publish(&writer, 1, 4, false);
    assert!(playlist(&o).contains("#EXT-X-PART-INF:PART-TARGET=0.5\n"));
    o.set_part_target(Duration::from_secs(1));
    let body = playlist(&o);
    assert!(body.contains("#EXT-X-PART-INF:PART-TARGET=1\n"), "{body}");
}

#[test]
fn the_hold_back_is_three_times_the_largest_sibling_part_target() {
    let (own_trunk, own_writer) = new_trunk(8);
    let (sibling_trunk, _sibling_writer) = new_trunk(8);
    let sibling = Arc::new(
        HlsOrigin::new(
            HlsOriginConfig::new(Arc::clone(&sibling_trunk), Duration::from_secs(4), nz(8))
                .with_instance(8)
                .with_low_latency(Duration::from_secs(1)),
        )
        .expect("sibling builds"),
    );
    let own = origin(&own_trunk, 8, true);
    publish(&own_writer, 1, 4, false);
    own.set_rendition_siblings(vec![RenditionSibling {
        origin: Arc::downgrade(&sibling),
        playlist_uri: "audio/a0.m3u8".to_string(),
    }]);
    let body = playlist(&own);
    assert!(
        body.contains("PART-HOLD-BACK=3"),
        "3 x the sibling's 1 s part target: {body}"
    );
}

#[test]
fn live_edge_reports_the_open_segment_and_its_last_part() {
    let (trunk, writer) = new_trunk(8);
    let o = origin(&trunk, 8, true);
    publish(&writer, 1, 4, false);
    publish(&writer, 2, 4, false);
    assert_eq!(o.current_live_edge(), (2, None));

    publish_part(&writer, 3, 0);
    publish_part(&writer, 3, 1);
    assert_eq!(o.current_live_edge(), (3, Some(HlsPartIndex::new(1))));
}

#[test]
fn live_edge_after_a_close_reports_the_closed_segments_last_part() {
    let (trunk, writer) = new_trunk(8);
    let o = origin(&trunk, 8, true);
    publish_part(&writer, 1, 0);
    publish_part(&writer, 1, 1);
    publish(&writer, 1, 4, false);
    assert_eq!(o.current_live_edge(), (1, Some(HlsPartIndex::new(1))));
}

#[test]
fn render_epoch_advances_once_per_setter() {
    let (trunk, _writer) = new_trunk(8);
    let o = origin(&trunk, 8, true);
    let start = o.render_epoch();
    o.set_part_target(Duration::from_millis(600));
    assert_eq!(o.render_epoch(), start + 1);
    o.set_rendition_siblings(Vec::new());
    assert_eq!(o.render_epoch(), start + 2);
    o.set_subtitle_renditions(Vec::new())
        .expect("empty is valid");
    assert_eq!(o.render_epoch(), start + 3);
}

#[test]
fn target_duration_violations_ignores_a_segment_exactly_at_the_target() {
    let (trunk, writer) = new_trunk(8);
    let o = origin(&trunk, 8, false);
    publish(&writer, 1, 4, false);
    let _ = playlist(&o);
    assert_eq!(o.target_duration_violations(), 0);
    publish(&writer, 2, 4, false);
    let _ = playlist(&o);
    assert_eq!(
        o.target_duration_violations(),
        0,
        "a segment exactly as long as the target is not a violation"
    );
}

fn specs_from(fixture: &str, only_video: bool) -> Option<Vec<transmux::TrackSpec>> {
    let data = test_fixtures::require_bytes(fixture, "origin_state")?;
    let media = transmux::ts_demux::TsDemux::new().demux(&data).ok()?;
    Some(
        media
            .tracks
            .into_iter()
            .filter(|t| !only_video || t.spec.config.is_video())
            .map(|t| t.spec)
            .collect(),
    )
}

fn first_audio(specs: &[transmux::TrackSpec]) -> Option<transmux::TrackSpec> {
    specs.iter().find(|s| !s.config.is_video()).cloned()
}

fn first_video(specs: &[transmux::TrackSpec]) -> Option<transmux::TrackSpec> {
    specs.iter().find(|s| s.config.is_video()).cloned()
}

fn audio_decl(
    group: &str,
    name: &str,
    lang: &str,
    default: bool,
    uri: &str,
) -> hls_runtime::server::AudioRenditionDecl {
    hls_runtime::server::AudioRenditionDecl {
        group_id: group.to_string(),
        name: name.to_string(),
        language: lang.to_string(),
        default,
        autoselect: true,
        channels: None,
        uri: uri.to_string(),
        characteristics: Vec::new(),
    }
}

fn sibling_with_specs(trunk: &Arc<Trunk>, specs: &[transmux::TrackSpec]) -> Arc<HlsOrigin> {
    let sibling = Arc::new(origin(trunk, 8, false));
    sibling.set_track_specs(specs).expect("sibling specs");
    sibling
}

fn master(origin: &HlsOrigin) -> String {
    origin
        .master_playlist("media.m3u8")
        .expect("master playlist")
}

#[test]
fn an_audio_siblings_window_and_codecs_feed_the_variant() {
    let Some(specs) = specs_from("ts/h264-aac.ts", false) else {
        return;
    };
    let video = first_video(&specs).expect("video spec");
    let audio = first_audio(&specs).expect("audio spec");
    let Some(six) = specs_from("ts/aac-5-1-640k.ts", false) else {
        return;
    };
    let audio6 = first_audio(&six).expect("5.1 audio spec");

    let (primary_trunk, _primary_writer) = new_trunk(8);
    let mut sibling_origins = Vec::new();
    let mut sibling_writers = Vec::new();
    let mut sibling_specs = Vec::new();
    for (i, id) in [2u32, 4].into_iter().enumerate() {
        let (trunk, writer) = new_trunk(8);
        let mut v = video.clone();
        v.track_id = transmux::TrackId::new(id);
        let mut a = audio6.clone();
        a.track_id = transmux::TrackId::new(id - 1);
        let origin = Arc::new(origin(&trunk, 8, false));
        origin
            .set_track_specs(&[v.clone(), a.clone()])
            .expect("specs");
        sibling_origins.push(origin);
        sibling_writers.push(writer);
        sibling_specs.push((v, a));
        let _ = i;
    }
    let primary = HlsOrigin::new(
        HlsOriginConfig::new(Arc::clone(&primary_trunk), Duration::from_secs(4), nz(8))
            .with_instance(7)
            .with_declared_bandwidth_bps(1_000_000)
            .with_audio_renditions(vec![
                audio_decl("aac", "Main", "en", true, "audio/a0.m3u8"),
                audio_decl("aac", "Alt", "es", false, "audio/a1.m3u8"),
            ])
            .with_audio_sibling_origins(sibling_origins.clone()),
    )
    .expect("primary builds");
    for writer in &sibling_writers {
        publish(writer, 1, 4, false);
    }
    let _ = (&audio, &sibling_specs);

    let body = master(&primary);
    assert!(body.contains("BANDWIDTH=1000016"), "{body}");
    assert!(body.contains("AVERAGE-BANDWIDTH=16"), "{body}");
    assert!(body.contains("CHANNELS=\"6\""), "{body}");
    assert_eq!(
        body.matches("mp4a.40.2").count(),
        1,
        "the same audio codec from both siblings is announced once: {body}"
    );
}

#[test]
fn each_audio_group_reads_its_own_sibling_bandwidth() {
    let Some(specs) = specs_from("ts/h264-aac.ts", false) else {
        return;
    };
    let audio = first_audio(&specs).expect("audio spec");

    let (primary_trunk, _primary_writer) = new_trunk(8);
    let (t1, w1) = new_trunk(8);
    let (t2, w2) = new_trunk(8);
    let small = sibling_with_specs(&t1, core::slice::from_ref(&audio));
    let large = sibling_with_specs(&t2, core::slice::from_ref(&audio));
    let primary = HlsOrigin::new(
        HlsOriginConfig::new(Arc::clone(&primary_trunk), Duration::from_secs(4), nz(8))
            .with_instance(7)
            .with_declared_bandwidth_bps(1_000_000)
            .with_audio_renditions(vec![
                audio_decl("g1", "Main", "en", true, "audio/g1.m3u8"),
                audio_decl("g2", "Main", "en", true, "audio/g2.m3u8"),
            ])
            .with_audio_sibling_origins(vec![Arc::clone(&small), Arc::clone(&large)]),
    )
    .expect("primary builds");
    publish(&w1, 1, 4, false);
    publish(&w2, 1, 1, false);
    let body = master(&primary);
    let parsed = broadcast_hls::MasterPlaylist::parse(&body).expect("parse master");
    let bandwidth = |group: &str| {
        parsed
            .variants
            .iter()
            .find(|v| v.audio.as_deref() == Some(group))
            .map(|v| v.bandwidth)
    };
    assert_eq!(bandwidth("g1"), Some(1_000_016), "{body}");
    assert_eq!(bandwidth("g2"), Some(1_000_064), "{body}");
}

#[test]
fn an_audio_description_rendition_gets_its_role_name_and_characteristics() {
    let Some(specs) = specs_from("ts/h264-aac.ts", false) else {
        return;
    };
    let mut audio = first_audio(&specs).expect("audio spec");
    // ISO 639 language "qad": the ISO code reserved for audio description.
    audio.es_info_descriptors = vec![0x0A, 4, b'q', b'a', b'd', 0x00];

    let (primary_trunk, _primary_writer) = new_trunk(8);
    let (sibling_trunk, _sibling_writer) = new_trunk(8);
    let sibling = sibling_with_specs(&sibling_trunk, &[audio]);
    let primary = HlsOrigin::new(
        HlsOriginConfig::new(Arc::clone(&primary_trunk), Duration::from_secs(4), nz(8))
            .with_instance(7)
            .with_declared_bandwidth_bps(1_000_000)
            .with_audio_renditions(vec![audio_decl("ad", "", "en", true, "audio/ad.m3u8")])
            .with_audio_sibling_origins(vec![sibling]),
    )
    .expect("primary builds");
    let body = master(&primary);
    assert!(body.contains("NAME=\"en (AD)\""), "{body}");
    assert!(
        body.contains("CHARACTERISTICS=\"public.accessibility.describes-video\""),
        "{body}"
    );
}

#[test]
fn the_iframe_playlist_name_is_track_scoped() {
    let (trunk, _writer) = new_trunk(8);
    let o = origin(&trunk, 8, false);
    assert_eq!(
        o.iframe_playlist_name(transmux::TrackId::new(3)),
        "iframe-3.m3u8"
    );
}

#[test]
fn a_playlist_in_an_absolute_directory_climbs_exactly_that_many_levels() {
    let (trunk, writer) = new_trunk(8);
    let o = HlsOrigin::new(
        HlsOriginConfig::new(Arc::clone(&trunk), Duration::from_secs(4), nz(8))
            .with_instance(7)
            .with_playlist_uri("/a/b/media.m3u8"),
    )
    .expect("origin builds");
    publish(&writer, 1, 4, false);
    let body = playlist(&o);
    assert!(
        body.contains("\n../../seg-1-7-1.m4s"),
        "two levels up from /a/b/: {body}"
    );
    assert!(!body.contains("\n../seg-1-7-1.m4s"), "{body}");
}

#[test]
fn set_subtitle_renditions_refuses_a_bad_declaration_and_applies_a_good_one() {
    let (trunk, writer) = new_trunk(8);
    let o = origin(&trunk, 8, false);
    publish(&writer, 1, 4, false);
    let bad = SubtitleRenditionDecl::new("", "en", "subs/en.m3u8");
    assert!(o.set_subtitle_renditions(vec![bad]).is_err());
    assert!(!master(&o).contains("TYPE=SUBTITLES"), "nothing applied");
    o.set_subtitle_renditions(vec![SubtitleRenditionDecl::new(
        "English",
        "en",
        "subs/en.m3u8",
    )])
    .expect("a valid declaration");
    let body = master(&o);
    assert!(body.contains("#EXT-X-MEDIA:TYPE=SUBTITLES"), "{body}");
}

#[test]
fn arrival_pdt_uses_the_segment_timeline_position() {
    const NOW_MS: i64 = 1_000_000_000_000;
    let (trunk, writer) = new_trunk(8);
    let clock = Arc::new(|| NOW_MS) as Arc<dyn Fn() -> i64 + Send + Sync>;
    let o = HlsOrigin::new(
        HlsOriginConfig::new(Arc::clone(&trunk), Duration::from_secs(4), nz(8))
            .with_instance(7)
            .with_program_date_time_arrival(clock),
    )
    .expect("origin builds");
    // A non-zero timeline position: the first 4 s segment starts at 4 s.
    publish_at(&writer, 1, 4, false, 4_000_000_000);
    publish_at(&writer, 2, 4, false, 8_000_000_000);
    let body = playlist(&o);
    let pdt = |ms: i64| {
        jiff::Timestamp::from_millisecond(ms)
            .expect("timestamp")
            .strftime("%Y-%m-%dT%H:%M:%S%.3fZ")
            .to_string()
    };
    assert!(
        body.contains(&format!(
            "#EXT-X-PROGRAM-DATE-TIME:{}\n",
            pdt(NOW_MS - 4_000)
        )),
        "{body}"
    );
    assert!(
        body.contains(&format!("#EXT-X-PROGRAM-DATE-TIME:{}\n", pdt(NOW_MS))),
        "{body}"
    );
}

#[test]
fn known_es_info_survives_an_init_that_cannot_carry_it() {
    let Some(specs) = specs_from("ts/h264-aac.ts", false) else {
        return;
    };
    let mut audio = first_audio(&specs).expect("audio spec");
    audio.es_info_descriptors = vec![0x0A, 4, b'f', b'r', b'a', 0x00];
    let video = first_video(&specs).expect("video spec");
    let specs = vec![audio, video];
    let scale = broadcast_common::timescale::Timescale::try_new(90_000).expect("timescale");
    let init = transmux::pipeline::build_init_segment(&specs, scale).expect("init segment");

    let (sibling_trunk, _sibling_writer) = new_trunk(8);
    let sibling = Arc::new(origin(&sibling_trunk, 8, false));
    sibling.set_track_specs(&specs).expect("known specs");
    sibling.set_init(init);

    let (primary_trunk, _primary_writer) = new_trunk(8);
    let primary = HlsOrigin::new(
        HlsOriginConfig::new(Arc::clone(&primary_trunk), Duration::from_secs(4), nz(8))
            .with_instance(7)
            .with_declared_bandwidth_bps(1_000_000)
            .with_audio_renditions(vec![audio_decl("aac", "", "", true, "audio/a0.m3u8")])
            .with_audio_sibling_origins(vec![sibling]),
    )
    .expect("primary builds");
    let body = master(&primary);
    assert!(
        body.contains("LANGUAGE=\"fr\""),
        "the ISO 639 descriptor from set_track_specs survives the init re-derivation: {body}"
    );
}

#[test]
fn an_hdr10plus_flag_survives_the_init_re_derivation() {
    let Some(specs) = specs_from("ts/hevc/main10.ts", true) else {
        return;
    };
    let Some(mut video) = first_video(&specs) else {
        return;
    };
    assert!(matches!(video.config, transmux::CodecConfig::Hevc { .. }));
    video.hdr10plus = true;
    let mut display = transmux::ir::DisplayGeometry::new(1920, 1080, None);
    display.video_signal = Some(transmux::ir::VideoSignalType::new(
        transmux::ir::VideoFormat::Component,
        false,
        Some(transmux::ir::ColourDescription::from_wire(9, 16, 9)),
    ));
    let video = video.with_display(Some(display));

    let (trunk, _writer) = new_trunk(8);
    let o = HlsOrigin::new(
        HlsOriginConfig::new(Arc::clone(&trunk), Duration::from_secs(4), nz(8))
            .with_instance(7)
            .with_declared_bandwidth_bps(1_000_000),
    )
    .expect("origin builds");
    o.set_track_specs(core::slice::from_ref(&video))
        .expect("known specs");
    let scale = broadcast_common::timescale::Timescale::try_new(90_000).expect("timescale");
    let init = transmux::pipeline::build_init_segment(core::slice::from_ref(&video), scale)
        .expect("init segment");
    o.set_init(init);

    let body = master(&o);
    assert!(
        body.contains("SUPPLEMENTAL-CODECS=\"") && body.contains("/cdm4"),
        "the HDR10+ brand is announced after the init re-derivation: {body}"
    );
}

#[test]
fn only_the_newest_eight_init_generations_are_remembered() {
    let (trunk, _writer) = new_trunk(8);
    let o = origin(&trunk, 8, false);
    for generation in 1..=12u8 {
        o.set_init(vec![generation; 8]);
    }
    assert_eq!(o.current_init_generation(), Generation::new(12));
    assert_eq!(
        o.init_generation_for(SegmentSeq::new(0)),
        Generation::new(5),
        "the oldest remembered generation is 5; older numbers fall to it"
    );
}
