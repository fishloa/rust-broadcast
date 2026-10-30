//! Split-audio (separate audio renditions) end to end (issue #1428 slice 2b).
//!
//! Every test drives the REAL production path — `ProgramSegmenter::pump` over
//! a `Trunk` fed with demuxed fixture samples — and reads the result back over
//! the axum router the way a player would.

use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use broadcast_common::mux::Unpackage;
use broadcast_hls::MediaPlaylist;
use hls_runtime::server::AudioRenditionDecl;
use media_plane::ProgramId;
use media_plane::trunk::{RetentionClass, Trunk, TrunkConfig};
use tower::ServiceExt;
use transmux::pipeline::{CodecConfig, Sample, TrackSpec};
use transmux::ts_demux::TsDemux;
use transmux::validate::{Severity, validate_init_segment, validate_media_segment};

use multimux::config::{AudioTrackSelector, Config};
use multimux::origin::{AppState, router};
use multimux::output::Output;
use multimux::output::llhls::LlHlsOutput;
use multimux::route::RouteHandle;
use multimux::source::segment::ProgramSegmenter;

const TARGET_DURATION_SECS: f64 = 1.0;
const PART_TARGET_MS: u32 = 200;
const WINDOW_SEGMENTS: usize = 64;
const RING: usize = 1 << 16;
const VIDEO_LEAD_SECS: f64 = 0.5;
const PUMP_EVERY: usize = 64;
const STREAM: &str = "cam";

struct Source {
    spec: TrackSpec,
    samples: Vec<Sample>,
}

impl Source {
    fn is_video(&self) -> bool {
        matches!(self.spec.config, CodecConfig::Avc { .. })
    }
    fn is_audio(&self) -> bool {
        !self.is_video() && self.spec.config.is_muxable_in_bmff()
    }
    fn secs(&self, ticks: i64) -> f64 {
        ticks as f64 / f64::from(self.spec.timescale)
    }
    fn max_frame_secs(&self) -> f64 {
        self.samples
            .iter()
            .filter_map(|s| s.duration)
            .map(|d| f64::from(d) / f64::from(self.spec.timescale))
            .fold(0.0, f64::max)
    }
}

fn load(fixture: &str) -> Vec<Source> {
    let bytes = test_fixtures::read(fixture);
    let media = TsDemux::new().unpackage(&bytes).expect("demux fixture");
    media
        .tracks
        .into_iter()
        .map(|t| Source {
            spec: t.spec,
            samples: t
                .samples
                .into_iter()
                .map(|mut s| {
                    s.dts = s.dts.or(s.pts);
                    s
                })
                .collect(),
        })
        .collect()
}

fn decls(languages: &[&str]) -> Vec<AudioRenditionDecl> {
    languages
        .iter()
        .enumerate()
        .map(|(i, lang)| AudioRenditionDecl {
            group_id: "aud".into(),
            name: format!("Audio {i}"),
            language: (*lang).into(),
            default: i == 0,
            autoselect: true,
            channels: None,
            uri: format!("audio/a{i}.m3u8"),
            characteristics: Vec::new(),
        })
        .collect()
}

fn index_selectors(n: u32) -> Vec<AudioTrackSelector> {
    (0..n).map(AudioTrackSelector::Index).collect()
}

fn new_trunk() -> Arc<Trunk> {
    let nz = |n: usize| NonZeroUsize::new(n).expect("non-zero");
    Trunk::new(TrunkConfig::new(
        nz(RING),
        nz(64),
        nz(WINDOWS_USIZE),
        nz(64),
        nz(RING),
    ))
}

const WINDOWS_USIZE: usize = WINDOW_SEGMENTS;

/// Publish `plan` samples (`(time key, track index)` order) into `trunk`,
/// pumping every [`PUMP_EVERY`] samples, then flush.
fn feed(
    trunk: &Arc<Trunk>,
    writer: &media_plane::trunk::TrunkWriter,
    route: &RouteHandle,
    segmenter: &mut ProgramSegmenter,
    sources: &[&Source],
    lead_secs: f64,
    keep: &dyn Fn(usize, &Sample) -> bool,
) {
    let mut order: Vec<(f64, usize, usize)> = Vec::new();
    for (ti, src) in sources.iter().enumerate() {
        if !src.is_video() && !src.is_audio() {
            continue;
        }
        for (si, s) in src.samples.iter().enumerate() {
            if !keep(ti, s) {
                continue;
            }
            let dts = s.dts.unwrap_or(0);
            let lead = if src.is_video() { lead_secs } else { 0.0 };
            order.push((src.secs(dts) - lead, ti, si));
        }
    }
    order.sort_by(|a, b| a.0.total_cmp(&b.0));
    for (n, (_, ti, si)) in order.into_iter().enumerate() {
        writer.publish(
            sources[ti].spec.track_id,
            RetentionClass::Timed,
            sources[ti].samples[si].clone(),
        );
        if n % PUMP_EVERY == 0 {
            segmenter.pump(ProgramId(0), trunk, route);
        }
    }
    segmenter.pump(ProgramId(0), trunk, route);
}

fn publish_interleaved(
    trunk: &Arc<Trunk>,
    writer: &media_plane::trunk::TrunkWriter,
    route: &RouteHandle,
    segmenter: &mut ProgramSegmenter,
    sources: &[&Source],
    keep: &dyn Fn(usize, &Sample) -> bool,
) {
    feed(
        trunk,
        writer,
        route,
        segmenter,
        sources,
        VIDEO_LEAD_SECS,
        keep,
    );
    segmenter.flush();
}

struct Live {
    trunk: Arc<Trunk>,
    writer: media_plane::trunk::TrunkWriter,
    route: Arc<RouteHandle>,
    segmenter: std::sync::Mutex<ProgramSegmenter>,
    lead_secs: f64,
}

struct Rig {
    app: Router,
    live: Option<Live>,
}

impl Rig {
    fn publish_more(&self, sources: &[Source], keep: &dyn Fn(usize, &Sample) -> bool) {
        let live = self.live.as_ref().expect("live rig");
        let refs: Vec<&Source> = sources.iter().collect();
        let mut segmenter = live.segmenter.lock().expect("segmenter");
        feed(
            &live.trunk,
            &live.writer,
            &live.route,
            &mut segmenter,
            &refs,
            live.lead_secs,
            keep,
        );
    }
}

fn rig(
    sources: &[Source],
    renditions: Vec<AudioRenditionDecl>,
    selectors: Vec<AudioTrackSelector>,
    keep: &dyn Fn(usize, &Sample) -> bool,
) -> Rig {
    rig_declared(sources, renditions, selectors, Some(DECLARED_BPS), keep)
}

const DECLARED_BPS: u64 = 16_000_000;

fn rig_declared(
    sources: &[Source],
    renditions: Vec<AudioRenditionDecl>,
    selectors: Vec<AudioTrackSelector>,
    declared: Option<u64>,
    keep: &dyn Fn(usize, &Sample) -> bool,
) -> Rig {
    build_rig(sources, renditions, selectors, declared, keep, None)
}

fn rig_live(
    sources: &[Source],
    renditions: Vec<AudioRenditionDecl>,
    selectors: Vec<AudioTrackSelector>,
    keep: &dyn Fn(usize, &Sample) -> bool,
) -> Rig {
    build_rig(
        sources,
        renditions,
        selectors,
        Some(DECLARED_BPS),
        keep,
        Some(VIDEO_LEAD_SECS),
    )
}

fn rig_live_aligned(
    sources: &[Source],
    renditions: Vec<AudioRenditionDecl>,
    selectors: Vec<AudioTrackSelector>,
    keep: &dyn Fn(usize, &Sample) -> bool,
) -> Rig {
    build_rig(
        sources,
        renditions,
        selectors,
        Some(DECLARED_BPS),
        keep,
        Some(0.0),
    )
}

fn build_rig(
    sources: &[Source],
    renditions: Vec<AudioRenditionDecl>,
    selectors: Vec<AudioTrackSelector>,
    declared: Option<u64>,
    keep: &dyn Fn(usize, &Sample) -> bool,
    live_lead_secs: Option<f64>,
) -> Rig {
    let trunk = new_trunk();
    let writer = trunk.writer().expect("trunk writer");
    writer.set_tracks(sources.iter().map(|s| s.spec.clone()).collect());
    let route = Arc::new(
        RouteHandle::new(TARGET_DURATION_SECS, PART_TARGET_MS, WINDOW_SEGMENTS)
            .with_declared_bandwidth_bps(declared)
            .with_audio_renditions(renditions)
            .with_audio_rendition_sources(selectors),
    );
    route.publish_program(ProgramId(0), Arc::clone(&trunk));
    route.set_track_specs(ProgramId(0), trunk.tracks().to_vec());
    let mut segmenter =
        ProgramSegmenter::try_new(&trunk, &route, TARGET_DURATION_SECS, PART_TARGET_MS)
            .expect("segmenter");
    if let Some(init) = segmenter.init_segment() {
        route.set_init(ProgramId(0), init);
    }
    let refs: Vec<&Source> = sources.iter().collect();
    let Some(lead_secs) = live_lead_secs else {
        publish_interleaved(&trunk, &writer, &route, &mut segmenter, &refs, keep);
        return Rig {
            app: app_for(route),
            live: None,
        };
    };
    feed(
        &trunk,
        &writer,
        &route,
        &mut segmenter,
        &refs,
        lead_secs,
        keep,
    );
    Rig {
        app: app_for(Arc::clone(&route)),
        live: Some(Live {
            trunk,
            writer,
            route,
            segmenter: std::sync::Mutex::new(segmenter),
            lead_secs,
        }),
    }
}

fn app_for(route: Arc<RouteHandle>) -> Router {
    let mut streams = HashMap::new();
    streams.insert(
        STREAM.to_string(),
        (
            route,
            vec![Arc::new(LlHlsOutput::default()) as Arc<dyn Output>],
        ),
    );
    router(Arc::new(AppState::new(streams)))
}

impl Rig {
    async fn get(&self, path: &str) -> (StatusCode, Vec<u8>) {
        let resp = self
            .app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/{STREAM}/{path}"))
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        let status = resp.status();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .expect("body")
            .to_vec();
        (status, body)
    }

    async fn ok(&self, path: &str) -> Vec<u8> {
        let (status, body) = self.get(path).await;
        assert_eq!(status, StatusCode::OK, "GET {path}");
        body
    }

    async fn playlist(&self, path: &str) -> MediaPlaylist {
        let text = String::from_utf8(self.ok(path).await).expect("utf8 playlist");
        MediaPlaylist::parse(&text).unwrap_or_else(|e| panic!("{path} must parse: {e}\n{text}"))
    }
}

fn be32(b: &[u8], at: usize) -> usize {
    u32::from_be_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]]) as usize
}

fn child<'a>(bytes: &'a [u8], kind: &[u8; 4]) -> Option<&'a [u8]> {
    let mut at = 0;
    while at + 8 <= bytes.len() {
        let size = be32(bytes, at);
        if size < 8 || at + size > bytes.len() {
            return None;
        }
        if &bytes[at + 4..at + 8] == kind {
            return Some(&bytes[at + 8..at + size]);
        }
        at += size;
    }
    None
}

fn first_tfdt(segment: &[u8]) -> (u32, u64) {
    let moof = child(segment, b"moof").expect("moof");
    let moof = transmux::movie_fragment::MovieFragmentBox::parse_body(moof).expect("moof parses");
    let traf = moof.traf.first().expect("traf");
    let tfdt = traf.tfdt.as_ref().expect("tfdt").base_media_decode_time();
    (traf.tfhd.track_id.value(), tfdt)
}

fn trak_count(init: &[u8]) -> usize {
    let moov = child(init, b"moov").expect("moov");
    let mut count = 0;
    let mut at = 0;
    while at + 8 <= moov.len() {
        let size = be32(moov, at);
        if size < 8 {
            break;
        }
        if &moov[at + 4..at + 8] == b"trak" {
            count += 1;
        }
        at += size;
    }
    count
}

fn seq_of(uri: &str) -> u32 {
    let stem = uri.strip_suffix(".m4s").expect("segment uri");
    stem.rsplit('-').next().expect("seq").parse().expect("seq")
}

fn assert_clean(what: &str, issues: Vec<transmux::validate::ConformanceIssue>) {
    let errors: Vec<_> = issues
        .iter()
        .filter(|i| i.severity == Severity::Error)
        .collect();
    assert!(errors.is_empty(), "{what}: {errors:?}");
}

/// Checks one audio rendition against the video playlist: same MEDIA-SEQUENCE
/// and MSN set, tfdt aligned within one audio frame, one audio trak in the
/// init, and every segment and part conformant. Returns the gap MSNs.
async fn check_rendition(
    rig: &Rig,
    video: &MediaPlaylist,
    video_src: &Source,
    audio: &Source,
    uri: &str,
) -> Vec<u32> {
    let video_ts = video_src.spec.timescale;
    let playlist = rig.playlist(uri).await;
    assert_eq!(
        playlist.media_sequence, video.media_sequence,
        "{uri}: MEDIA-SEQUENCE"
    );
    let video_seqs: Vec<u32> = video.segments.iter().map(|s| seq_of(&s.uri)).collect();
    let audio_seqs: Vec<u32> = playlist.segments.iter().map(|s| seq_of(&s.uri)).collect();
    assert_eq!(audio_seqs, video_seqs, "{uri}: MSN set");

    let map = playlist.segments[0].map.as_ref();
    let init_uri = map.expect("audio playlist has EXT-X-MAP").uri.clone();
    let init = rig.ok(&init_uri).await;
    assert_eq!(trak_count(&init), 1, "{uri}: audio init has one trak");
    assert!(
        init.windows(4).any(|w| w == b"soun"),
        "{uri}: init is an audio track"
    );
    assert_clean(&format!("{uri} init"), validate_init_segment(&init));

    let tolerance = audio.max_frame_secs() + 0.001;
    let audio_ts = f64::from(audio.spec.timescale);
    let mut gaps = Vec::new();
    let origin = video_src
        .samples
        .iter()
        .find(|s| s.flags.is_sync)
        .and_then(|s| s.dts)
        .map(|d| video_src.secs(d))
        .expect("video sync sample");
    let first_audio_rel = audio
        .samples
        .iter()
        .filter_map(|s| s.dts)
        .map(|d| audio.secs(d))
        .find(|t| *t >= origin)
        .map_or(0.0, |t| t - origin);
    for (idx, (v, a)) in video.segments.iter().zip(&playlist.segments).enumerate() {
        let seq = seq_of(&a.uri);
        if a.gap {
            gaps.push(seq);
            continue;
        }
        let vbytes = rig.ok(&v.uri).await;
        let abytes = rig.ok(&a.uri).await;
        let (_, vt) = first_tfdt(&vbytes);
        let (atrack, at) = first_tfdt(&abytes);
        assert_eq!(atrack, audio.spec.track_id.value(), "{uri}: traf track id");
        let video_secs = vt as f64 / f64::from(video_ts);
        let expected = if idx == 0 {
            video_secs.max(first_audio_rel)
        } else {
            video_secs
        };
        let delta = (at as f64 / audio_ts - expected).abs();
        assert!(
            delta <= tolerance,
            "{uri} MSN {seq}: audio tfdt {:.4}s vs video {expected:.4}s (delta {delta:.4}s > {tolerance:.4}s)",
            at as f64 / audio_ts,
        );
        assert_clean(&format!("{uri} seg {seq}"), validate_media_segment(&abytes));
        for part in &a.parts {
            let (status, pbytes) = rig.get(&part.uri).await;
            assert_eq!(status, StatusCode::OK, "{uri}: part {}", part.uri);
            assert_clean(
                &format!("{uri} part {}", part.uri),
                validate_media_segment(&pbytes),
            );
        }
    }
    assert!(
        gaps.len() < playlist.segments.len(),
        "{uri}: at least one real audio segment"
    );
    gaps
}

fn by_role(mut sources: Vec<Source>) -> Vec<Source> {
    sources.sort_by_key(|s| (!s.is_video(), !s.is_audio()));
    sources
}

fn france2() -> Vec<Source> {
    by_role(load("ts/france2.ts"))
}

#[tokio::test]
async fn split_audio_france2_three_renditions_align_with_video() {
    let sources = france2();
    let audio: Vec<&Source> = sources.iter().filter(|s| s.is_audio()).collect();
    assert_eq!(audio.len(), 3, "france2 carries three audio tracks");
    let video = &sources[0];
    let rig = rig(
        &sources,
        decls(&["fre", "qad", "qaa"]),
        index_selectors(3),
        &|_, _| true,
    );

    let master = String::from_utf8(rig.ok("master.m3u8").await).expect("utf8");
    assert_eq!(
        master.matches("#EXT-X-MEDIA:TYPE=AUDIO").count(),
        3,
        "{master}"
    );

    let video_playlist = rig.playlist("media.m3u8").await;
    assert!(
        video_playlist.segments.len() >= 2,
        "enough video segments to be meaningful: {}",
        video_playlist.segments.len()
    );
    let video_init = rig
        .ok(&video_playlist.segments[0].map.as_ref().expect("map").uri)
        .await;
    assert_eq!(trak_count(&video_init), 1, "primary init is video only");
    assert!(video_init.windows(4).any(|w| w == b"vide"));

    for (i, a) in audio.iter().enumerate() {
        let gaps =
            check_rendition(&rig, &video_playlist, video, a, &format!("audio/a{i}.m3u8")).await;
        assert!(
            gaps.len() <= 1,
            "continuous audio must not gap more than a trailing segment: {gaps:?}"
        );
    }
}

#[tokio::test]
async fn split_audio_h264_aac_track_id_selector() {
    let sources = by_role(load("ts/h264_aac.ts"));
    let aac = &sources[1];
    let rig = rig(
        &sources,
        decls(&["eng"]),
        vec![AudioTrackSelector::TrackId(aac.spec.track_id.value())],
        &|_, _| true,
    );
    let video = rig.playlist("media.m3u8").await;
    let gaps = check_rendition(&rig, &video, &sources[0], aac, "audio/a0.m3u8").await;
    assert!(gaps.len() <= 1, "{gaps:?}");
}

#[tokio::test]
async fn split_audio_silent_stretch_becomes_gap_with_contiguous_msns() {
    let sources = by_role(load("ts/h264_aac_40s.ts"));
    let baseline = rig(&sources, decls(&["eng"]), index_selectors(1), &|_, _| true);
    let video = baseline.playlist("media.m3u8").await;
    assert!(video.segments.len() >= 6, "need six segments");

    let first_sync = sources[0]
        .samples
        .iter()
        .find(|s| s.flags.is_sync)
        .and_then(|s| s.dts)
        .expect("video sync sample");
    let origin = sources[0].secs(first_sync);
    let mut starts = vec![0.0f64];
    for seg in &video.segments {
        let last = *starts.last().expect("start");
        starts.push(last + seg.duration.get());
    }
    let (from, to) = (origin + starts[2], origin + starts[4]);
    let audio_ts = f64::from(sources[1].spec.timescale);
    let rig = rig(&sources, decls(&["eng"]), index_selectors(1), &|ti, s| {
        let t = s.dts.unwrap_or(0) as f64 / audio_ts;
        !(ti == 1 && t >= from && t < to)
    });

    let video = rig.playlist("media.m3u8").await;
    let gaps = check_rendition(&rig, &video, &sources[0], &sources[1], "audio/a0.m3u8").await;
    let expected = [
        seq_of(&video.segments[2].uri),
        seq_of(&video.segments[3].uri),
    ];
    for want in &expected {
        assert!(gaps.contains(want), "MSN {want} must be a gap: {gaps:?}");
    }
    let text = String::from_utf8(rig.ok("audio/a0.m3u8").await).expect("utf8");
    assert!(text.contains("#EXT-X-GAP"), "{text}");
    let audio = rig.playlist("audio/a0.m3u8").await;
    for (v, a) in video.segments.iter().zip(&audio.segments) {
        assert!(!v.gap, "video never gaps");
        if a.gap {
            assert!(
                (a.duration.get() - v.duration.get()).abs() < 1e-3,
                "a gap segment carries the leader's duration"
            );
        }
    }
}

#[tokio::test]
async fn split_audio_resources_route_to_their_own_rendition() {
    let sources = france2();
    let rig = rig(
        &sources,
        decls(&["fre", "qad", "qaa"]),
        index_selectors(3),
        &|_, _| true,
    );
    let video = rig.playlist("media.m3u8").await;
    let video_seg = rig.ok(&video.segments[0].uri).await;
    let (video_track, _) = first_tfdt(&video_seg);
    assert_eq!(video_track, sources[0].spec.track_id.value());

    let mut seen_tracks = Vec::new();
    for i in 0..3 {
        let audio = rig.playlist(&format!("audio/a{i}.m3u8")).await;
        let seg = rig.ok(&audio.segments[0].uri).await;
        let (track, _) = first_tfdt(&seg);
        assert_eq!(
            track,
            sources[1 + i].spec.track_id.value(),
            "rendition {i} serves its own source track"
        );
        seen_tracks.push(track);
        let init = rig
            .ok(&audio.segments[0].map.as_ref().expect("map").uri)
            .await;
        assert!(init.windows(4).any(|w| w == b"soun"), "rendition {i} init");
    }
    seen_tracks.sort_unstable();
    seen_tracks.dedup();
    assert_eq!(seen_tracks.len(), 3, "three distinct renditions");
}

#[tokio::test]
async fn split_audio_off_leaves_default_route_byte_identical() {
    let sources = by_role(load("ts/h264_aac.ts"));
    let plain = rig(&sources, Vec::new(), Vec::new(), &|_, _| true);
    let declared_only = rig(&sources, decls(&["eng"]), Vec::new(), &|_, _| true);

    let a = String::from_utf8(plain.ok("media.m3u8").await).expect("utf8");
    let b = String::from_utf8(declared_only.ok("media.m3u8").await).expect("utf8");
    let (pa, pb) = (
        MediaPlaylist::parse(&a).expect("plain parses"),
        MediaPlaylist::parse(&b).expect("declared parses"),
    );
    assert_eq!(pa.segments.len(), pb.segments.len());
    for (x, y) in pa.segments.iter().zip(&pb.segments) {
        let (xb, yb) = (plain.ok(&x.uri).await, declared_only.ok(&y.uri).await);
        assert_eq!(xb, yb, "segment bytes with split off");
        assert_eq!(x.duration.get(), y.duration.get());
    }
    let ia = plain
        .ok(&pa.segments[0].map.as_ref().expect("map").uri)
        .await;
    let ib = declared_only
        .ok(&pb.segments[0].map.as_ref().expect("map").uri)
        .await;
    assert_eq!(ia, ib, "init bytes with split off");
    assert!(
        trak_count(&ia) == 2,
        "with split off the primary stays muxed (video + audio)"
    );
    let (status, _) = declared_only.get("audio/a0.m3u8").await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "no audio playlist unless split is enabled"
    );
    let master = String::from_utf8(plain.ok("master.m3u8").await).expect("utf8");
    assert!(!master.contains("EXT-X-MEDIA"), "{master}");
}

#[tokio::test]
async fn split_audio_track_appearing_mid_stream_starts_at_next_cut() {
    let mut sources = by_role(load("ts/h264_aac_40s.ts"));
    let second = Source {
        spec: {
            let mut spec = sources[1].spec.clone();
            spec.track_id = transmux::TrackId::new(99);
            spec
        },
        samples: sources[1].samples.clone(),
    };
    sources.push(second);
    let early_video = &sources[0];
    let a0 = &sources[1];
    let a1 = &sources[2];

    let trunk = new_trunk();
    let writer = trunk.writer().expect("writer");
    writer.set_tracks(vec![early_video.spec.clone(), a0.spec.clone()]);
    let route = Arc::new(
        RouteHandle::new(TARGET_DURATION_SECS, PART_TARGET_MS, WINDOW_SEGMENTS)
            .with_declared_bandwidth_bps(8_000_000)
            .with_audio_renditions(decls(&["fre", "qad"]))
            .with_audio_rendition_sources(vec![
                AudioTrackSelector::Index(0),
                AudioTrackSelector::TrackId(a1.spec.track_id.value()),
            ]),
    );
    route.publish_program(ProgramId(0), Arc::clone(&trunk));
    let mut segmenter =
        ProgramSegmenter::try_new(&trunk, &route, TARGET_DURATION_SECS, PART_TARGET_MS)
            .expect("segmenter");
    if let Some(init) = segmenter.init_segment() {
        route.set_init(ProgramId(0), init);
    }

    let half = |s: &Source| s.samples.len() / 2;
    let publish = |src: &Source, range: std::ops::Range<usize>| {
        for s in &src.samples[range] {
            writer.publish(src.spec.track_id, RetentionClass::Timed, s.clone());
        }
    };
    publish(early_video, 0..half(early_video));
    publish(a0, 0..half(a0));
    segmenter.pump(ProgramId(0), &trunk, &route);

    writer.set_tracks(vec![
        early_video.spec.clone(),
        a0.spec.clone(),
        a1.spec.clone(),
    ]);
    publish(early_video, half(early_video)..early_video.samples.len());
    publish(a0, half(a0)..a0.samples.len());
    publish(a1, half(a1)..a1.samples.len());
    segmenter.pump(ProgramId(0), &trunk, &route);
    segmenter.flush();

    let rig = Rig {
        app: app_for(route),
        live: None,
    };
    let video = rig.playlist("media.m3u8").await;
    let late = rig.playlist("audio/a1.m3u8").await;
    assert!(
        late.segments.iter().any(|s| !s.gap),
        "the late track produces real segments after it appears"
    );
    let first = rig.playlist("audio/a0.m3u8").await;
    assert_eq!(first.segments.len(), video.segments.len());
    assert_eq!(first.media_sequence, video.media_sequence);
}

#[test]
fn split_audio_config_validates_rendition_sources() {
    fn config(extra: &str, outputs: &str) -> String {
        format!(
            r#"{{"routes":[{{"name":"cam","input":{{"type":"rtsp","url":"rtsp://host/s1"}},
            "outputs":{outputs},
            "audio_renditions":[
              {{"group_id":"aud","name":"A","language":"fr","default":true,"autoselect":true,"channels":"2","uri":"audio/a.m3u8","characteristics":[]}},
              {{"group_id":"aud","name":"B","language":"en","default":false,"autoselect":true,"channels":"2","uri":"audio/b.m3u8","characteristics":[]}}
            ]{extra}}}]}}"#
        )
    }
    let check = |extra: &str, outputs: &str| {
        let cfg: Config = serde_json::from_str(&config(extra, outputs)).expect("config parses");
        cfg.validate()
    };
    check(
        r#","audio_rendition_sources":[{"index":0},{"track_id":258}]"#,
        r#"["llhls"]"#,
    )
    .expect("valid split config");
    check("", r#"["llhls"]"#).expect("declared-only config still valid");
    assert!(
        check(
            r#","audio_rendition_sources":[{"index":0}]"#,
            r#"["llhls"]"#
        )
        .is_err()
    );
    assert!(
        check(
            r#","audio_rendition_sources":[{"index":0},{"index":0}]"#,
            r#"["llhls"]"#
        )
        .is_err()
    );
    assert!(
        check(
            r#","audio_rendition_sources":[{"index":0},{"index":1}]"#,
            r#"["llhls","dash"]"#
        )
        .is_err()
    );
}

type Finding = (String, i64, String);

fn collect_findings(value: &serde_json::Value, path: &str, out: &mut Vec<Finding>) {
    match value {
        serde_json::Value::Object(map) => {
            let parse_failed = map
                .get("parseFailed")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if let Some(serde_json::Value::Array(messages)) = map.get("messages") {
                for m in messages {
                    let level = m
                        .get("errorRequirementLevel")
                        .and_then(|v| v.as_i64())
                        .unwrap_or(0);
                    out.push((
                        path.to_string(),
                        if parse_failed { 1 } else { level },
                        m.get("errorComment")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string(),
                    ));
                }
            }
            for (k, v) in map {
                if k != "messages" {
                    collect_findings(v, &format!("{path}/{k}"), out);
                }
            }
        }
        serde_json::Value::Array(items) => {
            for (i, v) in items.iter().enumerate() {
                collect_findings(v, &format!("{path}[{i}]"), out);
            }
        }
        _ => {}
    }
}

fn validator_available() -> bool {
    std::process::Command::new("mediastreamvalidator")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Mirrors every playlist and resource of `rig` into a temp dir and runs
/// Apple's `mediastreamvalidator` over its `master.m3u8`; returns the
/// level-1 (MUST) findings as `(path, message)`.
async fn validator_must_findings(rig: &Rig, audio_renditions: usize) -> Vec<(String, String)> {
    use std::fs;
    use std::process::Command;
    use std::time::Duration;

    let scratch = std::env::temp_dir().join(format!(
        "split-audio-validator-{}-{audio_renditions}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&scratch);
    fs::create_dir_all(scratch.join("audio")).expect("scratch");

    let mut names = vec![
        "master.m3u8".to_string(),
        "media.m3u8".to_string(),
        "iframe.m3u8".to_string(),
    ];
    names.extend((0..audio_renditions).map(|i| format!("audio/a{i}.m3u8")));
    let mut uris = std::collections::BTreeSet::new();
    for name in &names {
        let body = rig.ok(name).await;
        fs::write(scratch.join(name), &body).expect("write playlist");
        if name.ends_with("master.m3u8") {
            continue;
        }
        for line in String::from_utf8(body).expect("utf8").lines() {
            if let Some(start) = line.find("URI=\"") {
                let rest = &line[start + 5..];
                if let Some(end) = rest.find('"') {
                    uris.insert(rest[..end].to_string());
                }
            }
            let t = line.trim();
            if !t.is_empty() && !t.starts_with('#') {
                uris.insert(t.to_string());
            }
        }
    }
    for uri in &uris {
        let (status, body) = rig.get(uri).await;
        if status == StatusCode::OK {
            fs::write(scratch.join(uri), body).expect("write resource");
        }
    }

    let report = scratch.join("report.json");
    let mut cmd = Command::new("mediastreamvalidator");
    cmd.current_dir(&scratch)
        .arg("--quiet")
        .arg("-t")
        .arg("5")
        .arg("-O")
        .arg(&report)
        .arg("master.m3u8");
    let _ = test_bounded::output_bounded(&mut cmd, Duration::from_secs(60)).expect("run validator");
    let json: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&report).expect("report")).expect("json");
    eprintln!("SCRATCH={}", scratch.display());
    let mut findings = Vec::new();
    collect_findings(&json, "$", &mut findings);
    for f in &findings {
        eprintln!(
            "VALIDATOR[{audio_renditions}]: {} level={} {}",
            f.0, f.1, f.2
        );
    }
    findings
        .into_iter()
        .filter(|f| f.1 <= 1)
        .map(|f| (f.0, f.2))
        .collect()
}

/// Level-1 (MUST) validator messages split audio adds over the muxed route,
/// each knowingly left open. (slice 4 closed the rendition-report gap.)
const KNOWN_SPLIT_ONLY: &[&str] = &[];

/// The validator insists a file exists for an `EXT-X-GAP` segment, although
/// RFC 8216 §4.3.2.7 says a gap segment carries no media and clients should
/// not load it. Only tolerated when the audio playlist really has a gap.
const GAP_FILE_FINDING: &str = "Media file does not exist at the specified path.";

#[tokio::test]
async fn split_audio_validator_findings() {
    if !validator_available() {
        eprintln!("SKIP split_audio_validator_findings: `mediastreamvalidator` not on PATH");
        return;
    }
    let sources = france2();
    let split = rig(
        &sources,
        decls(&["fr", "qad", "qaa"]),
        index_selectors(3),
        &|_, _| true,
    );
    let split_findings = validator_must_findings(&split, 3).await;
    let mut has_gap = false;
    for i in 0..3 {
        let text = String::from_utf8(split.ok(&format!("audio/a{i}.m3u8")).await).expect("utf8");
        has_gap |= text.contains("#EXT-X-GAP");
    }

    let muxed = rig(&sources, Vec::new(), Vec::new(), &|_, _| true);
    let muxed_messages: std::collections::BTreeSet<String> = validator_must_findings(&muxed, 0)
        .await
        .into_iter()
        .map(|(_, m)| m)
        .collect();

    let unexpected: Vec<_> = split_findings
        .iter()
        .filter(|(_, m)| {
            let tolerated = KNOWN_SPLIT_ONLY.contains(&m.as_str())
                || muxed_messages.contains(m)
                || (has_gap && m == GAP_FILE_FINDING);
            !tolerated
        })
        .collect();
    assert!(
        !split_findings
            .iter()
            .any(|(_, m)| m.contains("Missing rendition report")
                || m.contains("No rendition reports")),
        "rendition report finding present: {split_findings:?}"
    );
    assert!(
        !split_findings
            .iter()
            .any(|(_, m)| m.contains("Non-canonical language subtag")),
        "non-canonical language subtag reported: {split_findings:?}"
    );
    assert!(
        unexpected.is_empty(),
        "level-1 findings split audio introduces over the muxed route: {unexpected:?}"
    );
}

fn derived_decls(n: usize) -> Vec<AudioRenditionDecl> {
    (0..n)
        .map(|i| AudioRenditionDecl {
            group_id: "aud".into(),
            default: i == 0,
            autoselect: true,
            uri: format!("audio/a{i}.m3u8"),
            ..AudioRenditionDecl::default()
        })
        .collect()
}

async fn master_of(rig: &Rig) -> (String, broadcast_hls::MasterPlaylist) {
    let text = String::from_utf8(rig.ok("master.m3u8").await).expect("utf8");
    let parsed = broadcast_hls::MasterPlaylist::parse(&text)
        .unwrap_or_else(|e| panic!("master must parse: {e}\n{text}"));
    (text, parsed)
}

fn peak_bps(segments: &[(usize, f64)]) -> u64 {
    segments
        .iter()
        .map(|(len, dur)| (*len as f64 * 8.0 / dur).ceil() as u64)
        .max()
        .expect("segments")
}

fn mean_bps(segments: &[(usize, f64)]) -> u64 {
    let bytes: usize = segments.iter().map(|(l, _)| *l).sum();
    let secs: f64 = segments.iter().map(|(_, d)| *d).sum();
    (bytes as f64 * 8.0 / secs).round() as u64
}

async fn served_segments(rig: &Rig, uri: &str) -> Vec<(usize, f64)> {
    let playlist = rig.playlist(uri).await;
    assert!(
        playlist.segments.len() < WINDOW_SEGMENTS,
        "no segment evicted, so the origin's cumulative totals equal the served window"
    );
    let mut out = Vec::new();
    for seg in &playlist.segments {
        if seg.gap {
            continue;
        }
        out.push((rig.ok(&seg.uri).await.len(), seg.duration.get()));
    }
    out
}

fn within(actual: u64, expected: u64, what: &str) {
    let tolerance = expected / 200;
    assert!(
        actual.abs_diff(expected) <= tolerance,
        "{what}: {actual} vs independently computed {expected} (tolerance {tolerance})"
    );
}

#[tokio::test]
async fn split_master_parses_round_trips_and_lists_one_variant_per_group() {
    let sources = france2();
    let rig = rig_declared(
        &sources,
        derived_decls(3),
        index_selectors(3),
        None,
        &|_, _| true,
    );
    let (text, master) = master_of(&rig).await;
    assert_eq!(master.media.len(), 3, "{text}");
    assert_eq!(master.variants.len(), 1, "{text}");
    assert_eq!(master.variants[0].audio.as_deref(), Some("aud"));
    let again = master.to_m3u8().expect("serialize");
    assert_eq!(
        broadcast_hls::MasterPlaylist::parse(&again).expect("reparse"),
        master,
        "parse -> serialize -> parse"
    );
}

#[tokio::test]
async fn split_master_codecs_carry_video_and_audio_codec() {
    let sources = france2();
    let rig = rig_declared(
        &sources,
        derived_decls(3),
        index_selectors(3),
        None,
        &|_, _| true,
    );
    let (text, master) = master_of(&rig).await;
    let codecs = master.variants[0].codecs.clone().expect("CODECS");
    let parts: Vec<&str> = codecs.split(',').collect();
    assert_eq!(parts.len(), 2, "video + one distinct audio codec: {text}");
    assert!(parts[0].starts_with("avc1."), "{text}");
    assert_eq!(parts[1], "ec-3", "{text}");
}

#[tokio::test]
async fn split_master_bandwidth_is_video_peak_plus_max_audio_peak() {
    let sources = france2();
    let rig = rig_declared(
        &sources,
        derived_decls(3),
        index_selectors(3),
        None,
        &|_, _| true,
    );
    let (text, master) = master_of(&rig).await;
    let video = served_segments(&rig, "media.m3u8").await;
    let mut audio_peak = 0;
    let mut audio_mean = 0;
    for i in 0..3 {
        let segs = served_segments(&rig, &format!("audio/a{i}.m3u8")).await;
        audio_peak = audio_peak.max(peak_bps(&segs));
        audio_mean = audio_mean.max(mean_bps(&segs));
    }
    let variant = &master.variants[0];
    within(
        u64::from(variant.bandwidth),
        peak_bps(&video) + audio_peak,
        &format!("BANDWIDTH {text}"),
    );
    within(
        u64::from(variant.average_bandwidth.expect("AVERAGE-BANDWIDTH")),
        mean_bps(&video) + audio_mean,
        &format!("AVERAGE-BANDWIDTH {text}"),
    );
    assert!(
        u64::from(variant.bandwidth) > peak_bps(&video) + audio_peak / 2,
        "audio is counted, not just video"
    );
}

#[tokio::test]
async fn split_master_declared_bandwidth_floors_the_video_part_only() {
    let sources = france2();
    let rig = rig_declared(
        &sources,
        derived_decls(3),
        index_selectors(3),
        Some(DECLARED_BPS),
        &|_, _| true,
    );
    let (text, master) = master_of(&rig).await;
    let video = served_segments(&rig, "media.m3u8").await;
    assert!(
        peak_bps(&video) < DECLARED_BPS,
        "fixture peak under the override"
    );
    let mut audio_peak = 0;
    for i in 0..3 {
        let segs = served_segments(&rig, &format!("audio/a{i}.m3u8")).await;
        audio_peak = audio_peak.max(peak_bps(&segs));
    }
    within(
        u64::from(master.variants[0].bandwidth),
        DECLARED_BPS + audio_peak,
        &format!("BANDWIDTH {text}"),
    );
}

#[tokio::test]
async fn split_master_derives_language_name_and_audio_description_from_the_pmt() {
    let sources = france2();
    let audio: Vec<&Source> = sources.iter().filter(|s| s.is_audio()).collect();
    let rig = rig_declared(
        &sources,
        derived_decls(3),
        index_selectors(3),
        None,
        &|_, _| true,
    );
    let (text, master) = master_of(&rig).await;
    for (i, a) in audio.iter().enumerate() {
        let meta = a.spec.audio_metadata();
        let lang = meta.languages[0].language.clone();
        let expected_language = hls_runtime::bcp47_language(&lang).unwrap_or_else(|| lang.clone());
        let media = &master.media[i];
        assert_eq!(
            media.language.as_deref(),
            Some(expected_language.as_str()),
            "{text}"
        );
        let ad = meta.role == transmux::AudioRole::AudioDescription;
        assert_eq!(
            media
                .characteristics
                .contains(&"public.accessibility.describes-video".to_string()),
            ad,
            "{text}"
        );
        let expected_name = if ad {
            format!("{lang} (AD)")
        } else {
            lang.clone()
        };
        assert_eq!(media.name, expected_name, "{text}");
        assert!(media.channels.is_some(), "CHANNELS derived: {text}");
    }
    let languages: Vec<String> = master
        .media
        .iter()
        .map(|m| m.language.clone().expect("language"))
        .collect();
    assert_eq!(
        languages,
        ["fr", "qad", "qaa"],
        "france2 PMT languages: {text}"
    );
    assert_eq!(
        master.media[1].characteristics,
        ["public.accessibility.describes-video"],
        "qad is the audio-description track: {text}"
    );
    assert!(master.media[0].characteristics.is_empty(), "{text}");
    assert!(master.media[2].characteristics.is_empty(), "{text}");
    let names: std::collections::BTreeSet<&str> =
        master.media.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(names.len(), 3, "NAME unique within the group: {text}");
}

#[tokio::test]
async fn split_master_config_values_win_over_derived() {
    let sources = france2();
    let mut declared = derived_decls(3);
    declared[1].language = "de".into();
    declared[1].name = "Deutsch".into();
    declared[1].characteristics = vec!["public.accessibility.transcribes-spoken-dialog".into()];
    declared[1].channels = Some("6".into());
    let rig = rig_declared(&sources, declared, index_selectors(3), None, &|_, _| true);
    let (text, master) = master_of(&rig).await;
    let m = &master.media[1];
    assert_eq!(m.language.as_deref(), Some("de"), "{text}");
    assert_eq!(m.name, "Deutsch", "{text}");
    assert_eq!(
        m.characteristics,
        ["public.accessibility.transcribes-spoken-dialog"],
        "{text}"
    );
    assert_eq!(m.channels.as_deref(), Some("6"), "{text}");
    assert_eq!(master.media[0].language.as_deref(), Some("fr"), "{text}");
}

#[tokio::test]
async fn split_master_duplicate_derived_names_are_made_unique() {
    let sources = france2();
    let selectors = vec![AudioTrackSelector::Index(0), AudioTrackSelector::Index(0)];
    let rig = rig_declared(&sources, derived_decls(2), selectors, None, &|_, _| true);
    let (text, master) = master_of(&rig).await;
    assert_ne!(master.media[0].name, master.media[1].name, "{text}");
}

#[tokio::test]
async fn non_split_master_is_unchanged() {
    let sources = by_role(load("ts/h264_aac.ts"));
    let plain = rig(&sources, Vec::new(), Vec::new(), &|_, _| true);
    let (text, master) = master_of(&plain).await;
    assert!(master.media.is_empty(), "{text}");
    assert_eq!(master.variants.len(), 1, "{text}");
    let v = &master.variants[0];
    assert_eq!(u64::from(v.bandwidth), DECLARED_BPS, "{text}");
    assert!(v.audio.is_none(), "{text}");
    let codecs = v.codecs.clone().expect("CODECS");
    assert!(
        codecs.starts_with("avc1.") && codecs.ends_with(",mp4a.40.2"),
        "muxed CODECS is the one the route always carried: {text}"
    );

    let declared_only = rig(&sources, decls(&["eng"]), Vec::new(), &|_, _| true);
    let (text2, master2) = master_of(&declared_only).await;
    assert_eq!(master2.media.len(), 1, "{text2}");
    assert_eq!(master2.media[0].name, "Audio 0", "{text2}");
    assert_eq!(master2.variants[0].bandwidth, v.bandwidth, "{text2}");
    assert_eq!(master2.variants[0].codecs, v.codecs, "{text2}");
}

#[tokio::test]
async fn split_audio_each_playlist_carries_rendition_reports() {
    let sources = france2();
    let rig = rig(
        &sources,
        decls(&["fre", "qad", "qaa"]),
        index_selectors(3),
        &|_, _| true,
    );
    let n_audio = 3usize;
    let n_reports = n_audio; // video + n_audio renditions -> n_audio reports
    // video playlist
    let pl = rig.playlist("media.m3u8").await;
    assert_eq!(pl.rendition_reports.len(), n_reports);
    let uris: std::collections::BTreeSet<String> =
        pl.rendition_reports.iter().map(|r| r.uri.clone()).collect();
    assert_eq!(
        uris,
        ["audio/a0.m3u8", "audio/a1.m3u8", "audio/a2.m3u8"]
            .iter()
            .map(|s| s.to_string())
            .collect()
    );
    // each audio playlist
    for i in 0..n_audio {
        let path = format!("audio/a{i}.m3u8");
        let pl = rig.playlist(&path).await;
        assert_eq!(pl.rendition_reports.len(), n_reports, "{path}");
        let uris: std::collections::BTreeSet<String> =
            pl.rendition_reports.iter().map(|r| r.uri.clone()).collect();
        let mut expected = std::collections::BTreeSet::new();
        expected.insert("../media.m3u8".to_string());
        for j in 0..n_audio {
            if j != i {
                expected.insert(format!("a{j}.m3u8"));
            }
        }
        assert_eq!(uris, expected, "{path}");
    }
}

fn served_edge(pl: &MediaPlaylist) -> (u64, Option<u64>) {
    let closed = pl.media_sequence + pl.segments.len() as u64;
    match &pl.open_segment {
        Some(open) if !open.parts.is_empty() => (closed, Some(open.parts.len() as u64 - 1)),
        _ => {
            let last_parts = pl.segments.last().map_or(0, |s| s.parts.len());
            (closed - 1, (last_parts > 0).then(|| last_parts as u64 - 1))
        }
    }
}

fn cut_secs(sources: &[Source], nth_sync: usize) -> f64 {
    let video = &sources[0];
    let s = video
        .samples
        .iter()
        .filter(|s| s.flags.is_sync)
        .nth(nth_sync)
        .expect("enough video keyframes");
    video.secs(s.dts.unwrap_or(0))
}

async fn assert_reports_match_served_edges(rig: &Rig, n_audio: usize) {
    let mut paths = vec!["media.m3u8".to_string()];
    paths.extend((0..n_audio).map(|i| format!("audio/a{i}.m3u8")));
    let mut edges = Vec::new();
    for path in &paths {
        edges.push(served_edge(&rig.playlist(path).await));
    }
    for (i, path) in paths.iter().enumerate() {
        let pl = rig.playlist(path).await;
        assert_eq!(pl.rendition_reports.len(), paths.len() - 1, "{path}");
        for (j, other) in paths.iter().enumerate() {
            if i == j {
                continue;
            }
            let from = path.rfind('/').map_or("", |k| &path[..=k]);
            let want_uri = other
                .strip_prefix(from)
                .unwrap_or(&format!("../{other}"))
                .to_string();
            let report = pl
                .rendition_reports
                .iter()
                .find(|r| r.uri == want_uri)
                .unwrap_or_else(|| panic!("{path} lacks report for {want_uri}"));
            assert_eq!(
                (report.last_msn, report.last_part),
                edges[j],
                "{path} reports {other}'s edge"
            );
        }
    }
}

#[tokio::test]
async fn split_audio_rendition_report_matches_sibling_live_edge() {
    let sources = france2();
    let cut = cut_secs(&sources, 1);
    let rig = rig_live(
        &sources,
        decls(&["fre", "qad", "qaa"]),
        index_selectors(3),
        &|ti, s| sources[ti].secs(s.dts.unwrap_or(0)) < cut,
    );
    assert_reports_match_served_edges(&rig, 3).await;
    let before = served_edge(&rig.playlist("media.m3u8").await);

    rig.publish_more(&sources, &|ti, s| {
        sources[ti].secs(s.dts.unwrap_or(0)) >= cut
    });
    let after = served_edge(&rig.playlist("media.m3u8").await);
    assert_ne!(before, after, "publishing more moved the video live edge");
    assert_reports_match_served_edges(&rig, 3).await;
    let pl = rig.playlist("audio/a0.m3u8").await;
    let video_report = pl
        .rendition_reports
        .iter()
        .find(|r| r.uri == "../media.m3u8")
        .expect("video report");
    assert_eq!((video_report.last_msn, video_report.last_part), after);
}

#[tokio::test]
async fn split_audio_delta_playlist_keeps_rendition_reports() {
    let sources = france2();
    let rig = rig(
        &sources,
        decls(&["fre", "qad", "qaa"]),
        index_selectors(3),
        &|_, _| true,
    );
    let text = String::from_utf8(rig.ok("media.m3u8?_HLS_skip=YES").await).expect("utf8");
    let pl = MediaPlaylist::parse(&text).expect("parse delta");
    assert!(pl.skip.is_some(), "delta skip info present");
    assert_eq!(pl.rendition_reports.len(), 3, "{text}");
}

#[tokio::test]
async fn split_audio_blocking_reload_audio_reports_current() {
    let sources = france2();
    let rig = rig(
        &sources,
        decls(&["fre", "qad", "qaa"]),
        index_selectors(3),
        &|_, _| true,
    );
    let (status, body) = rig.get("audio/a0.m3u8?_HLS_msn=2").await;
    assert_eq!(status, axum::http::StatusCode::OK);
    let text = String::from_utf8(body).expect("utf8");
    let pl = MediaPlaylist::parse(&text).expect("parse");
    assert_eq!(pl.rendition_reports.len(), 3, "{text}");
    let video = served_edge(&rig.playlist("media.m3u8").await);
    let report = pl
        .rendition_reports
        .iter()
        .find(|r| r.uri == "../media.m3u8")
        .expect("video report");
    assert_eq!((report.last_msn, report.last_part), video, "{text}");
}

#[tokio::test]
async fn non_split_route_rendition_reports_absent() {
    let sources = by_role(load("ts/h264_aac.ts"));
    let rig = rig(&sources, Vec::new(), Vec::new(), &|_, _| true);
    let pl = rig.playlist("media.m3u8").await;
    assert!(pl.rendition_reports.is_empty());
    let text = String::from_utf8(rig.ok("iframe.m3u8").await).expect("utf8");
    assert!(text.contains("#EXT-X-RENDITION-REPORT:URI=\"media.m3u8\""));
    assert_eq!(
        text.matches("#EXT-X-RENDITION-REPORT").count(),
        1,
        "iframe playlist keeps exactly its media report: {text}"
    );
    let media = String::from_utf8(rig.ok("media.m3u8").await).expect("utf8");
    assert!(!media.contains("#EXT-X-RENDITION-REPORT"), "{media}");
}

const CHECKPOINT_STEP_SECS: f64 = 0.5;
const BLOCKING_STEP_SECS: f64 = 0.1;
const BLOCKING_POLL: std::time::Duration = std::time::Duration::from_millis(50);
const MAX_PART_LAG: usize = 1;
const TARGET_PART: u64 = 1;
const WARMUP_SEGMENTS: usize = 2;
const MIN_CHECKPOINTS: usize = 40;

fn media_range_secs(sources: &[Source]) -> (f64, f64) {
    let times = sources
        .iter()
        .filter(|s| s.is_video() || s.is_audio())
        .flat_map(|s| s.samples.iter().map(|x| s.secs(x.dts.unwrap_or(0))));
    times.fold((f64::MAX, f64::MIN), |(lo, hi), t| (lo.min(t), hi.max(t)))
}

fn parts_of(pl: &MediaPlaylist) -> Vec<(u64, u64)> {
    let closed = pl.segments.iter().map(|s| s.parts.len());
    let open = pl.open_segment.iter().map(|o| o.parts.len());
    closed
        .chain(open)
        .enumerate()
        .flat_map(|(i, n)| (0..n as u64).map(move |p| (pl.media_sequence + i as u64, p)))
        .collect()
}

fn window(sources: &[Source], from: f64, to: f64) -> impl Fn(usize, &Sample) -> bool + '_ {
    move |ti, s| {
        let t = sources[ti].secs(s.dts.unwrap_or(0));
        t >= from && t < to
    }
}

fn aac_rig_live(sources: &[Source], until: f64) -> Rig {
    let aac = sources.iter().find(|s| s.is_audio()).expect("audio track");
    rig_live_aligned(
        sources,
        decls(&["eng"]),
        vec![AudioTrackSelector::TrackId(aac.spec.track_id.value())],
        &window(sources, f64::MIN, until),
    )
}

#[tokio::test]
async fn split_audio_live_edge_tracks_video_within_one_part_while_publishing() {
    let sources = by_role(load("ts/h264_aac_40s.ts"));
    let (start, end) = media_range_secs(&sources);
    let mut at = start + CHECKPOINT_STEP_SECS;
    let rig = aac_rig_live(&sources, at);
    let mut checkpoints = 0;
    let mut hinted = 0;
    while at < end {
        let video = rig.playlist("media.m3u8").await;
        let audio = rig.playlist("audio/a0.m3u8").await;
        let video_parts = parts_of(&video);
        if video.segments.len() >= WARMUP_SEGMENTS {
            let audio_edge = *parts_of(&audio)
                .last()
                .unwrap_or_else(|| panic!("audio playlist has no part at {at:.2}s"));
            let audio_at = video_parts
                .iter()
                .position(|p| *p == audio_edge)
                .unwrap_or_else(|| panic!("audio edge {audio_edge:?} is not a video part"));
            let lag = video_parts.len() - 1 - audio_at;
            assert!(
                lag <= MAX_PART_LAG,
                "at {at:.2}s audio edge {audio_edge:?} trails video edge {:?} by {lag} parts",
                video_parts.last()
            );
            if audio
                .open_segment
                .as_ref()
                .is_some_and(|o| !o.parts.is_empty())
            {
                let hint = audio
                    .low_latency
                    .as_ref()
                    .and_then(|ll| ll.preload_hint_part.as_deref())
                    .unwrap_or_else(|| panic!("audio playlist has no PRELOAD-HINT at {at:.2}s"));
                let listed = audio.open_segment.iter().flat_map(|o| &o.parts);
                assert!(
                    listed.into_iter().all(|p| p.uri != hint),
                    "PRELOAD-HINT {hint} names a part already listed"
                );
                hinted += 1;
            }
            checkpoints += 1;
        }
        let next = at + CHECKPOINT_STEP_SECS;
        rig.publish_more(&sources, &window(&sources, at, next));
        at = next;
    }
    assert!(
        checkpoints >= MIN_CHECKPOINTS,
        "only {checkpoints} checkpoints compared"
    );
    assert!(
        hinted > 0,
        "no checkpoint had an open audio segment to hint"
    );
}

#[tokio::test]
async fn split_audio_blocking_reload_returns_when_the_matching_audio_part_exists() {
    let sources = by_role(load("ts/h264_aac_40s.ts"));
    let (start, end) = media_range_secs(&sources);
    let mut at = start + BLOCKING_STEP_SECS;
    let rig = aac_rig_live(&sources, at);
    let target = loop {
        assert!(at < end, "video never reached part 1 of a later segment");
        let video = rig.playlist("media.m3u8").await;
        if let Some(&(msn, TARGET_PART)) = parts_of(&video).last()
            && msn > video.media_sequence
        {
            break (msn, TARGET_PART);
        }
        let next = at + BLOCKING_STEP_SECS;
        rig.publish_more(&sources, &window(&sources, at, next));
        at = next;
    };

    let reload_path = format!("audio/a0.m3u8?_HLS_msn={}&_HLS_part={}", target.0, target.1);
    let reload = rig.get(&reload_path);
    tokio::pin!(reload);
    assert!(
        tokio::time::timeout(BLOCKING_POLL, &mut reload)
            .await
            .is_err(),
        "reload for a part that does not exist yet must block"
    );

    let (status, body) = loop {
        assert!(at < end, "blocked reload never returned");
        let next = at + BLOCKING_STEP_SECS;
        rig.publish_more(&sources, &window(&sources, at, next));
        at = next;
        if let Ok(done) = tokio::time::timeout(BLOCKING_POLL, &mut reload).await {
            break done;
        }
    };
    assert_eq!(status, StatusCode::OK);
    let text = String::from_utf8(body).expect("utf8");
    let audio = MediaPlaylist::parse(&text).expect("parse");
    let audio_parts = parts_of(&audio);
    assert!(audio_parts.contains(&target), "{text}");

    let video = rig.playlist("media.m3u8").await;
    let video_parts = parts_of(&video);
    let target_at = video_parts
        .iter()
        .position(|p| *p == target)
        .expect("target is a video part");
    let video_lead = video_parts.len() - 1 - target_at;
    assert_eq!(
        video_parts.last().map(|p| p.0),
        Some(target.0),
        "reload returned only after the video moved on to the next segment"
    );
    assert!(
        video_lead <= 2 * MAX_PART_LAG,
        "reload returned with video {video_lead} parts past the matching part: the audio part \
         was held back until a later boundary"
    );

    rig.publish_more(&sources, &window(&sources, at, end + 1.0));
    let finished = rig.playlist("media.m3u8").await;
    let segment = &finished.segments[(target.0 - finished.media_sequence) as usize];
    let remaining_parts_secs = (target.1 + 3) as f64 * f64::from(PART_TARGET_MS) / 1000.0;
    assert!(
        segment.duration.get() >= remaining_parts_secs,
        "segment {} lasts {}s, too short to tell part-level from segment-level following",
        target.0,
        segment.duration.get()
    );
}

#[tokio::test]
async fn split_audio_validator_reports_no_new_part_findings_over_the_muxed_route() {
    if !validator_available() {
        eprintln!(
            "SKIP split_audio_validator_reports_no_new_part_findings_over_the_muxed_route: \
             `mediastreamvalidator` not on PATH"
        );
        return;
    }
    let sources = france2();
    let split = rig(
        &sources,
        decls(&["fr", "qad", "qaa"]),
        index_selectors(3),
        &|_, _| true,
    );
    let muxed = rig(&sources, Vec::new(), Vec::new(), &|_, _| true);
    let is_part_finding = |m: &str| m.to_ascii_lowercase().contains("part");
    let muxed_part: std::collections::BTreeSet<String> = validator_must_findings(&muxed, 0)
        .await
        .into_iter()
        .map(|(_, m)| m)
        .filter(|m| is_part_finding(m))
        .collect();
    let new_part: Vec<_> = validator_must_findings(&split, 3)
        .await
        .into_iter()
        .filter(|(_, m)| is_part_finding(m) && !muxed_part.contains(m))
        .collect();
    assert!(
        new_part.is_empty(),
        "level-1 part findings split audio adds over the muxed route: {new_part:?}"
    );
}

const NON_SPLIT_PARTS: usize = 18;
const NON_SPLIT_SEGMENTS: usize = 3;
const NON_SPLIT_DIGEST: u32 = 2_536_544_045;

#[tokio::test]
async fn non_split_route_output_is_unchanged() {
    let sources = by_role(load("ts/h264_aac.ts"));
    let rig = rig(&sources, Vec::new(), Vec::new(), &|_, _| true);
    let pl = rig.playlist("media.m3u8").await;
    let mut bytes = rig.ok(&pl.segments[0].map.as_ref().expect("map").uri).await;
    let mut parts = 0;
    for seg in &pl.segments {
        for part in &seg.parts {
            bytes.extend(rig.ok(&part.uri).await);
            parts += 1;
        }
        bytes.extend(rig.ok(&seg.uri).await);
    }
    assert_eq!(
        (
            parts,
            pl.segments.len(),
            broadcast_common::crc32_mpeg2::compute(&bytes)
        ),
        (NON_SPLIT_PARTS, NON_SPLIT_SEGMENTS, NON_SPLIT_DIGEST)
    );
}
