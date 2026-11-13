//! The explicit gap state (#2443: `#EXT-X-GAP` while the source has no
//! muxable media, `#EXT-X-DISCONTINUITY` on resume) on a real TS-HLS playlist,
//! validated by Apple's `mediastreamvalidator` (issue #2442).
//!
//! Real media: the committed `fixtures/ts/h264-aac.ts` capture (real H.264 +
//! AAC) is demuxed, and its samples are published into a `Trunk` on the
//! production path `split_audio`/`closed_captions` drive (`RouteHandle` +
//! `ProgramSegmenter::pump`, read back through the axum router). The source
//! then drops every track for a stretch (the gap) and resumes.
//!
//! The validator is Apple's own tool, independent of both our TS-HLS renderer
//! and `media-doctor`'s checker, so it catches a misreading of RFC 8216bis
//! §4.4.4.7 shared by both. It skips **loudly** when the binary is absent
//! (Linux/CI), so run it on macOS for the genuine check.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::Router;
use axum::http::StatusCode;
use broadcast_common::mux::Unpackage;
use test_fixtures::process::msv;
use tower::ServiceExt;

use media_plane::ProgramId;
use media_plane::segmenter::ProgramSegmenter;
use media_plane::trunk::{RetentionClass, Trunk, TrunkConfig, TrunkWriter};
use multimux::origin::{AppState, router};
use multimux::output::Output;
use multimux::output::hls::HlsOutput;
use multimux::route::RouteHandle;
use transmux::pipeline::Sample;
use transmux::ts_demux::TsDemux;

use crate::support::origin::{body_bytes, body_string, get};
use crate::support::shared_it::nz;

const FIXTURE: &str = "ts/h264-aac.ts";

const TARGET_DURATION_SECS: f64 = 1.0;
const PART_TARGET_MS: u32 = 200;
const WINDOW_SEGMENTS: usize = 16;
const RING: usize = 1 << 16;
const SIDE_RINGS: usize = 64;
const PUMP_EVERY: usize = 64;
const TICKS_PER_SEC: i64 = 1_000_000;

const SEGMENTS_BEFORE_GAP: usize = 4;
const GAP_PUMPS: usize = 2;
const SEGMENTS_AFTER_RESUME: usize = 4;

const STREAM: &str = "cam";
const MEDIA: &str = "media.m3u8";

const VALIDATOR_DEADLINE: Duration = Duration::from_secs(35);
const ANSWER_DEADLINE: Duration = Duration::from_secs(20);

const GAP_TAG: &str = "#EXT-X-GAP";
const DISCONTINUITY_TAG: &str = "#EXT-X-DISCONTINUITY";
const MUST_LEVEL: i64 = 1;

/// The scratch directory `scratch_dir` created, removed when the test ends.
struct Scratch(std::path::PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

type Track = (transmux::TrackSpec, Vec<Sample>);

struct Rig {
    route: Arc<RouteHandle>,
    trunk: Arc<Trunk>,
    writer: TrunkWriter,
    segmenter: ProgramSegmenter,
    app: Router,
    tracks: Vec<Track>,
}

impl Rig {
    fn build() -> Option<Self> {
        let bytes = test_fixtures::require_bytes(FIXTURE, "ts_hls_gap_oracle")?;
        let media = TsDemux::new().unpackage(&bytes).expect("demux fixture");
        let tracks: Vec<Track> = media
            .tracks
            .into_iter()
            .filter(|t| t.spec.config.is_muxable_in_bmff())
            .map(|t| {
                let samples = t
                    .samples
                    .into_iter()
                    .map(|mut s| {
                        s.dts = s.dts.or(s.pts);
                        s
                    })
                    .collect();
                (t.spec, samples)
            })
            .collect();
        assert!(!tracks.is_empty(), "fixture carries muxable media");

        let trunk = Trunk::new(
            TrunkConfig::default()
                .with_timed_capacity(nz(RING))
                .with_sparse_capacity(nz(SIDE_RINGS))
                .with_segment_capacity(nz(WINDOW_SEGMENTS))
                .with_event_capacity(nz(SIDE_RINGS))
                .with_part_capacity(nz(RING)),
        );
        let writer = trunk.writer().expect("trunk writer");
        writer.set_tracks(tracks.iter().map(|(spec, _)| spec.clone()).collect());
        let route = Arc::new(
            RouteHandle::new(TARGET_DURATION_SECS, PART_TARGET_MS, WINDOW_SEGMENTS)
                .with_container(transmux::Container::MpegTs),
        );
        route.publish_program(ProgramId(0), Arc::clone(&trunk));
        let segmenter = ProgramSegmenter::try_new_with_siblings(
            &trunk,
            route.segmenter_config(),
            route.as_ref(),
        )
        .expect("TS-HLS segmenter builds");
        route.set_track_specs(
            ProgramId(0),
            tracks.iter().map(|(spec, _)| spec.clone()).collect(),
        );

        let mut streams = HashMap::new();
        streams.insert(
            STREAM.to_string(),
            (
                Arc::clone(&route),
                vec![Arc::new(HlsOutput::segmented(
                    multimux::output::hls::DEFAULT_PLAYLIST_NAME,
                )) as Arc<dyn Output>],
            ),
        );
        let app = router(Arc::new(AppState::new(streams)));
        Some(Rig {
            route,
            trunk,
            writer,
            segmenter,
            app,
            tracks,
        })
    }

    fn pump(&mut self) {
        multimux::source::segment::pump_segmenter(
            &mut self.segmenter,
            ProgramId(0),
            &self.trunk,
            &self.route,
        );
    }

    /// Publishes every sample of the fixture once more, merging the tracks in
    /// decode order, and pumps the segmenter the way the production drive
    /// loop does.
    fn feed(&mut self) {
        let mut order: Vec<(i64, usize, usize)> = Vec::new();
        for (ti, (spec, samples)) in self.tracks.iter().enumerate() {
            for (si, sample) in samples.iter().enumerate() {
                let at = sample.dts.unwrap_or(0) * TICKS_PER_SEC / i64::from(spec.timescale);
                order.push((at, ti, si));
            }
        }
        order.sort_by_key(|(at, ..)| *at);
        for (n, (_, ti, si)) in order.into_iter().enumerate() {
            let (spec, samples) = &self.tracks[ti];
            self.writer
                .publish(spec.track_id, RetentionClass::Timed, samples[si].clone());
            if n.is_multiple_of(PUMP_EVERY) {
                self.pump();
            }
        }
        self.pump();
    }

    fn closed_segments(&self) -> usize {
        self.trunk.segment_len()
    }

    fn feed_until_segments(&mut self, wanted: usize) {
        let started = Instant::now();
        while self.closed_segments() < wanted {
            self.feed();
            assert!(
                started.elapsed() < ANSWER_DEADLINE,
                "only {} of {wanted} segments closed before the answer deadline",
                self.closed_segments()
            );
        }
    }

    async fn playlist(&self) -> String {
        let resp = self
            .app
            .clone()
            .oneshot(get(&format!("/{STREAM}/{MEDIA}")))
            .await
            .expect("response");
        assert_eq!(resp.status(), StatusCode::OK, "GET {MEDIA}");
        body_string(resp).await
    }

    async fn fetch_bytes(&self, uri: &str) -> Option<Vec<u8>> {
        let resp = self
            .app
            .clone()
            .oneshot(get(&format!("/{STREAM}/{uri}")))
            .await
            .expect("response");
        if resp.status() != StatusCode::OK {
            return None;
        }
        Some(body_bytes(resp).await)
    }
}

/// One media-playlist stanza per segment: the tags preceding a segment URI,
/// ending at that URI.
fn stanzas(playlist: &str) -> Vec<Vec<String>> {
    let mut out: Vec<Vec<String>> = Vec::new();
    let mut current: Vec<String> = Vec::new();
    for line in playlist.lines().map(str::trim) {
        if line.is_empty() {
            continue;
        }
        current.push(line.to_owned());
        if !line.starts_with('#') {
            out.push(std::mem::take(&mut current));
        }
    }
    out
}

fn has_tag(stanza: &[String], tag: &str) -> bool {
    stanza.iter().any(|l| l == tag)
}

fn gap_stanzas(playlist: &str) -> Vec<Vec<String>> {
    stanzas(playlist)
        .into_iter()
        .filter(|s| has_tag(s, GAP_TAG))
        .collect()
}

fn segment_uris(playlist: &str) -> Vec<String> {
    stanzas(playlist)
        .into_iter()
        .filter_map(|s| s.last().cloned())
        .collect()
}

fn scratch_dir(tag: &str) -> Scratch {
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../target")
        .join(format!("ts-hls-gap-{tag}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    Scratch(dir)
}

#[tokio::test]
async fn ts_hls_gap_playlist_passes_mediastreamvalidator() {
    if !msv::available() {
        eprintln!(
            "SKIP ts_hls_gap_playlist_passes_mediastreamvalidator: `mediastreamvalidator` not on \
             PATH (Apple Additional Tools for Xcode; macOS only)"
        );
        return;
    }
    let Some(mut rig) = Rig::build() else {
        return;
    };
    let specs: Vec<transmux::TrackSpec> = rig.tracks.iter().map(|(spec, _)| spec.clone()).collect();

    rig.feed_until_segments(SEGMENTS_BEFORE_GAP);

    rig.writer.set_tracks(Vec::new());
    rig.pump();
    assert!(
        !rig.route.has_active_tracks(ProgramId(0)),
        "dropping every muxable track must leave the program with no active tracks"
    );
    for _ in 0..GAP_PUMPS {
        rig.pump();
    }
    let gap_playlist = rig.playlist().await;
    assert!(
        !gap_stanzas(&gap_playlist).is_empty(),
        "the gap placeholder must be served while the source has no muxable media:\n{gap_playlist}"
    );
    let gap_uris: Vec<String> = gap_stanzas(&gap_playlist)
        .into_iter()
        .filter_map(|s| s.last().cloned())
        .collect();

    rig.writer.set_tracks(specs.clone());
    rig.pump();
    assert!(
        rig.route.has_active_tracks(ProgramId(0)),
        "re-announcing a track set must mark the program active again"
    );
    rig.feed_until_segments(rig.closed_segments() + SEGMENTS_AFTER_RESUME);

    let playlist = rig.playlist().await;
    let stanzas = stanzas(&playlist);
    assert!(
        stanzas.iter().any(|s| has_tag(s, GAP_TAG)),
        "the served playlist must still carry the gap stanza:\n{playlist}"
    );
    assert!(
        stanzas
            .iter()
            .any(|s| has_tag(s, DISCONTINUITY_TAG) && !has_tag(s, GAP_TAG)),
        "the first segment after the gap must carry {DISCONTINUITY_TAG}:\n{playlist}"
    );

    let scratch = scratch_dir("validator");
    let dir = scratch.0.as_path();
    std::fs::write(dir.join(MEDIA), playlist.as_bytes()).expect("write media playlist");
    for uri in segment_uris(&playlist) {
        let Some(bytes) = rig.fetch_bytes(&uri).await else {
            assert!(
                gap_uris.contains(&uri),
                "the origin serves no bytes for {uri}, which is not a gap segment"
            );
            continue;
        };
        std::fs::write(dir.join(&uri), bytes).expect("write segment");
    }

    let report = msv::run_report(dir, MEDIA, VALIDATOR_DEADLINE);
    let findings = msv::findings(&report);
    for (path, level, comment) in &findings {
        eprintln!("VALIDATOR: {path} level={level} {comment}");
    }
    let errors: Vec<_> = findings
        .iter()
        .filter(|(_, level, _)| *level == MUST_LEVEL)
        .collect();
    assert!(
        !report
            .get("parseFailed")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
        "the empty body of {FIXTURE} would let a parse failure pass as clean: {report}"
    );
    assert!(
        report
            .get("processedSegmentsCount")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0)
            > 0,
        "the validator processed no segment: {report}"
    );
    assert!(
        errors.is_empty(),
        "mediastreamvalidator reported MUST-level findings on the gap playlist: {errors:?}\n\
         {playlist}"
    );
}
