//! What a live viewer gets when the source timeline breaks (issue #4154).
//!
//! Every test drives the real production path — demuxed fixture samples into a
//! `Trunk`, a `ProgramSegmenter` pumping them through `LlHlsSegmenter` — and
//! reads the outputs over the real axum origin, exactly as a player would.
//!
//! The input is one committed capture played twice: the second run keeps its
//! own timestamps, so the seam is a real backward jump of the source clock
//! (the shape a live source makes when it reconnects or splices).

use crate::support::origin::get;
use axum::Router;
use axum::http::StatusCode;
use media_plane::ProgramId;
use media_plane::segmenter::ProgramSegmenter;
use media_plane::trunk::{RetentionClass, Trunk, TrunkConfig};
use multimux::origin::{AppState, router};
use multimux::output::Output;
use multimux::output::dash::DashOutput;
use multimux::output::hls::HlsOutput;
use multimux::output::ll_dash::LlDashOutput;
use multimux::output::smooth::SmoothOutput;
use multimux::route::{RouteConfig, RouteHandle};
use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::sync::Arc;
use std::time::Duration;
use tower::ServiceExt;
use transmux::Sample;
use transmux::pipeline::TrackSpec;

const STREAM: &str = "cam";
const TARGET_DURATION: Duration = Duration::from_secs(1);
const PART_TARGET: Duration = Duration::from_millis(300);
const WINDOW_SEGMENTS: usize = 16;
const RING: usize = 1 << 16;

fn nz(value: usize) -> NonZeroUsize {
    NonZeroUsize::new(value).expect("a non-zero test bound")
}

/// The committed capture, demuxed once, video track only.
fn source_video() -> (TrackSpec, Vec<Sample>) {
    let bytes = test_fixtures::read("ts/h264-aac.ts");
    let media = test_harness::collect::media_from_ts(&bytes).expect("demux the capture");
    let track = media
        .tracks
        .into_iter()
        .find(|track| track.spec.config.is_video())
        .expect("the capture has a video track");
    (track.spec, track.samples)
}

struct Rig {
    app: Router,
    trunk: Arc<Trunk>,
    writer: media_plane::trunk::TrunkWriter,
    route: Arc<RouteHandle>,
    segmenter: ProgramSegmenter,
    spec: TrackSpec,
    second: Vec<Sample>,
}

/// Build a route whose program gets the capture twice with a backward clock
/// jump at the seam, all pumped through the production segmenter.
fn rig(container: transmux::Container, window: usize, outputs: Vec<Arc<dyn Output>>) -> Rig {
    let (spec, samples) = source_video();
    rig_from(spec, samples, container, window, outputs)
}

/// [`rig`] over any source; the source is published twice, and the second run
/// keeps its own (earlier) timestamps, so the seam is a backward clock jump.
fn rig_from(
    spec: TrackSpec,
    samples: Vec<Sample>,
    container: transmux::Container,
    window: usize,
    outputs: Vec<Arc<dyn Output>>,
) -> Rig {
    let trunk = Trunk::new(
        TrunkConfig::default()
            .with_timed_capacity(nz(RING))
            .with_sparse_capacity(nz(64))
            .with_segment_capacity(nz(64))
            .with_event_capacity(nz(16))
            .with_part_capacity(nz(RING)),
    );
    let writer = trunk.writer().expect("trunk writer");
    writer.set_tracks(vec![spec.clone()]);
    let route = Arc::new(
        RouteHandle::new(
            RouteConfig::default()
                .with_target_duration(TARGET_DURATION)
                .with_part_target(PART_TARGET)
                .with_window_segments(window),
        )
        .with_hold_cold_playlist(false)
        .with_container(container),
    );
    route.publish_program(ProgramId(0), Arc::clone(&trunk));
    route.set_track_specs(ProgramId(0), vec![spec.clone()]);
    let (segmenter, _events) = ProgramSegmenter::try_new(&trunk, route.segmenter_config());
    let segmenter = segmenter.expect("a segmentable track set");
    if let Some(init) = segmenter.init_segment() {
        route.set_init(ProgramId(0), init);
    }
    let mut streams = HashMap::new();
    streams.insert(STREAM.to_string(), (Arc::clone(&route), outputs));
    Rig {
        app: router(Arc::new(AppState::new(streams))),
        trunk,
        writer,
        route,
        segmenter,
        spec,
        second: samples,
    }
}

impl Rig {
    /// Publish the capture once, the break, then the same capture again on its
    /// own (earlier) clock, pumping the segmenter as a live ingest would.
    fn run(&mut self) {
        let first = self.second.clone();
        let second = self.second.clone();
        let publish = |rig: &mut Rig, sample: &Sample| {
            rig.writer
                .publish(rig.spec.track_id, RetentionClass::Timed, sample.clone());
            multimux::source::segment::pump_segmenter(
                &mut rig.segmenter,
                ProgramId(0),
                &rig.trunk,
                &rig.route,
            );
        };
        for sample in &first {
            publish(self, sample);
        }
        for sample in &second {
            publish(self, sample);
        }
        self.segmenter.flush();
        multimux::source::segment::pump_segmenter(
            &mut self.segmenter,
            ProgramId(0),
            &self.trunk,
            &self.route,
        );
    }

    async fn get(&self, path: &str) -> (StatusCode, String) {
        let (status, body) = self.get_bytes(path).await;
        (status, String::from_utf8(body).expect("a text response"))
    }

    async fn get_bytes(&self, path: &str) -> (StatusCode, Vec<u8>) {
        let resp = self
            .app
            .clone()
            .oneshot(get(&format!("/{STREAM}/{path}")))
            .await
            .expect("the origin answers");
        let status = resp.status();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .expect("the response body");
        (status, body.to_vec())
    }

    async fn manifest(&self, path: &str) -> String {
        let (status, body) = self.get(path).await;
        assert_eq!(status, StatusCode::OK, "GET {path}:\n{body}");
        body
    }
}

#[tokio::test]
async fn live_ll_hls_marks_the_segment_after_the_break() {
    let mut rig = rig(
        transmux::Container::Fmp4,
        WINDOW_SEGMENTS,
        vec![Arc::new(HlsOutput::default()) as Arc<dyn Output>],
    );
    rig.run();
    let playlist = rig.manifest("media.m3u8").await;
    assert!(
        playlist.contains("#EXT-X-DISCONTINUITY\n"),
        "a live source break must reach the LL-HLS playlist:\n{playlist}"
    );
}

#[tokio::test]
async fn live_dash_opens_a_period_at_the_break() {
    let mut rig = rig(
        transmux::Container::Fmp4,
        WINDOW_SEGMENTS,
        vec![Arc::new(DashOutput) as Arc<dyn Output>],
    );
    rig.run();
    let mpd = rig.manifest("manifest.mpd").await;
    assert_periods("manifest.mpd", &mpd);
}

#[tokio::test]
async fn live_ll_dash_opens_a_period_at_the_break() {
    let mut rig = rig(
        transmux::Container::Fmp4,
        WINDOW_SEGMENTS,
        vec![Arc::new(LlDashOutput) as Arc<dyn Output>],
    );
    rig.run();
    let mpd = rig.manifest("manifest-ll.mpd").await;
    assert_periods("manifest-ll.mpd", &mpd);
}

/// The served MPD has one Period per side of the break, passes the ISO schema,
/// and its period timeline is continuous: each Period starts where the last
/// one ended, and the later Period maps its continuing media clock with its
/// own `presentationTimeOffset`.
fn assert_periods(name: &str, xml: &str) {
    if let Some(schema) = test_fixtures::process::xsd::dash_mpd_schema(name) {
        test_fixtures::process::xsd::validate(&schema, name, xml)
            .unwrap_or_else(|error| panic!("xmllint rejected {name}:\n{error}"));
    }
    let mpd = broadcast_dash::Mpd::parse(xml.as_bytes()).expect("the MPD parses");
    assert!(
        mpd.periods.len() >= 2,
        "{name}: the break must open a new Period, got {}:\n{xml}",
        mpd.periods.len()
    );
    let mut expected_start = std::time::Duration::ZERO;
    let mut offsets = Vec::new();
    for (index, period) in mpd.periods.iter().enumerate() {
        assert_eq!(
            period.start,
            Some(expected_start),
            "{name}: period {index} starts where the previous one closed:\n{xml}"
        );
        match period.duration {
            Some(duration) => expected_start += duration,
            None => assert_eq!(
                index,
                mpd.periods.len() - 1,
                "{name}: only the open last Period may omit @duration:\n{xml}"
            ),
        }
        let mut period_offsets = Vec::new();
        for set in &period.adaptation_sets {
            for representation in &set.representations {
                let template = representation
                    .segment
                    .template
                    .as_ref()
                    .expect("every Representation has a SegmentTemplate");
                period_offsets.push(template.multi.base.presentation_time_offset.unwrap_or(0));
            }
        }
        offsets.push(period_offsets);
    }
    assert!(
        offsets[0].iter().all(|offset| *offset == 0),
        "{name}: the first Period starts at media zero: {offsets:?}"
    );
    assert!(
        offsets[1..]
            .iter()
            .all(|period| period.iter().all(|offset| *offset > 0)),
        "{name}: later Periods map their continuing media clock: {offsets:?}"
    );
}

#[tokio::test]
async fn live_ll_hls_delta_and_rolled_off_break_keep_the_sequence_right() {
    let mut rig = rig(
        transmux::Container::Fmp4,
        2,
        vec![Arc::new(HlsOutput::default()) as Arc<dyn Output>],
    );
    rig.run();
    let full = rig.manifest("media.m3u8").await;
    let playlist = broadcast_hls::MediaPlaylist::parse(&full).expect("the playlist parses");
    assert!(
        playlist.discontinuity_sequence >= 1,
        "the break has rolled off the two-segment window and must be counted in \
         EXT-X-DISCONTINUITY-SEQUENCE:\n{full}"
    );
    let delta = rig.manifest("media.m3u8?_HLS_skip=YES").await;
    let delta = broadcast_hls::MediaPlaylist::parse(&delta).expect("the delta playlist parses");
    assert!(
        delta.discontinuity_sequence >= 1,
        "a delta playlist must keep the discontinuity sequence:\n{delta:?}"
    );
}

#[tokio::test]
async fn live_ts_hls_marks_the_segment_after_the_break() {
    let mut rig = rig(
        transmux::Container::MpegTs,
        WINDOW_SEGMENTS,
        vec![Arc::new(HlsOutput::default()) as Arc<dyn Output>],
    );
    rig.run();
    let playlist = rig.manifest("media.m3u8").await;
    assert!(
        playlist.contains("#EXT-X-DISCONTINUITY\n"),
        "a classic-TS route must state the break too:\n{playlist}"
    );
}

#[tokio::test]
async fn live_smooth_keeps_a_continuous_chunk_timeline() {
    let mut rig = rig(
        transmux::Container::Fmp4,
        WINDOW_SEGMENTS,
        vec![Arc::new(SmoothOutput) as Arc<dyn Output>],
    );
    rig.run();
    let manifest = rig.manifest("Manifest").await;
    let parsed = broadcast_smooth::SmoothManifest::parse(&manifest).expect("the Manifest parses");
    let mut starts = Vec::new();
    for stream in &parsed.streams {
        for (time, _duration) in stream.enumerate_chunks().expect("Smooth chunk timeline") {
            starts.push(time);
        }
    }
    starts.sort_unstable();
    for pair in starts.windows(2) {
        assert!(
            pair[1] >= pair[0],
            "Smooth chunk start times must not step backwards: {starts:?}"
        );
    }
}

/// Apple's own validator on the served LL-HLS origin, break included. Skips
/// loudly where `mediastreamvalidator` is not installed (macOS only).
#[tokio::test]
async fn live_ll_hls_is_clean_under_apple_validator() {
    use std::fs;

    if !test_fixtures::process::msv::available() {
        eprintln!("SKIP live_ll_hls_is_clean_under_apple_validator: no mediastreamvalidator");
        return;
    }
    // Exact-duration synthetic access units: the validator compares each
    // `#EXT-X-PART:DURATION` with the fragment's own `trun` sum, and a real
    // variable-rate capture carries unrelated mismatches that would drown the
    // one thing this test is about.
    let spec = crate::support::segmenter_feed::video_track_spec_sized(320, 240);
    let samples: Vec<Sample> = crate::support::segmenter_feed::video_batches_n(105)
        .into_iter()
        .flatten()
        .map(|(_, sample)| sample)
        .collect();
    let mut rig = rig_from(
        spec,
        samples,
        transmux::Container::Fmp4,
        WINDOW_SEGMENTS,
        vec![Arc::new(HlsOutput::default()) as Arc<dyn Output>],
    );
    rig.run();
    let scratch = test_fixtures::process::scratch_dir("live-timeline-break");
    for name in ["master.m3u8", "media.m3u8", "iframe.m3u8"] {
        let (status, body) = rig.get(name).await;
        assert_eq!(status, StatusCode::OK, "GET {name}");
        fs::write(scratch.join(name), body).expect("write playlist");
    }
    let mut uris = std::collections::HashSet::new();
    for name in ["media.m3u8", "iframe.m3u8"] {
        let text = fs::read_to_string(scratch.join(name)).expect("read playlist");
        for line in text.lines() {
            if let Some(start) = line.find("URI=\"") {
                let rest = &line[start + 5..];
                if let Some(end) = rest.find('"') {
                    uris.insert(rest[..end].to_string());
                }
            }
            let trimmed = line.trim();
            if !trimmed.is_empty() && !trimmed.starts_with('#') {
                uris.insert(trimmed.to_string());
            }
        }
    }
    for uri in &uris {
        let (status, body) = rig.get_bytes(uri).await;
        if status == StatusCode::OK {
            fs::write(scratch.join(uri), body).expect("write resource");
        }
    }
    let report =
        test_fixtures::process::msv::run_report(&scratch, "master.m3u8", Duration::from_secs(60));
    let must_fix: Vec<_> = test_fixtures::process::msv::findings(&report)
        .into_iter()
        .filter(|(_, level, _)| *level == 1)
        .collect();
    let variants = report["variants"].as_array().cloned().unwrap_or_default();
    let parsed: i64 = variants
        .iter()
        .filter_map(|variant| variant["parsedSegmentsCount"].as_i64())
        .sum();
    assert!(
        parsed >= 1,
        "the validator parsed no segments (must-fix: {must_fix:?})"
    );
    let domains = variants
        .iter()
        .filter_map(|variant| variant["discontinuities"].as_array().map(Vec::len))
        .max()
        .unwrap_or(0);
    assert!(
        domains >= 2,
        "the validator did not see the break as a discontinuity domain (known findings: {must_fix:?})"
    );
    let about_the_break: Vec<_> = must_fix
        .iter()
        .filter(|(_, _, comment)| comment.to_ascii_lowercase().contains("discontinu"))
        .collect();
    assert!(
        about_the_break.is_empty(),
        "mediastreamvalidator rejected the discontinuity: {about_the_break:?}"
    );
}
