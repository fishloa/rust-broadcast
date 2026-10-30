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
fn publish_interleaved(
    trunk: &Arc<Trunk>,
    writer: &media_plane::trunk::TrunkWriter,
    route: &RouteHandle,
    segmenter: &mut ProgramSegmenter,
    sources: &[&Source],
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
            let lead = if src.is_video() { VIDEO_LEAD_SECS } else { 0.0 };
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
    segmenter.flush();
}

struct Rig {
    app: Router,
}

fn rig(
    sources: &[Source],
    renditions: Vec<AudioRenditionDecl>,
    selectors: Vec<AudioTrackSelector>,
    keep: &dyn Fn(usize, &Sample) -> bool,
) -> Rig {
    let trunk = new_trunk();
    let writer = trunk.writer().expect("trunk writer");
    writer.set_tracks(sources.iter().map(|s| s.spec.clone()).collect());
    let route = Arc::new(
        RouteHandle::new(TARGET_DURATION_SECS, PART_TARGET_MS, WINDOW_SEGMENTS)
            .with_declared_bandwidth_bps(8_000_000)
            .with_audio_renditions(renditions)
            .with_audio_rendition_sources(selectors),
    );
    route.publish_program(ProgramId(0), Arc::clone(&trunk));
    let mut segmenter =
        ProgramSegmenter::try_new(&trunk, &route, TARGET_DURATION_SECS, PART_TARGET_MS)
            .expect("segmenter");
    if let Some(init) = segmenter.init_segment() {
        route.set_init(ProgramId(0), init);
    }
    let refs: Vec<&Source> = sources.iter().collect();
    publish_interleaved(&trunk, &writer, &route, &mut segmenter, &refs, keep);
    Rig {
        app: app_for(route),
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
    let _ = fs::remove_dir_all(&scratch);

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
/// each knowingly left open:
/// - "Missing rendition report": the video playlist carries no
///   `EXT-X-RENDITION-REPORT` for the audio renditions (RFC 8216bis §4.4.5.4);
///   sibling-origin rendition reports are the follow-up slice of #1428.
const KNOWN_SPLIT_ONLY: &[&str] = &["Missing rendition report"];

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
        decls(&["fre", "qad", "qaa"]),
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
        unexpected.is_empty(),
        "level-1 findings split audio introduces over the muxed route: {unexpected:?}"
    );
}
