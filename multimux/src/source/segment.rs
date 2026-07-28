//! Productionises the segmenter gap `ts_program`'s own test proved closed
//! (issue #805 task 2b): task 2 wired all eight `media_plane`-ported inputs
//! (rtsp/rtp/ts_udp/ts_http/srt/hls_pull/dash_pull/smooth_pull) so they
//! *ingest* — each publishes its driver-minted [`Trunk`] into
//! [`crate::route::RouteHandle`]'s program registry
//! (`crate::source::report_driver_progress`), and egress resolves through
//! that registry. But nothing turned the raw samples the driver publishes
//! into segments/parts: the eight inputs wrote samples via the driver's
//! `TrunkWriter` and nothing ever read them back out, so a driver-backed
//! route's LL-HLS/DASH playlists came back empty — ingest was observable,
//! playback was not.
//!
//! [`ProgramSegmenter`] is the missing stage: one per announced
//! [`ProgramId`], subscribing a [`SampleCursor`] to that program's
//! driver-minted `Trunk`, feeding a [`transmux::ll_hls::LlHlsSegmenter`], and
//! publishing the resulting parts/segments back into **the same `Trunk`**
//! via [`Trunk::segment_writer`] — never a second `Trunk`, and never copying
//! samples between trunks (decided in
//! `docs/superpowers/specs/2026-07-26-media-plane-architecture.md` §8: the
//! ring-group split, `Trunk::segment_writer`, is exactly what makes holding
//! both the driver's sample writer and this segmenter's segment writer live
//! simultaneously on one `Trunk` legal). [`drive_program_segmenters`] is the
//! per-iteration driver every `run_*` entry point calls, mirroring
//! `report_driver_progress`'s own shape: build a segmenter the first time a
//! program's tracks have landed, then pump every already-built segmenter's
//! cursor.
//!
//! Both are `pub` (not `pub(crate)`): a [`crate::registry::SchemeRegistry`]
//! `Custom` input factory driving its own `media_plane::ingress::Dialer`/
//! `IngestSession` over its own `IngestDriver` needs this exact per-iteration
//! call too, right after `crate::source::report_driver_progress` — otherwise
//! its ingested samples land in the driver-minted `Trunk` but never become
//! LL-HLS/DASH-servable segments/parts, the same "ingest observable, playback
//! not" gap this module closed for the eight in-tree sources. See
//! `examples/custom_scheme.rs`.
//!
//! # Why `SPTS_PROGRAM_ID`'s init bytes go through `RouteHandle::set_init`
//!
//! The fMP4 init segment has no home in a `Trunk` (no ring holds it — see
//! `ll_hls_runtime::server::engine`'s own module doc, "the one thing that
//! genuinely cannot come from the `Trunk` alone"); it lives inside the
//! route's [`ll_hls_runtime::server::LlHlsOrigin`] instead.
//! `RouteHandle::publish_program` (crate-private) rebuilds that `LlHlsOrigin`
//! (and `DashState`) over a driver-minted `Trunk` the first time one is
//! published under `SPTS_PROGRAM_ID` (crate-private) — see that method's own
//! doc — so calling [`crate::route::RouteHandle::set_init`] here, *after*
//! this program's `Trunk` has already been published into the registry,
//! lands the init bytes in the same place every driver-backed `run_*` does.
//!
//! # Why segmenting is per-program, not per-route
//!
//! An MPTS route carries several programs on one `IngestDriver`; each has
//! its own driver-minted `Trunk` (see `media_plane::ingress`'s own docs) and
//! must be segmented independently — [`drive_program_segmenters`] iterates
//! every program `driver.programs()` reports, not just one. Wiring the
//! *result* of a non-default program's segmentation to HTTP egress (as
//! opposed to segmenting it at all) is issue #805 task 6 (MPTS egress
//! resolution) — out of scope here; only `SPTS_PROGRAM_ID`'s (crate-private)
//! init bytes are pushed to `RouteHandle` today, but every program's `Trunk` still
//! carries its own real segments/parts, resolvable exactly like the
//! `ts_program` test proves.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use broadcast_common::Timestamp;
use media_plane::ingress::{IngestDriver, IngestSession, ProgramId};
use media_plane::trunk::{
    PartEntry, SampleCursor, SampleCursorItem, SegmentEntry, SegmentWriter, Trunk,
};
use transmux::SegmentMeta;
use transmux::ll_hls::LlHlsSegmenter;

use crate::route::{RouteHandle, SPTS_PROGRAM_ID};

/// One program's segmenting state: a [`SampleCursor`] on its driver-minted
/// [`Trunk`], an [`LlHlsSegmenter`] fed from it, and the *same* `Trunk`'s own
/// [`SegmentWriter`] the resulting parts/segments are published back
/// through.
pub struct ProgramSegmenter {
    cursor: SampleCursor,
    segment_writer: SegmentWriter,
    seg: LlHlsSegmenter,
    /// Cumulative nanoseconds of segment duration published so far by this
    /// segmenter — mirrors `crate::route::RouteHandle`'s own
    /// `next_timeline_ns`, kept per-program here since each program's
    /// timeline is independent.
    next_timeline_ns: u64,
}

impl ProgramSegmenter {
    /// Attempt to start segmenting `trunk`. `None` if its track set hasn't
    /// landed yet (a `NewProgram` announcement with samples pending is
    /// unusual but not impossible — retried on the next call), if its
    /// segment writer has already been taken (must never happen for a
    /// freshly-observed per-program `Trunk`, but defensive rather than
    /// panicking on a driver/caller bug), or if [`LlHlsSegmenter::with_part_target`]
    /// itself rejects the track set (logged, not propagated — a
    /// segmentation failure on one program must not tear down the whole
    /// ingest session; see [`drive_program_segmenters`]'s own doc).
    fn try_new(trunk: &Arc<Trunk>, target_duration_secs: f64, part_target_ms: u32) -> Option<Self> {
        let tracks = trunk.tracks();
        if tracks.is_empty() {
            return None;
        }
        let segment_writer = trunk.segment_writer()?;
        let seg = match LlHlsSegmenter::with_part_target(
            tracks.to_vec(),
            transmux::VIDEO_CLOCK_RATE,
            target_duration_secs,
            part_target_ms,
        ) {
            Ok(seg) => seg,
            Err(e) => {
                tracing::warn!(error = %e, "driver-backed segmenter build failed");
                return None;
            }
        };
        Some(ProgramSegmenter {
            // `subscribe_from_backlog`, not `subscribe` (issue #808): the
            // driver's own `feed` call that announced this program's
            // `NewProgram` routinely carries its first samples too (a
            // single MPEG-TS feed batch commonly holds the PMT *and* the
            // first PES packets) — those samples are already sitting in
            // the ring by the time this constructor runs, and a
            // live-tail `subscribe()` cursor would never see them (the
            // opening IDR risks landing in that skipped batch). Replaying
            // whatever backlog the ring still retains is exactly this
            // consumer's shape: it exists to segment everything a program
            // ever publishes, not just what arrives after it happens to be
            // built.
            cursor: trunk.subscribe_from_backlog(),
            segment_writer,
            seg,
            next_timeline_ns: 0,
        })
    }

    /// The init segment bytes — built once, at construction, stable for the
    /// life of the segmenter (mirrors [`LlHlsSegmenter::init_segment`]'s own
    /// doc).
    fn init_segment(&self) -> Option<Vec<u8>> {
        match self.seg.init_segment() {
            Ok(bytes) => Some(bytes),
            Err(e) => {
                tracing::warn!(error = %e, "driver-backed segmenter init segment build failed");
                None
            }
        }
    }

    /// Drain every sample this program's cursor has observed since the last
    /// call, push it through the segmenter, and publish whatever parts/
    /// segments that produced.
    fn pump(&mut self) {
        while let Some(item) = self.cursor.poll() {
            if let SampleCursorItem::Timed { track_id, sample } = item {
                if let Err(e) = self.seg.push(track_id, sample) {
                    tracing::warn!(error = %e, "driver-backed segmenter push failed");
                }
            }
            // `Sparse`/`Lagged`/`Degraded` items: a section-carried (sparse)
            // track has no place in an fMP4 media segment and `Lagged`/
            // `Degraded` are cursor bookkeeping, not media — nothing to feed
            // the segmenter with either way, mirroring
            // `ts_program`'s own reference test, which only ever matches
            // `SampleCursorItem::Timed`.
        }
        self.publish_ready();
    }

    /// Finalize any trailing buffered partial segment — called once the
    /// driver reaches a terminal [`media_plane::ingress::HealthState`], so a
    /// disconnecting driver-backed route doesn't silently drop its last
    /// partial segment the way the pre-flush `run_pipeline` regression once
    /// did (see that module's `eos_flush_emits_buffered_tail_segment` test).
    fn flush(&mut self) {
        if let Err(e) = self.seg.flush() {
            tracing::warn!(error = %e, "driver-backed segmenter flush failed");
        }
        self.publish_ready();
    }

    fn publish_ready(&mut self) {
        for part in self.seg.take_ready_parts() {
            self.segment_writer.publish_part(PartEntry::new(
                part.bytes,
                part.segment_seq,
                part.part_index,
                Duration::from_secs_f64(part.duration),
                part.independent,
            ));
        }
        for segment in self.seg.take_ready_segments() {
            let duration = Duration::from_secs_f64(segment.duration);
            let start_ns = self.next_timeline_ns;
            self.next_timeline_ns = self
                .next_timeline_ns
                .saturating_add(u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX));
            self.segment_writer.publish_segment(SegmentEntry::new(
                segment.bytes,
                segment.segment_seq,
                duration,
                Timestamp::from_nanos(start_ns),
                SegmentMeta {
                    discontinuous: false,
                },
            ));
        }
    }
}

/// Per-iteration driver every `run_*` entry point calls right after
/// [`crate::source::report_driver_progress`] — builds a [`ProgramSegmenter`]
/// for each newly-observed program (once its tracks have landed) and pumps
/// every already-built one. `segmenters` is caller-owned, opaque state,
/// exactly like `report_driver_progress`'s own `published: &mut
/// HashSet<ProgramId>`: a caller (this crate's own `run_*` entry points, or an
/// external `Custom`-scheme factory's own drive loop — see this module's own
/// doc) declares one fresh `HashMap::new()` per connection attempt and passes
/// it back in on every call for that attempt's whole lifetime, never
/// constructing or reading a [`ProgramSegmenter`] itself.
///
/// A segmenter's own errors (build failure, a push/flush rejecting a
/// malformed sample) are logged and otherwise swallowed — never propagated
/// to the caller — because a segmentation problem on one program must not
/// tear down a working ingest connection; the connection itself (and every
/// other program on it) keeps running. A program whose segmenter failed to
/// build is retried on the next call (it is never inserted into
/// `segmenters`, so this function's internal `ProgramSegmenter::try_new`
/// runs again).
pub fn drive_program_segmenters<S: IngestSession>(
    driver: &IngestDriver<S>,
    route_handle: &RouteHandle,
    segmenters: &mut HashMap<ProgramId, ProgramSegmenter>,
) {
    for program in driver.programs() {
        if segmenters.contains_key(&program) {
            continue;
        }
        let Some(trunk) = driver.trunk(program) else {
            continue;
        };
        let Some(segmenter) = ProgramSegmenter::try_new(
            trunk,
            route_handle.target_duration_secs(),
            route_handle.part_target_ms(),
        ) else {
            continue;
        };
        // SPTS_PROGRAM_ID's init bytes are the one piece of segmenter output
        // RouteHandle's own single-slot LL-HLS/DASH accessors can serve
        // today — see this module's own doc for why, and why every other
        // program still segments (just isn't wired to HTTP egress yet:
        // issue #805 task 6).
        if program == SPTS_PROGRAM_ID {
            if let Some(init) = segmenter.init_segment() {
                route_handle.set_init(init);
            }
        }
        segmenters.insert(program, segmenter);
    }

    for segmenter in segmenters.values_mut() {
        segmenter.pump();
    }

    if !driver.health().is_running() {
        for segmenter in segmenters.values_mut() {
            segmenter.flush();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::route::RouteHandle;
    use crate::source::report_driver_progress;
    use crate::source::ts_program::TsIngestSession;
    use crate::source::ts_program::test_support::{
        build_ts_bytes, handshake, track_spec, trunk_config,
    };
    use broadcast_common::{Demand, Stage};
    use ll_hls_runtime::server::{DEFAULT_TRACK_ID, LlHlsBody, LlHlsRequest};
    use media_plane::egress::{AwaitPolicy, EgressResponse, ServedEgress};
    use media_plane::ingress::{IngestDriver, IngestSession, SessionEvent};
    use std::collections::{HashSet, VecDeque};
    use std::convert::Infallible;
    use transmux::pipeline::Sample;

    /// Render `route`'s current LL-HLS media playlist synchronously — the
    /// exact production call every LL-HLS request resolves through
    /// (`crate::output::llhls::media_playlist` / `crate::http::resolve_blocking`,
    /// minus the axum/tokio wrapping), so a passing assertion here is
    /// genuinely "resolvable the way egress resolves them", not a
    /// Trunk-level shortcut.
    fn render_playlist(route: &RouteHandle) -> String {
        match route.ll_hls().resolve(
            LlHlsRequest::Playlist {
                track_id: DEFAULT_TRACK_ID,
                query: Default::default(),
            },
            Timestamp::from_nanos(0),
            AwaitPolicy::new(Timestamp::from_nanos(0)),
        ) {
            EgressResponse::Ready {
                body: LlHlsBody::Playlist(m),
                ..
            } => m,
            other => panic!("expected Ready(Playlist), got {other:?}"),
        }
    }

    /// **The production segmenter gap, closed** (issue #805 task 2b). Drives
    /// the exact production chain a driver-backed route uses: a real
    /// `TsMux`-muxed TS stream → `TsIngestSession` → `IngestDriver` →
    /// `report_driver_progress` (publishes the registry) →
    /// `drive_program_segmenters` (this module) → the route's own
    /// `RouteHandle::init_bytes`/`LlHlsOrigin`.
    ///
    /// MUTATION VERIFIED: disabling `RouteHandle::publish_program`'s
    /// `if program == SPTS_PROGRAM_ID { self.rebind_serving_trunk_if_new(&trunk); }`
    /// call (changed to `if program == SPTS_PROGRAM_ID && false { .. }`, so
    /// `ll_hls`/`dash` stay bound to the route's own never-written
    /// placeholder `Trunk` forever, exactly the pre-task-2b state) makes this
    /// test's second assertion fail:
    /// `assert!(playlist.contains("#EXTINF:") || playlist.contains("#EXT-X-PART:"), ...)`
    /// fails — actual rendered playlist body (verbatim from the panic):
    /// ```text
    /// #EXTM3U
    /// #EXT-X-VERSION:9
    /// #EXT-X-TARGETDURATION:1
    /// #EXT-X-MEDIA-SEQUENCE:1
    /// #EXT-X-SERVER-CONTROL:CAN-BLOCK-RELOAD=YES,PART-HOLD-BACK=0.75
    /// #EXT-X-PART-INF:PART-TARGET=0.25
    /// #EXT-X-MAP:URI="init-1.mp4"
    /// ```
    /// — `render_playlist` still returns `EgressResponse::Ready` (an empty
    /// query never triggers the blocking-reload path), but with a playlist
    /// rendered from a `Trunk` that never received a single segment/part
    /// (no `#EXTINF:`/`#EXT-X-PART:` line at all — only the unconditional
    /// `#EXT-X-PART-INF:` header), since this module's segmenter published
    /// everything into the *driver's* `Trunk` instead. The first assertion
    /// (`init_bytes`) still passes even with the mutation, because
    /// `set_init` writes through whichever `Arc<LlHlsOrigin>` `ll_hls()`
    /// currently resolves to at call time inside `drive_program_segmenters`
    /// — this is exactly why the test needs *both* assertions. Recompiled
    /// and re-run to confirm the exact failure above, then reverted.
    #[test]
    fn driver_backed_route_serves_real_media_through_ll_hls() {
        let route = RouteHandle::new(1.0, 250, 8);
        let mut driver = IngestDriver::new(
            TsIngestSession::new(),
            trunk_config(),
            handshake(),
            media_plane::DEFAULT_MAX_PROGRAMS,
        );
        let mut published = HashSet::new();
        let mut segmenters = HashMap::new();

        // First feed: resolves the PMT, mints program 0's driver-side Trunk.
        let ts_bytes = build_ts_bytes(1, 0xAB, 90);
        driver.feed(&ts_bytes, Timestamp::ZERO);
        report_driver_progress(&driver, &route, &mut published);
        drive_program_segmenters(&driver, &route, &mut segmenters);

        // Second feed: more real media, proving the segmenter's cursor
        // (subscribed just above via `subscribe_from_backlog` — issue #808)
        // keeps observing new samples live *in addition to* replaying the
        // first feed's own backlog, not merely as a substitute for it.
        let more = build_ts_bytes(1, 0xCD, 90);
        driver.feed(&more, Timestamp::from_nanos(1));
        report_driver_progress(&driver, &route, &mut published);
        drive_program_segmenters(&driver, &route, &mut segmenters);

        assert!(
            route.init_bytes().is_some_and(|b| !b.is_empty()),
            "a driver-backed route must end up with real, non-empty init bytes"
        );

        let playlist = render_playlist(&route);
        // `"#EXT-X-PART-INF:"` (the part-target *header*, RFC 8216bis
        // §4.4.3.7) is present in every low-latency playlist regardless of
        // whether any part/segment has actually landed, so it would make
        // this assertion pass even against a completely empty playlist —
        // deliberately checked for `"#EXT-X-PART:"` (an actual per-part tag,
        // §4.4.4.9, note the trailing colon distinguishing it from
        // `"#EXT-X-PART-INF:"`) and `"#EXTINF:"` (an actual closed segment's
        // duration tag, §4.3.2.1) instead.
        assert!(
            playlist.contains("#EXTINF:") || playlist.contains("#EXT-X-PART:"),
            "the route's own LlHlsOrigin must serve real closed segments/parts, \
             resolved exactly the way egress resolves them: {playlist}"
        );
    }

    /// **Issue #808, the regression test.** A source that announces
    /// `NewProgram` *and* publishes its samples in a **single** `feed`
    /// call — ordinary behaviour for MPEG-TS, where one PMT-carrying feed
    /// batch routinely also carries the first PES samples — must still get
    /// those samples segmented and served. Deliberately only one `feed`
    /// call, unlike `driver_backed_route_serves_real_media_through_ll_hls`
    /// above: that test's second feed would mask exactly the bug this test
    /// exists to catch.
    ///
    /// Asserts on `"#EXT-X-PART:"`/`"#EXTINF:"` specifically (not merely
    /// `EgressResponse::Ready`, and not `"#EXT-X-PART-INF:"`/`"#EXT-X-MAP:"`,
    /// which `ll_hls_runtime` renders unconditionally even for a route with
    /// zero parts/segments) — see `render_playlist`'s and this module's
    /// sibling test's own doc for why an unconditional header would let this
    /// test pass against the bug.
    ///
    /// **Confirmed this test fails against live-tail `subscribe()`**: with
    /// `ProgramSegmenter::try_new`'s `cursor: trunk.subscribe_from_backlog()`
    /// temporarily reverted to `cursor: trunk.subscribe()`, this test's
    /// `assert!(playlist.contains("#EXTINF:") || playlist.contains("#EXT-X-PART:"), ...)`
    /// fails — actual rendered playlist body (verbatim from the panic):
    /// ```text
    /// #EXTM3U
    /// #EXT-X-VERSION:9
    /// #EXT-X-TARGETDURATION:1
    /// #EXT-X-MEDIA-SEQUENCE:1
    /// #EXT-X-SERVER-CONTROL:CAN-BLOCK-RELOAD=YES,PART-HOLD-BACK=0.75
    /// #EXT-X-PART-INF:PART-TARGET=0.25
    /// #EXT-X-MAP:URI="init-1.mp4"
    /// ```
    /// — the single feed's `NewProgram` *and* all 90 samples land in the
    /// ring before `drive_program_segmenters` ever builds the
    /// `ProgramSegmenter`, so a `subscribe()` cursor (starting from "now",
    /// i.e. *after* that entire batch) never observes a single sample: no
    /// part, no segment, ever. Recompiled with the revert in place, re-ran
    /// to confirm this exact failure, then restored
    /// `subscribe_from_backlog()` and re-ran to confirm this test passes
    /// again.
    #[test]
    fn samples_published_in_the_same_feed_as_new_program_are_still_segmented() {
        let route = RouteHandle::new(1.0, 250, 8);
        let mut driver = IngestDriver::new(
            TsIngestSession::new(),
            trunk_config(),
            handshake(),
            media_plane::DEFAULT_MAX_PROGRAMS,
        );
        let mut published = HashSet::new();
        let mut segmenters = HashMap::new();

        // ONE feed: build_ts_bytes muxes the PMT (-> NewProgram) and 90 real
        // PES samples (-> Sample events) into the same TS byte stream, so
        // the driver's own `feed` drains both the NewProgram announcement
        // and every one of those samples in this single call — exactly the
        // "ordinary MPEG-TS" shape issue #808 describes, not a contrived
        // edge case.
        let ts_bytes = build_ts_bytes(1, 0xAB, 90);
        driver.feed(&ts_bytes, Timestamp::ZERO);
        report_driver_progress(&driver, &route, &mut published);
        drive_program_segmenters(&driver, &route, &mut segmenters);

        assert!(
            route.init_bytes().is_some_and(|b| !b.is_empty()),
            "a driver-backed route must end up with real, non-empty init bytes"
        );

        let playlist = render_playlist(&route);
        assert!(
            playlist.contains("#EXTINF:") || playlist.contains("#EXT-X-PART:"),
            "samples published in the SAME feed call as NewProgram must still reach the \
             served playlist as real closed segments/parts: {playlist}"
        );
    }

    /// The segmenter must feed the **same** `Trunk` the driver publishes
    /// samples into — never a copy, never a second `Trunk` (issue #805 §8's
    /// explicit decision; see this module's own doc).
    ///
    /// MUTATION VERIFIED: changing `ProgramSegmenter::try_new` to take its
    /// `segment_writer` from a freshly-constructed `Trunk` (i.e.
    /// `Trunk::new(media_plane::trunk::TrunkConfig::new(nz(1), nz(1), nz(8), nz(1), nz(8))).segment_writer()`)
    /// instead of `trunk.segment_writer()`, while still subscribing the
    /// *real* `trunk`'s `SampleCursor` (so samples keep flowing into the
    /// segmenter), makes this test's
    /// `assert!(driver.trunk(ProgramId(0)).unwrap().last_closed_segment().is_some(), ...)`
    /// fail: `last_closed_segment()` returns `None` (actual) instead of
    /// `Some(_)` (expected), because every produced segment/part landed in
    /// the discarded throwaway `Trunk` instead of the one `driver.trunk(..)`
    /// (and therefore `RouteHandle`'s registry) actually holds. Recompiled
    /// and re-run to confirm the failure, then reverted.
    #[test]
    fn segmenter_feeds_the_same_trunk_the_driver_writes_samples_into() {
        let route = RouteHandle::new(1.0, 250, 8);
        let mut driver = IngestDriver::new(
            TsIngestSession::new(),
            trunk_config(),
            handshake(),
            media_plane::DEFAULT_MAX_PROGRAMS,
        );
        let mut published = HashSet::new();
        let mut segmenters = HashMap::new();

        let ts_bytes = build_ts_bytes(1, 0xAB, 90);
        driver.feed(&ts_bytes, Timestamp::ZERO);
        report_driver_progress(&driver, &route, &mut published);
        drive_program_segmenters(&driver, &route, &mut segmenters);

        let more = build_ts_bytes(1, 0xCD, 90);
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
        // Also pins that the registry (what egress resolves) is the exact
        // same Trunk, not merely one with equal contents.
        match route.resolve_program(crate::route::SPTS_PROGRAM_ID) {
            crate::route::ProgramResolution::Found(resolved) => {
                assert!(Arc::ptr_eq(&resolved, &program_trunk));
            }
            _ => panic!("expected the route's registry to resolve program 0"),
        }
    }

    const FRAME_DUR: u32 = transmux::VIDEO_CLOCK_RATE / 30;

    fn sample_at(i: u32, is_sync: bool) -> Sample {
        Sample::new(
            vec![0xAAu8.wrapping_add(i as u8); 8],
            Some(i64::from(i) * i64::from(FRAME_DUR)),
            Some(i64::from(i) * i64::from(FRAME_DUR)),
            Some(FRAME_DUR),
            is_sync,
        )
    }

    /// A hand-scripted two-program `IngestSession`: queues `Established`,
    /// then (on its 2nd `feed` call) both `NewProgram` announcements, then
    /// (on every later call) one sample for *each* program's own track —
    /// enough to drive `drive_program_segmenters` through the MPTS case
    /// without needing a real multi-program TS mux (`transmux` has no
    /// `program_number` in its IR yet — see `ts_program`'s own module doc).
    struct TwoProgramSession {
        pending: VecDeque<SessionEvent>,
        feed_count: u32,
    }

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
            Demand::new(4096)
        }

        fn feed(&mut self, _input: &[u8], _now: Timestamp) -> Result<(), Infallible> {
            self.feed_count += 1;
            if self.feed_count == 2 {
                self.pending.push_back(SessionEvent::NewProgram {
                    program: ProgramId(0),
                    tracks: vec![track_spec(1)],
                });
                self.pending.push_back(SessionEvent::NewProgram {
                    program: ProgramId(1),
                    tracks: vec![track_spec(2)],
                });
            } else if self.feed_count >= 3 {
                let i = self.feed_count - 3;
                let is_sync = i == 0 || i == 45;
                self.pending.push_back(SessionEvent::Sample {
                    program: ProgramId(0),
                    track_id: 1,
                    retention: media_plane::trunk::RetentionClass::Timed,
                    sample: sample_at(i, is_sync),
                });
                self.pending.push_back(SessionEvent::Sample {
                    program: ProgramId(1),
                    track_id: 2,
                    retention: media_plane::trunk::RetentionClass::Timed,
                    sample: sample_at(i, is_sync),
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

        fn finish(&mut self) -> Result<(), Infallible> {
            Ok(())
        }
    }

    impl IngestSession for TwoProgramSession {
        type Request = bytes::Bytes;
    }

    /// An MPTS route (one `IngestDriver`, two announced programs) must
    /// segment each program independently — issue #805 task 2b's explicit
    /// "do not write anything that assumes one program per route".
    ///
    /// MUTATION VERIFIED: changing `drive_program_segmenters`'s per-program
    /// loop to key `segmenters`/publish init bytes by the constant
    /// `SPTS_PROGRAM_ID` instead of the loop's own `program` variable (i.e.
    /// `segmenters.contains_key(&SPTS_PROGRAM_ID)` /
    /// `segmenters.insert(SPTS_PROGRAM_ID, segmenter)`, simulating code that
    /// assumes a single program per route) makes this test's
    /// `assert!(driver.trunk(ProgramId(1)).unwrap().last_closed_segment().is_some(), ...)`
    /// fail: `last_closed_segment()` returns `None` (actual) instead of
    /// `Some(_)` (expected) — program 1 never gets its own `ProgramSegmenter`
    /// (every program collapses onto the single `SPTS_PROGRAM_ID` slot, and
    /// only whichever program is announced last actually accumulates
    /// samples), so its `Trunk` never sees a closed segment. Program 0's
    /// identical assertion still passes even with the mutation (misleading
    /// on its own — this is why the test checks *both* programs, and why an
    /// assertion that would still pass under a plausible mutation is not
    /// sufficient by itself). Recompiled and re-run to confirm program 1's
    /// assertion's failure, then reverted.
    #[test]
    fn two_programs_on_one_route_segment_independently() {
        let route = RouteHandle::new(1.0, 500, 8);
        let mut driver = IngestDriver::new(
            TwoProgramSession::new(),
            trunk_config(),
            handshake(),
            media_plane::DEFAULT_MAX_PROGRAMS,
        );
        let mut published = HashSet::new();
        let mut segmenters = HashMap::new();

        // Feed 1: Established. Feed 2: both NewProgram announcements (mints
        // both driver-side Trunks and, via `drive_program_segmenters`,
        // subscribes both `ProgramSegmenter`s *before* any sample exists).
        driver.feed(&[], Timestamp::from_nanos(0));
        report_driver_progress(&driver, &route, &mut published);
        drive_program_segmenters(&driver, &route, &mut segmenters);
        driver.feed(&[], Timestamp::from_nanos(1));
        report_driver_progress(&driver, &route, &mut published);
        drive_program_segmenters(&driver, &route, &mut segmenters);

        // 90 samples @ 3000 ticks/30fps = 3s of media per program,
        // comfortably over the 1.0s target duration — mirrors
        // `crate::pipeline`'s own `drives_source_through_segmenter_into_store`
        // fixture numbers, already proven to close at least one segment.
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
            !Arc::ptr_eq(&trunk_0, &trunk_1),
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

    /// Monotonic nanosecond source for `two_programs_on_one_route_segment_independently`'s
    /// feed loop — the exact `now` value is immaterial (`TwoProgramSession`
    /// never reads it), only that each call passes a value (an
    /// `IngestDriver` contract, not a real timing requirement here).
    fn driver_feed_nanos() -> u64 {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(2);
        N.fetch_add(1, Ordering::Relaxed)
    }
}
