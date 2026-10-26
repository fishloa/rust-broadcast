//! `dash_pull` against real fMP4 and real servers: the static fixtures of
//! `fixtures/dash/`, a simulated live origin that serves them under a dynamic
//! MPD with availability enforced against the true clock, and the drive loop's
//! wire rules (byte ranges, conditional refreshes, credentials, backoff).
//!
//! Every pull here runs the production loop (`drive::run`) over a real loopback
//! socket; only the clock is injected (`SkewedClock`) and the route registry
//! is replaced by a sink that reads the `Trunk` the way the segmenter does.

use std::future::Future;
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::Router;
use axum::extract::{Path as AxumPath, State};
use axum::http::{HeaderMap, Method, StatusCode as AxumStatus, header};
use axum::response::{IntoResponse, Response as AxumResponse};
use axum::routing::get;
use broadcast_common::{Timestamp, Unpackage};
use headers::HeaderMapExt;
use media_plane::ingress::{HandshakePolicy, IngestDriver};
use media_plane::trunk::{SampleCursor, SampleCursorItem, TrunkConfig};
use transmux::media::Fmp4Demux;
use transmux::{CodecConfig, Sample, TrackId, TrackSpec};

use super::drive::{self, RouteSink};
use super::session::PROGRAM;
use super::{DashIngestSession, DashPullRoute};
use crate::clock::{HostClock, SystemHostClock};
use crate::error::{MultimuxError, Result};
use crate::source::IngestTimeouts;

const CEILING: Duration = Duration::from_secs(60);
const POLL: Duration = Duration::from_millis(20);

fn nz(n: usize) -> NonZeroUsize {
    NonZeroUsize::new(n).expect("test capacity must be non-zero")
}

fn trunk_config() -> TrunkConfig {
    TrunkConfig::new(nz(4096), nz(16), nz(8), nz(8), nz(8))
}

fn handshake() -> HandshakePolicy {
    HandshakePolicy::establish_by(Timestamp::from_nanos(u64::MAX))
}

fn fixture(rel: &str) -> PathBuf {
    test_fixtures::path(&format!("dash/{rel}"))
}

fn now_unix_us() -> i64 {
    i64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after the epoch")
            .as_micros(),
    )
    .expect("fits")
}

fn xs_datetime(unix_us: i64) -> String {
    jiff::Timestamp::from_microsecond(unix_us)
        .expect("in range")
        .to_string()
}

/// The operating system's monotonic clock and a wall clock that is wrong by
/// `skew_us`.
struct SkewedClock {
    inner: SystemHostClock,
    skew_us: i64,
}

impl SkewedClock {
    fn new(skew_us: i64) -> Arc<Self> {
        Arc::new(Self {
            inner: SystemHostClock::new(),
            skew_us,
        })
    }
}

impl HostClock for SkewedClock {
    fn mono_us(&self) -> u64 {
        self.inner.mono_us()
    }

    fn unix_us(&self) -> u64 {
        u64::try_from(now_unix_us().saturating_add(self.skew_us)).unwrap_or(0)
    }
}

// ---------------------------------------------------------------------------
// What a pull delivered
// ---------------------------------------------------------------------------

#[derive(Default)]
struct Delivered {
    specs: Vec<TrackSpec>,
    samples: Vec<(TrackId, Sample)>,
    lagged: u64,
}

impl Delivered {
    fn of_kind(&self, video: bool) -> Vec<Sample> {
        let ids: Vec<TrackId> = self
            .specs
            .iter()
            .filter(|s| matches!(s.config, CodecConfig::Avc { .. }) == video)
            .map(|s| s.track_id)
            .collect();
        self.samples
            .iter()
            .filter(|(id, _)| ids.contains(id))
            .map(|(_, s)| s.clone())
            .collect()
    }

    fn video(&self) -> Vec<Sample> {
        self.of_kind(true)
    }

    fn audio(&self) -> Vec<Sample> {
        self.of_kind(false)
    }
}

struct CollectSink {
    delivered: Arc<Mutex<Delivered>>,
    cursor: Option<SampleCursor>,
}

impl RouteSink for CollectSink {
    fn advance<'a>(
        &'a mut self,
        driver: &'a IngestDriver<DashIngestSession>,
    ) -> impl Future<Output = ()> + Send + 'a {
        if let Some(trunk) = driver.trunk(PROGRAM) {
            let cursor = self
                .cursor
                .get_or_insert_with(|| trunk.subscribe_from_backlog());
            let mut delivered = self.delivered.lock().expect("lock");
            delivered.specs = trunk.tracks().to_vec();
            while let Some(item) = cursor.poll() {
                match item {
                    SampleCursorItem::Timed { track_id, sample } => {
                        delivered.samples.push((track_id, sample));
                    }
                    SampleCursorItem::Lagged { skipped } => delivered.lagged += skipped,
                    _ => {}
                }
            }
        }
        std::future::ready(())
    }

    fn finish(&mut self, result: Result<()>) -> Result<()> {
        result
    }
}

struct Pull {
    delivered: Arc<Mutex<Delivered>>,
    task: tokio::task::JoinHandle<Result<()>>,
}

impl Pull {
    fn start(route: DashPullRoute, clock: Arc<dyn HostClock>) -> Self {
        let delivered = Arc::new(Mutex::new(Delivered::default()));
        let sink = CollectSink {
            delivered: Arc::clone(&delivered),
            cursor: None,
        };
        let task = tokio::spawn(async move {
            drive::run(&route, trunk_config(), handshake(), clock, sink).await
        });
        Self { delivered, task }
    }

    fn now(&self) -> std::sync::MutexGuard<'_, Delivered> {
        self.delivered.lock().expect("lock")
    }

    /// Wait until `done` holds or the attempt ended; returns the attempt's
    /// result when it ended on its own.
    async fn until(&mut self, done: impl Fn(&Delivered) -> bool) -> Option<Result<()>> {
        let deadline = tokio::time::Instant::now() + CEILING;
        while tokio::time::Instant::now() < deadline {
            if done(&self.now()) {
                return None;
            }
            if self.task.is_finished() {
                return Some((&mut self.task).await.expect("pull task"));
            }
            tokio::time::sleep(POLL).await;
        }
        panic!("the pull neither delivered what the test waits for nor ended within the ceiling");
    }

    /// Run a static presentation to its end.
    async fn finish(mut self) -> (Delivered, Result<()>) {
        let result = tokio::time::timeout(CEILING, &mut self.task)
            .await
            .expect("a static presentation must end")
            .expect("pull task");
        let delivered = std::mem::take(&mut *self.now());
        (delivered, result)
    }

    fn stop(self) -> Delivered {
        self.task.abort();
        let mut guard = self.delivered.lock().expect("lock");
        std::mem::take(&mut *guard)
    }
}

// ---------------------------------------------------------------------------
// Oracles
// ---------------------------------------------------------------------------

/// The samples of `init ++ segment`, demuxed directly: the independent
/// expectation for what a segment must deliver.
fn demux_pair(init: &[u8], segment: &[u8]) -> Vec<Sample> {
    let mut whole = init.to_vec();
    whole.extend_from_slice(segment);
    Fmp4Demux::new()
        .unpackage(whole.as_slice())
        .expect("oracle demux")
        .tracks
        .into_iter()
        .flat_map(|t| t.samples)
        .collect()
}

fn expected(dir: &str, init: &str, segments: &[&str]) -> Vec<Sample> {
    let init = std::fs::read(fixture(&format!("{dir}{init}"))).expect("init fixture");
    segments
        .iter()
        .flat_map(|s| {
            let bytes = std::fs::read(fixture(&format!("{dir}{s}"))).expect("segment fixture");
            demux_pair(&init, &bytes)
        })
        .collect()
}

fn same(a: &Sample, b: &Sample) -> bool {
    a.data == b.data
        && a.dts == b.dts
        && a.pts == b.pts
        && a.duration == b.duration
        && a.flags == b.flags
}

fn assert_same(got: &[Sample], want: &[Sample], what: &str) {
    assert_eq!(got.len(), want.len(), "{what}: sample count");
    for (i, (g, w)) in got.iter().zip(want).enumerate() {
        assert!(same(g, w), "{what}: sample {i} differs from the oracle");
    }
}

// ---------------------------------------------------------------------------
// Static file origin
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
struct Hit {
    name: String,
    range: Option<String>,
    status: u16,
}

#[derive(Clone)]
struct FileState {
    dir: PathBuf,
    stall: Option<&'static str>,
    honour_range: bool,
    hits: Arc<Mutex<Vec<Hit>>>,
}

async fn file_handler(
    AxumPath(name): AxumPath<String>,
    State(state): State<FileState>,
    headers: HeaderMap,
) -> AxumResponse {
    if state.stall == Some(name.as_str()) {
        std::future::pending::<()>().await;
        unreachable!("pending future never resolves");
    }
    let range_text = headers
        .get(header::RANGE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let respond = |status: AxumStatus, response: AxumResponse| {
        state.hits.lock().expect("lock").push(Hit {
            name: name.clone(),
            range: range_text.clone(),
            status: status.as_u16(),
        });
        response
    };
    let Ok(bytes) = std::fs::read(state.dir.join(&name)) else {
        return respond(AxumStatus::NOT_FOUND, AxumStatus::NOT_FOUND.into_response());
    };
    if state.honour_range
        && let Some(range) = headers.typed_get::<headers::Range>()
        && let Some((start, end)) = range.satisfiable_ranges(bytes.len() as u64).next()
    {
        let first = match start {
            std::ops::Bound::Included(n) => n,
            std::ops::Bound::Excluded(n) => n + 1,
            std::ops::Bound::Unbounded => 0,
        };
        let last = match end {
            std::ops::Bound::Included(n) => n,
            std::ops::Bound::Excluded(n) => n - 1,
            std::ops::Bound::Unbounded => bytes.len() as u64 - 1,
        };
        let slice = bytes[first as usize..=last as usize].to_vec();
        let mut response = (AxumStatus::PARTIAL_CONTENT, slice).into_response();
        response.headers_mut().typed_insert(
            headers::ContentRange::bytes(first..=last, bytes.len() as u64).expect("valid range"),
        );
        return respond(AxumStatus::PARTIAL_CONTENT, response);
    }
    respond(AxumStatus::OK, bytes.into_response())
}

struct FileOrigin {
    base: String,
    hits: Arc<Mutex<Vec<Hit>>>,
    task: tokio::task::JoinHandle<()>,
}

impl FileOrigin {
    fn url(&self, name: &str) -> String {
        format!("{}/{name}", self.base)
    }

    fn hits(&self) -> Vec<Hit> {
        self.hits.lock().expect("lock").clone()
    }
}

impl Drop for FileOrigin {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn serve_files(
    dir: PathBuf,
    auth: Option<crate::testutil::MockAuthScheme>,
    stall: Option<&'static str>,
    honour_range: bool,
) -> FileOrigin {
    let hits = Arc::new(Mutex::new(Vec::new()));
    let mut app = Router::new()
        .route("/{name}", get(file_handler))
        .with_state(FileState {
            dir,
            stall,
            honour_range,
            hits: Arc::clone(&hits),
        });
    if let Some(scheme) = auth {
        app = crate::testutil::require_auth(app, scheme);
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral loopback port");
    let addr = listener.local_addr().expect("local addr");
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("axum server");
    });
    FileOrigin {
        base: format!("http://{addr}"),
        hits,
        task,
    }
}

fn route(url: String) -> DashPullRoute {
    DashPullRoute::new("dash-test", url)
}

// ---------------------------------------------------------------------------
// Static presentations: every addressing form, byte-exact against the oracle
// ---------------------------------------------------------------------------

const FFMPEG_VIDEO: [&str; 3] = [
    "chunk-stream0-00001.m4s",
    "chunk-stream0-00002.m4s",
    "chunk-stream0-00003.m4s",
];
const FFMPEG_AUDIO: [&str; 4] = [
    "chunk-stream1-00001.m4s",
    "chunk-stream1-00002.m4s",
    "chunk-stream1-00003.m4s",
    "chunk-stream1-00004.m4s",
];

/// The real ffmpeg presentation (SegmentTimeline, one AVC and one AAC
/// Representation): every sample of both tracks reaches the `Trunk` in order
/// and byte for byte, on unique track ids although both inits declare
/// `tkhd@track_ID` 1 (issue #738 gap 2).
#[tokio::test]
async fn a_static_timeline_presentation_reaches_the_trunk_byte_exact() {
    let origin = serve_files(fixture(""), None, None, false).await;
    let (delivered, result) = Pull::start(route(origin.url("manifest.mpd")), SkewedClock::new(0))
        .finish()
        .await;
    result.expect("a static presentation ends cleanly");

    assert_eq!(delivered.specs.len(), 2, "{:?}", delivered.specs);
    let mut ids: Vec<TrackId> = delivered.specs.iter().map(|s| s.track_id).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), 2, "track ids are unique");
    assert_eq!(delivered.lagged, 0);
    assert_same(
        &delivered.video(),
        &expected("", "init-stream0.m4s", &FFMPEG_VIDEO),
        "video",
    );
    assert_same(
        &delivered.audio(),
        &expected("", "init-stream1.m4s", &FFMPEG_AUDIO),
        "audio",
    );
}

/// `SegmentList`, a form the pre-core planner could not address at all.
#[tokio::test]
async fn a_segment_list_presentation_plays_byte_exact() {
    let origin = serve_files(fixture("segmentlist"), None, None, false).await;
    let (delivered, result) = Pull::start(route(origin.url("list.mpd")), SkewedClock::new(0))
        .finish()
        .await;
    result.expect("ends cleanly");
    let video: Vec<String> = (1..=6).map(|n| format!("seg-0-{n:03}.m4s")).collect();
    let audio: Vec<String> = (1..=6).map(|n| format!("seg-1-{n:03}.m4s")).collect();
    let video: Vec<&str> = video.iter().map(String::as_str).collect();
    let audio: Vec<&str> = audio.iter().map(String::as_str).collect();
    assert_same(
        &delivered.video(),
        &expected("segmentlist/", "init-0.m4s", &video),
        "video",
    );
    assert_same(
        &delivered.audio(),
        &expected("segmentlist/", "init-1.m4s", &audio),
        "audio",
    );
}

/// On-demand profile: one file per Representation, `SegmentBase` with a `sidx`,
/// every request a byte range. The delivered samples are those of the whole
/// files demuxed directly.
#[tokio::test]
async fn an_on_demand_sidx_presentation_plays_with_byte_range_requests() {
    let origin = serve_files(fixture("ondemand-sidx"), None, None, true).await;
    let (delivered, result) = Pull::start(route(origin.url("ondemand.mpd")), SkewedClock::new(0))
        .finish()
        .await;
    result.expect("ends cleanly");

    let whole = |name: &str| -> Vec<Sample> {
        let bytes = std::fs::read(fixture(&format!("ondemand-sidx/{name}"))).expect("fixture");
        Fmp4Demux::new()
            .unpackage(bytes.as_slice())
            .expect("oracle demux")
            .tracks
            .into_iter()
            .flat_map(|t| t.samples)
            .collect()
    };
    assert_same(&delivered.video(), &whole("v_dashinit.mp4"), "video");
    assert_same(&delivered.audio(), &whole("a_dashinit.mp4"), "audio");

    let hits = origin.hits();
    let ranged: Vec<&Hit> = hits
        .iter()
        .filter(|h| h.name.ends_with(".mp4") && h.range.is_some())
        .collect();
    assert!(
        ranged.len() >= 6 && ranged.iter().all(|h| h.status == 206),
        "init, sidx and subsegments are all byte-range requests: {hits:?}"
    );
}

/// A server that ignores `Range` and answers `200` with the whole file still
/// plays: the loop slices the part it asked for.
#[tokio::test]
async fn a_server_that_ignores_range_is_sliced_to_the_requested_part() {
    let origin = serve_files(fixture("ondemand-sidx"), None, None, false).await;
    let (delivered, result) = Pull::start(route(origin.url("ondemand.mpd")), SkewedClock::new(0))
        .finish()
        .await;
    result.expect("ends cleanly");
    let bytes = std::fs::read(fixture("ondemand-sidx/v_dashinit.mp4")).expect("fixture");
    let whole: Vec<Sample> = Fmp4Demux::new()
        .unpackage(bytes.as_slice())
        .expect("oracle demux")
        .tracks
        .into_iter()
        .flat_map(|t| t.samples)
        .collect();
    assert_same(&delivered.video(), &whole, "video");
    assert!(origin.hits().iter().all(|h| h.status == 200));
}

/// Two Periods whose media clock restarts at zero (GPAC `multiperiod`): both
/// play, every sample of both Periods arrives byte for byte, and the timeline
/// the `Trunk` carries moves forward across the boundary.
#[tokio::test]
async fn every_period_plays_and_the_timeline_continues_across_the_boundary() {
    let origin = serve_files(fixture("multiperiod-restart"), None, None, false).await;
    let (delivered, result) =
        Pull::start(route(origin.url("multiperiod.mpd")), SkewedClock::new(0))
            .finish()
            .await;
    result.expect("ends cleanly");

    let video = delivered.video();
    let audio = delivered.audio();
    let p1v = expected(
        "multiperiod-restart/",
        "p1_dash_track1_init.mp4",
        &[
            "p1_dash_track1_1.m4s",
            "p1_dash_track1_2.m4s",
            "p1_dash_track1_3.m4s",
        ],
    );
    let p2v = expected(
        "multiperiod-restart/",
        "p2_dash_track1_init.mp4",
        &[
            "p2_dash_track1_1.m4s",
            "p2_dash_track1_2.m4s",
            "p2_dash_track1_3.m4s",
        ],
    );
    let p1a = expected(
        "multiperiod-restart/",
        "p1_dash_track2_init.mp4",
        &[
            "p1_dash_track2_1.m4s",
            "p1_dash_track2_2.m4s",
            "p1_dash_track2_3.m4s",
        ],
    );
    let p2a = expected(
        "multiperiod-restart/",
        "p2_dash_track2_init.mp4",
        &[
            "p2_dash_track2_1.m4s",
            "p2_dash_track2_2.m4s",
            "p2_dash_track2_3.m4s",
        ],
    );
    assert_eq!(video.len(), p1v.len() + p2v.len(), "both Periods' video");
    assert_eq!(audio.len(), p1a.len() + p2a.len(), "both Periods' audio");
    for (got, want) in video.iter().zip(p1v.iter().chain(&p2v)) {
        assert_eq!(got.data, want.data, "video payload order");
    }
    for (got, want) in audio.iter().zip(p1a.iter().chain(&p2a)) {
        assert_eq!(got.data, want.data, "audio payload order");
    }

    for (name, samples, boundary) in [("video", &video, p1v.len()), ("audio", &audio, p1a.len())] {
        let dts: Vec<i64> = samples.iter().map(|s| s.dts.expect("dts")).collect();
        assert!(
            dts.windows(2).all(|w| w[0] < w[1]),
            "{name}: the stitched timeline is strictly increasing"
        );
        let last_of_first = &samples[boundary - 1];
        let end = last_of_first.dts.unwrap() + i64::from(last_of_first.duration.unwrap());
        let first_of_second = samples[boundary].dts.unwrap();
        assert!(
            first_of_second >= end,
            "{name}: the second Period starts after the first one ended ({first_of_second} < {end})"
        );
    }
    assert!(
        p2v[0].dts.unwrap() < p1v.last().unwrap().dts.unwrap(),
        "the source's second Period restarts its clock, so the stitch is what makes it monotonic"
    );
}

/// A video AdaptationSet with a low and a high bandwidth Representation: the
/// pull plays one of them, the highest the policy allows, not the first.
#[tokio::test]
async fn the_highest_bandwidth_representation_plays_not_the_first_listed() {
    let mpd = std::fs::read_to_string(fixture("manifest.mpd")).expect("fixture");
    let video_rep = |id: &str, bandwidth: u32, dir: &str| {
        format!(
            r#"<Representation id="{id}" mimeType="video/mp4" codecs="avc1.4d400d" bandwidth="{bandwidth}" width="320" height="240">
<SegmentTemplate timescale="90000" initialization="{dir}/init-stream0.m4s" media="{dir}/chunk-stream0-$Number%05d$.m4s" startNumber="1"><SegmentTimeline><S t="2070" d="90000" r="2"/></SegmentTimeline></SegmentTemplate>
</Representation>"#
        )
    };
    let start = mpd.find("<Representation id=\"0\"").expect("video rep");
    let end =
        mpd[start..].find("</Representation>").expect("end") + start + "</Representation>".len();
    let ladder = format!(
        "{}{}{}",
        video_rep("low", 50_000, "low"),
        video_rep("high", 900_000, "high"),
        ""
    );
    let mpd = format!("{}{}{}", &mpd[..start], ladder, &mpd[end..]);

    #[derive(Clone)]
    struct Ladder {
        dir: PathBuf,
        mpd: String,
        hits: Arc<Mutex<Vec<String>>>,
    }
    async fn handler(State(s): State<Ladder>, AxumPath(path): AxumPath<String>) -> AxumResponse {
        s.hits.lock().expect("lock").push(path.clone());
        if path == "ladder.mpd" {
            return s.mpd.clone().into_response();
        }
        let file = path.rsplit('/').next().expect("file name").to_owned();
        match std::fs::read(s.dir.join(file)) {
            Ok(b) => b.into_response(),
            Err(_) => AxumStatus::NOT_FOUND.into_response(),
        }
    }
    let hits = Arc::new(Mutex::new(Vec::new()));
    let app = Router::new()
        .route("/{*path}", get(handler))
        .with_state(Ladder {
            dir: fixture(""),
            mpd,
            hits: Arc::clone(&hits),
        });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    let server = tokio::spawn(async move { axum::serve(listener, app).await.expect("serve") });

    let (delivered, result) = Pull::start(
        route(format!("http://{addr}/ladder.mpd")),
        SkewedClock::new(0),
    )
    .finish()
    .await;
    server.abort();
    result.expect("ends cleanly");
    let hits = hits.lock().expect("lock");
    assert!(hits.iter().any(|p| p.starts_with("high/")), "{hits:?}");
    assert!(
        !hits.iter().any(|p| p.starts_with("low/")),
        "the low Representation is never fetched: {hits:?}"
    );
    assert_same(
        &delivered.video(),
        &expected("", "init-stream0.m4s", &FFMPEG_VIDEO),
        "video",
    );
}

// ---------------------------------------------------------------------------
// The four route-level behaviours the module always had
// ---------------------------------------------------------------------------

/// A missing MPD must fail the run, not hang or silently proceed.
#[tokio::test]
async fn a_missing_mpd_fails_the_attempt() {
    let origin = serve_files(fixture(""), None, None, false).await;
    let (_, result) = Pull::start(route(origin.url("does-not-exist.mpd")), SkewedClock::new(0))
        .finish()
        .await;
    assert!(result.is_err(), "a 404 MPD must fail the attempt");
}

/// A server that resolves the MPD and the inits but stalls on a segment must
/// fail within the retry budget of `IngestTimeouts::read`, not hang forever.
#[tokio::test]
async fn a_server_that_stalls_on_a_segment_fails_the_attempt() {
    let origin = serve_files(fixture(""), None, Some("chunk-stream0-00001.m4s"), false).await;
    let route = route(origin.url("manifest.mpd")).with_timeouts(IngestTimeouts {
        connect: Duration::from_secs(5),
        read: Duration::from_millis(150),
    });
    let (_, result) = Pull::start(route, SkewedClock::new(0)).finish().await;
    assert!(
        result.is_err(),
        "a stalled segment ends the attempt after its failures, it does not hang"
    );
}

const AUTH_USER: &str = "dash-user";
const AUTH_PASS: &str = "dash-pass";

/// Basic auth from URL userinfo is sent to the origin the MPD came from, for
/// the MPD, the inits and the segments alike.
#[tokio::test]
async fn basic_auth_from_url_userinfo_authenticates_and_pulls_samples() {
    let origin = serve_files(
        fixture(""),
        Some(crate::testutil::MockAuthScheme::Basic {
            username: AUTH_USER.into(),
            password: AUTH_PASS.into(),
        }),
        None,
        false,
    )
    .await;
    let credentialed = origin.url("manifest.mpd").replacen(
        "http://",
        &format!("http://{AUTH_USER}:{AUTH_PASS}@"),
        1,
    );
    let (delivered, result) = Pull::start(route(credentialed), SkewedClock::new(0))
        .finish()
        .await;
    result.expect("Basic auth from URL userinfo must authenticate");
    assert!(!delivered.video().is_empty());
}

/// A refused credential is an authentication error that ends the attempt at
/// once, not a failure retried with backoff.
#[tokio::test]
async fn a_wrong_password_ends_the_attempt_with_an_auth_error() {
    let origin = serve_files(
        fixture(""),
        Some(crate::testutil::MockAuthScheme::Basic {
            username: AUTH_USER.into(),
            password: AUTH_PASS.into(),
        }),
        None,
        false,
    )
    .await;
    let wrong =
        origin
            .url("manifest.mpd")
            .replacen("http://", &format!("http://{AUTH_USER}:nope@"), 1);
    let started = std::time::Instant::now();
    let (_, result) = Pull::start(route(wrong), SkewedClock::new(0))
        .finish()
        .await;
    assert!(
        matches!(result, Err(MultimuxError::Auth { .. })),
        "{result:?}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "no backoff first"
    );
}

// ---------------------------------------------------------------------------
// Cross-origin URLs (design R3)
// ---------------------------------------------------------------------------

fn ffmpeg_mpd_with_base_url(base: &str) -> String {
    std::fs::read_to_string(fixture("manifest.mpd"))
        .expect("fixture")
        .replacen(
            "<Period ",
            &format!("<BaseURL>{base}/</BaseURL><Period "),
            1,
        )
}

struct TwoOrigins {
    mpd_origin: tokio::task::JoinHandle<()>,
    mpd_url: String,
    media: FileOrigin,
}

impl Drop for TwoOrigins {
    fn drop(&mut self) {
        self.mpd_origin.abort();
    }
}

async fn mpd_here_media_there() -> TwoOrigins {
    let media = serve_files(fixture(""), None, None, false).await;
    let mpd = ffmpeg_mpd_with_base_url(&media.base);
    let app = Router::new().route(
        "/manifest.mpd",
        get(move || {
            let mpd = mpd.clone();
            async move { mpd }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    let mpd_origin = tokio::spawn(async move { axum::serve(listener, app).await.expect("serve") });
    TwoOrigins {
        mpd_origin,
        mpd_url: format!("http://{addr}/manifest.mpd"),
        media,
    }
}

/// A server-side pull does not fetch what a remote MPD points at on another
/// origin: nothing is requested from the second origin and the attempt ends.
#[tokio::test]
async fn a_base_url_on_another_origin_is_refused_by_default() {
    let origins = mpd_here_media_there().await;
    let (delivered, result) = Pull::start(route(origins.mpd_url.clone()), SkewedClock::new(0))
        .finish()
        .await;
    assert!(result.is_err(), "no Representation is left to play");
    assert!(delivered.samples.is_empty());
    assert!(
        origins.media.hits().is_empty(),
        "the other origin is never contacted: {:?}",
        origins.media.hits()
    );
}

/// The route's opt-in: the same MPD plays, its media coming from the other
/// origin.
#[tokio::test]
async fn a_base_url_on_another_origin_plays_when_the_route_allows_it() {
    let origins = mpd_here_media_there().await;
    let allowed = route(origins.mpd_url.clone()).with_cross_origin_urls(true);
    let (delivered, result) = Pull::start(allowed, SkewedClock::new(0)).finish().await;
    result.expect("plays");
    assert!(!origins.media.hits().is_empty());
    assert_same(
        &delivered.video(),
        &expected("", "init-stream0.m4s", &FFMPEG_VIDEO),
        "video",
    );
}

/// The same-origin default does not get in the way of a plain pull whose MPD
/// and media share an origin (the whole static suite above runs under it).
#[test]
fn a_route_refuses_cross_origin_urls_until_told_otherwise() {
    let route = DashPullRoute::new("r", "http://example.test/a.mpd");
    assert!(!route.allow_cross_origin);
    assert!(route.with_cross_origin_urls(true).allow_cross_origin);
    assert!(!super::session::dash_config(false).cross_origin_urls);
    assert!(super::session::dash_config(true).cross_origin_urls);
}

// ---------------------------------------------------------------------------
// A live origin that enforces availability against the true clock
// ---------------------------------------------------------------------------

const SEGMENT_US: i64 = 200_000;
const VIDEO_TIMESCALE: i64 = 90_000;
const AUDIO_TIMESCALE: i64 = 44_100;
const ETAG: &str = "\"live-v1\"";

#[derive(Clone, Copy)]
struct LiveShape {
    tsb_us: i64,
    utc_timing: bool,
    mup_secs: u32,
    etag: bool,
    garbage_mpds: (usize, usize),
    /// The segment this many numbers past the live edge at the first segment
    /// request answers `404` for good.
    missing_ahead: Option<u64>,
    head_utc: bool,
}

impl Default for LiveShape {
    fn default() -> Self {
        Self {
            tsb_us: 4_000_000,
            utc_timing: true,
            mup_secs: 1,
            etag: false,
            garbage_mpds: (usize::MAX, 0),
            missing_ahead: None,
            head_utc: false,
        }
    }
}

#[derive(Clone, Debug)]
struct LiveHit {
    name: String,
    method: String,
    status: u16,
}

struct LiveOrigin {
    shape: LiveShape,
    base: String,
    ast_us: i64,
    hits: Mutex<Vec<LiveHit>>,
    mpd_gets: AtomicUsize,
    conditional_hits: AtomicUsize,
    early: AtomicUsize,
    missing: std::sync::OnceLock<u64>,
}

impl LiveOrigin {
    fn mpd(&self) -> String {
        let utc = if self.shape.utc_timing {
            let scheme = if self.shape.head_utc {
                "http-head"
            } else {
                "http-xsdate"
            };
            format!(
                r#"<UTCTiming schemeIdUri="urn:mpeg:dash:utc:{scheme}:2014" value="{}/time"/>"#,
                self.base
            )
        } else {
            String::new()
        };
        let template = |timescale: i64| {
            format!(
                r#"<SegmentTemplate timescale="{timescale}" duration="{}" initialization="init-stream$RepresentationID$.m4s" media="seg-$RepresentationID$-$Number%05d$.m4s" startNumber="1"/>"#,
                timescale * SEGMENT_US / 1_000_000
            )
        };
        format!(
            r#"<?xml version="1.0" encoding="utf-8"?>
<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" profiles="urn:mpeg:dash:profile:isoff-live:2011" type="dynamic" availabilityStartTime="{ast}" publishTime="{ast}" minimumUpdatePeriod="PT{mup}S" suggestedPresentationDelay="PT0.4S" timeShiftBufferDepth="PT{tsb}S" maxSegmentDuration="PT0.2S" minBufferTime="PT0.4S">
{utc}
<Period id="0" start="PT0S">
<AdaptationSet id="0" contentType="video" segmentAlignment="true" startWithSAP="1">
<Representation id="0" mimeType="video/mp4" codecs="avc1.4d400d" bandwidth="58141" width="320" height="240">{video}</Representation>
</AdaptationSet>
<AdaptationSet id="1" contentType="audio" segmentAlignment="true" startWithSAP="1">
<Representation id="1" mimeType="audio/mp4" codecs="mp4a.40.2" bandwidth="96013" audioSamplingRate="44100">{audio}</Representation>
</AdaptationSet>
</Period>
</MPD>
"#,
            ast = xs_datetime(self.ast_us),
            mup = self.shape.mup_secs,
            tsb = self.shape.tsb_us / 1_000_000,
            video = template(VIDEO_TIMESCALE),
            audio = template(AUDIO_TIMESCALE),
        )
    }

    fn log(&self, name: &str, method: &Method, status: AxumStatus) {
        self.hits.lock().expect("lock").push(LiveHit {
            name: name.to_owned(),
            method: method.to_string(),
            status: status.as_u16(),
        });
    }

    fn served(&self, rep: u32) -> Vec<u64> {
        let prefix = format!("seg-{rep}-");
        let mut numbers: Vec<u64> = self
            .hits
            .lock()
            .expect("lock")
            .iter()
            .filter(|h| h.status == 200 && h.name.starts_with(&prefix))
            .filter_map(|h| {
                h.name
                    .strip_prefix(&prefix)?
                    .strip_suffix(".m4s")?
                    .parse()
                    .ok()
            })
            .collect();
        numbers.sort_unstable();
        numbers.dedup();
        numbers
    }

    fn requests_for(&self, number: u64) -> usize {
        let suffix = format!("-{number:05}.m4s");
        self.hits
            .lock()
            .expect("lock")
            .iter()
            .filter(|h| h.name.starts_with("seg-") && h.name.ends_with(&suffix))
            .count()
    }
}

async fn live_handler(
    AxumPath(name): AxumPath<String>,
    State(origin): State<Arc<LiveOrigin>>,
    method: Method,
    headers: HeaderMap,
) -> AxumResponse {
    let reply = |status: AxumStatus, response: AxumResponse| {
        origin.log(&name, &method, status);
        response
    };
    if name == "time" {
        let ms = now_unix_us() / 1_000;
        let text = jiff::Timestamp::from_millisecond(ms)
            .expect("in range")
            .to_string();
        return reply(AxumStatus::OK, text.into_response());
    }
    if name == "live.mpd" {
        let nth = origin.mpd_gets.fetch_add(1, Ordering::SeqCst) + 1;
        let (from, count) = origin.shape.garbage_mpds;
        if nth >= from && nth < from.saturating_add(count) {
            return reply(AxumStatus::OK, "<this is not an mpd".into_response());
        }
        if origin.shape.etag
            && headers
                .get(header::IF_NONE_MATCH)
                .and_then(|v| v.to_str().ok())
                == Some(ETAG)
        {
            origin.conditional_hits.fetch_add(1, Ordering::SeqCst);
            return reply(
                AxumStatus::NOT_MODIFIED,
                AxumStatus::NOT_MODIFIED.into_response(),
            );
        }
        let mut response = origin.mpd().into_response();
        if origin.shape.etag {
            response
                .headers_mut()
                .insert(header::ETAG, ETAG.parse().expect("etag"));
        }
        return reply(AxumStatus::OK, response);
    }
    if let Some(init) = name.strip_prefix("init-stream") {
        return match std::fs::read(fixture(&format!("init-stream{init}"))) {
            Ok(bytes) => reply(AxumStatus::OK, bytes.into_response()),
            Err(_) => reply(AxumStatus::NOT_FOUND, AxumStatus::NOT_FOUND.into_response()),
        };
    }
    let parsed = name
        .strip_prefix("seg-")
        .and_then(|rest| rest.split_once('-'))
        .and_then(|(rep, num)| {
            Some((
                rep.parse::<u32>().ok()?,
                num.strip_suffix(".m4s")?.parse::<u64>().ok()?,
            ))
        });
    let Some((rep, number)) = parsed else {
        return reply(AxumStatus::NOT_FOUND, AxumStatus::NOT_FOUND.into_response());
    };
    let available_from = origin.ast_us + i64::try_from(number).expect("small") * SEGMENT_US;
    let now = now_unix_us();
    if now < available_from {
        origin.early.fetch_add(1, Ordering::SeqCst);
        return reply(
            AxumStatus::from_u16(425).expect("425"),
            AxumStatus::from_u16(425).expect("425").into_response(),
        );
    }
    if now > available_from + origin.shape.tsb_us + 2 * SEGMENT_US {
        return reply(AxumStatus::GONE, AxumStatus::GONE.into_response());
    }
    if let Some(ahead) = origin.shape.missing_ahead {
        let edge = u64::try_from((now - origin.ast_us) / SEGMENT_US).unwrap_or(0);
        origin.missing.get_or_init(|| edge + ahead);
    }
    if origin.missing.get() == Some(&number) {
        return reply(AxumStatus::NOT_FOUND, AxumStatus::NOT_FOUND.into_response());
    }
    let (cycle, prefix) = if rep == 0 {
        (3, "chunk-stream0")
    } else {
        (4, "chunk-stream1")
    };
    let file = format!("{prefix}-{:05}.m4s", (number - 1) % cycle + 1);
    match std::fs::read(fixture(&file)) {
        Ok(bytes) => reply(AxumStatus::OK, bytes.into_response()),
        Err(_) => reply(AxumStatus::NOT_FOUND, AxumStatus::NOT_FOUND.into_response()),
    }
}

struct RunningLive {
    origin: Arc<LiveOrigin>,
    task: tokio::task::JoinHandle<()>,
}

impl RunningLive {
    fn url(&self) -> String {
        format!("{}/live.mpd", self.origin.base)
    }
}

impl Drop for RunningLive {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// An origin whose availability start time is 20 s in the past.
async fn serve_live(shape: LiveShape) -> RunningLive {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    let origin = Arc::new(LiveOrigin {
        shape,
        base: format!("http://{addr}"),
        ast_us: (now_unix_us() - 20_000_000) / 1_000 * 1_000,
        hits: Mutex::new(Vec::new()),
        mpd_gets: AtomicUsize::new(0),
        conditional_hits: AtomicUsize::new(0),
        early: AtomicUsize::new(0),
        missing: std::sync::OnceLock::new(),
    });
    let app = Router::new()
        .route("/{name}", get(live_handler))
        .with_state(Arc::clone(&origin));
    let task = tokio::spawn(async move { axum::serve(listener, app).await.expect("serve") });
    RunningLive { origin, task }
}

/// The oracle for a live pull: the samples of the segments the origin served,
/// in order, each demuxed directly from the fixture it was cut from.
fn live_expected(rep: u32, numbers: &[u64]) -> Vec<Sample> {
    let (init, prefix, cycle) = if rep == 0 {
        ("init-stream0.m4s", "chunk-stream0", 3)
    } else {
        ("init-stream1.m4s", "chunk-stream1", 4)
    };
    let init = std::fs::read(fixture(init)).expect("init");
    numbers
        .iter()
        .flat_map(|n| {
            let name = format!("{prefix}-{:05}.m4s", (n - 1) % cycle + 1);
            let bytes = std::fs::read(fixture(&name)).expect("segment");
            demux_pair(&init, &bytes)
        })
        .collect()
}

const FEW_SEGMENTS: usize = 8;

/// A live `$Number$` presentation over a real socket: the pull joins near the
/// live edge, and the samples that reach the `Trunk` are, in order and byte for
/// byte, those of the segments the origin served.
#[tokio::test]
async fn a_live_presentation_delivers_the_served_segments_byte_exact() {
    let live = serve_live(LiveShape::default()).await;
    let mut pull = Pull::start(route(live.url()), SkewedClock::new(0));
    let ended = pull.until(|d| d.video().len() >= FEW_SEGMENTS * 25).await;
    assert!(ended.is_none(), "a live pull does not end: {ended:?}");
    let delivered = pull.stop();

    let video_numbers = live.origin.served(0);
    let audio_numbers = live.origin.served(1);
    let want_video = live_expected(0, &video_numbers);
    let got_video = delivered.video();
    assert!(got_video.len() >= FEW_SEGMENTS * 25, "{}", got_video.len());
    for (i, (g, w)) in got_video.iter().zip(&want_video).enumerate() {
        assert!(same(g, w), "video sample {i} differs from the oracle");
    }
    let want_audio = live_expected(1, &audio_numbers);
    for (i, (g, w)) in delivered.audio().iter().zip(&want_audio).enumerate() {
        assert!(same(g, w), "audio sample {i} differs from the oracle");
    }
    assert!(!delivered.audio().is_empty());
    assert_eq!(delivered.lagged, 0);
    let first = *video_numbers.first().expect("served");
    assert!(
        first > 80,
        "the pull joined near the live edge (segment {first} of ~100), not at startNumber"
    );
    assert!(
        video_numbers.windows(2).all(|w| w[1] == w[0] + 1),
        "no segment skipped: {video_numbers:?}"
    );
}

/// No segment is requested before the origin says it exists, whatever the host
/// clock says, as long as the MPD announces a `UTCTiming` source: the ladder
/// trusts the packager's clock over the host's.
async fn utc_timing_run(skew_us: i64) -> (Vec<u64>, usize) {
    let live = serve_live(LiveShape::default()).await;
    let mut pull = Pull::start(route(live.url()), SkewedClock::new(skew_us));
    let ended = pull.until(|d| d.video().len() >= 60).await;
    assert!(ended.is_none(), "{ended:?}");
    let delivered = pull.stop();
    assert!(!delivered.video().is_empty());
    (
        live.origin.served(0),
        live.origin.early.load(Ordering::SeqCst),
    )
}

#[tokio::test]
async fn a_host_clock_one_minute_fast_never_makes_a_utc_timed_pull_request_early() {
    let (served, early) = utc_timing_run(60_000_000).await;
    assert!(!served.is_empty());
    assert_eq!(early, 0, "requests before availability: {early}");
}

#[tokio::test]
async fn a_host_clock_one_minute_slow_never_makes_a_utc_timed_pull_request_early() {
    let (served, early) = utc_timing_run(-60_000_000).await;
    assert!(!served.is_empty());
    assert_eq!(early, 0, "requests before availability: {early}");
}

/// Without a `UTCTiming` element the host clock is the best source there is,
/// and a fast one does make the pull ask too soon: the case `UTCTiming` exists
/// for, and the reason the test above bites.
#[tokio::test]
async fn without_utc_timing_a_fast_host_clock_requests_segments_early() {
    let live = serve_live(LiveShape {
        utc_timing: false,
        ..LiveShape::default()
    })
    .await;
    let mut pull = Pull::start(route(live.url()), SkewedClock::new(60_000_000));
    let started = tokio::time::Instant::now();
    while live.origin.early.load(Ordering::SeqCst) == 0
        && started.elapsed() < Duration::from_secs(10)
    {
        tokio::time::sleep(POLL).await;
    }
    assert!(
        live.origin.early.load(Ordering::SeqCst) > 0,
        "the fast host clock must show as early requests"
    );
    assert!(pull.until(|_| true).await.is_none());
    drop(pull.stop());
}

/// The first request after an MPD was fetched is conditional, a `304` is
/// absorbed, and the pull keeps playing through several refreshes.
#[tokio::test]
async fn a_conditional_refresh_is_sent_and_its_304_is_absorbed() {
    let live = serve_live(LiveShape {
        etag: true,
        mup_secs: 1,
        ..LiveShape::default()
    })
    .await;
    let mut pull = Pull::start(route(live.url()), SkewedClock::new(0));
    let ended = pull
        .until(|_| live.origin.conditional_hits.load(Ordering::SeqCst) >= 2)
        .await;
    assert!(ended.is_none(), "{ended:?}");
    let before = pull.now().video().len();
    tokio::time::sleep(Duration::from_millis(600)).await;
    let after = pull.now().video().len();
    drop(pull.stop());
    assert!(
        after > before,
        "still playing after the 304s ({before} -> {after})"
    );
}

/// A refresh answered with something that is not an MPD does not end a live
/// pull: it is retried, and playback goes on.
#[tokio::test]
async fn a_garbage_mpd_refresh_does_not_end_a_live_pull() {
    let live = serve_live(LiveShape {
        garbage_mpds: (2, 2),
        mup_secs: 1,
        ..LiveShape::default()
    })
    .await;
    let mut pull = Pull::start(route(live.url()), SkewedClock::new(0));
    let ended = pull
        .until(|_| live.origin.mpd_gets.load(Ordering::SeqCst) >= 4)
        .await;
    assert!(ended.is_none(), "the pull survived the bad MPDs: {ended:?}");
    let before = pull.now().video().len();
    tokio::time::sleep(Duration::from_millis(800)).await;
    assert!(pull.now().video().len() > before, "and kept playing");
    drop(pull.stop());
}

/// A garbage MPD as the very first answer fails the attempt (the supervisor
/// reconnects); there is nothing to keep playing.
#[tokio::test]
async fn a_garbage_first_mpd_fails_the_attempt() {
    let live = serve_live(LiveShape {
        garbage_mpds: (1, 100),
        ..LiveShape::default()
    })
    .await;
    let mut pull = Pull::start(route(live.url()), SkewedClock::new(0));
    let ended = pull.until(|_| false).await;
    assert!(matches!(ended, Some(Err(_))), "{ended:?}");
    drop(pull.stop());
}

/// A segment inside the window that never appears (`404` for good) is given up
/// after its failures; the segments after it still play.
#[tokio::test]
async fn a_segment_that_never_appears_is_given_up_and_playback_continues() {
    let live = serve_live(LiveShape {
        tsb_us: 12_000_000,
        missing_ahead: Some(4),
        ..LiveShape::default()
    })
    .await;
    let mut pull = Pull::start(route(live.url()), SkewedClock::new(0));
    let ended = pull
        .until(|_| {
            live.origin
                .missing
                .get()
                .is_some_and(|m| live.origin.served(0).iter().any(|n| *n > m + 3))
        })
        .await;
    assert!(ended.is_none(), "{ended:?}");
    drop(pull.stop());
    let missing = *live
        .origin
        .missing
        .get()
        .expect("a segment was made missing");
    assert!(
        live.origin.requests_for(missing) > 1,
        "the missing segment was retried before it was given up"
    );
    assert!(live.origin.served(0).iter().all(|n| *n != missing));
}

/// `http-head` is answered with `HEAD` requests whose `Date` is the time, and
/// a skewed host clock is still never trusted over it.
#[tokio::test]
async fn an_http_head_utc_timing_is_followed_with_head_requests() {
    let live = serve_live(LiveShape {
        head_utc: true,
        ..LiveShape::default()
    })
    .await;
    let mut pull = Pull::start(route(live.url()), SkewedClock::new(60_000_000));
    let ended = pull.until(|d| d.video().len() >= 50).await;
    assert!(ended.is_none(), "{ended:?}");
    drop(pull.stop());
    let hits = live.origin.hits.lock().expect("lock").clone();
    let time: Vec<&LiveHit> = hits.iter().filter(|h| h.name == "time").collect();
    assert!(
        !time.is_empty() && time.iter().all(|h| h.method == "HEAD"),
        "{time:?}"
    );
    assert_eq!(live.origin.early.load(Ordering::SeqCst), 0);
}

// ---------------------------------------------------------------------------
// Wire helpers
// ---------------------------------------------------------------------------

#[test]
fn the_retry_delay_doubles_from_half_a_second_to_a_cap() {
    let ms = |attempt| u64::try_from(drive::backoff(attempt).as_millis()).unwrap();
    assert_eq!(ms(1), 500);
    assert_eq!(ms(2), 1_000);
    assert_eq!(ms(3), 2_000);
    assert_eq!(ms(4), 4_000);
    assert_eq!(ms(5), 4_000, "capped");
    assert_eq!(ms(0), 500, "attempt zero is not a free retry");
    assert_eq!(ms(u32::MAX), 4_000, "no shift overflow");
}

#[test]
fn only_the_requested_part_of_a_ranged_answer_is_accepted() {
    let body: Vec<u8> = (0..100u8).collect();
    let ok = drive::ranged_body;
    let partial = axum::http::StatusCode::PARTIAL_CONTENT;
    let full = axum::http::StatusCode::OK;
    assert_eq!(
        ok(partial, Some((10, 19)), body[10..20].to_vec(), (10, 10)),
        Some(body[10..20].to_vec())
    );
    assert_eq!(
        ok(partial, None, body[10..20].to_vec(), (10, 10)),
        Some(body[10..20].to_vec()),
        "a 206 without Content-Range is judged by its length"
    );
    assert_eq!(
        ok(partial, Some((11, 20)), body[10..20].to_vec(), (10, 10)),
        None,
        "Content-Range names another part"
    );
    assert_eq!(
        ok(partial, Some((10, 19)), body[10..19].to_vec(), (10, 10)),
        None,
        "short body"
    );
    assert_eq!(
        ok(full, None, body.clone(), (10, 10)),
        Some(body[10..20].to_vec()),
        "a 200 is sliced"
    );
    assert_eq!(
        ok(full, None, body[..15].to_vec(), (10, 10)),
        None,
        "a 200 too short for the range"
    );
    assert_eq!(
        ok(full, None, body.clone(), (u64::MAX, 10)),
        None,
        "an overflowing range is refused, not wrapped"
    );
}
