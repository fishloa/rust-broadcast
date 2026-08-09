//! Ingest sources feeding the segmentation pipeline. `RtspSource` (RTSP
//! pull), `RtpUdpSource` (raw RTP over UDP, uni/multicast), `TsUdpSource`
//! (MPEG-2 TS over UDP, uni/multicast), `ts_http::TsHttpSource` (MPEG-2 TS
//! over HTTP), `hls_pull::HlsPullRoute` (pull a remote (LL-)HLS origin),
//! `dash_pull::DashPullRoute` (pull a remote MPEG-DASH origin, issue #758),
//! `smooth_pull::SmoothPullRoute` (pull a remote Microsoft Smooth Streaming
//! origin, issue #759), `rtmp::RtmpRoute` (RTMP push ingest, issue #738 — a
//! *push* source implementing `media_plane::ingress::Listener` since issue
//! #805 task 4; every other source above dials out), and `srt::SrtRoute`
//! (SRT-carried MPEG-2 TS ingest, issue #739 — listener *or* caller mode) all
//! implement the `Source` marker trait; every one — including
//! `InputSpec::Custom`, via a [`crate::registry::SchemeRegistry`]-provided
//! factory, since issue #805 task 5 deleted the old
//! `SourceConnector`/`supervise`/`pipeline` path — is driven over
//! `media_plane::ingress` (`Dialer`/`Listener` + `IngestSession`) by
//! [`crate::origin::supervisor::supervise_driver`]. [`advance_route`] is the
//! one per-iteration call every driver-backed `run_*` in this module makes
//! (and that a `SchemeRegistry`-registered `Custom` factory's own driver loop
//! must make too — see `examples/custom_scheme.rs`); it is `pub` for exactly
//! that reason, not just for this crate's own in-tree sources.
//! `report_driver_progress`/`segment::drive_program_segmenters` are the two
//! steps it bundles — both `pub(crate)` (issue #805 task 6 narrowed them back
//! from `pub`; see [`advance_route`]'s own doc for why one call replaced two).
//! `http_auth` is shared auth glue for the HTTP-based sources (issue #663
//! P3c).

pub mod dash_pull;
pub mod hls_pull;
pub mod http_auth;
pub mod rtmp;
pub mod rtp_udp;
pub mod rtsp;
pub mod sdp;
pub mod segment;
pub mod smooth_pull;
pub mod srt;
pub mod ts_http;
pub mod ts_program;
pub mod ts_udp;
pub(crate) mod udp;
// WHIP push input (issue #740) — needs `webrtc_runtime::media`, MSRV 1.88 (see
// the `whip` feature's doc in `Cargo.toml`); kept out of the default,
// MSRV-1.86-clean build entirely.
#[cfg(feature = "whip")]
pub mod whip;

use std::time::Duration;

/// Read-size hint every MPEG-2 TS transport reports via
/// [`broadcast_common::Stage::demand`], and the read-buffer size the
/// datagram transports allocate — comfortably above a typical 7×188-byte
/// (1316-byte) TS-over-UDP payload and any legal UDP datagram (65 507 bytes
/// over IPv4), so a single `recv` always captures a whole datagram.
pub const MAX_TS_READ: usize = 65_536;

/// Hard cap on concurrently in-flight HTTP fetches a pull source
/// (`hls_pull`/`dash_pull`/`smooth_pull`, plan step 5a round 3) keeps open at
/// once.
///
/// A pull source's sans-IO session can hand back many `poll_transmit`
/// requests in one drain — an LL-HLS playlist reload can reveal a dozen
/// already-available parts at once; a DASH/Smooth manifest refresh can extend
/// several Representations'/StreamIndexes' plans simultaneously — with
/// nothing in the session itself limiting how many the driver launches as
/// concurrent requests. This project has already shipped five
/// unbounded-allocation vectors in code driven by remote input (see
/// `media_plane::ingress`'s own `max_programs`/`max_sessions` docs); an
/// uncapped fan-out of concurrent fetches against a single origin is exactly
/// that class of bug (a hostile or malformed playlist/manifest could turn one
/// route into an unbounded number of open sockets), so each pull source's own
/// tokio drive loop launches at most this many fetches at once, queuing the
/// rest until a slot frees up — never blocking the sans-IO session from
/// producing more requests, only how many the IO side acts on concurrently.
pub const MAX_INFLIGHT_FETCHES: usize = 8;

/// `true` while a pull source's drive loop may launch one more concurrent
/// fetch — i.e. `inflight` is still below [`MAX_INFLIGHT_FETCHES`].
///
/// A named predicate rather than an inline `<` in each of the three loops so
/// the bound is one testable decision instead of three copies of a comparison
/// (the shape that lets one of them silently drift). Every
/// `source::{hls_pull, dash_pull, smooth_pull}` loop gates its
/// `JoinSet::spawn` on this.
pub fn may_spawn_fetch(inflight: usize) -> bool {
    inflight < MAX_INFLIGHT_FETCHES
}

#[cfg(test)]
mod inflight_tests {
    use super::{MAX_INFLIGHT_FETCHES, may_spawn_fetch};

    /// The in-flight cap actually caps. Bites on the two mutations that
    /// matter: dropping the gate (making this always `true`) and inverting
    /// the comparison.
    #[test]
    fn may_spawn_fetch_stops_exactly_at_the_cap() {
        assert!(may_spawn_fetch(0), "an idle loop must be able to spawn");
        assert!(
            may_spawn_fetch(MAX_INFLIGHT_FETCHES - 1),
            "one slot short of the cap must still spawn"
        );
        assert!(
            !may_spawn_fetch(MAX_INFLIGHT_FETCHES),
            "at the cap, no further fetch may be launched"
        );
        assert!(
            !may_spawn_fetch(MAX_INFLIGHT_FETCHES + 1),
            "past the cap (a caller that over-spawned) must not spawn more"
        );
    }
}

use transmux::pipeline::CodecConfig;
use transmux::rtp::RtpMediaKind;

/// Default bound on how long a source's `connect()` waits for the ingest
/// handshake to complete (TCP/TLS connect, plus any protocol handshake —
/// RTSP DESCRIBE/SETUP/PLAY, or waiting for the first PMT/init segment) —
/// issue #663 P5 (audit-ingest #3): a stalled/half-open server (accepts the
/// TCP connection but never replies) must not hang `connect()` forever,
/// starving [`crate::origin::supervisor::supervise_driver`]'s backoff of a
/// chance to retry.
pub const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Default bound on how long a source's per-read step (one RTSP interleaved
/// frame, one HTTP body chunk, one UDP datagram, one HLS-pull client output)
/// waits before the read is treated as a stall — issue #663 P5 (audit-ingest
/// #3): the supervisor already reconnects on an `Err`, but only if one is
/// ever produced; without a read timeout a source that goes silent (wedged
/// server, dropped multicast feed) never signals anything and the route
/// silently stops advancing forever. Generous relative to any real source's
/// normal packet cadence (even a low-bitrate stream sends *something* well
/// within 30 s) while still bounding a genuinely dead connection.
pub const DEFAULT_READ_TIMEOUT: Duration = Duration::from_secs(30);

/// Ingest connect/read timeout bounds (issue #663 P5, audit-ingest #3),
/// shared by every source kind so [`crate::config::Config`] only needs two
/// process-wide knobs rather than one pair per input type — mirrors
/// [`crate::origin::HttpLimits`]'s "one config-surfaced struct, sane
/// [`Default`], per-source `with_timeouts` builder" shape.
///
/// A source's `connect()` wraps its whole connect handshake in
/// [`Self::connect`]; its `next_samples()`/read loop wraps each individual
/// read in [`Self::read`]. Either expiring surfaces as a
/// [`crate::error::MultimuxError`], which
/// [`crate::origin::supervisor::supervise_driver`] treats exactly like any
/// other ingest error — log, mark the route reconnecting, retry with
/// backoff — never a silent hang.
#[derive(Debug, Clone, Copy)]
pub struct IngestTimeouts {
    /// Bound on the whole connect handshake.
    pub connect: Duration,
    /// Bound on a single read/receive step once connected.
    pub read: Duration,
}

impl Default for IngestTimeouts {
    fn default() -> Self {
        IngestTimeouts {
            connect: DEFAULT_CONNECT_TIMEOUT,
            read: DEFAULT_READ_TIMEOUT,
        }
    }
}

impl From<&crate::config::Config> for IngestTimeouts {
    fn from(cfg: &crate::config::Config) -> Self {
        IngestTimeouts {
            connect: Duration::from_secs_f64(cfg.ingest_connect_timeout_secs),
            read: Duration::from_secs_f64(cfg.ingest_read_timeout_secs),
        }
    }
}

// --- issue #805 task 2: driver-backed ingest <-> RouteHandle registry glue ---
//
// Shared by every `run_*` entry point in this module (`rtsp::run_rtsp`,
// `rtp_udp::run_rtp_udp`, `ts_udp::run_ts_udp`, `ts_http::run_ts_http`,
// `srt::drive_socket`, `hls_pull::run_hls_pull`, `dash_pull::run_dash_pull`,
// `smooth_pull::run_smooth_pull`) so none of them re-implements the "flip the
// route Live the first time the driver establishes" / "publish each
// newly-announced program" bookkeeping independently.

fn source_nz(n: usize) -> std::num::NonZeroUsize {
    std::num::NonZeroUsize::new(n).expect("source::mod.rs capacity constants are all non-zero")
}

/// Ring capacities for a driver-minted per-program `Trunk`
/// ([`driver_trunk_config`]) — chosen to match [`crate::route::RouteHandle`]'s
/// own defaults (that struct's own, private, `DEFAULT_*_CAPACITY` constants)
/// so a driver-backed route's `Trunk` behaves comparably to the legacy
/// segmenter-fed one, even though nothing here shares the constants directly
/// (a driver-minted `Trunk` is a distinct instance per program, never
/// `RouteHandle`'s own).
const DRIVER_TIMED_CAPACITY: usize = 64;
const DRIVER_SPARSE_CAPACITY: usize = 16;
const DRIVER_EVENT_CAPACITY: usize = 64;
const DRIVER_PART_CAPACITY: usize = 64;

/// Builds the [`media_plane::trunk::TrunkConfig`] every driver-backed `run_*`
/// entry point passes to its `IngestDriver::new` — see [`DRIVER_TIMED_CAPACITY`]
/// et al. for the chosen ring sizes. `window_segments` (the segment log's
/// capacity) is the one caller-supplied knob, mirroring
/// [`crate::config::Config::window_segments`]/[`crate::route::RouteHandle::new`]'s
/// own "advertised window == retained window" depth.
pub(crate) fn driver_trunk_config(window_segments: usize) -> media_plane::trunk::TrunkConfig {
    let window_segments =
        std::num::NonZeroUsize::new(window_segments).unwrap_or(std::num::NonZeroUsize::MIN);
    media_plane::trunk::TrunkConfig::new(
        source_nz(DRIVER_TIMED_CAPACITY),
        source_nz(DRIVER_SPARSE_CAPACITY),
        window_segments,
        source_nz(DRIVER_EVENT_CAPACITY),
        source_nz(DRIVER_PART_CAPACITY),
    )
}

/// Builds a production [`media_plane::ingress::HandshakePolicy`] bounding a
/// fresh session's handshake by `timeout`, expressed as nanoseconds-since-zero
/// rather than a real wall-clock [`broadcast_common::Timestamp`].
///
/// Every driver-backed `run_*` entry point measures its own
/// `Stage::feed`/`Stage::on_deadline` `now` from an internal `Instant` it
/// captures at entry (e.g. `rtsp::run_rtsp`'s own `start`) — an instant this
/// caller cannot observe in advance, since it doesn't exist until `run_*` is
/// actually called. Expressing the deadline as "nanoseconds since a start
/// near zero" (matching this crate's own test fixtures, e.g.
/// `ts_program::test_support::handshake`) rather than a predicted absolute
/// instant sidesteps that: both clocks start within microseconds of each
/// other in practice (this function runs immediately before the `run_*` call
/// it bounds), which is immaterial against a multi-second `timeout`.
pub(crate) fn handshake_policy(timeout: Duration) -> media_plane::ingress::HandshakePolicy {
    let nanos = u64::try_from(timeout.as_nanos()).unwrap_or(u64::MAX);
    media_plane::ingress::HandshakePolicy::establish_by(broadcast_common::Timestamp::from_nanos(
        nanos,
    ))
}

/// After draining a driver-backed session (`IngestDriver::feed`/
/// `on_deadline`), every `run_*` entry point calls this once per iteration to
/// keep `route_handle` in sync with what the driver has actually observed
/// (issue #805 task 2):
///
/// - The first time `driver.health()` reaches
///   [`media_plane::ingress::HealthState::Live`], flips `route_handle` to
///   [`crate::route::HealthState::Live`] — the driver-backed equivalent of
///   `origin::supervisor::supervise_driver`'s own health flip right after an
///   attempt reaches `Live`. Guarded on `route_handle.health()` rather than a
///   separate flag, since `route_handle`'s own health *is* the single source
///   of truth `crate::origin::supervisor::supervise_driver` reads back after
///   this attempt ends (see that function's own doc).
/// - Every [`media_plane::ingress::ProgramId`] `driver` has announced (via
///   `SessionEvent::NewProgram`) that this run hasn't already published gets
///   published into `route_handle`'s registry
///   (`RouteHandle::publish_program`, crate-private).
///
/// # `pub(crate)`, not `pub` (issue #805 task 6 narrowed this back)
///
/// This used to be `pub` so a [`crate::registry::SchemeRegistry`] `Custom`
/// factory driving its own `Dialer`/`IngestSession` could call it directly.
/// That left a plugin author hand-assembling `report_driver_progress` +
/// [`segment::drive_program_segmenters`] themselves, in the right order,
/// every iteration — a wrong order, or calling one without the other, could
/// silently ingest with nothing ever becoming servable. [`advance_route`] is
/// now the one supported call for that; this function (and
/// `drive_program_segmenters`) are its private implementation.
pub(crate) fn report_driver_progress<S: media_plane::ingress::IngestSession>(
    driver: &media_plane::ingress::IngestDriver<S>,
    route_handle: &crate::route::RouteHandle,
    published: &mut std::collections::HashSet<media_plane::ingress::ProgramId>,
    track_generations: &mut std::collections::HashMap<media_plane::ingress::ProgramId, u64>,
) {
    if matches!(driver.health(), media_plane::ingress::HealthState::Live)
        && route_handle.health() != crate::route::HealthState::Live
    {
        route_handle.set_health(crate::route::HealthState::Live);
    }
    for program in driver.programs() {
        if published.insert(program) {
            if let Some(trunk) = driver.trunk(program) {
                route_handle.publish_program(program, std::sync::Arc::clone(trunk));
            }
        }
    }
    // Sync track specs from each published program's trunk into the route
    // handle — the one piece of codec metadata the DASH/LL-DASH renderers
    // need that no Trunk ring holds (issue #831: this sync was missing,
    // shipping every driver-backed route with 503-forever DASH/LL-DASH).
    for program in driver.programs() {
        let Some(trunk) = driver.trunk(program) else {
            continue;
        };
        let generation = trunk.track_generation();
        if generation == 0 {
            continue;
        }
        let last = track_generations.get(&program).copied();
        if last == Some(generation) {
            continue;
        }
        let tracks = trunk.tracks();
        route_handle.set_track_specs(program, tracks.to_vec());
        track_generations.insert(program, generation);
    }
}

/// Opaque per-attempt state [`advance_route`] threads across every call for
/// one connection attempt: the dedup set `report_driver_progress` needs, plus
/// the `segment::ProgramSegmenter` map `segment::drive_program_segmenters`
/// needs. A caller (every in-tree `run_*`, or an external
/// [`crate::registry::SchemeRegistry`] `Custom` factory's own drive loop —
/// see `examples/custom_scheme.rs`) declares one fresh [`DriverProgress::new`]
/// per connection attempt and passes it, by `&mut` reference, to
/// [`advance_route`] on every iteration for that attempt's whole lifetime —
/// never constructing or reading either of its fields directly (both are
/// private; this type exists precisely so a caller never has to know their
/// shape).
#[derive(Default)]
pub struct DriverProgress {
    published: std::collections::HashSet<media_plane::ingress::ProgramId>,
    segmenters:
        std::collections::HashMap<media_plane::ingress::ProgramId, segment::ProgramSegmenter>,
    /// Last-seen [`media_plane::trunk::Trunk::track_generation`] per program
    /// — compared each call to avoid an unconditional `set_track_specs` on
    /// every poll (issue #831: a missing sync here shipped DASH/LL-DASH 503
    /// forever for every driver-backed route).
    track_generations: std::collections::HashMap<media_plane::ingress::ProgramId, u64>,
}

impl DriverProgress {
    /// Fresh, empty state for a new connection attempt.
    pub fn new() -> Self {
        DriverProgress::default()
    }
}

/// **The one facade call** a driver-backed drive loop makes once per
/// iteration, after every [`media_plane::ingress::IngestDriver::feed`]/
/// `on_deadline`/`finish` — replaces what used to be a caller-assembled pair,
/// `report_driver_progress` then `segment::drive_program_segmenters`,
/// over two separately-declared collections (`published`/`segmenters`) a
/// caller had to know to build, order correctly, and pass consistently.
///
/// Both steps still exist (as `pub(crate)` internals of this crate — see
/// their own docs) because they are genuinely two different jobs (registry
/// publish + health flip; sample-to-segment/part turning), but a caller
/// outside this crate has exactly one thing to call, over exactly one opaque
/// state value ([`DriverProgress`]), so the order can never be gotten wrong
/// and neither step can be silently skipped. See `examples/custom_scheme.rs`
/// for the supported shape this replaces (that example used to call
/// `report_driver_progress`/`drive_program_segmenters` directly; it now calls
/// only this).
pub fn advance_route<S: media_plane::ingress::IngestSession>(
    driver: &media_plane::ingress::IngestDriver<S>,
    route_handle: &crate::route::RouteHandle,
    state: &mut DriverProgress,
) {
    report_driver_progress(
        driver,
        route_handle,
        &mut state.published,
        &mut state.track_generations,
    );
    segment::drive_program_segmenters(driver, route_handle, &mut state.segmenters);
    // Drain DVR cursors for every published program — recording happens
    // after segmenters have published new segments to the Trunk.
    route_handle.drain_dvr();
}

#[cfg(test)]
mod driver_progress_tests {
    //! Coverage for [`report_driver_progress`] — the shared ingest-side
    //! registry/health bookkeeping every driver-backed `run_*` entry point
    //! calls (issue #805 task 2). Uses `media_plane::ingress`'s own
    //! `ScriptedSession`-style construction indirectly via a minimal fake
    //! `IngestSession`, so this is a fast, deterministic unit test rather
    //! than a real-socket loopback one.

    use super::*;
    use broadcast_common::{Demand, Stage, Timestamp};
    use media_plane::ingress::{
        Dialer, HandshakePolicy, IngestDriver, IngestSession, ProgramId, SessionEvent,
    };
    use media_plane::trunk::TrunkConfig;
    use std::collections::HashSet;

    fn nz(n: usize) -> std::num::NonZeroUsize {
        std::num::NonZeroUsize::new(n).unwrap()
    }

    fn trunk_config() -> TrunkConfig {
        TrunkConfig::new(nz(4), nz(4), nz(4), nz(4), nz(4))
    }

    /// A session that queues `Established` at construction, then a single
    /// `NewProgram { program: ProgramId(0), tracks: vec![] }` on its
    /// *second* `feed` call (not its first, which only drains `Established`)
    /// — enough to drive `report_driver_progress` through both of its jobs
    /// (health flip, then program publish) as two separate, observable
    /// steps, without a real transport.
    struct FakeSession {
        pending: std::collections::VecDeque<SessionEvent>,
        feed_count: u32,
    }

    impl Stage for FakeSession {
        type In<'a> = &'a [u8];
        type Out = SessionEvent;
        type Error = std::convert::Infallible;

        fn demand(&self) -> Demand {
            Demand::new(4096)
        }

        fn feed(&mut self, _input: &[u8], _now: Timestamp) -> Result<(), Self::Error> {
            self.feed_count += 1;
            if self.feed_count == 2 {
                self.pending.push_back(SessionEvent::NewProgram {
                    program: ProgramId(0),
                    tracks: Vec::new(),
                });
            }
            Ok(())
        }

        fn poll(&mut self) -> Option<SessionEvent> {
            self.pending.pop_front()
        }

        fn next_deadline(&self) -> Option<Timestamp> {
            None
        }

        fn on_deadline(&mut self, _now: Timestamp) {}

        fn finish(&mut self) -> Result<(), Self::Error> {
            Ok(())
        }
    }

    impl IngestSession for FakeSession {
        type Request = bytes::Bytes;
    }

    struct FakeDialer;

    impl Dialer for FakeDialer {
        type Session = FakeSession;
        type Error = std::convert::Infallible;

        fn dial(&mut self) -> Result<FakeSession, std::convert::Infallible> {
            let mut pending = std::collections::VecDeque::new();
            pending.push_back(SessionEvent::Established);
            Ok(FakeSession {
                pending,
                feed_count: 0,
            })
        }
    }

    fn driver() -> IngestDriver<FakeSession> {
        let mut dialer = FakeDialer;
        let session = dialer.dial().unwrap();
        IngestDriver::new(
            session,
            trunk_config(),
            HandshakePolicy::establish_by(Timestamp::from_nanos(u64::MAX)),
            nz(4),
        )
    }

    /// MUTATION VERIFIED: changing the health-flip guard from
    /// `route_handle.health() != crate::route::HealthState::Live` to `true`
    /// (always overwrite) still passes this specific assertion (both reach
    /// `Live`), but changing `matches!(driver.health(), ...HealthState::Live)`
    /// to unconditionally `false` (i.e. never flip on Live) makes this test
    /// fail: `assert_eq!(route.health(), crate::route::HealthState::Live)`
    /// fails, comparing actual `HealthState::Connecting` against expected
    /// `HealthState::Live` — the route never leaves its constructed default.
    /// Recompiled and re-run to confirm the failure, then reverted.
    #[test]
    fn first_live_flips_route_health_to_live() {
        let mut driver = driver();
        let route = crate::route::RouteHandle::new(4.0, 500, 4);
        let mut published = HashSet::new();
        let mut track_generations = std::collections::HashMap::new();

        // Establish: driver becomes Live once it drains SessionEvent::Established.
        driver.feed(&[], Timestamp::from_nanos(1));
        report_driver_progress(&driver, &route, &mut published, &mut track_generations);

        assert_eq!(
            route.health(),
            crate::route::HealthState::Live,
            "route must flip to Live the moment the driver itself reaches Live"
        );
    }

    /// MUTATION VERIFIED: changing `published.insert(program)` to always
    /// evaluate to `true` regardless of prior membership (i.e. dropping the
    /// dedup and calling `route_handle.publish_program` unconditionally every
    /// call) does not break this test's assertions (both are still
    /// `Found`+identical `Arc`), but changing the loop's body to skip calling
    /// `route_handle.publish_program` entirely (i.e. deleting the `if let
    /// Some(trunk) = driver.trunk(program) { ... }` publish) makes this test
    /// fail: `resolve_program` returns `NotYetAnnounced` (registry never
    /// populated), so `match ... { Found(_) => ..., other => panic!(...) }`
    /// panics naming the actual variant, not `Found`. Recompiled and re-run
    /// to confirm the failure, then reverted.
    #[test]
    fn new_program_is_published_into_the_registry() {
        let mut driver = driver();
        let route = crate::route::RouteHandle::new(4.0, 500, 4);
        let mut published = HashSet::new();
        let mut track_generations = std::collections::HashMap::new();

        driver.feed(&[], Timestamp::from_nanos(1)); // Established
        report_driver_progress(&driver, &route, &mut published, &mut track_generations);
        driver.feed(&[], Timestamp::from_nanos(2)); // NewProgram(0)
        report_driver_progress(&driver, &route, &mut published, &mut track_generations);

        let expected = driver.trunk(ProgramId(0)).expect("driver minted a Trunk");
        match route.resolve_program(crate::route::SPTS_PROGRAM_ID) {
            crate::route::ProgramResolution::Found(resolved) => {
                assert_eq!(
                    std::sync::Arc::as_ptr(&resolved.trunk()),
                    std::sync::Arc::as_ptr(expected),
                    "published Trunk must be the exact Arc the driver minted"
                );
            }
            _ => panic!("expected ProgramResolution::Found, got a variant that is not Found"),
        }
    }

    /// MUTATION VERIFIED: replacing the `for program in driver.programs()`
    /// loop body's dedup check (`if published.insert(program)`) with an
    /// unconditional `true` still republishes correctly, but replacing the
    /// whole loop with a no-op (never calling `driver.programs()` at all)
    /// makes this test fail identically to the previous one — `resolve_program`
    /// never sees an entry, so the `match` panics on the actual
    /// (`NotYetAnnounced`) variant instead of matching `Found`. This test
    /// additionally proves a *second* call with an already-published set does
    /// not clear or corrupt the registry (calling `report_driver_progress`
    /// twice in a row is exactly what a real per-iteration `run_*` loop does).
    /// Recompiled and re-run to confirm the failure, then reverted.
    #[test]
    fn repeated_calls_with_no_new_programs_are_idempotent() {
        let mut driver = driver();
        let route = crate::route::RouteHandle::new(4.0, 500, 4);
        let mut published = HashSet::new();
        let mut track_generations = std::collections::HashMap::new();

        driver.feed(&[], Timestamp::from_nanos(1));
        report_driver_progress(&driver, &route, &mut published, &mut track_generations);
        driver.feed(&[], Timestamp::from_nanos(2));
        report_driver_progress(&driver, &route, &mut published, &mut track_generations);
        // No new SessionEvents fed; calling again must be a harmless no-op.
        report_driver_progress(&driver, &route, &mut published, &mut track_generations);

        match route.resolve_program(crate::route::SPTS_PROGRAM_ID) {
            crate::route::ProgramResolution::Found(_) => {}
            _ => panic!("expected ProgramResolution::Found"),
        }
    }

    /// When a program's track set goes from populated to **empty**, the
    /// route's track specs must reflect the empty set — not keep serving the
    /// stale old specs ([`crate::source::report_driver_progress`] issue #831
    /// fix 1: the `if !tracks.is_empty()` guard skipped `set_track_specs`
    /// when `tracks` was empty, leaving stale specs forever).
    ///
    /// MUTATION VERIFIED: adding the `if !tracks.is_empty()` guard back
    /// (reverting fix 1) makes this test's `assert_eq!(specs.len(), 0, ...)`
    /// fail: `left: [1], right: []` — the route's track specs still hold the
    /// now-removed track from the first NewProgram, because the sync loop
    /// silently skipped the empty set.
    #[test]
    fn empty_track_set_replaces_previous_populated_set() {
        struct TrackSetSession {
            pending: std::collections::VecDeque<SessionEvent>,
            feed_count: u32,
        }

        impl Stage for TrackSetSession {
            type In<'a> = &'a [u8];
            type Out = SessionEvent;
            type Error = std::convert::Infallible;

            fn demand(&self) -> Demand {
                Demand::new(4096)
            }

            fn feed(&mut self, _input: &[u8], _now: Timestamp) -> Result<(), Self::Error> {
                self.feed_count += 1;
                if self.feed_count == 2 {
                    self.pending.push_back(SessionEvent::NewProgram {
                        program: ProgramId(0),
                        tracks: vec![crate::source::ts_program::test_support::track_spec(1)],
                    });
                } else if self.feed_count == 3 {
                    self.pending.push_back(SessionEvent::TracksChanged {
                        program: ProgramId(0),
                        tracks: Vec::new(),
                    });
                }
                Ok(())
            }

            fn poll(&mut self) -> Option<SessionEvent> {
                self.pending.pop_front()
            }
            fn next_deadline(&self) -> Option<Timestamp> {
                None
            }
            fn on_deadline(&mut self, _now: Timestamp) {}
            fn finish(&mut self) -> Result<(), Self::Error> {
                Ok(())
            }
        }

        impl IngestSession for TrackSetSession {
            type Request = bytes::Bytes;
        }

        let mut pending = std::collections::VecDeque::new();
        pending.push_back(SessionEvent::Established);
        let session = TrackSetSession {
            pending,
            feed_count: 0,
        };
        let mut driver = IngestDriver::new(
            session,
            trunk_config(),
            HandshakePolicy::establish_by(Timestamp::from_nanos(u64::MAX)),
            nz(4),
        );
        let route = crate::route::RouteHandle::new(4.0, 500, 4);
        let mut published = HashSet::new();
        let mut track_generations = std::collections::HashMap::new();

        // Feed 1: Established.
        driver.feed(&[], Timestamp::from_nanos(1));
        report_driver_progress(&driver, &route, &mut published, &mut track_generations);
        // Feed 2: NewProgram(0, [track_spec(1)]) — populates tracks.
        driver.feed(&[], Timestamp::from_nanos(2));
        report_driver_progress(&driver, &route, &mut published, &mut track_generations);
        let specs = route.track_specs(ProgramId(0));
        assert_eq!(
            specs.len(),
            1,
            "track specs must reflect the announced track"
        );

        // Feed 3: TracksChanged(0, []) — clears tracks.
        driver.feed(&[], Timestamp::from_nanos(3));
        report_driver_progress(&driver, &route, &mut published, &mut track_generations);
        let specs = route.track_specs(ProgramId(0));
        assert_eq!(
            specs.len(),
            0,
            "empty TracksChanged must replace the old track set — \
             the route's track specs must reflect empty, not the stale old set"
        );
    }
}

#[cfg(test)]
mod advance_route_tests {
    //! Coverage for [`advance_route`] — the one facade call replacing a
    //! caller-assembled `report_driver_progress` + `segment::drive_program_segmenters`
    //! pair (issue #805 task 6). Drives a real muxed TS stream through it,
    //! exactly mirroring `segment`'s own
    //! `driver_backed_route_serves_real_media_through_ll_hls` test, to prove
    //! the facade performs *both* steps (registry publish AND
    //! sample-to-segment turning), not just one.

    use super::*;
    use crate::route::{ProgramResolution, RouteHandle, SPTS_PROGRAM_ID};
    use crate::source::ts_program::TsIngestSession;
    use crate::source::ts_program::test_support::{build_ts_bytes, handshake, trunk_config};
    use broadcast_common::Timestamp;
    use media_plane::ingress::IngestDriver;

    /// MUTATION VERIFIED: changing `advance_route`'s body to call only
    /// `report_driver_progress` (dropping the
    /// `segment::drive_program_segmenters` line entirely) makes this test's
    /// `assert!(route.init_bytes(SPTS_PROGRAM_ID).is_some_and(|b| !b.is_empty()), ...)`
    /// fail: actual value `None` — the program is `Found` in the registry
    /// (the first assertion below still passes), but nothing ever turned its
    /// raw samples into a segmenter/init segment, exactly the "ingest
    /// observable, playback not" gap this facade exists to make impossible to
    /// half-wire. Recompiled and re-run to confirm the failure, then
    /// reverted.
    #[test]
    fn advance_route_both_publishes_and_segments() {
        let route = RouteHandle::new(1.0, 250, 8);
        let mut driver = IngestDriver::new(
            TsIngestSession::new(),
            trunk_config(),
            handshake(),
            media_plane::DEFAULT_MAX_PROGRAMS,
        );
        let mut progress = DriverProgress::new();

        let ts_bytes = build_ts_bytes(1, 0xAB, 90);
        driver.feed(&ts_bytes, Timestamp::ZERO);
        advance_route(&driver, &route, &mut progress);
        let more = build_ts_bytes(1, 0xCD, 90);
        driver.feed(&more, Timestamp::from_nanos(1));
        advance_route(&driver, &route, &mut progress);

        assert!(
            matches!(
                route.resolve_program(SPTS_PROGRAM_ID),
                ProgramResolution::Found(_)
            ),
            "advance_route must publish the program into the registry"
        );
        assert!(
            route
                .init_bytes(SPTS_PROGRAM_ID)
                .is_some_and(|b| !b.is_empty()),
            "advance_route must also turn samples into a real, servable init segment"
        );
    }
}

/// Per-track init derived from an SDP (RTSP's DESCRIBE body, or the
/// out-of-band SDP configured for [`rtp_udp::RtpUdpRoute`]).
#[derive(Debug, Clone)]
pub struct TrackInit {
    /// 1-based track id used across the segmenter + playlist URIs.
    pub track_id: u32,
    /// Payload kind (H.264 / AAC).
    pub kind: RtpMediaKind,
    /// Codec config built from the SDP fmtp.
    pub config: CodecConfig,
    /// RTP clock rate (Hz) = IR timescale.
    pub clock_rate: u32,
    /// Per-media `a=control` URL suffix for SETUP (RTSP only; unused by
    /// [`rtp_udp::RtpUdpRoute`], which has no control plane).
    pub control: Option<String>,
    /// Interleaved RTP channel assigned to this media (RTCP = channel + 1).
    /// RTSP-only framing; unused by [`rtp_udp::RtpUdpRoute`].
    pub channel: u8,
    /// The media's declared RTP payload type (`m=<kind> <port> <proto>
    /// <fmt>`, RFC 4566 §5.14) — the only signal a raw RTP/UDP source has to
    /// route an incoming packet to its track (there is no interleaved
    /// channel framing outside RTSP). RTSP ignores this field today (it
    /// routes by interleaved channel instead) but it is populated
    /// identically for both ingest paths since both go through the same
    /// [`sdp::parse_sdp_tracks`].
    pub payload_type: u8,
}

/// An ingest source that can be identified by name (e.g. for logging/metrics).
///
/// Kept minimal here; Task 5's `RtspSource` extends the ingest surface with
/// the actual RTSP session driving.
pub trait Source {
    /// Human-readable stream name (e.g. the RTSP URL or config-file key).
    fn stream_name(&self) -> &str;
}
