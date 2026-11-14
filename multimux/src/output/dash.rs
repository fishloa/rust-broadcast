//! `DashOutput`: the DASH [`crate::output::Output`] implementation (issue
//! #663 P4) — renders `manifest.mpd` from the shared [`crate::route::RouteHandle`]'s
//! `Trunk`-drained window via `transmux::dash::DashPackager`, resolved
//! through the same one adapter every other output uses
//! (`crate::http::resolve_blocking`/`crate::http::into_response`).
//! Init/segment byte ranges are the origin's *shared* resource route
//! (`crate::origin::resource`) — the exact same bytes [`crate::output::hls`]
//! serves; this module only renders the manifest (see `crate::output`
//! module docs for why the byte-serving is shared, not per-output).
//!
//! # Addressing (why `$Number$`, not `$Time$`)
//!
//! The shared resource route names closed segments `seg-{track}-{seq}.m4s`,
//! where `{seq}` is a plain monotonic sequence number
//! ([`media_plane::trunk::SegmentEntry::sequence_number`]) — **not** a
//! cumulative-duration timestamp. `SegmentTemplate`'s `$Time$` substitution
//! (ISO/IEC 23009-1 §5.3.9.4.4 / §5.3.9.6) is the segment's *start time*,
//! which would not match those filenames. `$Number$` substitution is a
//! literal, caller-chosen integer per segment (this module sets
//! [`DashPackager::start_number`] to the **first** `segment_seq` the program
//! ever published (`FIRST_SEGMENT_NUMBER`, `1`), constant for its life, and relies on `$Number$` counting up
//! from there — NOT the window's oldest, which would make a client's
//! `availabilityStartTime`-derived live edge point at segments that do not
//! exist once the window slides, issue #1058) — that *is* `segment_seq`,
//! so [`transmux::Addressing::Number`] is the only mode that produces URIs
//! the shared resource route actually resolves.
//!
//! # Single-rendition model
//!
//! Like [`crate::output::hls`] (see [`DEFAULT_TRACK_ID`]'s docs), exactly
//! one `Representation` is described, using one of the route's recorded
//! [`transmux::TrackSpec`]s ([`crate::route::RouteHandle::set_track_specs`])
//! — but with its `track_id` forced to [`DEFAULT_TRACK_ID`] so the DASH
//! client's `$RepresentationID$` substitution produces the same
//! `init-1.mp4`/`seg-1-<N>.m4s` filenames the shared resource route serves,
//! regardless of the source's own track numbering.
//!
//! # Track selection (issue #776)
//!
//! `render_mpd` used to take the route's *first* recorded track
//! unconditionally. A routine DVB multiplex's first elementary stream is
//! often teletext, DSM-CC, or SCTE-35 — an opaque
//! [`transmux::CodecConfig::Data`] track with no derivable RFC 6381 codec
//! string — so `DashPackager::package` rejected the built [`Media`] and the
//! whole DASH route returned a **permanent** `503`, even though a perfectly
//! representable video/audio track sat right behind it in the PMT's track
//! list. `select_representable_track` fixes this: it selects the first
//! track (preferring a video-shaped codec, then any other, e.g. audio) that
//! actually produces a derivable codec string — proven by trial-packaging it
//! through the real [`DashPackager`] rather than re-deriving "is this codec
//! supported" here (which would drift from `DashPackager::codec_string`'s
//! own, more authoritative, list). Only a track set with genuinely **no**
//! representable track is still a `503`.
//!
//! # One renderer, two flavours
//!
//! This module renders both `manifest.mpd` and the LL-DASH `manifest-ll.mpd`:
//! the MPD is built once and `DashFlavour` selects whether the low-latency
//! fields are added (see `DashFlavour::LowLatency`; the route itself is
//! mounted by [`crate::output::ll_dash`], issue #663 P4.2 / #721 / #2448).

use broadcast_common::units::{MILLIS_PER_SEC, NANOS_PER_MILLI};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
#[cfg(test)]
use transmux::TrackId;

use axum::Router;
use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use broadcast_common::Package;
use broadcast_dash::duration::xs_duration;
use hls_runtime::server::DEFAULT_TRACK_ID;
use media_plane::egress::{AwaitPolicy, CachePolicy, EgressResponse, ServedEgress};
use transmux::{Addressing, DashPackager, Media, Track, TrackSegments, TrackSpec};

use crate::http::{self, BLOCKING_RELOAD_TIMEOUT};
use crate::origin::resource::cors_preflight;
use crate::output::{Output, OutputKind};
use crate::route::RouteHandle;

/// `pub(crate)` (not private) since issue #663 P4.2: `crate::output::ll_dash`
/// serves the same `application/dash+xml` content type for its LL-DASH
/// manifest and reuses this constant rather than duplicating the literal.
pub(crate) const DASH_MANIFEST_CONTENT_TYPE: &str = "application/dash+xml";

/// The DASH [`Output`]: `manifest.mpd` only. Init/segment bytes are the
/// origin's shared resource route — see the module docs.
pub struct DashOutput;

impl Output for DashOutput {
    fn kind(&self) -> OutputKind {
        OutputKind::Dash
    }

    /// Routes (relative — mounted by the origin under `/{stream}/`):
    /// - `GET /manifest.mpd` — the live MPD.
    fn manifest_routes(&self, route: Arc<RouteHandle>) -> Router {
        Router::new()
            .route("/manifest.mpd", get(manifest).options(cors_preflight))
            .with_state(route)
    }
}

/// `GET /manifest.mpd` — `503 Service Unavailable` until the route has at
/// least one representable track (issue #776: no track with a derivable
/// codec string, not merely "no segment has closed yet", which still
/// renders a valid, if near-empty, MPD).
async fn manifest(State(route): State<Arc<RouteHandle>>) -> Response {
    serve_mpd(route, DashFlavour::Standard).await
}

/// The shared DASH/LL-DASH manifest handler: resolves `route`'s program and
/// serves whatever `render` produces (`503` when it renders nothing).
pub(crate) async fn serve_mpd(route: Arc<RouteHandle>, flavour: DashFlavour) -> Response {
    let serving = match http::resolve_route_program(&route) {
        Ok(serving) => serving,
        Err(resp) => return *resp,
    };
    let trunk = serving.trunk();
    let origin = Arc::new(MpdOrigin { route, flavour });
    let resp = http::resolve_blocking(
        &trunk,
        serving.warms(),
        &origin,
        (),
        BLOCKING_RELOAD_TIMEOUT,
        || (),
    )
    .await;
    http::into_response(resp, StatusCode::SERVICE_UNAVAILABLE, |body| {
        ([(header::CONTENT_TYPE, DASH_MANIFEST_CONTENT_TYPE)], body).into_response()
    })
}

/// The DASH/LL-DASH manifest [`ServedEgress`]: a stateless render of `route`'s
/// current window/track specs — never answers
/// [`EgressResponse::Await`] (there is nothing to wait for; an MPD can
/// always be rendered from whatever window state currently exists, or
/// answers `NotFound` once and for all if no track is representable), so
/// [`http::resolve_blocking`] resolves it on the very first check without
/// ever touching `Trunk::listen`. Still routed through the same one adapter
/// as every other output, rather than a bespoke handler, so a manifest route
/// can never drift into its own ad hoc HTTP-mapping logic — see
/// `crate::http`'s own module doc.
struct MpdOrigin {
    route: Arc<RouteHandle>,
    flavour: DashFlavour,
}

impl ServedEgress for MpdOrigin {
    type Request = ();
    type Body = String;

    fn resolve(
        &self,
        _request: (),
        _now: broadcast_common::Timestamp,
        _await_policy: AwaitPolicy,
    ) -> EgressResponse<String> {
        match render_mpd_cached(&self.route, self.flavour) {
            Some(body) => EgressResponse::Ready {
                body,
                cache: CachePolicy::NoCache,
            },
            None => EgressResponse::NotFound,
        }
    }
}

/// `SegmentTemplate@startNumber` of every DASH/LL-DASH MPD: the number of the
/// first segment a program publishes. A `Trunk` numbers its segments from `1`
/// for the life of the program and never renumbers
/// ([`media_plane::trunk::SegmentWriter::next_sequence_number`] resumes from the
/// last accepted number, or `1` for a fresh `Trunk`), so this is the same number
/// the segment at timeline position `0` carries -- constant however far the
/// window has slid, which the live-edge arithmetic of ISO/IEC 23009-1
/// §5.3.9.5.3 requires (issue #1058).
pub(crate) const FIRST_SEGMENT_NUMBER: u64 = 1;

/// Which manifest flavour the shared renderer produces. Either flavour builds
/// the MPD model the same way: the Period split at a gap placeholder and at
/// each new init generation, the per-init-generation `initialization` URL and
/// the SCTE-35 inband event declaration are common to both.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) enum DashFlavour {
    /// Standard DASH (`manifest.mpd`): no low-latency signalling, refreshed
    /// once per whole segment (`minimumUpdatePeriod` at the segment target).
    Standard,
    /// Low-latency DASH (`manifest-ll.mpd`): adds only the LL fields on top of
    /// the same MPD — a `ServiceDescription`/`Latency` target derived from the
    /// part target, per-segment `availabilityTimeOffset` (segment target minus
    /// chunk) with `availabilityTimeComplete="false"`, and
    /// `minimumUpdatePeriod` at the chunk interval so a client refreshes the
    /// MPD as chunks land rather than once per segment.
    LowLatency,
}

/// `SegmentTemplate@startNumber`: the public number of the first segment the
/// route ever served, unchanged by a source reconnect. The numbers keep
/// counting through the reconnect (the resource route, the HLS playlist and
/// the DVR archive share them), `availabilityStartTime` and the Period are
/// unchanged, and only the segments still in the window are advertised.
pub(crate) fn start_number(route: &RouteHandle) -> u64 {
    route
        .epoch(crate::route::SPTS_PROGRAM_ID)
        .map_or(FIRST_SEGMENT_NUMBER, |epoch| epoch.first_number)
}

/// Select the first of `specs` that produces a derivable RFC 6381 codec
/// string, preferring a video-shaped codec over any other kind (audio,
/// or a future codec this classifier doesn't recognise) — see this
/// module's own "Track selection (issue #776)" doc.
///
/// Two passes over `specs`, each in original (PMT) order: video-classified
/// tracks first, then every other track. Within each pass, the first track
/// that [`track_is_representable`] accepts wins.
pub(crate) fn select_representable_track(specs: &[TrackSpec]) -> Option<TrackSpec> {
    specs
        .iter()
        .filter(|s| s.config.is_video())
        .find(|s| track_is_representable(s))
        .or_else(|| specs.iter().find(|s| track_is_representable(s)))
        .cloned()
}

/// `true` if `spec`'s codec produces a derivable RFC 6381 codec string —
/// determined by actually trial-packaging a one-track [`Media`] through the
/// real [`DashPackager`], rather than re-deriving `DashPackager::codec_string`'s
/// own (private) codec-support list here, which could silently drift from
/// it. An opaque [`transmux::CodecConfig::Data`]/`Subtitle` track (teletext,
/// DSM-CC, SCTE-35) fails this — the exact case issue #776 exists to skip
/// over rather than reject the whole route for.
fn track_is_representable(spec: &TrackSpec) -> bool {
    let media = Media::new(
        vec![Track::new(spec.clone(), Vec::new())],
        spec.timescale.get().max(1),
    );
    DashPackager::default().package(&media).is_ok()
}

/// The live MPD of `flavour`, served from the program's cache when nothing it
/// depends on has changed since it was rendered.
fn render_mpd_cached(route: &RouteHandle, flavour: DashFlavour) -> Option<String> {
    let serving = route.serving(crate::route::SPTS_PROGRAM_ID)?;
    serving.cached_mpd(flavour, || match flavour {
        DashFlavour::Standard => render_mpd(route),
        DashFlavour::LowLatency => render_ll_dash_mpd(route),
    })
}

/// Render the live MPD for `route`'s current window. `None` if no track in
/// [`RouteHandle::track_specs`] is [`select_representable_track`]-selectable
/// (nothing recorded yet, or every recorded track is opaque — issue #776).
pub(crate) fn render_mpd(route: &RouteHandle) -> Option<String> {
    let origin = route.presentation_origin(crate::route::SPTS_PROGRAM_ID)?;
    render_mpd_at_with_flavour(route, origin, DashFlavour::Standard)
}

/// Render `route`'s current window as the low-latency flavour of the MPD, with
/// `availabilityStartTime` taken from the program's own presentation origin.
pub(crate) fn render_ll_dash_mpd(route: &RouteHandle) -> Option<String> {
    let origin = route.presentation_origin(crate::route::SPTS_PROGRAM_ID)?;
    render_mpd_at_with_flavour(route, origin, DashFlavour::LowLatency)
}

/// Render `route`'s current window as the low-latency flavour of the MPD with
/// an explicit presentation `origin` for `availabilityStartTime`, so a test can
/// freeze the wall-clock value.
#[doc(hidden)]
pub fn render_ll_dash_mpd_at(route: &RouteHandle, origin: SystemTime) -> Option<String> {
    render_mpd_at_with_flavour(route, origin, DashFlavour::LowLatency)
}

/// One Period of the MPD: a run of consecutive media segments of one Period, closed by the
/// gap placeholder or the first segment of the next Period that follows it (if any).
struct PeriodRun<'a> {
    segments: &'a [crate::route::DashWindowSegment],
    ended_by: Option<&'a crate::route::DashWindowSegment>,
}

/// Split `window` into the runs that each become one Period: a run ends at a gap placeholder
/// (which belongs to no Period) and where the next Period opens (an init generation change,
/// see [`crate::route::PeriodAnchor`]).
fn period_runs(window: &[crate::route::DashWindowSegment]) -> Vec<PeriodRun<'_>> {
    let mut runs = Vec::new();
    let mut start = 0;
    for (at, segment) in window.iter().enumerate() {
        let period_boundary = at > 0 && at > start && segment.period != window[at - 1].period;
        if segment.gap || period_boundary {
            if start < at {
                runs.push(PeriodRun {
                    segments: &window[start..at],
                    ended_by: Some(segment),
                });
            }
            if segment.gap {
                start = at + 1;
            } else {
                start = at;
            }
        }
    }
    if start < window.len() {
        runs.push(PeriodRun {
            segments: &window[start..],
            ended_by: None,
        });
    }
    runs
}

/// Render the MPD with an explicit presentation `origin` for `availabilityStartTime`, so a
/// golden can freeze the wall-clock value (`render_mpd` delegates with the
/// program's own `presentation_origin`). `#[doc(hidden)]` and only reachable from tests.
///
/// Each run of media segments of one Period is its own `Period` element (ISO/IEC 23009-1
/// §5.3.2: a Period is a span of the presentation with a fixed set of encoded versions;
/// `private/specs-verified/dash/23009-1/05-3-2-period.md`). A Period opens at the first segment
/// after a gap placeholder and at the first segment of a new init generation, including the
/// first segment of a reloaded route whose init differs from the one it replaced (issue #2611).
/// See [`render_mpd_at_with_flavour`] for the attributes each Period carries.
#[doc(hidden)]
pub fn render_mpd_at(route: &RouteHandle, origin: SystemTime) -> Option<String> {
    render_mpd_at_with_flavour(route, origin, DashFlavour::Standard)
}

/// Render the MPD with an explicit presentation `origin` and `flavour`.
///
/// The Period that opened at the presentation's first segment is `id="0"`, `start="PT0S"`
/// with the presentation's `startNumber` and no `@presentationTimeOffset`. Every later Period
/// is identified by its [`crate::route::PeriodAnchor`], fixed when it opened, so an MPD update
/// never changes it (`private/specs-verified/dash/23009-1/05-4_to_5-7.md` §5.4.1):
///
/// - `@id` is the Period's ordinal in the presentation (§5.3.2.2: unique, present and unchanged
///   in a dynamic MPD);
/// - `@start` is its first segment's nominal MPD start, `(number - startNumber) * @duration`
///   of the presentation's numbering, and a Period followed by another or by a gap carries
///   `@duration` to the end of its last listed segment (§5.3.2.1; an early terminated Period when
///   a replaced program's unfinished last segment leaves a hole before the next `@start`);
/// - a replaced program's Period stays listed, with its own init URL, while its segments are in
///   the window (see `ProgramServing::dash_window_segments`), so nothing inside the time-shift
///   buffer is withdrawn (§5.3.9.5.3);
/// - `SegmentTemplate@startNumber` is its first segment's number and
///   `@presentationTimeOffset` that segment's media start in the template's timescale, so the
///   first segment presents at the Period start (§5.3.9.5;
///   `private/specs-verified/dash/dashif-iop-4.3/ch04-06-deployment-scenarios.md` 4.11.4.2).
///
/// A Period opened at an init change is neither period-continuous nor period-connected
/// (§5.3.2.4: both need functionally equivalent Initialization Segments), so no
/// `urn:mpeg:dash:period-continuity:2015`/`period-connectivity:2015` descriptor is written, and
/// no `AssetIdentifier`: the route has no asset identity to name, and DASH-IF IOP 5.5.1 forbids
/// one for a Period without a known continuation
/// (`private/specs-verified/dash/dashif-iop-4.3/ch05-ad-insertion.md`).
///
/// A continued program whose init is unchanged stays in the open Period unless its numbering
/// skipped the replaced program's unfinished segment, which no origin serves (issue #2625). Then
/// the open Period is early terminated at its last finished segment (§5.3.2.1) and the next
/// Period opens at the first new segment, as DASH-IF IOP 4.3 4.11.4.2 prescribes for a `@duration`
/// live service whose media resumes after an outage
/// (`private/specs-verified/dash/dashif-iop-4.3/ch04-06-deployment-scenarios.md`): the same
/// `AdaptationSet@id` and `Representation@id`, an equivalent `AssetIdentifier` ([`ASSET_ID_SCHEME`])
/// on both Periods so their Adaptation Sets are associated, and a
/// `urn:mpeg:dash:period-connectivity:2015` descriptor naming the earlier Period's `@id` while
/// that Period is listed (§5.3.2.4: the identical init allows connectivity; the skipped segment
/// rules out continuity). A `SegmentTimeline` cannot express the hole instead: `$Number$` there
/// numbers segments consecutively, and `S@n` would change the addressing every client of this
/// `@duration` MPD relies on (§5.3.9.6).
#[doc(hidden)]
pub(crate) fn render_mpd_at_with_flavour(
    route: &RouteHandle,
    origin: SystemTime,
    flavour: DashFlavour,
) -> Option<String> {
    let program = crate::route::SPTS_PROGRAM_ID;
    let window = route.dash_window_segments(program);
    let runs = period_runs(&window);
    let first_number = start_number(route);
    let first_period = crate::route::PeriodAnchor {
        ordinal: 0,
        first_public_seq: first_number,
        media_start_ns: 0,
        connected_to: None,
        asset: None,
    };
    let serving = route.serving(program);
    let own_instance = serving.as_ref().map(|s| s.ll_hls().instance());
    let mut plans: Vec<PeriodPlan> = Vec::new();
    for run in &runs {
        let first = run.segments[0];
        let Some(last) = run.segments.last() else {
            continue;
        };
        let specs = if Some(last.instance) == own_instance {
            if run.ended_by.is_some() {
                route.track_specs_at(program, first.segment_seq)
            } else {
                route.track_specs(program)
            }
        } else {
            serving
                .as_ref()
                .and_then(|s| s.retained_serving(last.instance))
                .map(|retained| retained.track_specs_at(last.segment_seq))
                .unwrap_or_default()
        };
        plans.push(PeriodPlan {
            anchor: first.period.unwrap_or(first_period),
            instance: last.instance,
            init_generation: last.init_generation,
            specs,
            has_segments: true,
            end_seq: run.ended_by.map(|_| last.public_seq.saturating_add(1)),
        });
    }
    let own_segments = window.iter().any(|s| Some(s.instance) == own_instance);
    if !own_segments {
        let anchor = serving.as_ref().map_or(first_period, |s| s.first_period());
        if plans.last().is_none_or(|last| last.anchor != anchor) {
            if let Some(previous) = plans.last_mut()
                && let Some(last) = runs.last().and_then(|run| run.segments.last())
            {
                previous.end_seq = Some(last.public_seq.saturating_add(1));
            }
            plans.push(PeriodPlan {
                anchor,
                instance: own_instance.unwrap_or(1),
                init_generation: serving.as_ref().map_or_else(
                    || media_plane::ids::Generation::new(1),
                    |s| s.ll_hls().current_init_generation(),
                ),
                specs: route.track_specs(program),
                has_segments: anchor != first_period,
                end_seq: None,
            });
        }
    }
    if let [only] = plans.as_slice()
        && only.end_seq.is_none()
        && only.anchor == first_period
    {
        return render_period_document(
            route,
            origin,
            &window,
            &only.specs,
            first_number,
            only.has_segments,
            only.instance,
            only.init_generation,
            flavour,
        );
    }
    let nominal_secs = route.target_duration_secs();
    let mut mpd: Option<broadcast_dash::Mpd> = None;
    let mut rendered: Vec<u64> = Vec::new();
    for plan in &plans {
        let anchor = plan.anchor;
        let Some(xml) = render_period_document(
            route,
            origin,
            &window,
            &plan.specs,
            anchor.first_public_seq,
            plan.has_segments,
            plan.instance,
            plan.init_generation,
            flavour,
        ) else {
            continue;
        };
        let mut parsed = broadcast_dash::Mpd::parse(xml.as_bytes()).ok()?;
        if parsed.periods.is_empty() {
            continue;
        }
        let mut period = parsed.periods.remove(0);
        let start = period_start(anchor.first_public_seq, first_number, nominal_secs)?;
        period.id = Some(anchor.ordinal.to_string());
        period.start = Some(start);
        period.duration = match plan.end_seq {
            Some(end) => Some(period_start(end, first_number, nominal_secs)?.checked_sub(start)?),
            None => None,
        };
        set_presentation_time_offset(&mut period, anchor.media_start_ns)?;
        let asset = anchor.asset.or_else(|| {
            plans
                .iter()
                .any(|later| later.anchor.asset == Some(anchor.ordinal))
                .then_some(anchor.ordinal)
        });
        if let Some(asset) = asset {
            let mut identifier = broadcast_dash::Descriptor::default();
            identifier.scheme_id_uri = ASSET_ID_SCHEME.to_owned();
            identifier.value = Some(format!("{}/{asset}", route.name()));
            period.asset_identifier = Some(identifier);
        }
        if let Some(previous) = anchor.connected_to
            && rendered.contains(&previous)
        {
            for adaptation_set in &mut period.adaptation_sets {
                let mut connectivity = broadcast_dash::Descriptor::default();
                connectivity.scheme_id_uri =
                    broadcast_dash::descriptors::SCHEME_PERIOD_CONNECTIVITY.to_owned();
                connectivity.value = Some(previous.to_string());
                adaptation_set.supplemental_properties.push(connectivity);
            }
        }
        rendered.push(anchor.ordinal);
        match mpd.as_mut() {
            Some(base) => base.periods.push(period),
            None => {
                parsed.periods.push(period);
                mpd = Some(parsed);
            }
        }
    }
    mpd?.to_xml().ok()
}

/// `AssetIdentifier@schemeIdUri` naming a route's run of period-connected Periods
/// (ISO/IEC 23009-1 §5.8.4.10; a private scheme, DASH-IF IOP 4.3 5.5.1), with
/// `@value` `<route name>/<first Period @id of the run>`.
pub(crate) const ASSET_ID_SCHEME: &str =
    "https://github.com/fishloa/rust-broadcast/multimux/dash-asset-id";

/// What one `Period` element is rendered from.
struct PeriodPlan {
    anchor: crate::route::PeriodAnchor,
    /// The origin whose init segment the Period names, and its generation there.
    instance: u64,
    init_generation: media_plane::ids::Generation,
    specs: Vec<TrackSpec>,
    has_segments: bool,
    /// The number after the Period's last listed segment, when a later Period or a gap follows:
    /// `@duration` ends there.
    end_seq: Option<u64>,
}

/// Set `@presentationTimeOffset` on every `SegmentTemplate` of `period` to `media_start_ns` in
/// that template's timescale, rounded to the nearest tick (left absent when it is zero, the attribute's default). `None` if
/// the offset does not fit the attribute.
fn set_presentation_time_offset(
    period: &mut broadcast_dash::Period,
    media_start_ns: u64,
) -> Option<()> {
    for adaptation_set in &mut period.adaptation_sets {
        let templates = std::iter::once(&mut adaptation_set.segment).chain(
            adaptation_set
                .representations
                .iter_mut()
                .map(|r| &mut r.segment),
        );
        for template in templates.filter_map(|layer| layer.template.as_mut()) {
            let timescale = template.multi.base.timescale.unwrap_or(1);
            let nanos_per_sec = u128::from(broadcast_common::units::NANOS_PER_SEC);
            let ticks = (u128::from(media_start_ns) * u128::from(timescale) + nanos_per_sec / 2)
                / nanos_per_sec;
            let ticks = u64::try_from(ticks).ok()?;
            template.multi.base.presentation_time_offset = (ticks > 0).then_some(ticks);
        }
    }
    Some(())
}

fn period_start(public_seq: u64, first_number: u64, nominal_secs: f64) -> Option<Duration> {
    let segments = public_seq.saturating_sub(first_number);
    Duration::try_from_secs_f64(nominal_secs * segments as f64).ok()
}

fn render_period_document(
    route: &RouteHandle,
    origin: SystemTime,
    window: &[crate::route::DashWindowSegment],
    specs: &[TrackSpec],
    start_number: u64,
    has_segments: bool,
    instance: u64,
    init_generation: media_plane::ids::Generation,
    flavour: DashFlavour,
) -> Option<String> {
    // `@id` forced to DEFAULT_TRACK_ID (see module docs' "Single-rendition
    // model") regardless of which track this route's own PMT numbered it.
    let mut spec = select_representable_track(specs)?;
    spec.track_id = DEFAULT_TRACK_ID;
    let timescale = spec.timescale.get().max(1);

    // `@startNumber` is the number of the first segment this program EVER
    // published ([`FIRST_SEGMENT_NUMBER`]), fixed for its life -- never the
    // window's oldest. ISO/IEC 23009-1 §5.3.9.5.3:
    // with `@duration` addressing segment N starts at
    // `(N - startNumber) * @duration` from the period start (and is available
    // `AST + (N - startNumber + 1) * @duration`), so a client computes the
    // live edge from `availabilityStartTime` + `startNumber`. A startNumber
    // that slid with the window while AST stayed fixed made it request
    // segments that do not exist (404) once the window slid (issue #1058).
    // `@duration` is the nominal target, constant across MPD refreshes (the
    // window's own first-segment duration changes as it slides, which would
    // change the client's whole number<->time mapping).
    let nominal_ticks =
        ((route.target_duration_secs() * f64::from(u32::from(timescale))).round() as u64).max(1);
    let segments = if has_segments || flavour == DashFlavour::LowLatency {
        vec![TrackSegments {
            track_id: DEFAULT_TRACK_ID,
            durations: vec![nominal_ticks],
        }]
    } else {
        Vec::new()
    };

    // timeShiftBufferDepth: the window's total buffered duration — the
    // configured target times the number of full segments retained
    // (`Config::window_segments`, reflected here as `window.len()` once the
    // window has filled). An approximation while the window is still
    // filling (fewer closed segments than the configured depth), which is
    // fine: the attribute only needs to bound how far back a client may
    // seek, never exactly.
    let target_duration_secs = route.target_duration_secs();
    let time_shift_buffer_depth_secs = target_duration_secs * (window.len().max(1) as f64);

    let media = Media::new(vec![Track::new(spec, Vec::new())], timescale);

    let init_template = format!(
        "init-$RepresentationID$-{}-{}.mp4",
        instance,
        init_generation.value()
    );
    let mut packager = DashPackager {
        dynamic: true,
        addressing: Addressing::Number,
        start_number,
        // `$RepresentationID$` is substituted by the DASH *client*, not
        // here (real DASH template tokens) — left literal so it resolves to
        // "1" (DEFAULT_TRACK_ID), matching the shared resource route's
        // `init-1.mp4`/`seg-1-<N>.m4s` filenames exactly.
        init_template,
        media_template: "seg-$RepresentationID$-$Number$.m4s".to_string(),
        availability_start_time: Some(format_iso8601(origin)),
        publish_time: Some(publish_time(origin, window)),
        minimum_update_period: Some(xs_duration_secs_checked(
            "minimum_update_period",
            if flavour == DashFlavour::LowLatency {
                (route.part_target_ms().max(1) as f64) / 1000.0
            } else {
                target_duration_secs
            },
        )),
        time_shift_buffer_depth: Some(xs_duration_secs_checked(
            "time_shift_buffer_depth",
            time_shift_buffer_depth_secs,
        )),
        segments,
        ..DashPackager::default()
    };

    // Declare SCTE-35 as an inband event stream (issue #969): the DASH client
    // is told "segments may carry `emsg` boxes with this scheme" (ANSI/SCTE
    // 214-3 §8.3.3). Always declared -- a client seeing the declaration with
    // no emsg boxes in any segment is harmless.
    packager
        .inband_event_streams
        .push(transmux::InbandEventStream {
            scheme_id_uri: "urn:scte:scte35:2013:bin".to_string(),
            value: None,
        });

    let xml = packager.package(&media).ok()?;
    if flavour == DashFlavour::LowLatency {
        let part_target_ms = route.part_target_ms().max(1);
        let chunk_duration_secs = f64::from(part_target_ms) / 1000.0;
        let latency_target_ms = u32::try_from(u64::from(part_target_ms) * 3).unwrap_or(u32::MAX);
        let ato = (target_duration_secs - chunk_duration_secs).max(0.0);
        let ato = (ato * 1000.0).round() / 1000.0;
        let mut mpd = broadcast_dash::Mpd::parse(xml.as_bytes()).ok()?;
        for template in mpd
            .periods
            .iter_mut()
            .flat_map(|p| p.adaptation_sets.iter_mut())
            .flat_map(|a| a.representations.iter_mut())
            .filter_map(|r| r.segment.template.as_mut())
        {
            template.multi.base.availability_time_offset = Some(ato);
            template.multi.base.availability_time_complete = Some(false);
        }
        // ServiceDescription/Latency
        if mpd.service_descriptions.is_empty() {
            use broadcast_dash::{Latency, ServiceDescription};
            let mut latency = Latency::default();
            latency.target = Some(u64::from(latency_target_ms));
            let mut sd = ServiceDescription::default();
            sd.id = Some(0);
            sd.latency = Some(latency);
            mpd.service_descriptions.push(sd);
        }
        mpd.to_xml().ok()
    } else {
        Some(xml)
    }
}

/// Format `t` as an ISO-8601 UTC timestamp (`YYYY-MM-DDTHH:MM:SSZ`) — the
/// `MPD@availabilityStartTime` wire format (ISO/IEC 23009-1 §5.3.1.2 Table 3).
/// Delegates to `jiff::Timestamp` (SP5), whose whole-second `Display` spelling
/// is byte-identical to the previous hand-rolled `civil_from_days` output (the
/// `dash_mpd`/`ll_dash_mpd` goldens were frozen before this migration and are
/// the witness).
///
/// `pub(crate)` (not private) since issue #663 P4.2: `crate::output::ll_dash`
/// needs the same `availabilityStartTime` formatting and reuses this rather
/// than duplicating the algorithm.
pub(crate) fn format_iso8601(t: SystemTime) -> String {
    let secs = t
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs();
    let secs = i64::try_from(secs).unwrap_or(i64::MAX);
    format_iso8601_secs(secs)
}

/// `#[doc(hidden)]` test seam over [`format_iso8601`].
#[doc(hidden)]
pub fn format_iso8601_for_test(t: SystemTime) -> String {
    format_iso8601(t)
}

/// `MPD@publishTime` (ISO/IEC 23009-1 §5.3.1.2 Table 3: shall be present for
/// `@type="dynamic"`, and an MPD with a later value shall be an update of one
/// with an earlier value). The MPD only changes when a segment closes, so the
/// stamp is the wall-clock instant the newest listed segment became available
/// (`origin` plus that segment's end on the program timeline): identical for
/// an unchanged window, strictly later after every new segment.
pub(crate) fn publish_time(
    origin: SystemTime,
    window: &[crate::route::DashWindowSegment],
) -> String {
    let end_ms = window.last().map_or(0, |s| {
        let dur = Duration::try_from_secs_f64(s.duration_secs).unwrap_or(Duration::ZERO);
        u128::from(s.timeline_position_ns / NANOS_PER_MILLI).saturating_add(dur.as_millis())
    });
    let ast_secs = origin
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs();
    let total_ms = u128::from(ast_secs)
        .saturating_mul(MILLIS_PER_SEC as u128)
        .saturating_add(end_ms);
    i64::try_from(total_ms)
        .ok()
        .and_then(|ms| jiff::Timestamp::from_millisecond(ms).ok())
        .map_or_else(|| format_iso8601(origin), |ts| ts.to_string())
}

fn format_iso8601_secs(secs: i64) -> String {
    jiff::Timestamp::from_second(secs)
        .map(|ts| ts.to_string())
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

/// Why [`xs_duration_secs`] could not render an `xs:duration`.
#[derive(Debug, thiserror::Error)]
pub(crate) enum DurationError {
    /// `secs` was NaN, infinite, negative, or too large to represent as a
    /// [`Duration`].
    #[error("invalid xs:duration input: {secs}")]
    InvalidInput { secs: f64 },
}

/// A whole/ fractional second count as an `xs:duration` via [`broadcast_dash::duration::xs_duration`] (the workspace's one `xs:duration` spelling rule),
/// without panicking on a bad input.
///
/// `secs` must be finite, strictly positive, and within [`Duration`]'s range
/// (`Duration::from_secs_f64` PANICS on NaN/negative/overflow — this returns
/// [`DurationError::InvalidInput`] instead). A zero/negative input is NOT
/// silently collapsed to `PT0S`: an invalid duration is a caller error, not a
/// valid zero.
pub(crate) fn xs_duration_secs(secs: f64) -> Result<String, DurationError> {
    if !secs.is_finite() || secs <= 0.0 {
        return Err(DurationError::InvalidInput { secs });
    }
    // `Duration::try_from_secs_f64` rejects NaN/inf/negative/overflow — far
    // larger than any realisable segment duration — as an `Err`, unlike the
    // panicking `from_secs_f64`.
    let d = Duration::try_from_secs_f64(secs).map_err(|_| DurationError::InvalidInput { secs })?;
    Ok(xs_duration(d))
}

/// [`xs_duration_secs`] for a duration that [`crate::config::Config::validate`]
/// should already have rejected — a route's `target_duration_secs` /
/// `part_target_ms` is validated at config time, so an invalid value here is a
/// programming error, not user input. Renders `PT0S` and logs an error rather
/// than silently returning `None` (which would make the DASH manifest route
/// 404, hiding the defect); `debug_assert!` trips it in tests.
pub(crate) fn xs_duration_secs_checked(field: &'static str, secs: f64) -> String {
    match xs_duration_secs(secs) {
        Ok(s) => s,
        Err(e) => {
            // Unreachable for a validated config: the manifest must not
            // silently 404 on it.
            debug_assert!(false, "invalid {field} {secs}: {e}");
            tracing::error!(
                field,
                secs,
                error = %e,
                "DASH xs:duration could not be rendered for a value that should have been \
                 rejected at config validation; emitting PT0S"
            );
            "PT0S".to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::route::RouteConfig;
    use crate::testutil::video_spec;
    use transmux::CodecConfig;

    const CONCURRENT_MANIFESTS: usize = 64;

    /// #2526: concurrent manifest requests at one stamp share one render, and
    /// a publish makes the next request render again.
    #[tokio::test(flavor = "multi_thread")]
    async fn concurrent_manifest_requests_render_once_until_a_publish() {
        let route = Arc::new(RouteHandle::new(
            RouteConfig::default()
                .with_target_duration_secs(1.0)
                .with_part_target_ms(500)
                .with_window_segments(8),
        ));
        let app = crate::testutil::app_with(&route, Arc::new(DashOutput));
        let _first = crate::testutil::ingest_run(&route, 4).await;
        let serving = route
            .serving(crate::route::SPTS_PROGRAM_ID)
            .expect("program published");
        let renders = || {
            serving
                .mpd_renders
                .load(std::sync::atomic::Ordering::Relaxed)
        };

        let requests: Vec<_> = (0..CONCURRENT_MANIFESTS)
            .map(|_| {
                let app = app.clone();
                tokio::spawn(
                    async move { crate::testutil::get_bytes(&app, "/cam/manifest.mpd").await },
                )
            })
            .collect();
        let mut bodies = Vec::new();
        for request in requests {
            let (status, body) = request.await.expect("request task");
            assert_eq!(status, StatusCode::OK);
            bodies.push(body);
        }
        assert!(bodies.iter().all(|body| *body == bodies[0]));
        assert_eq!(renders(), 1, "one render serves every concurrent request");

        let (_, again) = crate::testutil::get_bytes(&app, "/cam/manifest.mpd").await;
        assert_eq!(again, bodies[0]);
        assert_eq!(renders(), 1, "nothing changed, so the cache is served");

        let _second = crate::testutil::ingest_run(&route, 6).await;
        let (status, _) = crate::testutil::get_bytes(&app, "/cam/manifest.mpd").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(renders(), 2, "new segments must invalidate the cache");
    }

    #[tokio::test]
    async fn dash_continues_across_a_source_reconnect() {
        crate::testutil::assert_dash_continues_across_a_reconnect(
            Arc::new(DashOutput),
            "/cam/manifest.mpd",
        )
        .await;
    }

    /// Howard Hinnant's `civil_from_days` — the PRE-jiff algorithm, retained
    /// only to prove the old spelling and `jiff` agree (SP5). Not used by any
    /// production path.
    fn civil_from_days(days: i64) -> (i64, u32, u32) {
        let z = days + 719_468;
        let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
        let doe = (z - era * 146_097) as u64; // [0, 146096]
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
        let y = yoe as i64 + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
        let mp = (5 * doy + 2) / 153; // [0, 11]
        let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
        let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32; // [1, 12]
        (y + i64::from(m <= 2), m, d)
    }
    use transmux::ll_hls::SegmentInfo;

    fn without_publish_time(mpd: &str) -> String {
        let at = mpd.find(" publishTime=\"").expect("publishTime present");
        let close = at + " publishTime=\"".len();
        let end = close + mpd[close..].find('"').expect("closing quote") + 1;
        format!("{}{}", &mpd[..at], &mpd[end..])
    }

    /// Folds the per-init-generation `initialization` template (issue #2441)
    /// back to main's bare `init-$RepresentationID$.mp4`, so the `*-main.golden`
    /// comparisons hold a *historical* pin of main and still differ from it by
    /// only the token each of them is about.
    fn without_init_generation(mpd: &str) -> String {
        let key = "$RepresentationID$";
        let mut out = mpd.to_string();
        while let Some(at) = out.find(key) {
            let rest = &out[at + key.len()..];
            let Some(suffix) = rest.strip_prefix("-INSTANCE-") else {
                break;
            };
            let Some(close) = suffix.find(".mp4") else {
                break;
            };
            out.replace_range(
                at + key.len()..at + key.len() + "-INSTANCE-".len() + close + ".mp4".len(),
                ".mp4",
            );
        }
        out
    }

    fn publish_time_of(mpd: &str) -> String {
        let key = "publishTime=\"";
        let at = mpd.find(key).expect("dynamic MPD carries publishTime") + key.len();
        mpd[at..]
            .split('"')
            .next()
            .expect("closing quote")
            .to_string()
    }

    #[test]
    fn publish_time_present_stable_when_unchanged_and_later_per_new_segment() {
        let route = RouteHandle::new(
            RouteConfig::default()
                .with_target_duration_secs(4.0)
                .with_part_target_ms(500)
                .with_window_segments(2),
        );
        route.publish_new_program(crate::route::SPTS_PROGRAM_ID);
        route.set_track_specs(
            crate::route::SPTS_PROGRAM_ID,
            vec![video_spec(TrackId::new(1))],
        );
        let now = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        route
            .add_segment(crate::route::SPTS_PROGRAM_ID, seg(1, 4.0))
            .expect("add_segment");
        let first = publish_time_of(&render_mpd_at(&route, now).expect("renders"));
        assert_eq!(
            first,
            publish_time_of(&render_mpd_at(&route, now).expect("renders"))
        );
        route
            .add_segment(crate::route::SPTS_PROGRAM_ID, seg(2, 0.5))
            .expect("add_segment");
        let second = publish_time_of(&render_mpd_at(&route, now).expect("renders"));
        let (t1, t2): (jiff::Timestamp, jiff::Timestamp) = (
            first.parse().expect("xs:dateTime"),
            second.parse().expect("xs:dateTime"),
        );
        assert!(t2 > t1, "{second} must be later than {first}");
        assert_eq!(first, "2023-11-14T22:13:24Z");
        assert_eq!(second, "2023-11-14T22:13:24.5Z");
    }

    /// A teletext-shaped opaque track — no derivable RFC 6381 codec string
    /// (issue #776's regression case: a routine DVB multiplex's first
    /// elementary stream, ahead of the video/audio tracks in PMT order).
    fn teletext_spec(track_id: TrackId) -> TrackSpec {
        TrackSpec::new(
            track_id,
            90_000,
            CodecConfig::Data {
                stream_type: 0x06,
                descriptors: Vec::new(),
                carriage: transmux::ir::DataCarriage::Pes,
            },
        )
    }

    fn seg(seq: u32, duration: f64) -> SegmentInfo {
        SegmentInfo {
            bytes: vec![seq as u8; 8],
            duration,
            segment_seq: seq,
            part_count: 1,
        }
    }

    fn gap_seg(seq: u32, duration: f64) -> SegmentInfo {
        SegmentInfo {
            bytes: Vec::new(),
            ..seg(seq, duration)
        }
    }

    #[test]
    fn a_gap_ends_the_period_and_the_next_segments_open_a_new_one() {
        let program = crate::route::SPTS_PROGRAM_ID;
        let route = RouteHandle::new(
            RouteConfig::default()
                .with_target_duration_secs(4.0)
                .with_part_target_ms(500)
                .with_window_segments(8),
        );
        route.publish_new_program(program);
        route.set_track_specs(program, vec![video_spec(TrackId::new(1))]);
        for info in [seg(1, 4.0), seg(2, 4.0), gap_seg(3, 4.0), seg(4, 4.0)] {
            route.add_segment(program, info).expect("add_segment");
        }
        let now = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let mpd = render_mpd_at(&route, now).expect("renders");
        assert_eq!(mpd.matches("<Period ").count(), 2, "{mpd}");
        assert!(
            mpd.contains("<Period id=\"0\" start=\"PT0S\" duration=\"PT8S\">"),
            "{mpd}"
        );
        assert!(mpd.contains("<Period id=\"1\" start=\"PT12S\">"), "{mpd}");
        assert!(mpd.contains("startNumber=\"4\""), "{mpd}");
    }

    #[test]
    fn a_window_of_only_gap_placeholders_renders_no_mpd() {
        let program = crate::route::SPTS_PROGRAM_ID;
        let route = RouteHandle::new(
            RouteConfig::default()
                .with_target_duration_secs(4.0)
                .with_part_target_ms(500)
                .with_window_segments(8),
        );
        route.publish_new_program(program);
        route.set_track_specs(program, vec![video_spec(TrackId::new(1))]);
        route
            .add_segment(program, gap_seg(1, 4.0))
            .expect("add_segment");
        let now = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        assert!(render_mpd_at(&route, now).is_none());
    }

    #[test]
    fn civil_from_days_matches_known_dates() {
        // 1970-01-01 is day 0 by definition.
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        // 2024-01-01: a well-known reference date used elsewhere in this
        // workspace's DASH tests (`transmux/tests/it/dash_mpd.rs`).
        // 19723 days between 1970-01-01 and 2024-01-01 (54 years incl. 13
        // leap days beyond the flat 365*54).
        let days_2024_01_01 = 19_723;
        assert_eq!(civil_from_days(days_2024_01_01), (2024, 1, 1));
    }

    #[test]
    fn format_iso8601_renders_utc_z_suffix() {
        let t = UNIX_EPOCH + Duration::from_secs(0);
        assert_eq!(format_iso8601(t), "1970-01-01T00:00:00Z");
    }

    /// Byte-for-byte golden of the whole rendered MPD with the `now`-dependent
    /// `availabilityStartTime` frozen, so the jiff migration (SP5) is proved
    /// to change no wire byte. `GOLDEN_BLESS=<dir>` writes instead of compares.
    #[test]
    fn render_mpd_matches_frozen_now_golden() {
        let route = RouteHandle::new(
            RouteConfig::default()
                .with_target_duration_secs(4.0)
                .with_part_target_ms(500)
                .with_window_segments(2),
        );
        route.publish_new_program(crate::route::SPTS_PROGRAM_ID);
        route.set_track_specs(
            crate::route::SPTS_PROGRAM_ID,
            vec![video_spec(TrackId::new(1))],
        );
        route
            .add_segment(crate::route::SPTS_PROGRAM_ID, seg(1, 4.0))
            .expect("add_segment");
        route
            .add_segment(crate::route::SPTS_PROGRAM_ID, seg(2, 4.0))
            .expect("add_segment");
        let now = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let instance = route
            .serving(crate::route::SPTS_PROGRAM_ID)
            .unwrap()
            .ll_hls()
            .instance()
            .to_string();
        let actual = render_mpd_at(&route, now)
            .expect("renders")
            .replace(&instance, "INSTANCE");
        let file = "golden/dash/dash-mpd.golden";
        if let Ok(dir) = std::env::var("GOLDEN_BLESS") {
            std::fs::create_dir_all(&dir).expect("create golden dir");
            std::fs::write(std::path::Path::new(&dir).join(file), &actual).expect("write");
            return;
        }
        let path = test_fixtures::path(file);
        let expected = std::fs::read_to_string(&path).expect("read golden");
        assert_eq!(actual, expected, "{file} differs from the golden output");
    }

    /// A window large enough that `@timeShiftBufferDepth` is >= 60 s (4 s x 15
    /// segments), golden-pinned so the >= 60 s spelling is a checked wire byte,
    /// not an untested assumption. The pre-jiff code printed `PT{secs}S`
    /// (`PT60S`); the jiff spelling balances to `PT1M`. This crate keeps the
    /// jiff (balanced) spelling — one rule, shared with `transmux`'s DASH
    /// writer (W1-T) — and the difference is recorded in the CHANGELOG.
    /// `GOLDEN_BLESS=<dir>` writes instead of compares.
    #[test]
    fn render_mpd_window_over_60s_matches_frozen_now_golden() {
        let route = RouteHandle::new(
            RouteConfig::default()
                .with_target_duration_secs(4.0)
                .with_part_target_ms(500)
                .with_window_segments(15),
        );
        route.publish_new_program(crate::route::SPTS_PROGRAM_ID);
        route.set_track_specs(
            crate::route::SPTS_PROGRAM_ID,
            vec![video_spec(TrackId::new(1))],
        );
        for seq in 1..=15u32 {
            route
                .add_segment(crate::route::SPTS_PROGRAM_ID, seg(seq, 4.0))
                .expect("add_segment");
        }
        let now = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let instance = route
            .serving(crate::route::SPTS_PROGRAM_ID)
            .unwrap()
            .ll_hls()
            .instance()
            .to_string();
        let actual = render_mpd_at(&route, now)
            .expect("renders")
            .replace(&instance, "INSTANCE");
        // A 15-segment x 4 s window => timeShiftBufferDepth = 60 s.
        assert!(
            actual.contains("timeShiftBufferDepth=\"PT1M\""),
            "a 60 s window must spell the balanced jiff xs:duration PT1M: {actual}"
        );
        let file = "golden/dash/dash-mpd-window60.golden";
        if let Ok(dir) = std::env::var("GOLDEN_BLESS") {
            std::fs::create_dir_all(&dir).expect("create golden dir");
            std::fs::write(std::path::Path::new(&dir).join(file), &actual).expect("write");
            return;
        }
        let path = test_fixtures::path(file);
        let expected = std::fs::read_to_string(&path).expect("read golden");
        assert_eq!(actual, expected, "{file} differs from the golden output");
    }

    /// The deliberate >= 60 s wire change vs main, pinned against a golden
    /// **captured from `origin/main`** (`dash-mpd-window60-main.golden`): main
    /// prints `PT60S`, this crate prints the balanced `PT1M` — and the two
    /// golden files must differ by EXACTLY that one token, nothing else. This
    /// is the "pin the intended difference" gate: a future change that alters
    /// any other byte, or that silently reverts to `PT60S`, fails here.
    /// A FRACTIONAL target duration (`0.1` s x 3 segments) pinned against
    /// main's captured bytes (`dash-mpd-frac-main.golden`): main printed raw
    /// `f64` (`timeShiftBufferDepth="PT0.30000000000000004S"`), this crate's
    /// jiff form prints `PT0.3S`. The integral-second goldens above cannot see
    /// this, so this is the fractional-target guard — the two must differ by
    /// exactly that one token. `GOLDEN_BLESS=<dir>` writes our side instead of
    /// comparing.
    #[test]
    fn the_fractional_target_differs_from_main_by_exactly_the_rounded_token() {
        let route = RouteHandle::new(
            RouteConfig::default()
                .with_target_duration_secs(0.1)
                .with_part_target_ms(30)
                .with_window_segments(3),
        );
        route.publish_new_program(crate::route::SPTS_PROGRAM_ID);
        route.set_track_specs(
            crate::route::SPTS_PROGRAM_ID,
            vec![video_spec(TrackId::new(1))],
        );
        for seq in 1..=3u32 {
            route
                .add_segment(crate::route::SPTS_PROGRAM_ID, seg(seq, 0.1))
                .expect("add_segment");
        }
        let now = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let instance = route
            .serving(crate::route::SPTS_PROGRAM_ID)
            .unwrap()
            .ll_hls()
            .instance()
            .to_string();
        let ours = render_mpd_at(&route, now)
            .expect("renders")
            .replace(&instance, "INSTANCE");
        assert!(
            ours.contains("timeShiftBufferDepth=\"PT0.3S\""),
            "a 0.3 s window must spell the jiff PT0.3S, not main's raw f64: {ours}"
        );
        let file = "golden/dash/dash-mpd-frac.golden";
        if let Ok(bless) = std::env::var("GOLDEN_BLESS") {
            std::fs::create_dir_all(&bless).expect("create golden dir");
            std::fs::write(std::path::Path::new(&bless).join(file), &ours).expect("write");
            return;
        }
        let ours_file =
            std::fs::read_to_string(test_fixtures::path(file)).expect("read our frac golden");
        let main =
            std::fs::read_to_string(test_fixtures::path("golden/dash/dash-mpd-frac-main.golden"))
                .expect("read main's frac golden");
        assert_eq!(
            ours, ours_file,
            "the live render must match our frac golden"
        );
        assert_ne!(
            ours, main,
            "the jiff spelling must differ from main's raw f64"
        );
        assert!(
            main.contains("timeShiftBufferDepth=\"PT0.30000000000000004S\""),
            "main's frac golden must carry the raw f64 depth: {main}"
        );
        assert_eq!(
            without_init_generation(&without_publish_time(&ours)).replace(
                "timeShiftBufferDepth=\"PT0.3S\"",
                "timeShiftBufferDepth=\"PT0.30000000000000004S\""
            ),
            main,
            "our fractional MPD must differ from main's by ONLY PT0.30000000000000004S -> PT0.3S"
        );
    }

    #[test]
    fn the_over_60s_window_differs_from_main_by_exactly_the_balanced_token() {
        // Render a live 60 s window and compare it to main's captured golden
        // (also check the two committed golden FILES agree) — so a code
        // regression to `PT60S` fails here, not just a golden edit.
        let route = RouteHandle::new(
            RouteConfig::default()
                .with_target_duration_secs(4.0)
                .with_part_target_ms(500)
                .with_window_segments(15),
        );
        route.publish_new_program(crate::route::SPTS_PROGRAM_ID);
        route.set_track_specs(
            crate::route::SPTS_PROGRAM_ID,
            vec![video_spec(TrackId::new(1))],
        );
        for seq in 1..=15u32 {
            route
                .add_segment(crate::route::SPTS_PROGRAM_ID, seg(seq, 4.0))
                .expect("add_segment");
        }
        let now = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let instance = route
            .serving(crate::route::SPTS_PROGRAM_ID)
            .unwrap()
            .ll_hls()
            .instance()
            .to_string();
        let ours = render_mpd_at(&route, now)
            .expect("renders")
            .replace(&instance, "INSTANCE");

        let ours_file =
            std::fs::read_to_string(test_fixtures::path("golden/dash/dash-mpd-window60.golden"))
                .expect("read our window60 golden");
        let main = std::fs::read_to_string(test_fixtures::path(
            "golden/dash/dash-mpd-window60-main.golden",
        ))
        .expect("read main's window60 golden");
        assert_eq!(ours, ours_file, "the live render must match our golden");
        assert_ne!(
            ours, main,
            "the jiff spelling must differ from main's PT60S"
        );
        assert!(
            main.contains("timeShiftBufferDepth=\"PT60S\""),
            "main's golden must carry PT60S: {main}"
        );
        // Exactly the one intended substitution: PT60S -> PT1M.
        assert_eq!(
            without_init_generation(&without_publish_time(&ours)).replace(
                "timeShiftBufferDepth=\"PT1M\"",
                "timeShiftBufferDepth=\"PT60S\""
            ),
            main,
            "our 60 s MPD must differ from main's by ONLY PT60S -> PT1M"
        );
    }

    /// The >= 3600 s wire change vs main, on a real 900-segment window
    /// (4 s x 900 = 3600 s) — main's golden is `PT3600S`, ours is `PT1H`, and
    /// again the two differ by exactly that token.
    #[test]
    fn the_over_an_hour_window_differs_from_main_by_exactly_the_balanced_token() {
        let route = RouteHandle::new(
            RouteConfig::default()
                .with_target_duration_secs(4.0)
                .with_part_target_ms(500)
                .with_window_segments(900),
        );
        route.publish_new_program(crate::route::SPTS_PROGRAM_ID);
        route.set_track_specs(
            crate::route::SPTS_PROGRAM_ID,
            vec![video_spec(TrackId::new(1))],
        );
        for seq in 1..=900u32 {
            route
                .add_segment(crate::route::SPTS_PROGRAM_ID, seg(seq, 4.0))
                .expect("add_segment");
        }
        let now = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let instance = route
            .serving(crate::route::SPTS_PROGRAM_ID)
            .unwrap()
            .ll_hls()
            .instance()
            .to_string();
        let ours = render_mpd_at(&route, now)
            .expect("renders")
            .replace(&instance, "INSTANCE");
        assert!(
            ours.contains("timeShiftBufferDepth=\"PT1H\""),
            "a 3600 s window must spell PT1H: {ours}"
        );
        let main = std::fs::read_to_string(test_fixtures::path(
            "golden/dash/dash-mpd-window3600-main.golden",
        ))
        .expect("read main's window3600 golden");
        assert!(
            main.contains("timeShiftBufferDepth=\"PT3600S\""),
            "main's golden must carry PT3600S: {main}"
        );
        assert_eq!(
            without_init_generation(&without_publish_time(&ours)).replace(
                "timeShiftBufferDepth=\"PT1H\"",
                "timeShiftBufferDepth=\"PT3600S\""
            ),
            main,
            "our 3600 s MPD must differ from main's by ONLY PT3600S -> PT1H"
        );
    }

    /// The >= 3600 s spelling, pinned directly (no route this long is
    /// realisable in a unit test): the balanced jiff form is `PT1H`.
    #[test]
    fn a_duration_at_or_above_an_hour_uses_the_balanced_jiff_spelling() {
        assert_eq!(xs_duration_secs(60.0).unwrap(), "PT1M");
        assert_eq!(xs_duration_secs(90.0).unwrap(), "PT1M30S");
        assert_eq!(xs_duration_secs(3600.0).unwrap(), "PT1H");
        assert_eq!(xs_duration_secs(3660.0).unwrap(), "PT1H1M");
    }

    /// A finite `f64` between `i64::MAX` and `u64::MAX` seconds is a valid
    /// `Duration` but not a `SignedDuration`: it renders the saturated
    /// maximum (the one shared `broadcast_dash::duration::xs_duration` rule), never a
    /// panic and never a wrapped small value.
    #[test]
    fn xs_duration_secs_saturates_beyond_i64_seconds() {
        let max =
            jiff::fmt::temporal::SpanPrinter::new().duration_to_string(&jiff::SignedDuration::MAX);
        assert_eq!(xs_duration_secs(1.0e19).unwrap(), max);
    }

    /// `xs_duration_secs` must REJECT NaN / infinite / negative / overflow with
    /// an error, never panic (the old `Duration::from_secs_f64` panicked and
    /// its `secs <= 0.0` guard let NaN through).
    #[test]
    fn xs_duration_secs_rejects_nan_inf_negative_and_overflow() {
        assert!(
            matches!(
                xs_duration_secs(f64::NAN),
                Err(crate::output::dash::DurationError::InvalidInput { .. })
            ),
            "NaN must be rejected"
        );
        assert!(
            matches!(
                xs_duration_secs(f64::INFINITY),
                Err(crate::output::dash::DurationError::InvalidInput { .. })
            ),
            "+inf must be rejected"
        );
        assert!(
            matches!(
                xs_duration_secs(f64::NEG_INFINITY),
                Err(crate::output::dash::DurationError::InvalidInput { .. })
            ),
            "-inf must be rejected"
        );
        assert!(
            matches!(
                xs_duration_secs(-1.0),
                Err(crate::output::dash::DurationError::InvalidInput { .. })
            ),
            "negative must be rejected"
        );
        assert!(
            matches!(
                xs_duration_secs(0.0),
                Err(crate::output::dash::DurationError::InvalidInput { .. })
            ),
            "zero must be rejected"
        );
        assert!(
            matches!(
                xs_duration_secs(f64::MAX),
                Err(crate::output::dash::DurationError::InvalidInput { .. })
            ),
            "overflow must be rejected, not panic"
        );
        // A valid value still renders.
        assert_eq!(xs_duration_secs(4.0).unwrap(), "PT4S");
    }

    /// `xs_duration_secs_checked` (m6) must never silently 404 the manifest for
    /// an invalid duration: it renders `PT0S` and logs, and `debug_assert!`
    /// trips it in tests. Deleting the `debug_assert!(false, …)` makes this
    /// test pass silently instead of panicking.
    ///
    /// `debug_assertions`-gated: a release build has no `debug_assert!`, so the
    /// panic this asserts on cannot happen there (`cargo test --release` would
    /// fail otherwise). The `PT0S` fallback itself is covered in EVERY profile
    /// by `xs_duration_secs_checked_renders_pt0s_in_a_release_build` below.
    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "invalid minimum_update_period")]
    fn xs_duration_secs_checked_debug_asserts_on_an_invalid_value() {
        let _ = xs_duration_secs_checked("minimum_update_period", f64::NAN);
    }

    /// The release-build behaviour of the checked path: an invalid value is
    /// rendered as `PT0S` (and logged), NOT propagated as a `None` that would
    /// make the manifest 404. Runs in every profile (in a debug build the
    /// `debug_assert!` panics, so this is asserted only without assertions).
    #[test]
    #[cfg(not(debug_assertions))]
    fn xs_duration_secs_checked_renders_pt0s_in_a_release_build() {
        assert_eq!(
            xs_duration_secs_checked("minimum_update_period", f64::NAN),
            "PT0S"
        );
    }

    /// A value `Config::validate` accepts renders a REAL `xs:duration` — not the
    /// `PT0S` fallback — so the checked path is only ever taken for a value that
    /// should already have been rejected.
    #[test]
    fn xs_duration_secs_checked_renders_a_valid_value() {
        assert_eq!(
            xs_duration_secs_checked("minimum_update_period", 4.0),
            "PT4S"
        );
        assert_eq!(
            xs_duration_secs_checked("time_shift_buffer_depth", 60.0),
            "PT1M"
        );
    }

    #[test]
    fn render_mpd_none_without_track_specs() {
        let route = RouteHandle::new(
            RouteConfig::default()
                .with_target_duration_secs(4.0)
                .with_part_target_ms(500)
                .with_window_segments(4),
        );
        assert!(
            render_mpd(&route).is_none(),
            "no track specs recorded yet -> nothing to describe"
        );
    }

    #[test]
    fn render_mpd_valid_before_any_segment_closes() {
        // Track specs known, but the window is still empty (no segment has
        // closed yet) -- must still render a syntactically valid MPD (a
        // degenerate SegmentTemplate@duration=0), not None/panic.
        let route = RouteHandle::new(
            RouteConfig::default()
                .with_target_duration_secs(4.0)
                .with_part_target_ms(500)
                .with_window_segments(4),
        );
        route.publish_new_program(crate::route::SPTS_PROGRAM_ID);
        route.set_track_specs(
            crate::route::SPTS_PROGRAM_ID,
            vec![video_spec(TrackId::new(7))],
        );
        let mpd = render_mpd(&route).expect("must render even with an empty window");
        assert!(mpd.contains("<MPD"));
        assert!(mpd.contains("type=\"dynamic\""));
    }

    #[test]
    fn render_mpd_forces_representation_id_to_default_track() {
        // The source's own track_id (7) must NOT leak into the Representation
        // @id -- it must be forced to DEFAULT_TRACK_ID (1) so
        // $RepresentationID$ substitution matches the shared resource
        // route's init-1.mp4/seg-1-<N>.m4s filenames.
        let route = RouteHandle::new(
            RouteConfig::default()
                .with_target_duration_secs(4.0)
                .with_part_target_ms(500)
                .with_window_segments(4),
        );
        route.publish_new_program(crate::route::SPTS_PROGRAM_ID);
        route.set_track_specs(
            crate::route::SPTS_PROGRAM_ID,
            vec![video_spec(TrackId::new(7))],
        );
        route
            .add_segment(crate::route::SPTS_PROGRAM_ID, seg(1, 4.0))
            .expect("add_segment");
        let mpd = render_mpd(&route).unwrap();
        assert!(
            mpd.contains(&format!("id=\"{DEFAULT_TRACK_ID}\"")),
            "Representation @id must be the DEFAULT_TRACK_ID, not the source's own \
             track_id (7): {mpd}"
        );
        assert!(
            !mpd.contains("id=\"7\""),
            "source track_id must not leak into the MPD: {mpd}"
        );
    }

    #[test]
    fn render_mpd_number_addressing_and_start_number_is_fixed_at_first_segment() {
        let route = RouteHandle::new(
            RouteConfig::default()
                .with_target_duration_secs(4.0)
                .with_part_target_ms(500)
                .with_window_segments(2),
        );
        route.publish_new_program(crate::route::SPTS_PROGRAM_ID);
        route.set_track_specs(
            crate::route::SPTS_PROGRAM_ID,
            vec![video_spec(TrackId::new(1))],
        );
        route
            .add_segment(crate::route::SPTS_PROGRAM_ID, seg(1, 4.0))
            .expect("add_segment");
        route
            .add_segment(crate::route::SPTS_PROGRAM_ID, seg(2, 4.0))
            .expect("add_segment");
        route
            .add_segment(crate::route::SPTS_PROGRAM_ID, seg(3, 4.0))
            .expect("add_segment"); // evicts seq 1 (window_segments == 2)

        let mpd = render_mpd(&route).unwrap();
        assert!(
            mpd.contains("startNumber=\"1\""),
            "startNumber must stay at the stream's FIRST segment (1) even though seq 1 \
             was evicted from the window: with availabilityStartTime fixed, a startNumber \
             that slid to the window's oldest (2) made players compute a live edge of \
             segments that do not exist (issue #1058): {mpd}"
        );
        assert!(
            !mpd.contains("startNumber=\"2\""),
            "startNumber must not slide with the window: {mpd}"
        );
        assert!(
            mpd.contains("$Number$"),
            "media template must use literal $Number$ substitution: {mpd}"
        );
        assert!(
            !mpd.contains("$Time$"),
            "must not use $Time$ addressing -- store filenames are seq-numbered, \
             not time-addressed: {mpd}"
        );
        assert!(mpd.contains("seg-$RepresentationID$-$Number$.m4s"));
        assert!(mpd.contains("init-$RepresentationID$-"));
    }

    /// #1187: `availabilityStartTime` is the program's presentation origin
    /// (ISO/IEC 23009-1 §5.3.1.2: the wall-clock anchor of the Media
    /// Presentation, from which segment availability is measured), NOT the
    /// route's construction time. The route here is constructed, then the
    /// program starts over a second later: the AST must follow the program.
    /// Reverting `render_mpd` to `route.created_at()` makes this fail.
    #[test]
    fn availability_start_time_is_presentation_origin_not_route_creation() {
        let route = RouteHandle::new(
            RouteConfig::default()
                .with_target_duration_secs(2.0)
                .with_part_target_ms(500)
                .with_window_segments(4),
        );
        let created = route.created_at();
        std::thread::sleep(Duration::from_millis(1_100));
        route.publish_new_program(crate::route::SPTS_PROGRAM_ID);
        route.set_track_specs(
            crate::route::SPTS_PROGRAM_ID,
            vec![video_spec(TrackId::new(1))],
        );
        route
            .add_segment(crate::route::SPTS_PROGRAM_ID, seg(1, 2.0))
            .expect("add_segment");
        let origin = route
            .presentation_origin(crate::route::SPTS_PROGRAM_ID)
            .expect("program registered");
        assert!(origin >= created + Duration::from_millis(1_000));
        let mpd = render_mpd(&route).unwrap();
        let want = format!("availabilityStartTime=\"{}\"", format_iso8601(origin));
        let stale = format!("availabilityStartTime=\"{}\"", format_iso8601(created));
        assert!(mpd.contains(&want), "want {want}: {mpd}");
        assert_ne!(want, stale, "origin must differ from creation time");
        assert!(!mpd.contains(&stale), "route creation time leaked: {mpd}");
        // No program, no origin, no MPD (rather than a made-up AST).
        assert!(
            route
                .presentation_origin(media_plane::ingress::ProgramId(99))
                .is_none()
        );
    }

    #[test]
    fn render_mpd_carries_live_attributes() {
        let route = RouteHandle::new(
            RouteConfig::default()
                .with_target_duration_secs(2.0)
                .with_part_target_ms(500)
                .with_window_segments(4),
        );
        route.publish_new_program(crate::route::SPTS_PROGRAM_ID);
        route.set_track_specs(
            crate::route::SPTS_PROGRAM_ID,
            vec![video_spec(TrackId::new(1))],
        );
        route
            .add_segment(crate::route::SPTS_PROGRAM_ID, seg(1, 2.0))
            .expect("add_segment");
        let mpd = render_mpd(&route).unwrap();
        assert!(mpd.contains("availabilityStartTime="), "{mpd}");
        assert!(mpd.contains("minimumUpdatePeriod=\"PT2S\""), "{mpd}");
        assert!(mpd.contains("timeShiftBufferDepth=\"PT2S\""), "{mpd}");
    }

    // --- issue #776: a leading opaque track must not 503 the whole route ---

    #[test]
    fn select_representable_track_skips_leading_opaque_track() {
        let specs = vec![teletext_spec(TrackId::new(1)), video_spec(TrackId::new(2))];
        let selected =
            select_representable_track(&specs).expect("the video track must be selected");
        assert_eq!(selected.track_id, TrackId::new(2));
    }

    /// MUTATION VERIFIED: reverting `render_mpd` to `specs.remove(0)`
    /// unconditionally (the pre-#776 behaviour) makes this test fail: with
    /// teletext first in the recorded track list, `DashPackager::package`
    /// rejects the opaque `CodecConfig::Data` track and `render_mpd` returns
    /// `None` instead of `Some`, so `manifest` would 503 despite a
    /// perfectly representable video track sitting right behind it.
    /// Recompiled and re-run to confirm the failure, then reverted.
    #[test]
    fn render_mpd_skips_leading_opaque_track_instead_of_503ing() {
        let route = RouteHandle::new(
            RouteConfig::default()
                .with_target_duration_secs(4.0)
                .with_part_target_ms(500)
                .with_window_segments(4),
        );
        route.publish_new_program(crate::route::SPTS_PROGRAM_ID);
        route.set_track_specs(
            crate::route::SPTS_PROGRAM_ID,
            vec![teletext_spec(TrackId::new(1)), video_spec(TrackId::new(2))],
        );
        route
            .add_segment(crate::route::SPTS_PROGRAM_ID, seg(1, 4.0))
            .expect("add_segment");
        let mpd = render_mpd(&route)
            .expect("a representable track behind an opaque one must still render");
        assert!(
            mpd.contains(&format!("id=\"{DEFAULT_TRACK_ID}\"")),
            "the selected (video) track's @id must still be forced to DEFAULT_TRACK_ID: {mpd}"
        );
    }

    #[test]
    fn render_mpd_none_when_every_track_is_opaque() {
        let route = RouteHandle::new(
            RouteConfig::default()
                .with_target_duration_secs(4.0)
                .with_part_target_ms(500)
                .with_window_segments(4),
        );
        route.publish_new_program(crate::route::SPTS_PROGRAM_ID);
        route.set_track_specs(
            crate::route::SPTS_PROGRAM_ID,
            vec![
                teletext_spec(TrackId::new(1)),
                teletext_spec(TrackId::new(2)),
            ],
        );
        assert!(
            render_mpd(&route).is_none(),
            "a track set with no representable track is still a genuine 503"
        );
    }

    /// Publishes its own program first (`publish_new_program`) so this
    /// isolates the "no representable track yet" 503 (issue #776) from the
    /// registry-level "not yet announced" 503 (issue #805 task 4) — both
    /// currently render the same status here, but only publishing first
    /// proves this test is actually exercising `render_mpd`'s `None` path,
    /// not short-circuiting on `http::resolve_route_program` before ever
    /// reaching it.
    #[tokio::test]
    async fn manifest_handler_503_before_track_specs_known() {
        let route = Arc::new(RouteHandle::new(
            RouteConfig::default()
                .with_target_duration_secs(4.0)
                .with_part_target_ms(500)
                .with_window_segments(4),
        ));
        route.publish_new_program(crate::route::SPTS_PROGRAM_ID);
        let resp = manifest(State(route)).await;
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn manifest_handler_200_with_dash_content_type() {
        let route = Arc::new(RouteHandle::new(
            RouteConfig::default()
                .with_target_duration_secs(4.0)
                .with_part_target_ms(500)
                .with_window_segments(4),
        ));
        route.publish_new_program(crate::route::SPTS_PROGRAM_ID);
        route.set_track_specs(
            crate::route::SPTS_PROGRAM_ID,
            vec![video_spec(TrackId::new(1))],
        );
        route
            .add_segment(crate::route::SPTS_PROGRAM_ID, seg(1, 4.0))
            .expect("add_segment");
        let resp = manifest(State(route)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get(header::CONTENT_TYPE).unwrap(),
            DASH_MANIFEST_CONTENT_TYPE
        );
    }

    /// MUTATION VERIFIED (issue #805 task 4): a route with no program
    /// announced yet must answer `503`, not `404` — see
    /// `output::hls`'s identical test for the mutation this guards
    /// (`http::resolve_route_program`'s `NotYetAnnounced` arm).
    #[tokio::test]
    async fn manifest_not_yet_announced_is_503_not_404() {
        let route = Arc::new(RouteHandle::new(
            RouteConfig::default()
                .with_target_duration_secs(4.0)
                .with_part_target_ms(500)
                .with_window_segments(4),
        ));
        let resp = manifest(State(route)).await;
        assert_eq!(
            resp.status(),
            StatusCode::SERVICE_UNAVAILABLE,
            "a route with no program announced yet must be 503 (not ready), not 404 (gone)"
        );
    }

    #[test]
    fn render_mpd_declares_scte35_inband_event_stream() {
        // issue #969: the MPD must advertise SCTE-35 as an inband event
        // stream so DASH clients know segments may carry `emsg` boxes with
        // this scheme (ANSI/SCTE 214-3 §8.3.3).
        let route = RouteHandle::new(
            RouteConfig::default()
                .with_target_duration_secs(4.0)
                .with_part_target_ms(500)
                .with_window_segments(4),
        );
        route.publish_new_program(crate::route::SPTS_PROGRAM_ID);
        route.set_track_specs(
            crate::route::SPTS_PROGRAM_ID,
            vec![video_spec(TrackId::new(1))],
        );
        route
            .add_segment(crate::route::SPTS_PROGRAM_ID, seg(1, 4.0))
            .expect("add_segment");
        let mpd = render_mpd(&route).unwrap();
        assert!(
            mpd.contains("<InbandEventStream"),
            "MPD must declare an InbandEventStream: {mpd}"
        );
        assert!(
            mpd.contains("urn:scte:scte35:2013:bin"),
            "InbandEventStream must use the SCTE-35 scheme URI: {mpd}"
        );
    }

    /// The facts of one rendered `Period` the issue #2611 rules are checked against.
    #[derive(Debug, Clone, PartialEq)]
    struct PeriodFacts {
        id: String,
        start: Duration,
        duration: Option<Duration>,
        start_number: u64,
        presentation_time_offset: Option<u64>,
        timescale: u64,
        template_duration: u64,
    }

    fn period_facts(mpd: &str) -> Vec<PeriodFacts> {
        let parsed = broadcast_dash::Mpd::parse(mpd.as_bytes()).expect("the MPD parses");
        parsed
            .periods
            .iter()
            .map(|period| {
                let template = period.adaptation_sets[0].representations[0]
                    .segment
                    .template
                    .as_ref()
                    .expect("a SegmentTemplate");
                PeriodFacts {
                    id: period.id.clone().expect("Period@id"),
                    start: period.start.expect("Period@start"),
                    duration: period.duration,
                    start_number: template.multi.start_number.expect("startNumber"),
                    presentation_time_offset: template.multi.base.presentation_time_offset,
                    timescale: template.multi.base.timescale.expect("timescale"),
                    template_duration: template.multi.duration.unwrap_or_default(),
                }
            })
            .collect()
    }

    fn validate_against_ed6_schema(mpd: &str, name: &str) {
        if let Some(schema) = test_fixtures::process::xsd::dash_mpd_schema("multimux dash") {
            test_fixtures::process::xsd::validate(&schema, name, mpd)
                .unwrap_or_else(|e| panic!("{name} fails the ISO ed6 MPD schema: {e}\n{mpd}"));
        }
    }

    fn reload_route() -> RouteHandle {
        RouteHandle::new(
            RouteConfig::default()
                .with_target_duration_secs(4.0)
                .with_part_target_ms(500)
                .with_window_segments(3),
        )
    }

    /// A route that published `init` and segments `1..=3`, and its replacement continuing it
    /// ([`RouteHandle::continue_from`]) with its program published and its init set to
    /// `new_init`, before its first segment.
    fn replaced(init: Vec<u8>, new_init: Vec<u8>) -> (RouteHandle, RouteHandle) {
        let program = crate::route::SPTS_PROGRAM_ID;
        let old = reload_route();
        old.publish_new_program(program);
        old.set_init(program, init);
        old.set_track_specs(program, vec![video_spec(TrackId::new(1))]);
        for seq in 1..=3 {
            old.add_segment(program, seg(seq, 4.0))
                .expect("add_segment");
        }
        let new = reload_route();
        new.continue_from(&old);
        new.publish_new_program(program);
        new.set_init(program, new_init);
        new.set_track_specs(program, vec![video_spec(TrackId::new(1))]);
        (old, new)
    }

    const RELOAD_NOW_SECS: u64 = 1_700_000_000;

    /// Issue #2611, ISO/IEC 23009-1 §5.3.2.1/§5.3.9.5: a replacement whose init differs opens
    /// a new Period at its first segment. The Period's `@start` is that segment's MPD start on
    /// the presentation's numbering, so the replaced Period (open-ended in the old MPD) ends
    /// exactly there; `startNumber` is the segment's own number and `@presentationTimeOffset`
    /// its media start, so it presents at the Period start.
    #[test]
    fn a_replacement_with_a_different_init_opens_a_new_period_at_its_first_segment() {
        let program = crate::route::SPTS_PROGRAM_ID;
        let now = UNIX_EPOCH + Duration::from_secs(RELOAD_NOW_SECS);
        let (old, new) = replaced(vec![0xA1; 8], vec![0xB2; 8]);
        let before = period_facts(&render_mpd_at(&old, now).expect("old MPD"));
        assert_eq!(before.len(), 1);
        assert_eq!(before[0].id, "0");
        assert_eq!(before[0].start, Duration::ZERO);

        new.add_segment(program, seg(1, 4.0)).expect("add_segment");
        let mpd = render_mpd_at(&new, now).expect("new MPD");
        let after = period_facts(&mpd);
        assert_eq!(after.len(), 2, "the replaced Period stays listed: {mpd}");
        let (replaced_period, period) = (&after[0], &after[1]);
        assert_eq!(
            PeriodFacts {
                duration: None,
                ..replaced_period.clone()
            },
            before[0],
            "the replaced Period keeps its identity: {mpd}"
        );
        assert_eq!(
            replaced_period.duration,
            Some(Duration::from_secs(12)),
            "the replaced Period ends with the last segment it closed (1..=3): {mpd}"
        );
        assert!(
            replaced_period.start + Duration::from_secs(12) <= period.start,
            "and before the new Period starts: {mpd}"
        );
        assert_ne!(period.id, before[0].id, "a new Period has a new id: {mpd}");
        assert_eq!(period.id, "1", "{mpd}");
        assert_eq!(
            period.start_number,
            new.window_segments(program)[0].public_seq,
            "startNumber is the first new segment's number: {mpd}"
        );
        assert!(
            period.start_number > 3,
            "the numbering continues past the replaced route's: {mpd}"
        );
        let ticks_per_segment = before[0].template_duration;
        let elapsed_ticks = (period.start_number - before[0].start_number) * ticks_per_segment;
        assert_eq!(
            period.start,
            Duration::from_secs(elapsed_ticks / before[0].timescale),
            "Period@start is the first new segment's MPD start on the replaced numbering: {mpd}"
        );
        assert_eq!(
            period.presentation_time_offset,
            Some(elapsed_ticks),
            "@presentationTimeOffset is the first new segment's media start: {mpd}"
        );
        validate_against_ed6_schema(&mpd, "reload-new-period");
    }

    /// The MPD a replacement serves before its first segment already names the Period that
    /// segment will open, so no update ever shows the new init's Representation under the
    /// replaced Period's id (§5.4.1).
    #[test]
    fn before_its_first_segment_a_changed_replacement_already_lists_the_new_period() {
        let program = crate::route::SPTS_PROGRAM_ID;
        let now = UNIX_EPOCH + Duration::from_secs(RELOAD_NOW_SECS);
        let (_old, new) = replaced(vec![0xA1; 8], vec![0xB2; 8]);
        let empty = period_facts(&render_mpd_at(&new, now).expect("MPD before a segment"));
        new.add_segment(program, seg(1, 4.0)).expect("add_segment");
        let first = period_facts(&render_mpd_at(&new, now).expect("MPD after a segment"));
        assert_eq!(empty, first);
    }

    /// §5.4.1: `Period@id`, `@start`, `startNumber` and `@presentationTimeOffset` never change
    /// in an MPD update, even once the window has slid past the Period's first segment.
    #[test]
    fn a_reload_period_keeps_its_identity_as_the_window_slides() {
        let program = crate::route::SPTS_PROGRAM_ID;
        let now = UNIX_EPOCH + Duration::from_secs(RELOAD_NOW_SECS);
        let (_old, new) = replaced(vec![0xA1; 8], vec![0xB2; 8]);
        new.add_segment(program, seg(1, 4.0)).expect("add_segment");
        let opened = period_facts(&render_mpd_at(&new, now).expect("MPD"));
        assert_eq!(opened.len(), 2);
        let serving = new.serving(program).expect("published");
        for seq in 2..=6 {
            new.add_segment(program, seg(seq, 4.0))
                .expect("add_segment");
            let mpd = render_mpd_at(&new, now).expect("MPD");
            let facts = period_facts(&mpd);
            assert_eq!(facts.last(), opened.last(), "after segment {seq}: {mpd}");
            let replaced_listed = facts.len() == 2;
            if replaced_listed {
                assert_eq!(facts[0], opened[0], "after segment {seq}: {mpd}");
            }
            assert_eq!(
                serving.retained_servings().is_empty(),
                !replaced_listed,
                "the replaced program is retained exactly while its Period is listed: {mpd}"
            );
            validate_against_ed6_schema(&mpd, "reload-slide");
        }
        assert_eq!(
            period_facts(&render_mpd_at(&new, now).expect("MPD")).len(),
            1,
            "the replaced Period ages out with its last segment"
        );
    }

    /// The in-stream Periods keep their identity too: once the window has slid past the first
    /// Period entirely, the second is still `id="1"` starting at its own first segment, not
    /// renumbered to the window's front.
    #[test]
    fn an_init_change_period_keeps_its_identity_once_the_earlier_period_slid_out() {
        let program = crate::route::SPTS_PROGRAM_ID;
        let now = UNIX_EPOCH + Duration::from_secs(RELOAD_NOW_SECS);
        let route = reload_route();
        route.publish_new_program(program);
        route.set_init(program, vec![0xA1; 8]);
        route.set_track_specs(program, vec![video_spec(TrackId::new(1))]);
        for seq in 1..=2 {
            route
                .add_segment(program, seg(seq, 4.0))
                .expect("add_segment");
        }
        route.set_init(program, vec![0xB2; 8]);
        route
            .add_segment(program, seg(3, 4.0))
            .expect("add_segment");
        let both = period_facts(&render_mpd_at(&route, now).expect("MPD"));
        assert_eq!(both.len(), 2);
        assert_eq!(
            both[0].duration.map(|d| both[0].start + d),
            Some(both[1].start),
            "the earlier Period ends where the next one starts"
        );
        for seq in 4..=6 {
            route
                .add_segment(program, seg(seq, 4.0))
                .expect("add_segment");
        }
        let mpd = render_mpd_at(&route, now).expect("MPD");
        let later = period_facts(&mpd);
        assert_eq!(later.len(), 1, "{mpd}");
        assert_eq!(later[0], both[1], "{mpd}");
        validate_against_ed6_schema(&mpd, "slid-period");
    }

    /// Issue #2611's other half: a byte-identical init keeps the replaced Period's identity and
    /// init generation. Issue #2625: the replacement's numbering skips the number the replaced
    /// origin's open segment was shown with, which nothing serves, so the replaced Period is
    /// early terminated before it (§5.3.2.1) and the replacement continues in the next Period,
    /// period-connected to it with an equivalent `AssetIdentifier` (§5.3.2.4, §5.8.4.10).
    #[test]
    fn a_replacement_with_an_identical_init_stays_in_the_same_period() {
        let program = crate::route::SPTS_PROGRAM_ID;
        let now = UNIX_EPOCH + Duration::from_secs(RELOAD_NOW_SECS);
        let (old, new) = replaced(vec![0xA1; 8], vec![0xA1; 8]);
        let before = period_facts(&render_mpd_at(&old, now).expect("old MPD"));
        new.add_segment(program, seg(1, 4.0)).expect("add_segment");
        let mpd = render_mpd_at(&new, now).expect("new MPD");
        let after = period_facts(&mpd);
        assert_eq!(after.len(), 2, "{mpd}");
        assert_eq!(
            PeriodFacts {
                duration: None,
                ..after[0].clone()
            },
            before[0],
            "{mpd}"
        );
        let skipped = new.window_segments(program)[0].public_seq - 1;
        assert_eq!(after[1].start_number, skipped + 1, "{mpd}");
        let at = |number: u64| Duration::from_secs(4 * (number - before[0].start_number));
        assert_eq!(after[0].duration, Some(at(skipped)), "{mpd}");
        assert_eq!(after[1].start, at(skipped + 1), "{mpd}");
        let parsed = broadcast_dash::Mpd::parse(mpd.as_bytes()).expect("parses");
        let connectivity = &parsed.periods[1].adaptation_sets[0].supplemental_properties;
        assert_eq!(connectivity.len(), 1, "{mpd}");
        assert_eq!(
            connectivity[0].scheme_id_uri,
            broadcast_dash::descriptors::SCHEME_PERIOD_CONNECTIVITY
        );
        assert_eq!(connectivity[0].value.as_deref(), Some("0"), "{mpd}");
        assert!(parsed.periods[0].asset_identifier.is_some(), "{mpd}");
        assert_eq!(
            parsed.periods[0].asset_identifier,
            parsed.periods[1].asset_identifier
        );
        assert_eq!(
            new.window_segments(program)[0].init_generation,
            media_plane::ids::Generation::new(1),
            "no new init generation"
        );
        validate_against_ed6_schema(&mpd, "reload-same-period");
    }

    /// §5.3.2.4: continuity and connectivity both need functionally equivalent Initialization
    /// Segments, which an init change rules out; and with no asset identity there is no
    /// `AssetIdentifier` (DASH-IF IOP 5.5.1). None of the three is written.
    #[test]
    fn a_period_opened_by_an_init_change_signals_no_continuity_connectivity_or_asset() {
        let program = crate::route::SPTS_PROGRAM_ID;
        let now = UNIX_EPOCH + Duration::from_secs(RELOAD_NOW_SECS);
        let (_old, new) = replaced(vec![0xA1; 8], vec![0xB2; 8]);
        new.add_segment(program, seg(1, 4.0)).expect("add_segment");
        let parsed = broadcast_dash::Mpd::parse(render_mpd_at(&new, now).expect("MPD").as_bytes())
            .expect("parses");
        for period in &parsed.periods {
            assert!(period.asset_identifier.is_none());
            assert!(period.supplemental_properties.is_empty());
            for adaptation_set in &period.adaptation_sets {
                assert!(adaptation_set.supplemental_properties.is_empty());
            }
        }
    }

    /// `MPD@publishTime` keeps increasing across the hand-off and `availabilityStartTime` /
    /// `minimumUpdatePeriod` stay as they were (ISO/IEC 23009-1 §5.3.1.2, §5.4.1).
    #[test]
    fn a_reload_keeps_the_mpd_level_timing() {
        let program = crate::route::SPTS_PROGRAM_ID;
        let now = UNIX_EPOCH + Duration::from_secs(RELOAD_NOW_SECS);
        let (old, new) = replaced(vec![0xA1; 8], vec![0xB2; 8]);
        let before = broadcast_dash::Mpd::parse(render_mpd_at(&old, now).expect("old").as_bytes())
            .expect("parses");
        new.add_segment(program, seg(1, 4.0)).expect("add_segment");
        let after = broadcast_dash::Mpd::parse(render_mpd_at(&new, now).expect("new").as_bytes())
            .expect("parses");
        assert_eq!(
            after.availability_start_time_us,
            before.availability_start_time_us
        );
        assert_eq!(after.minimum_update_period, before.minimum_update_period);
        assert!(after.publish_time_us > before.publish_time_us);
    }
    #[test]
    fn a_second_continue_from_the_same_route_keeps_the_retained_programs() {
        let program = crate::route::SPTS_PROGRAM_ID;
        let (_a, b) = replaced(vec![0xA1; 8], vec![0xB2; 8]);
        b.add_segment(program, seg(1, 4.0)).expect("add_segment");
        let make = || {
            let c = reload_route();
            c.continue_from(&b);
            c.publish_new_program(program);
            c.set_init(program, vec![0xB2; 8]);
            c.set_track_specs(program, vec![video_spec(TrackId::new(1))]);
            c
        };
        let (c1, c2) = (make(), make());
        let first = c1
            .serving(program)
            .expect("first serving")
            .retained_servings();
        let second = c2
            .serving(program)
            .expect("second serving")
            .retained_servings();
        assert!(first.len() >= 2, "the earlier program must be retained");
        assert_eq!(first.len(), second.len(), "retained programs");
        for (a, b) in first.iter().zip(second.iter()) {
            assert!(std::sync::Arc::ptr_eq(a, b), "same retained serving");
        }
    }
}
