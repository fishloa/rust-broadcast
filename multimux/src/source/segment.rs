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
//! `ProgramSegmenter` is the missing stage: one per announced
//! [`ProgramId`], subscribing a [`SampleCursor`] to that program's
//! driver-minted `Trunk`, feeding a [`transmux::ll_hls::LlHlsSegmenter`], and
//! publishing the resulting parts/segments back into **the same `Trunk`**
//! via [`Trunk::segment_writer`] — never a second `Trunk`, and never copying
//! samples between trunks (decided in
//! `docs/superpowers/specs/2026-07-26-media-plane-architecture.md` §8: the
//! ring-group split, `Trunk::segment_writer`, is exactly what makes holding
//! both the driver's sample writer and this segmenter's segment writer live
//! simultaneously on one `Trunk` legal). `drive_program_segmenters` is the
//! per-iteration driver every `run_*` entry point calls, mirroring
//! `report_driver_progress`'s own shape: build a segmenter the first time a
//! program's tracks have landed, then pump every already-built segmenter's
//! cursor.
//!
//! **Issue #887**: `ProgramSegmenter` feeds one of *two* segmenter kinds —
//! [`transmux::ll_hls::LlHlsSegmenter`] (fMP4/CMAF, LL-HLS parts) or
//! [`transmux::ts_hls::StreamingTsHlsSegmenter`] (classic whole-`.ts`
//! segments, no parts) — selected per-program by that program's route's
//! configured `crate::route::RouteHandle::container()` (see `AnySegmenter`,
//! below). A route's container is fixed at route-construction time
//! (`crate::config::OutputKind::TsHls` is mutually exclusive with
//! `llhls`/`dash`/`ll_dash` on the same route, enforced by
//! `crate::config::Route::validate_standalone`), so every program on one
//! route always takes the same branch.
//!
//! Both are `pub(crate)` (issue #805 task 6 narrowed them back from `pub`):
//! the supported extension surface for a [`crate::registry::SchemeRegistry`]
//! `Custom` input factory is now the single [`crate::source::advance_route`]
//! facade over `report_driver_progress` + this module's own
//! `drive_program_segmenters`, not either call directly — a factory that
//! called them separately (in the wrong order, or one without the other)
//! could silently ingest with nothing ever becoming servable, the exact
//! footgun `advance_route` exists to remove. See `examples/custom_scheme.rs`.
//!
//! # Why `SPTS_PROGRAM_ID`'s init bytes go through `RouteHandle::set_init`
//!
//! The fMP4 init segment has no home in a `Trunk` (no ring holds it — see
//! `hls_runtime::server::engine`'s own module doc, "the one thing that
//! genuinely cannot come from the `Trunk` alone"); it lives inside the
//! route's [`hls_runtime::server::HlsOrigin`] instead.
//! `RouteHandle::publish_program` (crate-private) builds that program's
//! `ProgramServing` bundle (`HlsOrigin`+`DashState`) the first time its
//! `Trunk` is published — see that method's own doc — so calling
//! [`crate::route::RouteHandle::set_init`] here, *after* this program's
//! `Trunk` has already been published into the registry, lands the init
//! bytes in the same bundle every driver-backed `run_*` reads from.
//!
//! # Why segmenting is per-program, not per-route
//!
//! An MPTS route carries several programs on one `IngestDriver`; each has
//! its own driver-minted `Trunk` (see `media_plane::ingress`'s own docs) and
//! must be segmented independently — `drive_program_segmenters` iterates
//! every program `driver.programs()` reports, not just one. Every program
//! gets its own [`crate::route::RouteHandle`] registry entry (issue #805 task
//! 6: per-program serving state), so every program's `Trunk` carries its own
//! real segments/parts, individually resolvable exactly like the
//! `ts_program` test proves and `crate::route`'s own
//! `two_programs_serve_distinct_media` test proves end to end. *Selecting*
//! one from an incoming HTTP request (as opposed to segmenting/serving it at
//! all) is the addressing question `crate::route`'s own module doc records
//! options for — only `SPTS_PROGRAM_ID`'s init bytes are pushed here as the
//! one program every current egress call site resolves.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use broadcast_common::Timestamp;
use hls_runtime::server::Container;
use media_plane::ingress::{IngestDriver, IngestSession, ProgramId};
use media_plane::trunk::{
    PartEntry, SampleCursor, SampleCursorItem, SegmentEntry, SegmentWriter, Trunk,
};
use transmux::SegmentMeta;
use transmux::ll_hls::{LlHlsSegmenter, PartInfo};
use transmux::pipeline::Sample;
use transmux::ts_hls::StreamingTsHlsSegmenter;

use crate::route::{RouteHandle, SPTS_PROGRAM_ID};

/// One closed segment, normalised across the two segmenter kinds
/// [`AnySegmenter`] wraps — [`transmux::ll_hls::SegmentInfo`] (fMP4) and
/// [`transmux::ts_hls::TsSegment`] (classic TS) carry the same information
/// under different field names/numbering conventions (see
/// [`AnySegmenter::take_ready_segments`]), so [`ProgramSegmenter::publish_ready`]
/// works against this one shape regardless of which segmenter produced it.
struct ReadySegment {
    bytes: Vec<u8>,
    /// 1-based, matching [`media_plane::trunk::SegmentEntry::sequence_number`]'s
    /// own convention (which in turn mirrors
    /// [`transmux::ll_hls::SegmentInfo::segment_seq`]) — a
    /// [`transmux::ts_hls::TsSegment::sequence`] is 0-based, so
    /// [`AnySegmenter::take_ready_segments`] adds one when normalising it.
    segment_seq: u32,
    duration: f64,
    discontinuous: bool,
}

impl From<transmux::ll_hls::SegmentInfo> for ReadySegment {
    fn from(info: transmux::ll_hls::SegmentInfo) -> Self {
        ReadySegment {
            bytes: info.bytes,
            segment_seq: info.segment_seq,
            duration: info.duration,
            // The fMP4 segmenter never itself flags a discontinuity (see
            // `ProgramSegmenter::publish_ready`'s pre-#887 hardcoded `false` —
            // preserved here, not a behaviour change).
            discontinuous: false,
        }
    }
}

impl From<transmux::ts_hls::TsSegment> for ReadySegment {
    fn from(seg: transmux::ts_hls::TsSegment) -> Self {
        ReadySegment {
            bytes: seg.bytes,
            segment_seq: u32::try_from(seg.sequence)
                .unwrap_or(u32::MAX)
                .saturating_add(1),
            duration: seg.duration,
            discontinuous: seg.discontinuous,
        }
    }
}

/// The two segmenter kinds a [`ProgramSegmenter`] can drive, selected by the
/// route's configured [`Container`] (`crate::route::RouteHandle::container`,
/// issue #887): [`Container::Fmp4`] feeds an [`LlHlsSegmenter`] (parts +
/// segments), [`Container::MpegTs`] feeds a [`StreamingTsHlsSegmenter`]
/// (whole self-initialising `.ts` segments only — no parts, no init segment;
/// see that type's own module doc). Kept as one enum (rather than
/// `ProgramSegmenter` being generic) so `drive_program_segmenters` and its
/// `HashMap<ProgramId, ProgramSegmenter>` stay a single concrete type
/// regardless of which container any given route is configured for.
enum AnySegmenter {
    Fmp4(LlHlsSegmenter),
    Ts(StreamingTsHlsSegmenter),
}

impl AnySegmenter {
    fn push(&mut self, track_id: u32, sample: Sample) -> transmux::Result<()> {
        match self {
            AnySegmenter::Fmp4(seg) => seg.push(track_id, sample),
            AnySegmenter::Ts(seg) => seg.push(track_id, sample),
        }
    }

    /// Finalize any trailing buffered partial segment — `LlHlsSegmenter::flush`
    /// for fMP4, `StreamingTsHlsSegmenter::finish` for classic TS (same job,
    /// different name: see that method's own doc for why it is not named
    /// `flush` there).
    fn finish(&mut self) -> transmux::Result<()> {
        match self {
            AnySegmenter::Fmp4(seg) => seg.flush(),
            AnySegmenter::Ts(seg) => seg.finish(),
        }
    }

    /// The init segment bytes, fMP4 only — `Container::MpegTs`'s classic TS
    /// segments are self-initialising (in-band PAT/PMT at the head of every
    /// segment; see [`StreamingTsHlsSegmenter`]'s own doc), so this is always
    /// `None` for [`AnySegmenter::Ts`] — mirroring
    /// `hls_runtime::server::HlsOrigin::set_init`'s documented no-op under
    /// [`Container::MpegTs`].
    fn init_segment(&self) -> Option<Vec<u8>> {
        match self {
            AnySegmenter::Fmp4(seg) => match seg.init_segment() {
                Ok(bytes) => Some(bytes),
                Err(e) => {
                    tracing::warn!(error = %e, "driver-backed segmenter init segment build failed");
                    None
                }
            },
            AnySegmenter::Ts(_) => None,
        }
    }

    /// LL-HLS partial segments — always empty for [`AnySegmenter::Ts`], which
    /// has no partial-segment concept at all (see [`AnySegmenter`]'s own doc).
    fn take_ready_parts(&mut self) -> Vec<PartInfo> {
        match self {
            AnySegmenter::Fmp4(seg) => seg.take_ready_parts(),
            AnySegmenter::Ts(_) => Vec::new(),
        }
    }

    /// Closed segments, normalised to [`ReadySegment`] — see that type's own
    /// doc for the fMP4/TS field-mapping differences this bridges.
    fn take_ready_segments(&mut self) -> Vec<ReadySegment> {
        match self {
            AnySegmenter::Fmp4(seg) => seg
                .take_ready_segments()
                .into_iter()
                .map(Into::into)
                .collect(),
            AnySegmenter::Ts(seg) => seg.take_ready().into_iter().map(Into::into).collect(),
        }
    }
}

/// One program's segmenting state: a [`SampleCursor`] on its driver-minted
/// [`Trunk`], an [`LlHlsSegmenter`] fed from it, and the *same* `Trunk`'s own
/// [`SegmentWriter`] the resulting parts/segments are published back
/// through.
///
/// `pub(crate)` (not `pub`, issue #805 task 6): the one caller outside this
/// module, [`crate::source::advance_route`], is the facade a
/// [`crate::registry::SchemeRegistry`] `Custom` factory calls instead of
/// touching this type or [`drive_program_segmenters`] directly — see that
/// facade's own doc.
pub(crate) struct ProgramSegmenter {
    cursor: SampleCursor,
    segment_writer: SegmentWriter,
    seg: AnySegmenter,
    /// Cumulative nanoseconds of segment duration published so far by this
    /// segmenter — mirrors `crate::route::RouteHandle`'s own
    /// `next_timeline_ns`, kept per-program here since each program's
    /// timeline is independent.
    next_timeline_ns: u64,
}

/// Rolling-window depth [`StreamingTsHlsSegmenter::new`] is given —
/// irrelevant to a driver-backed route in practice (its own `.playlist()`
/// method, the only consumer of this window, is never called here: the
/// served playlist is rendered from the `Trunk`'s own segment log via
/// `crate::route::ProgramServing`'s `HlsOrigin`, exactly like the fMP4 path).
/// Any positive value works; matches this route's own advertised-window
/// depth (`crate::route::RouteHandle::window_segments_cap`) so memory use is
/// at least bounded consistently with everything else per-route.
fn ts_segmenter_window(route_handle: &RouteHandle) -> usize {
    route_handle.window_segments_cap().get()
}

impl ProgramSegmenter {
    /// Attempt to start segmenting `trunk`. `None` if its track set hasn't
    /// landed yet (a `NewProgram` announcement with samples pending is
    /// unusual but not impossible — retried on the next call), if its
    /// segment writer has already been taken (must never happen for a
    /// freshly-observed per-program `Trunk`, but defensive rather than
    /// panicking on a driver/caller bug), or if the underlying segmenter
    /// constructor ([`LlHlsSegmenter::with_part_target`] for
    /// [`Container::Fmp4`], [`StreamingTsHlsSegmenter::new`] for
    /// [`Container::MpegTs`]) itself rejects the track set (logged, not
    /// propagated — a segmentation failure on one program must not tear down
    /// the whole ingest session; see [`drive_program_segmenters`]'s own doc).
    fn try_new(
        trunk: &Arc<Trunk>,
        route_handle: &RouteHandle,
        target_duration_secs: f64,
        part_target_ms: u32,
    ) -> Option<Self> {
        let tracks = trunk.tracks();
        if tracks.is_empty() {
            return None;
        }
        let segment_writer = trunk.segment_writer()?;
        let seg = match route_handle.container() {
            Container::Fmp4 => match LlHlsSegmenter::with_part_target(
                tracks.to_vec(),
                transmux::VIDEO_CLOCK_RATE,
                target_duration_secs,
                part_target_ms,
            ) {
                Ok(seg) => AnySegmenter::Fmp4(seg),
                Err(e) => {
                    tracing::warn!(error = %e, "driver-backed fMP4 segmenter build failed");
                    return None;
                }
            },
            Container::MpegTs => {
                // Whole seconds, clamped >= 1 (mirrors
                // `StreamingTsHlsSegmenter::new`'s own clamp) — the target
                // duration is configured as `f64` seconds route-wide
                // (`crate::config::Config::target_duration_secs`), but the
                // classic-TS segmenter's cut rule is integer-second, per
                // `transmux::ts_hls`'s own module doc ("no_std-friendly").
                let target_secs = target_duration_secs.round().max(1.0) as u32;
                match StreamingTsHlsSegmenter::new(
                    tracks.to_vec(),
                    target_secs,
                    ts_segmenter_window(route_handle),
                ) {
                    Ok(seg) => AnySegmenter::Ts(seg),
                    Err(e) => {
                        tracing::warn!(error = %e, "driver-backed TS-HLS segmenter build failed");
                        return None;
                    }
                }
            }
            // `Container` is `#[non_exhaustive]`: a future container variant
            // this segmenter has no branch for is refused (logged, not
            // propagated — see this function's own doc), not silently
            // defaulted into one of the two existing branches.
            other => {
                tracing::warn!(
                    ?other,
                    "driver-backed segmenter: no segmenter implementation for this container"
                );
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
    /// doc). Always `None` under [`Container::MpegTs`] — see
    /// [`AnySegmenter::init_segment`].
    fn init_segment(&self) -> Option<Vec<u8>> {
        self.seg.init_segment()
    }

    /// Drain every sample this program's cursor has observed since the last
    /// call, push it through the segmenter, and publish whatever parts/
    /// segments that produced. Returns `(parts_published, segments_published)`
    /// this call — issue #809: [`drive_program_segmenters`] sums these across
    /// every segmenter to drive `crate::prometheus::PARTS_PRODUCED_TOTAL`/
    /// `SEGMENTS_PRODUCED_TOTAL`.
    fn pump(&mut self) -> (usize, usize) {
        while let Some(item) = self.cursor.poll() {
            if let SampleCursorItem::Timed { track_id, sample } = item {
                if let Err(e) = self.seg.push(track_id, sample) {
                    tracing::warn!(error = %e, "driver-backed segmenter push failed");
                }
            }
            // `Sparse`/`Lagged`/`Degraded` items: a section-carried (sparse)
            // track has no place in either segmenter's media segment (fMP4 or
            // classic TS) and `Lagged`/`Degraded` are cursor bookkeeping, not
            // media — nothing to feed the segmenter with either way, mirroring
            // `ts_program`'s own reference test, which only ever matches
            // `SampleCursorItem::Timed`.
        }
        self.publish_ready()
    }

    /// Finalize any trailing buffered partial segment — called once the
    /// driver reaches a terminal [`media_plane::ingress::HealthState`], so a
    /// disconnecting driver-backed route doesn't silently drop its last
    /// partial segment the way the pre-flush `run_pipeline` regression once
    /// did (see that module's `eos_flush_emits_buffered_tail_segment` test).
    /// Returns `(parts_published, segments_published)` this call — see
    /// [`Self::pump`].
    fn flush(&mut self) -> (usize, usize) {
        if let Err(e) = self.seg.finish() {
            tracing::warn!(error = %e, "driver-backed segmenter flush failed");
        }
        self.publish_ready()
    }

    fn publish_ready(&mut self) -> (usize, usize) {
        let mut parts_published = 0usize;
        for part in self.seg.take_ready_parts() {
            self.segment_writer.publish_part(PartEntry::new(
                part.bytes,
                part.segment_seq,
                part.part_index,
                Duration::from_secs_f64(part.duration),
                part.independent,
            ));
            parts_published += 1;
        }
        let mut segments_published = 0usize;
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
                    discontinuous: segment.discontinuous,
                },
            ));
            segments_published += 1;
        }
        (parts_published, segments_published)
    }
}

/// Per-iteration driver every `run_*` entry point calls right after
/// [`crate::source::report_driver_progress`] (together, [`crate::source::advance_route`]
/// is the one facade call that does both, in order — see that function's own
/// doc) — builds a [`ProgramSegmenter`] for each newly-observed program (once
/// its tracks have landed) and pumps every already-built one. `segmenters` is
/// caller-owned, opaque state, exactly like `report_driver_progress`'s own
/// `published: &mut HashSet<ProgramId>`: a caller declares one fresh
/// `HashMap::new()` per connection attempt and passes it back in on every
/// call for that attempt's whole lifetime, never constructing or reading a
/// [`ProgramSegmenter`] itself.
///
/// A segmenter's own errors (build failure, a push/flush rejecting a
/// malformed sample) are logged and otherwise swallowed — never propagated
/// to the caller — because a segmentation problem on one program must not
/// tear down a working ingest connection; the connection itself (and every
/// other program on it) keeps running. A program whose segmenter failed to
/// build is retried on the next call (it is never inserted into
/// `segmenters`, so this function's internal `ProgramSegmenter::try_new`
/// runs again).
///
/// `pub(crate)` (issue #805 task 6 narrowed this back from `pub`): the
/// supported extension surface for a [`crate::registry::SchemeRegistry`]
/// `Custom` factory is now the single [`crate::source::advance_route`]
/// facade, not this function directly — see that function's own doc for why.
///
/// **Issue #809**: bumps `crate::prometheus::PARTS_PRODUCED_TOTAL`/
/// `SEGMENTS_PRODUCED_TOTAL`, labelled by `route_handle.name()`, for the total
/// parts/segments every segmenter on this route actually published this call
/// (pump, plus flush if the driver just went terminal) — the only place in
/// the driver-backed architecture samples actually turn into parts/segments,
/// so the only place that can honestly say a part/segment was "produced".
/// Only increments when the total is nonzero (an idle call with nothing new
/// to publish must not spuriously bump a counter meant to reflect real
/// throughput).
pub(crate) fn drive_program_segmenters<S: IngestSession>(
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
            route_handle,
            route_handle.target_duration_secs(),
            route_handle.part_target_ms(),
        ) else {
            continue;
        };
        // SPTS_PROGRAM_ID's init bytes are the one piece of segmenter output
        // pushed to RouteHandle here directly — see this module's own doc for
        // why, and why every other program still segments (just isn't wired
        // to HTTP egress yet: issue #805 task 6's MPTS-addressing doc in
        // `crate::route`).
        if program == SPTS_PROGRAM_ID {
            if let Some(init) = segmenter.init_segment() {
                route_handle.set_init(SPTS_PROGRAM_ID, init);
            }
        }
        segmenters.insert(program, segmenter);
    }

    let mut parts_total = 0usize;
    let mut segments_total = 0usize;
    for segmenter in segmenters.values_mut() {
        let (parts, segments) = segmenter.pump();
        parts_total += parts;
        segments_total += segments;
    }

    if !driver.health().is_running() {
        for segmenter in segmenters.values_mut() {
            let (parts, segments) = segmenter.flush();
            parts_total += parts;
            segments_total += segments;
        }
    }

    if parts_total > 0 {
        metrics::counter!(
            crate::prometheus::PARTS_PRODUCED_TOTAL,
            "route" => route_handle.name().to_string(),
        )
        .increment(parts_total as u64);
    }
    if segments_total > 0 {
        metrics::counter!(
            crate::prometheus::SEGMENTS_PRODUCED_TOTAL,
            "route" => route_handle.name().to_string(),
        )
        .increment(segments_total as u64);
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
    use hls_runtime::server::{DEFAULT_TRACK_ID, HlsBody, HlsRequest};
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

    /// **The production segmenter gap, closed** (issue #805 task 2b). Drives
    /// the exact production chain a driver-backed route uses: a real
    /// `TsMux`-muxed TS stream → `TsIngestSession` → `IngestDriver` →
    /// `report_driver_progress` (publishes the registry) →
    /// `drive_program_segmenters` (this module) → the route's own
    /// `RouteHandle::init_bytes`/`HlsOrigin`.
    ///
    /// MUTATION VERIFIED: changing the `if program == SPTS_PROGRAM_ID { ...
    /// route_handle.set_init(SPTS_PROGRAM_ID, init); }` push in
    /// `drive_program_segmenters` to `if false { .. }` (never push init bytes
    /// to `RouteHandle` at all) makes this test's first assertion fail:
    /// `assert!(route.init_bytes(crate::route::SPTS_PROGRAM_ID).is_some_and(|b| !b.is_empty()), ...)`
    /// fails — actual value `None` (the registry's `ProgramServing` for
    /// program 0 exists and its `Trunk` carries real segments/parts, but its
    /// `HlsOrigin` never received the init segment bytes at all), not the
    /// expected `Some(bytes)`. Recompiled and re-run to confirm the failure,
    /// then reverted.
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
        let mut track_generations = HashMap::new();

        // First feed: resolves the PMT, mints program 0's driver-side Trunk.
        let ts_bytes = build_ts_bytes(1, 0xAB, 90);
        driver.feed(&ts_bytes, Timestamp::ZERO);
        report_driver_progress(&driver, &route, &mut published, &mut track_generations);
        drive_program_segmenters(&driver, &route, &mut segmenters);

        // Second feed: more real media, proving the segmenter's cursor
        // (subscribed just above via `subscribe_from_backlog` — issue #808)
        // keeps observing new samples live *in addition to* replaying the
        // first feed's own backlog, not merely as a substitute for it.
        let more = build_ts_bytes(1, 0xCD, 90);
        driver.feed(&more, Timestamp::from_nanos(1));
        report_driver_progress(&driver, &route, &mut published, &mut track_generations);
        drive_program_segmenters(&driver, &route, &mut segmenters);

        assert!(
            route
                .init_bytes(crate::route::SPTS_PROGRAM_ID)
                .is_some_and(|b| !b.is_empty()),
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
            "the route's own HlsOrigin must serve real closed segments/parts, \
             resolved exactly the way egress resolves them: {playlist}"
        );
    }

    /// **Issue #809.** `multimux_parts_produced_total`/
    /// `multimux_segments_produced_total` must actually move for a
    /// driver-backed route — these two counters had no emitter at all since
    /// the media-plane port (silently reading zero, then deleted outright)
    /// until `drive_program_segmenters` started bumping them directly. Uses a
    /// distinctively-named route (`with_name`) so the assertion reads this
    /// test's own series, not whatever another test in this shared-process
    /// binary already recorded under a different (or default) route label.
    ///
    /// MUTATION VERIFIED: commenting out both `if parts_total > 0 { ... }`/
    /// `if segments_total > 0 { ... }` metric-emitting blocks in
    /// `drive_program_segmenters` (simulating the exact #809 regression: the
    /// segmenting logic runs correctly, nothing ever reports it) makes this
    /// test's `assert!(parts_after > parts_before, ...)` fail: actual
    /// `parts_after == parts_before` (both `0.0`, since nothing ever
    /// increments the counter) instead of `parts_after > parts_before` —
    /// real parts/segments are demonstrably produced (this test's sibling,
    /// `driver_backed_route_serves_real_media_through_ll_hls`, proves that via
    /// the served playlist), but the metric stays silently at zero. Recompiled
    /// and re-run to confirm the failure, then reverted.
    #[test]
    fn drive_program_segmenters_bumps_parts_and_segments_produced_counters() {
        crate::prometheus::install();
        let route = RouteHandle::new(1.0, 250, 8).with_name("segment-metrics-probe-route");
        let mut driver = IngestDriver::new(
            TsIngestSession::new(),
            trunk_config(),
            handshake(),
            media_plane::DEFAULT_MAX_PROGRAMS,
        );
        let mut published = HashSet::new();
        let mut segmenters = HashMap::new();
        let mut track_generations = HashMap::new();

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

        let ts_bytes = build_ts_bytes(1, 0xAB, 90);
        driver.feed(&ts_bytes, Timestamp::ZERO);
        report_driver_progress(&driver, &route, &mut published, &mut track_generations);
        drive_program_segmenters(&driver, &route, &mut segmenters);
        let more = build_ts_bytes(1, 0xCD, 90);
        driver.feed(&more, Timestamp::from_nanos(1));
        report_driver_progress(&driver, &route, &mut published, &mut track_generations);
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
    /// which `hls_runtime` renders unconditionally even for a route with
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
        let mut track_generations = HashMap::new();

        // ONE feed: build_ts_bytes muxes the PMT (-> NewProgram) and 90 real
        // PES samples (-> Sample events) into the same TS byte stream, so
        // the driver's own `feed` drains both the NewProgram announcement
        // and every one of those samples in this single call — exactly the
        // "ordinary MPEG-TS" shape issue #808 describes, not a contrived
        // edge case.
        let ts_bytes = build_ts_bytes(1, 0xAB, 90);
        driver.feed(&ts_bytes, Timestamp::ZERO);
        report_driver_progress(&driver, &route, &mut published, &mut track_generations);
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
        let mut track_generations = HashMap::new();

        let ts_bytes = build_ts_bytes(1, 0xAB, 90);
        driver.feed(&ts_bytes, Timestamp::ZERO);
        report_driver_progress(&driver, &route, &mut published, &mut track_generations);
        drive_program_segmenters(&driver, &route, &mut segmenters);

        let more = build_ts_bytes(1, 0xCD, 90);
        driver.feed(&more, Timestamp::from_nanos(1));
        report_driver_progress(&driver, &route, &mut published, &mut track_generations);
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
                assert!(Arc::ptr_eq(&resolved.trunk(), &program_trunk));
            }
            _ => panic!("expected the route's registry to resolve program 0"),
        }
    }

    const FRAME_DUR: u32 = transmux::VIDEO_CLOCK_RATE / 30;

    /// A length-prefixed AVC NAL sample — the same per-sample byte layout
    /// `ts_program::test_support::build_ts_bytes` uses (`(nal.len() as
    /// u32).to_be_bytes()` prefix, then `[0x65, nal_byte, seq_byte]`), rather
    /// than an arbitrary opaque byte blob. `LlHlsSegmenter` never
    /// re-interprets a sample's bytes (fMP4 stores whatever it's given
    /// verbatim), so the older opaque-blob shape worked for that path; issue
    /// #887's `StreamingTsHlsSegmenter`/`ts_mux` path DOES parse AVC samples
    /// as length-prefixed NALs (to re-wrap them Annex-B for the muxed TS), so
    /// `TwoProgramSession`'s samples must be real ones for
    /// `ts_hls_mpts_route_segments_each_program_into_single_pmt_segments`
    /// (below) to mux successfully — this shape also still satisfies every
    /// existing fMP4-path test in this module, which never inspects sample
    /// content.
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
        let mut track_generations = HashMap::new();

        // Feed 1: Established. Feed 2: both NewProgram announcements (mints
        // both driver-side Trunks and, via `drive_program_segmenters`,
        // subscribes both `ProgramSegmenter`s *before* any sample exists).
        driver.feed(&[], Timestamp::from_nanos(0));
        report_driver_progress(&driver, &route, &mut published, &mut track_generations);
        drive_program_segmenters(&driver, &route, &mut segmenters);
        driver.feed(&[], Timestamp::from_nanos(1));
        report_driver_progress(&driver, &route, &mut published, &mut track_generations);
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

    /// Render `route`'s current classic-HLS media playlist for `program`
    /// synchronously — the multi-program analogue of `render_playlist` (which
    /// hardcodes `SPTS_PROGRAM_ID`), needed by the MPTS test below since it
    /// must render two DIFFERENT programs' playlists off the SAME route.
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
    /// `crate::origin::resource::dynamic_file` drives in production, minus
    /// the axum/tokio wrapping (mirrors `render_playlist`/`render_playlist_for`
    /// resolving playlists the same way).
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

    /// The first `.ts` segment URI line in a rendered classic-HLS media
    /// playlist (a bare `seg-{track}-{seq}.ts` line, distinct from every
    /// `#`-prefixed tag line) — `None` if the playlist advertises no closed
    /// segment yet.
    fn first_ts_segment_uri(playlist: &str) -> Option<&str> {
        playlist
            .lines()
            .find(|l| !l.starts_with('#') && l.ends_with(".ts"))
    }

    /// Count PMT sections (`table_id == 0x02`, ISO/IEC 13818-1 Table 2-31) in
    /// one whole-packet MPEG-2 TS byte buffer — a genuine parse of the wire
    /// bytes (per-PID PSI section reassembly via `mpeg_ts::ts::SectionReassembler`,
    /// same machinery `dvb-si`'s own `ts` feature is built on), not an
    /// assumption about which PID `transmux::ts_mux` happens to place the PMT
    /// on (issue #887's MPTS test needs this to actually *prove* RFC
    /// 8216bis §3.1.1's "a Transport Stream Segment MUST contain a single
    /// MPEG-2 Program" constraint, not merely trust `ts_mux`'s own
    /// single-PMT-by-construction doc).
    fn count_pmt_sections(ts_bytes: &[u8]) -> usize {
        use broadcast_common::Parse;
        use mpeg_ts::section::Section;
        use mpeg_ts::ts::{SectionReassembler, TsPacket};

        const TS_PACKET_SIZE: usize = 188;
        const PMT_TABLE_ID: u8 = 0x02;

        let mut reassemblers: HashMap<u16, SectionReassembler> = HashMap::new();
        let mut pmt_count = 0usize;
        for chunk in ts_bytes.chunks_exact(TS_PACKET_SIZE) {
            let packet = TsPacket::parse(chunk).expect("a muxed TS segment is well-formed packets");
            let Some(payload) = packet.payload else {
                continue;
            };
            let reassembler = reassemblers.entry(packet.header.pid).or_default();
            reassembler.feed(payload, packet.header.pusi);
            while let Some(section_bytes) = reassembler.pop_section() {
                if let Ok(section) = Section::parse(section_bytes.as_ref()) {
                    if section.table_id == PMT_TABLE_ID {
                        pmt_count += 1;
                    }
                }
            }
        }
        pmt_count
    }

    /// **Issue #887 — RFC 8216bis §3.1.1's "single MPEG-2 Program"
    /// constraint, proven by parsing served bytes.** Feeds the same
    /// hand-scripted two-program (MPTS) session
    /// `two_programs_on_one_route_segment_independently` uses into a
    /// `ts_hls` route (`Container::MpegTs`) instead of the default fMP4
    /// container, then asserts:
    ///
    /// 1. each program renders its OWN classic-HLS playlist — resolved off
    ///    its own `Trunk`/`HlsOrigin`, never a shared one — referencing `.ts`
    ///    segment URIs with no `#EXT-X-MAP` (classic TS is
    ///    self-initialising), and
    /// 2. each program's first served `.ts` segment contains EXACTLY ONE
    ///    PMT section — never zero (a malformed mux) and never more than
    ///    one (which would make the segment a genuine multi-program
    ///    stream, exactly what RFC 8216bis §3.1.1 says playback is
    ///    undefined for).
    ///
    /// The architecture already satisfies this by construction
    /// (`transmux::ts_mux` mints one PAT + one PMT per mux call — see that
    /// module's own doc — and `drive_program_segmenters` gives each program
    /// its own independent `StreamingTsHlsSegmenter`/mux, never a shared
    /// one), so this test is the missing proof, not a fix: it parses the
    /// actual served TS bytes via `count_pmt_sections` rather than trusting
    /// that construction.
    #[test]
    fn ts_hls_mpts_route_serves_each_program_as_its_own_single_pmt_playlist() {
        let route = RouteHandle::new(1.0, 500, 8).with_container(Container::MpegTs);
        let mut driver = IngestDriver::new(
            TwoProgramSession::new(),
            trunk_config(),
            handshake(),
            media_plane::DEFAULT_MAX_PROGRAMS,
        );
        let mut published = HashSet::new();
        let mut segmenters = HashMap::new();
        let mut track_generations = HashMap::new();

        // Feed 1: Established. Feed 2: both NewProgram announcements.
        driver.feed(&[], Timestamp::from_nanos(0));
        report_driver_progress(&driver, &route, &mut published, &mut track_generations);
        drive_program_segmenters(&driver, &route, &mut segmenters);
        driver.feed(&[], Timestamp::from_nanos(1));
        report_driver_progress(&driver, &route, &mut published, &mut track_generations);
        drive_program_segmenters(&driver, &route, &mut segmenters);

        // Same 90-sample budget as `two_programs_on_one_route_segment_independently`
        // -- comfortably over the 1.0s target duration for both programs.
        for _ in 0..90 {
            let now = Timestamp::from_nanos(driver_feed_nanos());
            driver.feed(&[], now);
            drive_program_segmenters(&driver, &route, &mut segmenters);
        }

        // "Own playlist" means independently resolvable off its own
        // Trunk/HlsOrigin, not merely textually different — with both
        // programs fed identical sample cadences (`TwoProgramSession` pushes
        // the same `sample_at(i, is_sync)` bytes to each), their rendered
        // playlist text can legitimately coincide (same URI-naming scheme,
        // same durations) even though the underlying `.ts` segment bytes and
        // Trunks are genuinely distinct — proven the same way
        // `two_programs_on_one_route_segment_independently` proves it.
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
