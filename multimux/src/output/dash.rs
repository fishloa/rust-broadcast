//! `DashOutput`: the DASH [`crate::output::Output`] implementation (issue
//! #663 P4) — renders `manifest.mpd` from the shared [`crate::route::RouteHandle`]'s
//! `Trunk`-drained window via `transmux::dash::DashPackager`, resolved
//! through the same one adapter every other output uses
//! ([`crate::http::resolve_blocking`]/[`crate::http::into_response`]).
//! Init/segment byte ranges are the origin's *shared* resource route
//! (`crate::origin::resource`) — the exact same bytes [`crate::output::llhls`]
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
//! [`DashPackager::start_number`] to the window's oldest `segment_seq` and
//! relies on `$Number$` counting up from there) — that *is* `segment_seq`,
//! so [`transmux::Addressing::Number`] is the only mode that produces URIs
//! the shared resource route actually resolves.
//!
//! # Single-rendition model
//!
//! Like [`crate::output::llhls`] (see [`DEFAULT_TRACK_ID`]'s docs), exactly
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
//! list. [`select_representable_track`] fixes this: it selects the first
//! track (preferring a video-shaped codec, then any other, e.g. audio) that
//! actually produces a derivable codec string — proven by trial-packaging it
//! through the real [`DashPackager`] rather than re-deriving "is this codec
//! supported" here (which would drift from `DashPackager::codec_string`'s
//! own, more authoritative, list). Only a track set with genuinely **no**
//! representable track is still a `503`.
//!
//! # Scope: standard DASH only, LL-DASH is a follow-up
//!
//! This ships `type="dynamic"` DASH (`minimumUpdatePeriod`/
//! `timeShiftBufferDepth`/`availabilityStartTime`), **not** LL-DASH — see
//! [`crate::output::ll_dash`] for that (issue #663 P4.2 / #721).

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::Router;
use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use broadcast_common::Package;
use ll_hls_runtime::server::DEFAULT_TRACK_ID;
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
    let origin = DashOrigin { route };
    let resp = http::resolve_blocking(
        origin.route.trunk(),
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

/// The DASH manifest [`ServedEgress`]: a stateless render of `route`'s
/// current window/track specs — never answers
/// [`EgressResponse::Await`] (there is nothing to wait for; an MPD can
/// always be rendered from whatever window state currently exists, or
/// answers `NotFound` once and for all if no track is representable), so
/// [`http::resolve_blocking`] resolves it on the very first check without
/// ever touching `Trunk::listen`. Still routed through the same one adapter
/// as every other output, rather than a bespoke handler, so a manifest route
/// can never drift into its own ad hoc HTTP-mapping logic — see
/// `crate::http`'s own module doc.
struct DashOrigin {
    route: Arc<RouteHandle>,
}

impl ServedEgress for DashOrigin {
    type Request = ();
    type Body = String;

    fn resolve(
        &self,
        _request: (),
        _now: broadcast_common::Timestamp,
        _await_policy: AwaitPolicy,
    ) -> EgressResponse<String> {
        match render_mpd(&self.route) {
            Some(body) => EgressResponse::Ready {
                body,
                cache: CachePolicy::NoCache,
            },
            None => EgressResponse::NotFound,
        }
    }
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
        .filter(|s| is_video_like(&s.config))
        .find(|s| track_is_representable(s))
        .or_else(|| specs.iter().find(|s| track_is_representable(s)))
        .cloned()
}

/// `true` for the video codecs this crate currently knows to prefer when
/// describing a single-rendition DASH `Representation`. Deliberately a
/// closed, local list (not a call into `transmux`, whose own equivalent
/// classifier is private) — a future video codec `CodecConfig` gains
/// (`transmux::CodecConfig` is `#[non_exhaustive]`) simply falls into "not
/// preferred" here until this list is updated, which only affects which
/// track is *preferred*, never which tracks are considered representable at
/// all ([`track_is_representable`] is authoritative for that).
fn is_video_like(config: &transmux::CodecConfig) -> bool {
    matches!(
        config,
        transmux::CodecConfig::Avc { .. }
            | transmux::CodecConfig::Hevc { .. }
            | transmux::CodecConfig::Vvc { .. }
            | transmux::CodecConfig::Av1 { .. }
            | transmux::CodecConfig::Vp9 { .. }
            | transmux::CodecConfig::Vp8 { .. }
            | transmux::CodecConfig::Mpeg2Video { .. }
    )
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
        spec.timescale.max(1),
    );
    DashPackager::default().package(&media).is_ok()
}

/// Render the live MPD for `route`'s current window. `None` if no track in
/// [`RouteHandle::track_specs`] is [`select_representable_track`]-selectable
/// (nothing recorded yet, or every recorded track is opaque — issue #776).
fn render_mpd(route: &RouteHandle) -> Option<String> {
    let specs = route.track_specs();
    // `@id` forced to DEFAULT_TRACK_ID (see module docs' "Single-rendition
    // model") regardless of which track this route's own PMT numbered it.
    let mut spec = select_representable_track(&specs)?;
    spec.track_id = DEFAULT_TRACK_ID;
    let timescale = spec.timescale.max(1);

    let window = route.window_segments();
    let start_number = window
        .first()
        .map(|s| u64::from(s.segment_seq))
        .unwrap_or(1);
    let duration_ticks: Vec<u64> = window
        .iter()
        .map(|s| (s.duration_secs * f64::from(timescale)).round() as u64)
        .collect();
    let segments = if duration_ticks.is_empty() {
        Vec::new()
    } else {
        vec![TrackSegments {
            track_id: DEFAULT_TRACK_ID,
            durations: duration_ticks,
        }]
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

    let mut packager = DashPackager {
        dynamic: true,
        addressing: Addressing::Number,
        start_number,
        // `$RepresentationID$` is substituted by the DASH *client*, not
        // here (real DASH template tokens) — left literal so it resolves to
        // "1" (DEFAULT_TRACK_ID), matching the shared resource route's
        // `init-1.mp4`/`seg-1-<N>.m4s` filenames exactly.
        init_template: "init-$RepresentationID$.mp4".to_string(),
        media_template: "seg-$RepresentationID$-$Number$.m4s".to_string(),
        availability_start_time: Some(format_iso8601(route.created_at())),
        minimum_update_period: Some(format!("PT{target_duration_secs}S")),
        time_shift_buffer_depth: Some(format!("PT{time_shift_buffer_depth_secs}S")),
        segments,
        ..DashPackager::default()
    };

    packager.package(&media).ok()
}

/// Format `t` as an ISO-8601 UTC timestamp (`YYYY-MM-DDTHH:MM:SSZ`) — the
/// `MPD@availabilityStartTime` wire format (ISO/IEC 23009-1 §5.3.1.2 Table 3).
/// Hand-rolled (this crate has no date/time dependency): converts the Unix
/// timestamp's day count to a proleptic-Gregorian civil date via the
/// well-known "civil_from_days" algorithm (Howard Hinnant,
/// <https://howardhinnant.github.io/date_algorithms.html>, public domain),
/// exact for every representable date.
///
/// `pub(crate)` (not private) since issue #663 P4.2: `crate::output::ll_dash`
/// needs the same `availabilityStartTime` formatting and reuses this rather
/// than duplicating the algorithm.
pub(crate) fn format_iso8601(t: SystemTime) -> String {
    let secs = t
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs();
    let days = (secs / 86_400) as i64;
    let time_of_day = secs % 86_400;
    let (h, m, s) = (
        time_of_day / 3600,
        (time_of_day / 60) % 60,
        time_of_day % 60,
    );
    let (y, mo, d) = civil_from_days(days);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{m:02}:{s:02}Z")
}

/// Howard Hinnant's `civil_from_days`: days since the Unix epoch
/// (1970-01-01) to a proleptic-Gregorian `(year, month, day)`.
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

#[cfg(test)]
mod tests {
    use super::*;
    use transmux::CodecConfig;
    use transmux::ll_hls::SegmentInfo;

    fn video_spec(track_id: u32) -> TrackSpec {
        TrackSpec::new(
            track_id,
            90_000,
            CodecConfig::Vp8 {
                width: 1280,
                height: 720,
            },
        )
    }

    /// A teletext-shaped opaque track — no derivable RFC 6381 codec string
    /// (issue #776's regression case: a routine DVB multiplex's first
    /// elementary stream, ahead of the video/audio tracks in PMT order).
    fn teletext_spec(track_id: u32) -> TrackSpec {
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

    #[test]
    fn civil_from_days_matches_known_dates() {
        // 1970-01-01 is day 0 by definition.
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        // 2024-01-01: a well-known reference date used elsewhere in this
        // workspace's DASH tests (`transmux/tests/dash_mpd.rs`).
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

    #[test]
    fn render_mpd_none_without_track_specs() {
        let route = RouteHandle::new(4.0, 500, 4);
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
        let route = RouteHandle::new(4.0, 500, 4);
        route.set_track_specs(vec![video_spec(7)]);
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
        let route = RouteHandle::new(4.0, 500, 4);
        route.set_track_specs(vec![video_spec(7)]);
        route.add_segment(seg(1, 4.0));
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
    fn render_mpd_number_addressing_and_start_number_track_window() {
        let route = RouteHandle::new(4.0, 500, 2);
        route.set_track_specs(vec![video_spec(1)]);
        route.add_segment(seg(1, 4.0));
        route.add_segment(seg(2, 4.0));
        route.add_segment(seg(3, 4.0)); // evicts seq 1 (window_segments == 2)

        let mpd = render_mpd(&route).unwrap();
        assert!(
            mpd.contains("startNumber=\"2\""),
            "startNumber must track the window's oldest retained segment_seq (2, \
             since seq 1 was evicted): {mpd}"
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
        assert!(mpd.contains("init-$RepresentationID$.mp4"));
    }

    #[test]
    fn render_mpd_carries_live_attributes() {
        let route = RouteHandle::new(2.0, 500, 4);
        route.set_track_specs(vec![video_spec(1)]);
        route.add_segment(seg(1, 2.0));
        let mpd = render_mpd(&route).unwrap();
        assert!(mpd.contains("availabilityStartTime="), "{mpd}");
        assert!(mpd.contains("minimumUpdatePeriod=\"PT2S\""), "{mpd}");
        assert!(mpd.contains("timeShiftBufferDepth=\"PT2S\""), "{mpd}");
    }

    // --- issue #776: a leading opaque track must not 503 the whole route ---

    #[test]
    fn select_representable_track_skips_leading_opaque_track() {
        let specs = vec![teletext_spec(1), video_spec(2)];
        let selected =
            select_representable_track(&specs).expect("the video track must be selected");
        assert_eq!(selected.track_id, 2);
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
        let route = RouteHandle::new(4.0, 500, 4);
        route.set_track_specs(vec![teletext_spec(1), video_spec(2)]);
        route.add_segment(seg(1, 4.0));
        let mpd = render_mpd(&route)
            .expect("a representable track behind an opaque one must still render");
        assert!(
            mpd.contains(&format!("id=\"{DEFAULT_TRACK_ID}\"")),
            "the selected (video) track's @id must still be forced to DEFAULT_TRACK_ID: {mpd}"
        );
    }

    #[test]
    fn render_mpd_none_when_every_track_is_opaque() {
        let route = RouteHandle::new(4.0, 500, 4);
        route.set_track_specs(vec![teletext_spec(1), teletext_spec(2)]);
        assert!(
            render_mpd(&route).is_none(),
            "a track set with no representable track is still a genuine 503"
        );
    }

    #[tokio::test]
    async fn manifest_handler_503_before_track_specs_known() {
        let route = Arc::new(RouteHandle::new(4.0, 500, 4));
        let resp = manifest(State(route)).await;
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn manifest_handler_200_with_dash_content_type() {
        let route = Arc::new(RouteHandle::new(4.0, 500, 4));
        route.set_track_specs(vec![video_spec(1)]);
        route.add_segment(seg(1, 4.0));
        let resp = manifest(State(route)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get(header::CONTENT_TYPE).unwrap(),
            DASH_MANIFEST_CONTENT_TYPE
        );
    }
}
