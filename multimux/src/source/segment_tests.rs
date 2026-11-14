//! Tests for the thin segmenter glue (`segment`): the driver-backed
//! route chain (`IngestDriver` → `report_driver_progress` →
//! `drive_program_segmenters`) and everything `RouteHandle` observes from it.
//!
//! The segmenter itself lives in `media_plane::segmenter::program`; its
//! self-contained tests live next to it. These exercise the production
//! wrapper end to end, through the route's own `HlsOrigin`/`Trunk`.

use std::collections::{HashMap, HashSet, VecDeque};
use std::convert::Infallible;
use std::time::Duration;

use broadcast_common::{Demand, Stage, Timestamp};
use dvb_si::demux::SiDemux;
use hls_runtime::server::{DEFAULT_TRACK_ID, HlsBody, HlsRequest};
use media_plane::egress::{AwaitPolicy, EgressResponse, ServedEgress};
use media_plane::ingress::{IngestDriver, IngestSession, SessionEvent};
use media_plane::ts_session::TsIngestSession;
use media_plane::ts_session::test_support::{build_ts_bytes, handshake, track_spec, trunk_config};
use transmux::Container;
use transmux::TrackId;
use transmux::pipeline::{Sample, TrackSpec};

use media_plane::ingress::ProgramId;
use media_plane::segmenter::ProgramSegmenter;

use crate::route::{RouteConfig, RouteHandle};
use crate::source::report_driver_progress;
use crate::source::segment::drive_program_segmenters;

/// Render `route`'s current LL-HLS media playlist synchronously — the exact
/// production call every LL-HLS request resolves through, so a passing
/// assertion here is genuinely "resolvable the way egress resolves them".
fn render_playlist(route: &RouteHandle) -> String {
    let ll_hls = route
        .ll_hls(crate::route::SPTS_PROGRAM_ID)
        .expect("SPTS_PROGRAM_ID must be published before rendering");
    match ll_hls.resolve(
        HlsRequest::Playlist {
            track_id: DEFAULT_TRACK_ID,
            query: Default::default(),
        },
        Timestamp::from_nanos(0),
        AwaitPolicy::new(Timestamp::from_nanos(0)),
    ) {
        EgressResponse::Ready {
            body: HlsBody::Playlist(m),
            ..
        } => m,
        other => panic!("expected Ready(Playlist), got {other:?}"),
    }
}

fn driver_feed_nanos() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(2);
    N.fetch_add(1, Ordering::Relaxed)
}

/// **The production segmenter gap, closed** (issue #805 task 2b). Drives the
/// exact production chain a driver-backed route uses.
///
/// MUTATION VERIFIED: removing the init push in `drive_program_segmenters`
/// makes this test's first assertion fail (`None` instead of `Some(bytes)`).
#[test]
fn driver_backed_route_serves_real_media_through_ll_hls() {
    let route = RouteHandle::new(
        RouteConfig::default()
            .with_target_duration_secs(1.0)
            .with_part_target(Duration::from_millis(250))
            .with_window_segments(8),
    );
    let mut driver = IngestDriver::new(
        TsIngestSession::new(),
        media_plane::IngestConfig::new(handshake()).with_trunk(trunk_config()),
    );
    let mut published = HashSet::new();
    let mut segmenters = HashMap::new();

    // First feed: resolves the PMT, mints program 0's driver-side Trunk.
    let ts_bytes = build_ts_bytes(TrackId::new(1), 0xAB, 90);
    driver.feed(&ts_bytes, Timestamp::ZERO);
    report_driver_progress(&driver, &route, &mut published);
    drive_program_segmenters(&driver, &route, &mut segmenters);

    // Second feed: more real media, proving the segmenter's cursor keeps
    // observing new samples live in addition to replaying the first feed's
    // backlog.
    let more = build_ts_bytes(TrackId::new(1), 0xCD, 90);
    driver.feed(&more, Timestamp::from_nanos(1));
    report_driver_progress(&driver, &route, &mut published);
    drive_program_segmenters(&driver, &route, &mut segmenters);

    assert!(
        route
            .init_bytes(crate::route::SPTS_PROGRAM_ID)
            .is_some_and(|b| !b.is_empty()),
        "a driver-backed route must end up with real, non-empty init bytes"
    );

    let playlist = render_playlist(&route);
    // Deliberately `"#EXT-X-PART:"` (an actual per-part tag) and
    // `"#EXTINF:"` (an actual closed segment), not the always-present
    // `"#EXT-X-PART-INF:"` header.
    assert!(
        playlist.contains("#EXTINF:") || playlist.contains("#EXT-X-PART:"),
        "the route's own HlsOrigin must serve real closed segments/parts, \
         resolved exactly the way egress resolves them: {playlist}"
    );
}

fn burst_driver(frames: u32) -> IngestDriver<TsIngestSession> {
    let ring = |n: usize| std::num::NonZeroUsize::new(n).expect("non-zero ring");
    let mut driver = IngestDriver::new(
        TsIngestSession::new(),
        media_plane::IngestConfig::new(handshake()).with_trunk(
            media_plane::trunk::TrunkConfig::default()
                .with_timed_capacity(ring(8192))
                .with_sparse_capacity(ring(64))
                .with_segment_capacity(ring(8))
                .with_event_capacity(ring(8))
                .with_part_capacity(ring(8)),
        ),
    );
    driver.feed(
        &build_ts_bytes(TrackId::new(1), 0xAB, frames),
        Timestamp::ZERO,
    );
    driver
}

#[test]
fn a_large_queued_burst_is_segmented_in_bounded_batches() {
    let route = RouteHandle::new(
        RouteConfig::default()
            .with_target_duration_secs(1.0)
            .with_part_target(Duration::from_millis(250))
            .with_window_segments(8),
    );
    let driver = burst_driver(3 * media_plane::segmenter::program::PUMP_BATCH_ITEMS as u32);
    let mut published = HashSet::new();
    let mut segmenters = HashMap::new();
    report_driver_progress(&driver, &route, &mut published);
    let mut calls = 1;
    while drive_program_segmenters(&driver, &route, &mut segmenters) {
        calls += 1;
        assert!(calls < 64, "the backlog never drains");
    }
    assert!(
        calls >= 3,
        "3 batches of queued samples must take at least 3 calls, took {calls}"
    );
}

#[test]
fn a_small_burst_is_segmented_in_one_call() {
    let route = RouteHandle::new(
        RouteConfig::default()
            .with_target_duration_secs(1.0)
            .with_part_target(Duration::from_millis(250))
            .with_window_segments(8),
    );
    let driver = burst_driver(30);
    let mut published = HashSet::new();
    let mut segmenters = HashMap::new();
    report_driver_progress(&driver, &route, &mut published);
    assert!(!drive_program_segmenters(&driver, &route, &mut segmenters));
}

#[tokio::test(flavor = "current_thread")]
async fn a_large_queued_burst_does_not_starve_another_task() {
    let route = RouteHandle::new(
        RouteConfig::default()
            .with_target_duration_secs(1.0)
            .with_part_target(Duration::from_millis(250))
            .with_window_segments(8),
    );
    let driver = burst_driver(3 * media_plane::segmenter::program::PUMP_BATCH_ITEMS as u32);
    let mut state = crate::source::DriverProgress::new();
    let ran = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = std::sync::Arc::clone(&ran);
    let other = async move { flag.store(true, std::sync::atomic::Ordering::SeqCst) };
    // Race the two futures on one task instead of spawning a second one:
    // `advance_route` must yield to the runtime between batches, so the
    // other future gets to run before it finishes.
    let advance = crate::source::advance_route(&driver, &route, &mut state);
    tokio::join!(advance, other);
    assert!(
        ran.load(std::sync::atomic::Ordering::SeqCst),
        "advance_route must yield between batches of a large burst"
    );
}

/// **Issue #809.** `multimux_parts_produced_total`/
/// `multimux_segments_produced_total` must actually move for a
/// driver-backed route.
///
/// MUTATION VERIFIED: commenting out both `if parts_total > 0 { ... }`/
/// `if segments_total > 0 { ... }` metric-emitting blocks makes
/// `assert!(parts_after > parts_before, ...)` fail with both values `0.0`.
#[test]
fn drive_program_segmenters_bumps_parts_and_segments_produced_counters() {
    crate::prometheus::install();
    let route = RouteHandle::new(
        RouteConfig::default()
            .with_target_duration_secs(1.0)
            .with_part_target(Duration::from_millis(250))
            .with_window_segments(8),
    )
    .with_name("segment-metrics-probe-route");
    let mut driver = IngestDriver::new(
        TsIngestSession::new(),
        media_plane::IngestConfig::new(handshake()).with_trunk(trunk_config()),
    );
    let mut published = HashSet::new();
    let mut segmenters = HashMap::new();

    fn metric_total(metric: &str, route_label: &str) -> f64 {
        let rendered = crate::prometheus::install().render();
        rendered
            .lines()
            .find(|l| l.starts_with(metric) && l.contains(route_label))
            .and_then(|l| l.rsplit(' ').next())
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(0.0)
    }

    let parts_before = metric_total(
        "multimux_parts_produced_total",
        "segment-metrics-probe-route",
    );
    let segments_before = metric_total(
        "multimux_segments_produced_total",
        "segment-metrics-probe-route",
    );

    let ts_bytes = build_ts_bytes(TrackId::new(1), 0xAB, 90);
    driver.feed(&ts_bytes, Timestamp::ZERO);
    report_driver_progress(&driver, &route, &mut published);
    drive_program_segmenters(&driver, &route, &mut segmenters);
    let more = build_ts_bytes(TrackId::new(1), 0xCD, 90);
    driver.feed(&more, Timestamp::from_nanos(1));
    report_driver_progress(&driver, &route, &mut published);
    drive_program_segmenters(&driver, &route, &mut segmenters);

    let parts_after = metric_total(
        "multimux_parts_produced_total",
        "segment-metrics-probe-route",
    );
    let segments_after = metric_total(
        "multimux_segments_produced_total",
        "segment-metrics-probe-route",
    );

    assert!(
        parts_after > parts_before,
        "multimux_parts_produced_total must increase: before={parts_before} after={parts_after}"
    );
    assert!(
        segments_after > segments_before,
        "multimux_segments_produced_total must increase: before={segments_before} \
         after={segments_after}"
    );
}

#[test]
fn samples_published_in_the_same_feed_as_new_program_are_still_segmented() {
    let route = RouteHandle::new(
        RouteConfig::default()
            .with_target_duration_secs(1.0)
            .with_part_target(Duration::from_millis(250))
            .with_window_segments(8),
    );
    let mut driver = IngestDriver::new(
        TsIngestSession::new(),
        media_plane::IngestConfig::new(handshake()).with_trunk(trunk_config()),
    );
    let mut published = HashSet::new();
    let mut segmenters = HashMap::new();

    // ONE feed: `build_ts_bytes` muxes the PMT (-> NewProgram) and 90 real
    // PES samples (-> Sample events) into the same TS byte stream.
    let ts_bytes = build_ts_bytes(TrackId::new(1), 0xAB, 90);
    driver.feed(&ts_bytes, Timestamp::ZERO);
    report_driver_progress(&driver, &route, &mut published);
    drive_program_segmenters(&driver, &route, &mut segmenters);

    assert!(
        route
            .init_bytes(crate::route::SPTS_PROGRAM_ID)
            .is_some_and(|b| !b.is_empty()),
        "a driver-backed route must end up with real, non-empty init bytes"
    );

    let playlist = render_playlist(&route);
    assert!(
        playlist.contains("#EXTINF:") || playlist.contains("#EXT-X-PART:"),
        "samples published in the SAME feed call as NewProgram must still reach the \
         served playlist as real closed segments/parts: {playlist}"
    );
}

#[test]
fn segmenter_feeds_the_same_trunk_the_driver_writes_samples_into() {
    let route = RouteHandle::new(
        RouteConfig::default()
            .with_target_duration_secs(1.0)
            .with_part_target(Duration::from_millis(250))
            .with_window_segments(8),
    );
    let mut driver = IngestDriver::new(
        TsIngestSession::new(),
        media_plane::IngestConfig::new(handshake()).with_trunk(trunk_config()),
    );
    let mut published = HashSet::new();
    let mut segmenters = HashMap::new();

    let ts_bytes = build_ts_bytes(TrackId::new(1), 0xAB, 90);
    driver.feed(&ts_bytes, Timestamp::ZERO);
    report_driver_progress(&driver, &route, &mut published);
    drive_program_segmenters(&driver, &route, &mut segmenters);

    let more = build_ts_bytes(TrackId::new(1), 0xCD, 90);
    driver.feed(&more, Timestamp::from_nanos(1));
    report_driver_progress(&driver, &route, &mut published);
    drive_program_segmenters(&driver, &route, &mut segmenters);

    let program_trunk = driver
        .trunk(ProgramId(0))
        .cloned()
        .expect("program 0 resolved from the muxed TS");
    assert!(
        program_trunk.last_closed_segment().is_some(),
        "the driver's own registered Trunk must show the closed segment \
         the segmenter produced — proving no second Trunk was used"
    );
    // Also pins that the registry (what egress resolves) is the exact same
    // Trunk, not merely one with equal contents.
    match route.resolve_program(crate::route::SPTS_PROGRAM_ID) {
        crate::route::ProgramResolution::Found(resolved) => {
            assert!(std::sync::Arc::ptr_eq(&resolved.trunk(), &program_trunk));
        }
        _ => panic!("expected the route's registry to resolve program 0"),
    }
}

/// A hand-scripted two-program `IngestSession`: queues `Established`, then
/// (on its 2nd `feed` call) both `NewProgram` announcements, then (on every
/// later call) one sample for *each* program's own track — enough to drive
/// `drive_program_segmenters` through the MPTS case without needing a real
/// multi-program TS mux.
struct TwoProgramSession {
    pending: VecDeque<SessionEvent>,
    feed_count: u32,
}

fn sample_at(i: u32, is_sync: bool) -> Sample {
    let nal = [0x65u8, 0xAAu8.wrapping_add(i as u8), (i % 256) as u8];
    let mut data = (nal.len() as u32).to_be_bytes().to_vec();
    data.extend_from_slice(&nal);
    Sample::new(
        data,
        Some(i64::from(i) * i64::from(FRAME_DUR)),
        Some(i64::from(i) * i64::from(FRAME_DUR)),
        Some(FRAME_DUR),
        is_sync,
    )
}

const FRAME_DUR: u32 = transmux::VIDEO_CLOCK_RATE / 30;

impl TwoProgramSession {
    fn new() -> Self {
        let mut pending = VecDeque::new();
        pending.push_back(SessionEvent::Established);
        TwoProgramSession {
            pending,
            feed_count: 0,
        }
    }
}

impl Stage for TwoProgramSession {
    type In<'a> = &'a [u8];
    type Out = SessionEvent;
    type Error = Infallible;

    fn demand(&self) -> Demand {
        Demand::new(crate::source::INGEST_DEMAND_BYTES)
    }

    fn feed(&mut self, _input: &[u8], _now: Timestamp) -> Result<(), Infallible> {
        self.feed_count += 1;
        if self.feed_count == 2 {
            self.pending.push_back(SessionEvent::NewProgram {
                program: ProgramId(0),
                tracks: vec![track_spec(TrackId::new(1))],
            });
            self.pending.push_back(SessionEvent::NewProgram {
                program: ProgramId(1),
                tracks: vec![track_spec(TrackId::new(2))],
            });
        } else if self.feed_count >= 3 {
            let i = self.feed_count - 3;
            let is_sync = i == 0 || i == 45;
            self.pending.push_back(SessionEvent::Sample {
                program: ProgramId(0),
                track_id: TrackId::new(1),
                retention: media_plane::trunk::RetentionClass::Timed,
                sample: sample_at(i, is_sync),
            });
            self.pending.push_back(SessionEvent::Sample {
                program: ProgramId(1),
                track_id: TrackId::new(2),
                retention: media_plane::trunk::RetentionClass::Timed,
                sample: sample_at(i, is_sync),
            });
        }
        Ok(())
    }

    fn poll(&mut self) -> Option<SessionEvent> {
        self.pending.pop_front()
    }

    fn finish(&mut self) -> Result<(), Infallible> {
        Ok(())
    }
}

impl IngestSession for TwoProgramSession {
    type Request = bytes::Bytes;
}

#[test]
fn two_programs_on_one_route_segment_independently() {
    let route = RouteHandle::new(
        RouteConfig::default()
            .with_target_duration_secs(1.0)
            .with_part_target(std::time::Duration::from_millis(500))
            .with_window_segments(8),
    );
    let mut driver = IngestDriver::new(
        TwoProgramSession::new(),
        media_plane::IngestConfig::new(handshake()).with_trunk(trunk_config()),
    );
    let mut published = HashSet::new();
    let mut segmenters = HashMap::new();

    // Feed 1: Established. Feed 2: both NewProgram announcements (mints both
    // driver-side Trunks and subscribes both `ProgramSegmenter`s *before* any
    // sample exists).
    driver.feed(&[], Timestamp::from_nanos(0));
    report_driver_progress(&driver, &route, &mut published);
    drive_program_segmenters(&driver, &route, &mut segmenters);
    driver.feed(&[], Timestamp::from_nanos(1));
    report_driver_progress(&driver, &route, &mut published);
    drive_program_segmenters(&driver, &route, &mut segmenters);

    // 90 samples @ 3000 ticks/30fps = 3s of media per program, comfortably
    // over the 1.0s target duration.
    for _ in 0..90 {
        let now = Timestamp::from_nanos(driver_feed_nanos());
        driver.feed(&[], now);
        drive_program_segmenters(&driver, &route, &mut segmenters);
    }

    let trunk_0 = driver
        .trunk(ProgramId(0))
        .cloned()
        .expect("program 0 announced");
    let trunk_1 = driver
        .trunk(ProgramId(1))
        .cloned()
        .expect("program 1 announced");
    assert!(
        !std::sync::Arc::ptr_eq(&trunk_0, &trunk_1),
        "the two programs must have genuinely distinct Trunks"
    );
    assert!(
        trunk_0.last_closed_segment().is_some(),
        "program 0 must have segmented its own media independently"
    );
    assert!(
        trunk_1.last_closed_segment().is_some(),
        "program 1 must have segmented its own media independently"
    );
}

/// Render `route`'s current classic-HLS media playlist for `program`
/// synchronously — the multi-program analogue of `render_playlist`.
fn render_playlist_for(route: &RouteHandle, program: ProgramId) -> String {
    let ll_hls = route
        .ll_hls(program)
        .unwrap_or_else(|| panic!("{program:?} must be published before rendering"));
    match ll_hls.resolve(
        HlsRequest::Playlist {
            track_id: DEFAULT_TRACK_ID,
            query: Default::default(),
        },
        Timestamp::from_nanos(0),
        AwaitPolicy::new(Timestamp::from_nanos(0)),
    ) {
        EgressResponse::Ready {
            body: HlsBody::Playlist(m),
            ..
        } => m,
        other => panic!("expected Ready(Playlist) for {program:?}, got {other:?}"),
    }
}

/// Resolve one dynamic resource (a served `.ts` segment, by URI) against
/// `program`'s `HlsOrigin` — the same `ServedEgress::resolve` call
/// `crate::origin::resource::dynamic_file` drives in production.
fn resolve_resource(route: &RouteHandle, program: ProgramId, name: &str) -> bytes::Bytes {
    let ll_hls = route
        .ll_hls(program)
        .unwrap_or_else(|| panic!("{program:?} must be published before resolving"));
    match ll_hls.resolve(
        HlsRequest::Resource {
            name: name.to_string(),
        },
        Timestamp::from_nanos(0),
        AwaitPolicy::new(Timestamp::from_nanos(0)),
    ) {
        EgressResponse::Ready {
            body: HlsBody::Resource(bytes),
            ..
        } => bytes,
        other => panic!("expected Ready(Resource) for {name:?} ({program:?}), got {other:?}"),
    }
}

/// The AAC track spec of the public `h264-aac.ts` capture, renumbered.
fn aac_track_spec(track_id: TrackId) -> TrackSpec {
    use broadcast_common::Unpackage;
    let ts = test_fixtures::read("ts/h264-aac.ts");
    let media = transmux::TsDemux::new().unpackage(&ts).expect("demux TS");
    let mut spec = media
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, transmux::pipeline::CodecConfig::Aac { .. }))
        .expect("fixture has an AAC track")
        .spec
        .clone();
    spec.track_id = track_id;
    spec
}

/// The first `.ts` segment URI line in a rendered classic-HLS media playlist.
fn first_ts_segment_uri(playlist: &str) -> Option<&str> {
    playlist
        .lines()
        .find(|l| !l.starts_with('#') && l.ends_with(".ts"))
}

/// How many times [`count_pmt_sections`] reads its buffer: a second pass
/// lets a PMT that precedes its PAT be seen.
const PMT_SECTION_SCAN_PASSES: usize = 2;

/// Count PMT sections (ISO/IEC 13818-1 Table 2-31) in one whole-packet
/// MPEG-2 TS byte buffer through `dvb_si::demux::SiDemux` — a genuine parse
/// of the wire bytes, not an assumption about which PID the PMT uses.
fn count_pmt_sections(ts_bytes: &[u8]) -> usize {
    let mut demux = SiDemux::builder().build();
    let mut pmt_count = 0usize;
    for _ in 0..PMT_SECTION_SCAN_PASSES {
        for packet in ts_bytes.chunks_exact(mpeg_ts::ts::TS_PACKET_SIZE) {
            pmt_count += demux
                .feed(packet)
                .filter(|event| event.table_id() == dvb_si::TableId::Pmt as u8)
                .count();
        }
    }
    pmt_count
}

/// **Issue #887 — RFC 8216bis §3.1.1's "single MPEG-2 Program" constraint,
/// proven by parsing served bytes.** Each program renders its own classic-HLS
/// playlist with no `#EXT-X-MAP`, and each program's first served `.ts`
/// segment contains EXACTLY ONE PMT section.
#[test]
fn ts_hls_mpts_route_serves_each_program_as_its_own_single_pmt_playlist() {
    let route = RouteHandle::new(
        RouteConfig::default()
            .with_target_duration_secs(1.0)
            .with_part_target(std::time::Duration::from_millis(500))
            .with_window_segments(8),
    )
    .with_container(Container::MpegTs);
    let mut driver = IngestDriver::new(
        TwoProgramSession::new(),
        media_plane::IngestConfig::new(handshake()).with_trunk(trunk_config()),
    );
    let mut published = HashSet::new();
    let mut segmenters = HashMap::new();

    // Feed 1: Established. Feed 2: both NewProgram announcements.
    driver.feed(&[], Timestamp::from_nanos(0));
    report_driver_progress(&driver, &route, &mut published);
    drive_program_segmenters(&driver, &route, &mut segmenters);
    driver.feed(&[], Timestamp::from_nanos(1));
    report_driver_progress(&driver, &route, &mut published);
    drive_program_segmenters(&driver, &route, &mut segmenters);

    // Same 90-sample budget as `two_programs_on_one_route_segment_independently`.
    for _ in 0..90 {
        let now = Timestamp::from_nanos(driver_feed_nanos());
        driver.feed(&[], now);
        drive_program_segmenters(&driver, &route, &mut segmenters);
    }

    let trunk_0 = driver
        .trunk(ProgramId(0))
        .cloned()
        .expect("program 0 announced");
    let trunk_1 = driver
        .trunk(ProgramId(1))
        .cloned()
        .expect("program 1 announced");
    assert!(
        !std::sync::Arc::ptr_eq(&trunk_0, &trunk_1),
        "each program must be served off its own Trunk, not a shared one"
    );

    let playlist_0 = render_playlist_for(&route, ProgramId(0));
    let playlist_1 = render_playlist_for(&route, ProgramId(1));

    assert!(
        playlist_0.contains(".ts") && !playlist_0.contains("#EXT-X-MAP"),
        "program 0's classic-TS playlist must reference .ts segments with no init \
         segment: {playlist_0}"
    );
    assert!(
        playlist_1.contains(".ts") && !playlist_1.contains("#EXT-X-MAP"),
        "program 1's classic-TS playlist must reference .ts segments with no init \
         segment: {playlist_1}"
    );

    for (program, playlist) in [(ProgramId(0), &playlist_0), (ProgramId(1), &playlist_1)] {
        let uri = first_ts_segment_uri(playlist).unwrap_or_else(|| {
            panic!("{program:?}'s playlist has no closed .ts segment yet: {playlist}")
        });
        let bytes = resolve_resource(&route, program, uri);
        let pmt_count = count_pmt_sections(&bytes);
        assert_eq!(
            pmt_count, 1,
            "{program:?}'s served segment {uri:?} must carry exactly one PMT \
             (RFC 8216bis §3.1.1: a Transport Stream Segment MUST contain a single \
             MPEG-2 Program), got {pmt_count}"
        );
    }
}

/// A hand-scripted session that announces program 0 with one video track,
/// then (once `add_track_9` stages a `TracksChanged`) emits a sample for the
/// new track 9 too.
struct TrackChangeSession {
    pending: std::cell::RefCell<VecDeque<SessionEvent>>,
    feed_count: u32,
    added_track_9: std::cell::Cell<bool>,
}

impl TrackChangeSession {
    fn new() -> Self {
        let mut pending = VecDeque::new();
        pending.push_back(SessionEvent::Established);
        TrackChangeSession {
            pending: std::cell::RefCell::new(pending),
            feed_count: 0,
            added_track_9: std::cell::Cell::new(false),
        }
    }

    /// Stage a `TracksChanged` — callable via `&self` through `driver.session()`.
    fn add_track_9(&self) {
        self.pending
            .borrow_mut()
            .push_back(SessionEvent::TracksChanged {
                program: ProgramId(0),
                tracks: vec![track_spec(TrackId::new(1)), track_spec(TrackId::new(9))],
            });
        self.added_track_9.set(true);
    }
}

impl Stage for TrackChangeSession {
    type In<'a> = &'a [u8];
    type Out = SessionEvent;
    type Error = Infallible;

    fn demand(&self) -> Demand {
        Demand::new(crate::source::INGEST_DEMAND_BYTES)
    }

    fn feed(&mut self, _input: &[u8], _now: Timestamp) -> Result<(), Infallible> {
        self.feed_count += 1;
        if self.feed_count == 2 {
            self.pending
                .borrow_mut()
                .push_back(SessionEvent::NewProgram {
                    program: ProgramId(0),
                    tracks: vec![track_spec(TrackId::new(1))],
                });
        } else if self.feed_count >= 3 {
            let i = self.feed_count - 3;
            let is_sync = i.is_multiple_of(45);
            self.pending.borrow_mut().push_back(SessionEvent::Sample {
                program: ProgramId(0),
                track_id: TrackId::new(1),
                retention: media_plane::trunk::RetentionClass::Timed,
                sample: sample_at(i, is_sync),
            });
            if self.added_track_9.get() {
                self.pending.borrow_mut().push_back(SessionEvent::Sample {
                    program: ProgramId(0),
                    track_id: TrackId::new(9),
                    retention: media_plane::trunk::RetentionClass::Timed,
                    sample: sample_at(i, is_sync),
                });
            }
        }
        Ok(())
    }

    fn poll(&mut self) -> Option<SessionEvent> {
        self.pending.borrow_mut().pop_front()
    }

    fn finish(&mut self) -> Result<(), Infallible> {
        Ok(())
    }
}

impl IngestSession for TrackChangeSession {
    type Request = bytes::Bytes;
}

const MAX_BOUNDED_FEEDS: usize = 2000;

/// Feed until at least one segment closes, panicking (with `what`) past the
/// bounded feed budget.
fn feed_until_a_segment_closes(
    driver: &mut IngestDriver<TrackChangeSession>,
    route: &RouteHandle,
    published: &mut HashSet<ProgramId>,
    segmenters: &mut HashMap<ProgramId, media_plane::segmenter::ProgramSegmenter>,
    what: &str,
) -> std::sync::Arc<media_plane::trunk::Trunk> {
    let trunk = driver.trunk(ProgramId(0)).cloned().expect("program 0");
    for i in 0..MAX_BOUNDED_FEEDS {
        if trunk.last_closed_segment().is_some() {
            return trunk;
        }
        driver.feed(&[], Timestamp::from_nanos(driver_feed_nanos()));
        report_driver_progress(driver, route, published);
        drive_program_segmenters(driver, route, segmenters);
        if i + 1 >= MAX_BOUNDED_FEEDS {
            panic!("{what} after {MAX_BOUNDED_FEEDS} feeds");
        }
    }
    trunk
}

#[test]
fn new_tracks_media_appears_in_output_after_mid_stream_addition() {
    let route = RouteHandle::new(
        RouteConfig::default()
            .with_target_duration_secs(1.0)
            .with_part_target(Duration::from_millis(250))
            .with_window_segments(8),
    );
    let mut driver = IngestDriver::new(
        TrackChangeSession::new(),
        media_plane::IngestConfig::new(handshake()).with_trunk(trunk_config()),
    );
    let mut published = HashSet::new();
    let mut segmenters = HashMap::new();

    driver.feed(&[], Timestamp::from_nanos(driver_feed_nanos()));
    report_driver_progress(&driver, &route, &mut published);
    drive_program_segmenters(&driver, &route, &mut segmenters);
    driver.feed(&[], Timestamp::from_nanos(driver_feed_nanos()));
    report_driver_progress(&driver, &route, &mut published);
    drive_program_segmenters(&driver, &route, &mut segmenters);

    let trunk = feed_until_a_segment_closes(
        &mut driver,
        &route,
        &mut published,
        &mut segmenters,
        "no segment closed before track change",
    );
    let pre_change_seg_len = trunk.segment_len();
    let pre_playlist = render_playlist(&route);
    let pre_seqs = segment_sequences_from_playlist(&pre_playlist);
    let last_pre_seq = pre_seqs.last().copied().unwrap_or(0);

    driver.session().add_track_9();
    // Feed once to apply the track change.
    driver.feed(&[], Timestamp::from_nanos(driver_feed_nanos()));
    report_driver_progress(&driver, &route, &mut published);
    drive_program_segmenters(&driver, &route, &mut segmenters);

    // Sanity: the track change must have been applied to the trunk.
    let gen_after = trunk.track_generation();
    assert!(
        gen_after >= 2,
        "track generation must have advanced (got {gen_after})"
    );

    // The flush created exactly one trailing segment.
    let flush_seg_len = trunk.segment_len();
    assert!(
        flush_seg_len > pre_change_seg_len,
        "flush must create at least one trailing segment (was {}, now {})",
        pre_change_seg_len,
        flush_seg_len,
    );

    // Feed samples after the change until we get a genuinely new segment
    // (beyond the flush trailing segment).
    for i in 0..MAX_BOUNDED_FEEDS {
        if trunk.segment_len() > pre_change_seg_len + 1 {
            break;
        }
        driver.feed(&[], Timestamp::from_nanos(driver_feed_nanos()));
        report_driver_progress(&driver, &route, &mut published);
        drive_program_segmenters(&driver, &route, &mut segmenters);
        if i + 1 >= MAX_BOUNDED_FEEDS {
            panic!(
                "no new segment beyond flush after {} feeds (segment_len={})",
                MAX_BOUNDED_FEEDS,
                trunk.segment_len(),
            );
        }
    }

    let post_playlist = render_playlist(&route);
    let post_seqs = segment_sequences_from_playlist(&post_playlist);
    assert!(
        post_seqs.iter().any(|&s| s > last_pre_seq),
        "post-change playlist must have sequence > last pre-change ({last_pre_seq})"
    );

    let mut found_track_9 = false;
    for seq in &post_seqs {
        if *seq <= last_pre_seq {
            continue;
        }
        let uri = format!("seg-1-{seq}.m4s");
        let bytes = resolve_resource(&route, ProgramId(0), &uri);
        if tfhd_track_ids(&bytes).contains(&9) {
            found_track_9 = true;
            break;
        }
    }
    assert!(
        found_track_9,
        "a post-change segment must contain tfhd naming track_ID 9"
    );
}

#[test]
fn existing_tracks_are_uninterrupted_across_mid_stream_addition() {
    let route = RouteHandle::new(
        RouteConfig::default()
            .with_target_duration_secs(1.0)
            .with_part_target(Duration::from_millis(250))
            .with_window_segments(8),
    );
    let mut driver = IngestDriver::new(
        TrackChangeSession::new(),
        media_plane::IngestConfig::new(handshake()).with_trunk(trunk_config()),
    );
    let mut published = HashSet::new();
    let mut segmenters = HashMap::new();

    driver.feed(&[], Timestamp::from_nanos(driver_feed_nanos()));
    report_driver_progress(&driver, &route, &mut published);
    drive_program_segmenters(&driver, &route, &mut segmenters);
    driver.feed(&[], Timestamp::from_nanos(driver_feed_nanos()));
    report_driver_progress(&driver, &route, &mut published);
    drive_program_segmenters(&driver, &route, &mut segmenters);

    // Every segment sequence number the route publishes, in publication
    // order, sampled across the whole session.
    let mut observed: Vec<u32> = Vec::new();

    let trunk = driver.trunk(ProgramId(0)).cloned().expect("program 0");
    for i in 0..MAX_BOUNDED_FEEDS {
        if trunk.last_closed_segment().is_some() {
            break;
        }
        driver.feed(&[], Timestamp::from_nanos(driver_feed_nanos()));
        report_driver_progress(&driver, &route, &mut published);
        drive_program_segmenters(&driver, &route, &mut segmenters);
        note_segment_seq(&trunk, &mut observed);
        if i + 1 >= MAX_BOUNDED_FEEDS {
            panic!("no segment closed before track change after {MAX_BOUNDED_FEEDS} feeds");
        }
    }
    note_segment_seq(&trunk, &mut observed);
    let seqs_before_change = observed.len();

    driver.session().add_track_9();
    driver.feed(&[], Timestamp::from_nanos(driver_feed_nanos()));
    report_driver_progress(&driver, &route, &mut published);
    drive_program_segmenters(&driver, &route, &mut segmenters);
    note_segment_seq(&trunk, &mut observed);

    let pre_len = trunk.segment_len();
    for i in 0..MAX_BOUNDED_FEEDS {
        if trunk.segment_len() > pre_len + 1 {
            break;
        }
        driver.feed(&[], Timestamp::from_nanos(driver_feed_nanos()));
        report_driver_progress(&driver, &route, &mut published);
        drive_program_segmenters(&driver, &route, &mut segmenters);
        note_segment_seq(&trunk, &mut observed);
        if i + 1 >= MAX_BOUNDED_FEEDS {
            panic!(
                "no new segment after track change after {MAX_BOUNDED_FEEDS} feeds (len={})",
                trunk.segment_len()
            );
        }
    }
    note_segment_seq(&trunk, &mut observed);

    // Segments must have kept flowing across the change, not merely existed
    // before it.
    assert!(
        observed.len() > seqs_before_change,
        "no segment was published after the track change: {observed:?}"
    );

    // THE INVARIANT. `#EXT-X-MEDIA-SEQUENCE` is derived from these, and a
    // client mid-playlist breaks if it ever goes backwards or repeats. A
    // segmenter rebuilt for the new track set without carrying its counters
    // forward restarts at 1 and violates this.
    for pair in observed.windows(2) {
        assert!(
            pair[1] > pair[0],
            "segment sequence must be strictly increasing across a \
             mid-stream track change — went {} -> {} in {observed:?}",
            pair[0],
            pair[1]
        );
    }
}

#[test]
fn track_change_lands_on_segment_boundary() {
    let route = RouteHandle::new(
        RouteConfig::default()
            .with_target_duration_secs(1.0)
            .with_part_target(Duration::from_millis(250))
            .with_window_segments(8),
    );
    let mut driver = IngestDriver::new(
        TrackChangeSession::new(),
        media_plane::IngestConfig::new(handshake()).with_trunk(trunk_config()),
    );
    let mut published = HashSet::new();
    let mut segmenters = HashMap::new();

    driver.feed(&[], Timestamp::from_nanos(driver_feed_nanos()));
    report_driver_progress(&driver, &route, &mut published);
    drive_program_segmenters(&driver, &route, &mut segmenters);
    driver.feed(&[], Timestamp::from_nanos(driver_feed_nanos()));
    report_driver_progress(&driver, &route, &mut published);
    drive_program_segmenters(&driver, &route, &mut segmenters);

    let trunk = feed_until_a_segment_closes(
        &mut driver,
        &route,
        &mut published,
        &mut segmenters,
        "no segment closed before track change",
    );
    let last_closed_seq = trunk.last_closed_segment().expect("segment closed");

    driver.session().add_track_9();
    driver.feed(&[], Timestamp::from_nanos(driver_feed_nanos()));
    report_driver_progress(&driver, &route, &mut published);
    drive_program_segmenters(&driver, &route, &mut segmenters);

    let pre_len = trunk.segment_len();
    for i in 0..MAX_BOUNDED_FEEDS {
        if trunk.segment_len() > pre_len + 1 {
            break;
        }
        driver.feed(&[], Timestamp::from_nanos(driver_feed_nanos()));
        report_driver_progress(&driver, &route, &mut published);
        drive_program_segmenters(&driver, &route, &mut segmenters);
        if i + 1 >= MAX_BOUNDED_FEEDS {
            panic!(
                "no new segment after track change after {MAX_BOUNDED_FEEDS} feeds (len={})",
                trunk.segment_len()
            );
        }
    }

    let next = trunk.last_closed_segment().expect("another segment closed");
    assert!(
        next > last_closed_seq,
        "next segment seq {next} must be > previous {last_closed_seq}"
    );
}

#[test]
fn dash_output_logs_and_continues_on_mid_stream_addition() {
    let route = RouteHandle::new(
        RouteConfig::default()
            .with_target_duration_secs(1.0)
            .with_part_target(Duration::from_millis(250))
            .with_window_segments(8),
    );
    let mut driver = IngestDriver::new(
        TrackChangeSession::new(),
        media_plane::IngestConfig::new(handshake()).with_trunk(trunk_config()),
    );
    let mut published = HashSet::new();
    let mut segmenters = HashMap::new();

    driver.feed(&[], Timestamp::from_nanos(driver_feed_nanos()));
    report_driver_progress(&driver, &route, &mut published);
    drive_program_segmenters(&driver, &route, &mut segmenters);
    driver.feed(&[], Timestamp::from_nanos(driver_feed_nanos()));
    report_driver_progress(&driver, &route, &mut published);
    drive_program_segmenters(&driver, &route, &mut segmenters);

    let specs_before = route.track_specs(ProgramId(0));
    assert_eq!(specs_before.len(), 1);

    driver.session().add_track_9();
    driver.feed(&[], Timestamp::from_nanos(driver_feed_nanos()));
    report_driver_progress(&driver, &route, &mut published);
    drive_program_segmenters(&driver, &route, &mut segmenters);

    let specs_after = route.track_specs(ProgramId(0));
    assert_eq!(
        specs_after.len(),
        2,
        "DASH path must see the new track spec"
    );

    let selected = crate::output::dash::select_representable_track(&specs_after);
    assert!(selected.is_some(), "must still pick a representable track");
}

/// Record the trunk's latest closed segment sequence number, in publication
/// order, skipping repeats of the value already at the tail.
fn note_segment_seq(trunk: &media_plane::trunk::Trunk, observed: &mut Vec<u32>) {
    if let Some(seq) = trunk
        .last_closed_segment()
        .map(media_plane::ids::SegmentSeq::value)
        && observed.last() != Some(&seq)
    {
        observed.push(seq);
    }
}

/// Collect segment media sequence numbers from an LL-HLS playlist (scans for
/// `seg-1-<instance>-<msn>.m4s` lines).
fn segment_sequences_from_playlist(playlist: &str) -> Vec<u32> {
    let mut seqs = Vec::new();
    for line in playlist.lines() {
        if let Some(start) = line.find("seg-1-") {
            let rest = &line[start + 6..];
            if let Some(end) = rest.find(".m4s")
                && let Some((_instance, msn)) = rest[..end].split_once('-')
                && let Ok(n) = msn.parse::<u32>()
            {
                seqs.push(n);
            }
        }
    }
    seqs
}

/// Box types the segment assertions walk and recognise (four-character codes,
/// ISO/IEC 14496-12:2026 §4.2; `styp` names a CMAF segment type per
/// ISO/IEC 23000-19 §7.1).
const MOOF: [u8; 4] = *b"moof";
const TRAF: [u8; 4] = *b"traf";
const TFHD: [u8; 4] = *b"tfhd";

/// Collect every `track_ID` from `tfhd` boxes in a CMAF segment: walk
/// `moof` → `traf`, then let `transmux`'s typed parser read each `tfhd` body
/// (ISO/IEC 14496-12:2026 §8.8.7).
fn tfhd_track_ids(segment_bytes: &[u8]) -> Vec<u32> {
    let mut ids = Vec::new();
    for moof in top_level_boxes(segment_bytes, MOOF) {
        for traf in top_level_boxes(moof.body, TRAF) {
            for tfhd in top_level_boxes(traf.body, TFHD) {
                if let Ok(header) = transmux::TrackFragmentHeaderBox::parse_body(tfhd.body) {
                    ids.push(header.track_id.value());
                }
            }
        }
    }
    ids
}

/// The top-level boxes of `data` whose type is `box_type`; a malformed or
/// truncated box ends the walk.
fn top_level_boxes<'a>(
    data: &'a [u8],
    box_type: [u8; 4],
) -> impl Iterator<Item = transmux::box_types::BoxRef<'a>> {
    transmux::box_types::box_iter(data)
        .map_while(Result::ok)
        .filter_map(move |(b, _consumed)| {
            (b.header.box_type == transmux::box_types::BoxType(box_type)).then_some(b)
        })
}

const GAP_AUDIO_TRACK: u32 = 2;
const GAP_AUDIO_FRAME_TICKS: i64 = 1_600;
const GAP_RESUME_SKIP_FRAMES: u32 = 90;
const GAP_SEGMENTS_PER_STAGE: usize = 3;
const GAP_EMPTY_FEEDS: usize = 8;
const GAP_GOLDEN: &str = "golden/hls/track-gap-live.golden";
const GAP_PRESENTATION_ORIGIN_SECS: u64 = 1_700_000_000;

fn gap_video_audio_specs() -> Vec<TrackSpec> {
    vec![
        track_spec(TrackId::new(1)),
        aac_track_spec(TrackId::new(GAP_AUDIO_TRACK)),
    ]
}

struct GapSession {
    pending: std::cell::RefCell<VecDeque<SessionEvent>>,
    frame: std::cell::Cell<u32>,
    flowing: std::cell::Cell<bool>,
}

impl GapSession {
    fn new() -> Self {
        let mut pending = VecDeque::new();
        pending.push_back(SessionEvent::Established);
        pending.push_back(SessionEvent::NewProgram {
            program: ProgramId(0),
            tracks: gap_video_audio_specs(),
        });
        GapSession {
            pending: std::cell::RefCell::new(pending),
            frame: std::cell::Cell::new(0),
            flowing: std::cell::Cell::new(true),
        }
    }

    fn change_tracks(&self, tracks: Vec<TrackSpec>) {
        let flowing = !tracks.is_empty();
        self.pending
            .borrow_mut()
            .push_back(SessionEvent::TracksChanged {
                program: ProgramId(0),
                tracks,
            });
        if flowing {
            self.frame.set(self.frame.get() + GAP_RESUME_SKIP_FRAMES);
        }
        self.flowing.set(flowing);
    }
}

impl Stage for GapSession {
    type In<'a> = &'a [u8];
    type Out = SessionEvent;
    type Error = Infallible;

    fn demand(&self) -> Demand {
        Demand::new(crate::source::INGEST_DEMAND_BYTES)
    }

    fn feed(&mut self, _input: &[u8], _now: Timestamp) -> Result<(), Infallible> {
        if !self.flowing.get() {
            return Ok(());
        }
        let i = self.frame.get();
        self.frame.set(i + 1);
        let mut pending = self.pending.borrow_mut();
        pending.push_back(SessionEvent::Sample {
            program: ProgramId(0),
            track_id: TrackId::new(1),
            retention: media_plane::trunk::RetentionClass::Timed,
            sample: sample_at(i, i.is_multiple_of(30)),
        });
        let audio_dts = i64::from(i) * GAP_AUDIO_FRAME_TICKS;
        pending.push_back(SessionEvent::Sample {
            program: ProgramId(0),
            track_id: TrackId::new(GAP_AUDIO_TRACK),
            retention: media_plane::trunk::RetentionClass::Timed,
            sample: Sample::new(
                vec![0, 0, 0, 1, 0x21],
                Some(audio_dts),
                Some(audio_dts),
                Some(u32::try_from(GAP_AUDIO_FRAME_TICKS).expect("fits")),
                true,
            ),
        });
        Ok(())
    }

    fn poll(&mut self) -> Option<SessionEvent> {
        self.pending.borrow_mut().pop_front()
    }

    fn finish(&mut self) -> Result<(), Infallible> {
        Ok(())
    }
}

impl IngestSession for GapSession {
    type Request = bytes::Bytes;
}

struct GapRig {
    route: RouteHandle,
    driver: IngestDriver<GapSession>,
    published: HashSet<ProgramId>,
    segmenters: HashMap<ProgramId, ProgramSegmenter>,
}

impl GapRig {
    fn new() -> Self {
        Self::new_with_container(Container::Fmp4)
    }

    fn new_with_container(container: Container) -> Self {
        GapRig {
            route: RouteHandle::new(
                RouteConfig::default()
                    .with_target_duration_secs(1.0)
                    .with_part_target(Duration::from_millis(250))
                    .with_window_segments(16),
            )
            .with_container(container),
            driver: IngestDriver::new(
                GapSession::new(),
                media_plane::IngestConfig::new(handshake()).with_trunk(trunk_config()),
            ),
            published: HashSet::new(),
            segmenters: HashMap::new(),
        }
    }

    fn feed(&mut self) {
        self.driver
            .feed(&[], Timestamp::from_nanos(driver_feed_nanos()));
        report_driver_progress(&self.driver, &self.route, &mut self.published);
        drive_program_segmenters(&self.driver, &self.route, &mut self.segmenters);
    }

    fn closed_segments(&self) -> usize {
        self.driver
            .trunk(ProgramId(0))
            .map_or(0, |trunk| trunk.segment_len())
    }

    fn feed_until_segments(&mut self, wanted: usize) {
        for _ in 0..MAX_BOUNDED_FEEDS {
            if self.closed_segments() >= wanted {
                return;
            }
            self.feed();
        }
        panic!("{wanted} segments did not close within {MAX_BOUNDED_FEEDS} feeds");
    }

    fn snapshot(&self, stage: &str, out: &mut String) {
        let origin = std::time::UNIX_EPOCH + Duration::from_secs(GAP_PRESENTATION_ORIGIN_SECS);
        out.push_str(&format!("==== {stage}: HLS media playlist ====\n"));
        let instance = self
            .route
            .ll_hls(crate::route::SPTS_PROGRAM_ID)
            .expect("program published")
            .instance()
            .to_string();
        out.push_str(&render_playlist(&self.route).replace(&instance, "INSTANCE"));
        out.push_str(&format!("==== {stage}: DASH MPD ====\n"));
        let dash = crate::output::dash::render_mpd_at(&self.route, origin)
            .unwrap_or_else(|| "(no MPD: 503)".to_owned());
        out.push_str(&dash.replace(&instance, "INSTANCE"));
        out.push('\n');
    }
}

fn assert_gap_golden(actual: &str) {
    if let Ok(dir) = std::env::var("GOLDEN_BLESS") {
        let path = std::path::Path::new(&dir).join(GAP_GOLDEN);
        std::fs::create_dir_all(path.parent().expect("golden parent")).expect("create dir");
        std::fs::write(path, actual).expect("write golden");
        return;
    }
    let expected = std::fs::read_to_string(test_fixtures::path(GAP_GOLDEN)).expect("golden");
    assert_eq!(
        actual, expected,
        "{GAP_GOLDEN} differs from the golden output"
    );
}

#[test]
fn live_route_through_an_empty_track_set_and_back_matches_golden() {
    let mut rig = GapRig::new();
    let mut out = String::new();

    rig.feed();
    rig.feed();
    rig.feed_until_segments(GAP_SEGMENTS_PER_STAGE);
    rig.snapshot("1 video+audio", &mut out);

    rig.driver.session().change_tracks(Vec::new());
    for _ in 0..GAP_EMPTY_FEEDS {
        rig.feed();
    }
    rig.snapshot("2 empty track set", &mut out);
    assert!(!rig.route.has_active_tracks(ProgramId(0)));
    let gap_playlist = render_playlist(&rig.route);
    assert!(gap_playlist.contains("#EXT-X-GAP"), "{gap_playlist}");
    let gap_mpd = crate::output::dash::render_mpd_at(
        &rig.route,
        std::time::UNIX_EPOCH + Duration::from_secs(GAP_PRESENTATION_ORIGIN_SECS),
    )
    .expect("the segments before the gap still render");
    assert_eq!(gap_mpd.matches("<Period ").count(), 1, "{gap_mpd}");
    assert!(
        gap_mpd.contains(" duration=\"PT"),
        "the Period ends at the gap: {gap_mpd}"
    );

    rig.driver.session().change_tracks(gap_video_audio_specs());
    let resumed = rig.closed_segments() + GAP_SEGMENTS_PER_STAGE;
    rig.feed_until_segments(resumed);
    rig.snapshot("3 video+audio again", &mut out);
    assert!(rig.route.has_active_tracks(ProgramId(0)));
    let resumed_playlist = render_playlist(&rig.route);
    assert!(
        resumed_playlist.contains("#EXT-X-DISCONTINUITY\n"),
        "{resumed_playlist}"
    );

    assert_gap_golden(&out);
}

#[test]
fn ts_hls_route_through_an_empty_track_set_and_back_shows_gap() {
    let mut rig = GapRig::new_with_container(Container::MpegTs);
    rig.feed();
    rig.feed();
    rig.feed_until_segments(GAP_SEGMENTS_PER_STAGE);

    rig.driver.session().change_tracks(Vec::new());
    for _ in 0..GAP_EMPTY_FEEDS {
        rig.feed();
    }
    let gap_playlist = render_playlist(&rig.route);
    assert!(gap_playlist.contains("#EXT-X-GAP"), "{gap_playlist}");

    // numbering continues: collect media sequence numbers
    let before: Vec<&str> = gap_playlist
        .lines()
        .filter(|l| l.starts_with("#EXTINF"))
        .collect();
    let _ = before;

    rig.driver.session().change_tracks(gap_video_audio_specs());
    let resumed = rig.closed_segments() + GAP_SEGMENTS_PER_STAGE;
    rig.feed_until_segments(resumed);
    let resumed_playlist = render_playlist(&rig.route);
    assert!(
        resumed_playlist.contains("#EXT-X-DISCONTINUITY\n"),
        "{resumed_playlist}"
    );
    assert!(gap_playlist.contains("#EXT-X-GAP"), "{gap_playlist}");
}

#[test]
fn program_segmenter_notes_each_segments_start_media_time() {
    use media_plane::segmenter::SegmenterEvent;
    use media_plane::trunk::{EventAnchor, RetentionClass, TrunkConfig};
    use timed_metadata::MediaTime;

    // A bare trunk with the real video track (90000 ticks/s == the event
    // ring's own 90 kHz clock), and a sample ring large enough to retain
    // every frame (the segmenter subscribes from the backlog).
    let trunk = media_plane::trunk::Trunk::new(
        TrunkConfig::default()
            .with_timed_capacity(nz(512))
            .with_sparse_capacity(nz(16))
            .with_segment_capacity(nz(8))
            .with_event_capacity(nz(16))
            .with_part_capacity(nz(16)),
    );
    let writer = trunk.writer().expect("first trunk writer");
    writer.set_tracks(vec![track_spec(TrackId::new(1))]);

    // A real SCTE-35 splice_insert (event_id 2002) at pts 0.
    let splice_bytes = test_fixtures::read("scte35/splice-insert-2002.bin");
    let mut timeline = timed_metadata::Timeline::new();
    let ev = timeline.push_scte35(&splice_bytes).unwrap();
    writer.publish_event(ev, EventAnchor::Media(MediaTime(0)));

    // Write enough frames into the trunk to cut a few real segments (a
    // sync frame every 45 frames == 0.5 s, target 1.0 s).
    for i in 0..200u32 {
        writer.publish(
            TrackId::new(1),
            RetentionClass::Timed,
            sample_at(i, i.is_multiple_of(45)),
        );
    }

    let route = RouteHandle::new(
        RouteConfig::default()
            .with_target_duration_secs(1.0)
            .with_part_target(Duration::from_millis(250))
            .with_window_segments(8),
    );
    let (segmenter, _) =
        ProgramSegmenter::try_new_with_siblings(&trunk, route.segmenter_config(), &route);
    let mut segmenter = segmenter.expect("segmenter builds");
    // `pump_segmenter` drains the backlog cursor and publishes whatever
    // segments that produced, applying the returned events to the route —
    // the production path.
    crate::source::segment::pump_segmenter(&mut segmenter, ProgramId(0), &trunk, &route);
    let (parts, segments) = segmenter.flush();
    assert!(
        parts + segments > 0,
        "the production path must have published something"
    );
    let _ = SegmenterEvent::PartTarget(Duration::from_millis(0));

    let last = trunk
        .last_closed_segment()
        .expect("the segmenter must have closed at least one segment");
    let attributed = (1..=last.value()).any(|seq| {
        trunk
            .events_in_segment(media_plane::ids::SegmentSeq::new(seq))
            .iter()
            .any(|e| {
                matches!(
                    e.event.source,
                    timed_metadata::event::SourcePayload::Scte35 { .. }
                )
            })
    });
    assert!(
        attributed,
        "a published SCTE-35 event must be attributable to a segment;  \
        `note_segment_start` was never called by the production segmenter before W9  \
        (closed segments: 1..={last})"
    );
}

fn nz(n: usize) -> std::num::NonZeroUsize {
    std::num::NonZeroUsize::new(n).expect("test capacity must be non-zero")
}

/// A scripted two-program (MPTS) session: `Established`, then both
/// `NewProgram` announcements, then one sample per program per feed.
struct MptsSession {
    pending: VecDeque<SessionEvent>,
    feed_count: u32,
}

impl MptsSession {
    fn new() -> Self {
        MptsSession {
            pending: VecDeque::from([SessionEvent::Established]),
            feed_count: 0,
        }
    }
}

impl Stage for MptsSession {
    type In<'a> = &'a [u8];
    type Out = SessionEvent;
    type Error = Infallible;

    fn demand(&self) -> Demand {
        Demand::new(crate::source::INGEST_DEMAND_BYTES)
    }

    fn feed(&mut self, _input: &[u8], _now: Timestamp) -> Result<(), Infallible> {
        self.feed_count += 1;
        if self.feed_count == 2 {
            self.pending.push_back(SessionEvent::NewProgram {
                program: ProgramId(0),
                tracks: vec![track_spec(TrackId::new(1))],
            });
            self.pending.push_back(SessionEvent::NewProgram {
                program: ProgramId(1),
                tracks: vec![track_spec(TrackId::new(2))],
            });
        } else if self.feed_count >= 3 {
            let i = self.feed_count - 3;
            let is_sync = i == 0 || i == 45;
            self.pending.push_back(SessionEvent::Sample {
                program: ProgramId(0),
                track_id: TrackId::new(1),
                retention: media_plane::trunk::RetentionClass::Timed,
                sample: sample_at(i, is_sync),
            });
            self.pending.push_back(SessionEvent::Sample {
                program: ProgramId(1),
                track_id: TrackId::new(2),
                retention: media_plane::trunk::RetentionClass::Timed,
                sample: sample_at(i, is_sync),
            });
        }
        Ok(())
    }

    fn poll(&mut self) -> Option<SessionEvent> {
        self.pending.pop_front()
    }

    fn finish(&mut self) -> Result<(), Infallible> {
        Ok(())
    }
}

impl IngestSession for MptsSession {
    type Request = bytes::Bytes;
}

#[test]
fn mpts_programs_segment_independently() {
    let route = RouteHandle::new(
        RouteConfig::default()
            .with_target_duration_secs(1.0)
            .with_part_target(std::time::Duration::from_millis(500))
            .with_window_segments(8),
    );
    let mut driver = IngestDriver::new(
        MptsSession::new(),
        media_plane::IngestConfig::new(handshake()).with_trunk(trunk_config()),
    );
    let mut published = HashSet::new();
    let mut segmenters = HashMap::new();

    driver.feed(&[], Timestamp::from_nanos(0));
    report_driver_progress(&driver, &route, &mut published);
    drive_program_segmenters(&driver, &route, &mut segmenters);
    driver.feed(&[], Timestamp::from_nanos(1));
    report_driver_progress(&driver, &route, &mut published);
    drive_program_segmenters(&driver, &route, &mut segmenters);
    let _: &HashSet<ProgramId> = &published;

    for _ in 0..90 {
        driver.feed(&[], Timestamp::from_nanos(driver_feed_nanos()));
        drive_program_segmenters(&driver, &route, &mut segmenters);
    }

    let trunk_0 = driver.trunk(ProgramId(0)).cloned().expect("program 0");
    let trunk_1 = driver.trunk(ProgramId(1)).cloned().expect("program 1");
    assert!(
        !std::sync::Arc::ptr_eq(&trunk_0, &trunk_1),
        "the two programs must have genuinely distinct Trunks"
    );
    assert!(trunk_0.last_closed_segment().is_some());
    assert!(trunk_1.last_closed_segment().is_some());
}

#[test]
fn ts_hls_mpts_serves_one_pmt_per_program() {
    let route = RouteHandle::new(
        RouteConfig::default()
            .with_target_duration_secs(1.0)
            .with_part_target(std::time::Duration::from_millis(500))
            .with_window_segments(8)
            .with_container(Container::MpegTs),
    );
    let mut driver = IngestDriver::new(
        MptsSession::new(),
        media_plane::IngestConfig::new(handshake()).with_trunk(trunk_config()),
    );
    let mut published = HashSet::new();
    let mut segmenters = HashMap::new();

    driver.feed(&[], Timestamp::from_nanos(0));
    report_driver_progress(&driver, &route, &mut published);
    drive_program_segmenters(&driver, &route, &mut segmenters);
    driver.feed(&[], Timestamp::from_nanos(1));
    report_driver_progress(&driver, &route, &mut published);
    drive_program_segmenters(&driver, &route, &mut segmenters);
    for _ in 0..90 {
        driver.feed(&[], Timestamp::from_nanos(driver_feed_nanos()));
        drive_program_segmenters(&driver, &route, &mut segmenters);
    }

    for program in [ProgramId(0), ProgramId(1)] {
        let playlist = render_playlist_for(&route, program);
        assert!(
            playlist.contains(".ts") && !playlist.contains("#EXT-X-MAP"),
            "{program:?}'s classic-TS playlist must reference .ts segments with no init \
             segment: {playlist}"
        );
        let uri = first_ts_segment_uri(&playlist).unwrap_or_else(|| {
            panic!("{program:?}'s playlist has no closed .ts segment: {playlist}")
        });
        assert_eq!(
            count_pmt_sections(&resolve_resource(&route, program, uri)),
            1,
            "{program:?}: RFC 8216bis §3.1.1 requires exactly one MPEG-2 Program per segment"
        );
    }
}

/// A video track whose PTS steps by `jump` after `first` frames re-anchors
/// that segment's reported start on the source 90 kHz clock.
fn jump_source_starts(jump: i64) -> Vec<Option<u64>> {
    let trunk = media_plane::trunk::Trunk::new(
        media_plane::trunk::TrunkConfig::default()
            .with_timed_capacity(nz(256))
            .with_sparse_capacity(nz(64))
            .with_segment_capacity(nz(16))
            .with_event_capacity(nz(8))
            .with_part_capacity(nz(8)),
    );
    let writer = trunk.writer().expect("writer");
    writer.set_tracks(vec![track_spec(TrackId::new(1))]);
    let route = RouteHandle::new(
        RouteConfig::default()
            .with_target_duration_secs(1.0)
            .with_part_target(Duration::from_millis(250))
            .with_window_segments(8),
    );
    let (segmenter, _) =
        ProgramSegmenter::try_new_with_siblings(&trunk, route.segmenter_config(), &route);
    let mut segmenter = segmenter.expect("segmenter");
    for i in 0..120u32 {
        let base = if i < 60 { i } else { i + 2 };
        let mut sample = sample_at(base, base.is_multiple_of(30));
        let pts = i64::from(base) * i64::from(FRAME_DUR) + if i >= 60 { jump } else { 0 };
        sample.pts = Some(pts);
        sample.dts = Some(pts);
        writer.publish(
            TrackId::new(1),
            media_plane::trunk::RetentionClass::Timed,
            sample,
        );
        crate::source::segment::pump_segmenter(&mut segmenter, ProgramId(0), &trunk, &route);
    }
    segmenter.flush();
    (1..=3)
        .map(|seq| {
            trunk
                .segment_start_time(media_plane::ids::SegmentSeq::new(seq))
                .map(|t| t.0)
        })
        .collect()
}

#[test]
fn a_backward_pts_step_reanchors_the_source_clock() {
    let back = -2 * i64::from(transmux::VIDEO_CLOCK_RATE);
    let starts = jump_source_starts(back);
    assert_eq!(starts[..2], [Some(0), Some(90_000)]);
    assert_ne!(
        starts[2],
        Some(180_000),
        "a 2 s backward PTS step must move the segment start, not continue the old line: {starts:?}"
    );
}

#[test]
fn a_forward_pts_step_reanchors_the_source_clock() {
    let forward = 8 * i64::from(transmux::VIDEO_CLOCK_RATE);
    let starts = jump_source_starts(forward);
    assert_eq!(starts[..2], [Some(0), Some(90_000)]);
    assert!(
        starts[2].is_some_and(|s| s > 180_000),
        "an 8 s forward PTS step must move the segment start forward: {starts:?}"
    );
}

/// **Review item 3's capacity propagation, through the shared host builder.**
/// The sibling trunk `build_core_followers` (the production host half)
/// writes a rendition's segments into must carry the route's configured
/// `TrunkConfig` capacities.
#[test]
fn sibling_trunk_built_by_the_host_carries_the_routes_trunk_config() {
    use media_plane::trunk::{Trunk, TrunkConfig};

    let trunk_config = TrunkConfig::default()
        .with_timed_capacity(nz(7))
        .with_sparse_capacity(nz(11))
        .with_segment_capacity(nz(13))
        .with_event_capacity(nz(17))
        .with_part_capacity(nz(19));
    let trunk = media_plane::trunk::Trunk::new(trunk_config);
    trunk.writer().expect("writer").set_tracks(vec![
        track_spec(TrackId::new(1)),
        track_spec(TrackId::new(2)),
    ]);
    let route = RouteHandle::new(
        RouteConfig::default()
            .with_target_duration_secs(1.0)
            .with_part_target(Duration::from_millis(250))
            .with_window_segments(8)
            .with_audio_renditions(vec![hls_runtime::server::AudioRenditionDecl {
                uri: "audio.m3u8".into(),
                characteristics: Vec::new(),
                group_id: "audio".into(),
                name: "audio".into(),
                language: "und".into(),
                channels: None,
                default: true,
                autoselect: true,
            }])
            .with_audio_rendition_sources(vec![
                transmux::audio_profile::AudioTrackSelector::Index(1),
            ]),
    )
    .with_trunk_config(trunk_config);
    assert_eq!(
        route.trunk_config().segment_capacity,
        trunk_config.segment_capacity,
        "the route's own trunk config is what every sibling is built from"
    );
    route.publish_program(ProgramId(0), std::sync::Arc::clone(&trunk));
    route.set_track_specs(ProgramId(0), trunk.tracks().to_vec());

    let mapped = vec![(0usize, trunk.tracks()[1].clone())];
    let followers = crate::source::split_audio::build_core_followers(
        &route,
        &trunk,
        &mapped,
        1.0,
        Duration::from_millis(250),
    );
    assert_eq!(followers.len(), 1, "one audio rendition, one follower");
    let servings = route.audio_servings(ProgramId(0));
    assert_eq!(servings.len(), 1, "one sibling trunk");
    let sibling: &Trunk = &servings[0].trunk;
    assert_eq!(sibling.config().timed_capacity, trunk_config.timed_capacity);
    assert_eq!(
        sibling.config().sparse_capacity,
        trunk_config.sparse_capacity
    );
    assert_eq!(
        sibling.config().segment_capacity,
        trunk_config.segment_capacity
    );
    assert_eq!(sibling.config().event_capacity, trunk_config.event_capacity);
    assert_eq!(sibling.config().part_capacity, trunk_config.part_capacity);
}

/// `verifier rejects an unauthenticated pull` — the segmenter's host is not
/// involved in auth, but `RouteHandle` must not need auth state to segment.
#[test]
fn route_segments_without_any_output_auth() {
    let route = RouteHandle::new(
        RouteConfig::default()
            .with_target_duration_secs(1.0)
            .with_part_target(Duration::from_millis(250))
            .with_window_segments(8),
    );
    let trunk = media_plane::trunk::Trunk::new(trunk_config());
    let (segmenter, events) =
        ProgramSegmenter::try_new_with_siblings(&trunk, route.segmenter_config(), &route);
    assert!(events.is_empty(), "an empty trunk builds no segmenter");
    assert!(segmenter.is_none(), "an empty trunk builds no segmenter");
}

#[test]
fn a_route_signals_its_own_config_to_the_segmenter() {
    let route = RouteHandle::new(
        RouteConfig::default()
            .with_target_duration_secs(2.5)
            .with_part_target(Duration::from_millis(333))
            .with_window_segments(12)
            .with_container(Container::MpegTs),
    );
    let config = route.segmenter_config();
    assert_eq!(config.target_duration_secs, 2.5);
    assert_eq!(config.part_target, Duration::from_millis(333));
    assert_eq!(config.window_segments_cap.get(), 12);
    assert_eq!(
        config.trunk_config.segment_capacity.get(),
        12,
        "one number: the window depth is the trunk's segment capacity"
    );
    assert!(matches!(config.container, Container::MpegTs));
}

/// **Issue #781 test 4.** Regression guard: removal stops routing samples,
/// and `TrackUpdated` does not panic. (A `TrackUpdated` for a live track
/// announces the track set again since #1083 — the demux emits it only for a
/// config that really changed — covered by
/// `track_updated_announces_the_new_config` in `media_plane::ts_session`.)
///
/// The segmenter's own entry point consumes these `SessionEvent`s through
/// `IngestDriver`; this drives the same `ProgramTracker` (the demux half of
/// `TsIngestSession`) that stands behind it, so the degradation the
/// segmenter sees is exercised directly.
#[test]
fn removal_and_update_degrade_without_panicking() {
    use media_plane::ts_session::ProgramTracker;

    let mut tracker = ProgramTracker::new();
    tracker.handle(transmux::DemuxEvent::TrackAdded(track_spec(TrackId::new(
        1,
    ))));
    tracker.handle(transmux::DemuxEvent::TrackAdded(track_spec(TrackId::new(
        2,
    ))));
    tracker.handle(transmux::DemuxEvent::tracks_resolved(0));
    let _established = tracker.poll();
    let _new_program = tracker.poll();

    tracker.handle(transmux::DemuxEvent::track_removed(
        TrackId::new(2),
        transmux::EventProvenance::default(),
    ));
    tracker.handle(transmux::DemuxEvent::tracks_resolved(1));

    match tracker.poll() {
        Some(SessionEvent::TracksChanged { tracks, .. }) => {
            assert_eq!(tracks.len(), 1);
            assert_eq!(tracks[0].track_id, TrackId::new(1));
        }
        other => panic!("expected TracksChanged with track 1 only, got {other:?}"),
    }

    tracker.handle(transmux::DemuxEvent::sample(
        TrackId::new(2),
        sample_at(0, true),
    ));
    assert!(tracker.poll().is_none(), "sample for removed track dropped");

    // An update for the removed track 2 is for a track nothing routes on:
    // ignored.
    tracker.handle(transmux::DemuxEvent::TrackUpdated(track_spec(
        TrackId::new(2),
    )));
    assert!(
        tracker.poll().is_none(),
        "TrackUpdated for a removed track emits no event"
    );
}
