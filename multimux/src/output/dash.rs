//! `DashOutput`: the DASH [`crate::output::Output`] implementation (issue
//! #663 P4) — renders `manifest.mpd` from the shared [`crate::route::RouteHandle`]'s
//! `Trunk`-drained window via `transmux::dash::DashPackager`, resolved
//! through the same one adapter every other output uses
//! (`crate::http::resolve_blocking`/`crate::http::into_response`).
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
//! list. `select_representable_track` fixes this: it selects the first
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
    let serving = match http::resolve_route_program(&route) {
        Ok(serving) => serving,
        Err(resp) => return *resp,
    };
    let trunk = serving.trunk();
    let origin = DashOrigin { route };
    let resp = http::resolve_blocking(&trunk, &origin, (), BLOCKING_RELOAD_TIMEOUT, || ()).await;
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
    render_mpd_at(route, route.created_at())
}

/// Render the MPD with an explicit `now` for `availabilityStartTime`, so a
/// golden can freeze the wall-clock value (`render_mpd` delegates with the
/// route's own `created_at`). `#[doc(hidden)]` and only reachable from tests.
#[doc(hidden)]
pub fn render_mpd_at(route: &RouteHandle, now: SystemTime) -> Option<String> {
    let specs = route.track_specs(crate::route::SPTS_PROGRAM_ID);
    // `@id` forced to DEFAULT_TRACK_ID (see module docs' "Single-rendition
    // model") regardless of which track this route's own PMT numbered it.
    let mut spec = select_representable_track(&specs)?;
    spec.track_id = DEFAULT_TRACK_ID;
    let timescale = spec.timescale.max(1);

    let window = route.window_segments(crate::route::SPTS_PROGRAM_ID);
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
        availability_start_time: Some(format_iso8601(now)),
        minimum_update_period: Some(xs_duration_secs_checked(
            "minimum_update_period",
            target_duration_secs,
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

    packager.package(&media).ok()
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

fn format_iso8601_secs(secs: i64) -> String {
    jiff::Timestamp::from_second(secs)
        .map(|ts| ts.to_string())
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

/// Format a duration as the DASH `xs:duration` shortest spelling (`PT4S`,
/// `PT2M`, `PT0.5S`) via `jiff`'s `SpanPrinter` (SP5) — the form
/// `@minimumUpdatePeriod` and `@timeShiftBufferDepth` use
/// (ISO/IEC 23009-1 §5.3.9.2).
///
/// **One spelling rule, shared with `transmux`'s DASH writer (W1-T):** both
/// call `jiff`'s `SpanPrinter::duration_to_string`, which balances up to
/// hours, so a value >= 60 s prints `PT2M` (not `PT120S`) and >= 3600 s prints
/// `PT1H`. `jiff` cannot print unbalanced seconds, and hand-rolling that
/// spelling here to match the pre-jiff `PT{secs}S` output would be a second,
/// divergent rule — so the balanced jiff spelling is kept and the >= 60 s wire
/// difference is recorded in the CHANGELOG (the pre-migration code printed
/// `PT{secs}S`, which is why the sub-60 s goldens were byte-identical).
pub(crate) fn xs_duration(d: Duration) -> String {
    match jiff::SignedDuration::try_from(d) {
        Ok(signed) => jiff::fmt::temporal::SpanPrinter::new().duration_to_string(&signed),
        // `u64` seconds > `i64::MAX` seconds (~292 billion years) — no
        // realisable media duration; fall back to the whole-second spelling
        // rather than panic.
        Err(_) => format!("PT{}S", d.as_secs()),
    }
}

/// `#[doc(hidden)]` test seam over [`xs_duration`].
#[doc(hidden)]
pub fn xs_duration_for_test(d: Duration) -> String {
    xs_duration(d)
}

/// Why [`xs_duration_secs`] could not render an `xs:duration`.
#[derive(Debug, thiserror::Error)]
pub(crate) enum DurationError {
    /// `secs` was NaN, infinite, negative, or too large to represent as a
    /// [`Duration`].
    #[error("invalid xs:duration input: {secs}")]
    InvalidInput { secs: f64 },
}

/// A whole/ fractional second count as an `xs:duration` via [`xs_duration`],
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
    use transmux::CodecConfig;

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

    /// Byte-for-byte golden of the whole rendered MPD with the `now`-dependent
    /// `availabilityStartTime` frozen, so the jiff migration (SP5) is proved
    /// to change no wire byte. `GOLDEN_BLESS=<dir>` writes instead of compares.
    #[test]
    fn render_mpd_matches_frozen_now_golden() {
        let route = RouteHandle::new(4.0, 500, 2);
        route.publish_new_program(crate::route::SPTS_PROGRAM_ID);
        route.set_track_specs(crate::route::SPTS_PROGRAM_ID, vec![video_spec(1)]);
        route
            .add_segment(crate::route::SPTS_PROGRAM_ID, seg(1, 4.0))
            .expect("add_segment");
        route
            .add_segment(crate::route::SPTS_PROGRAM_ID, seg(2, 4.0))
            .expect("add_segment");
        let now = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let actual = render_mpd_at(&route, now).expect("renders");
        let file = "dash_mpd.golden";
        if let Ok(dir) = std::env::var("GOLDEN_BLESS") {
            std::fs::create_dir_all(&dir).expect("create golden dir");
            std::fs::write(std::path::Path::new(&dir).join(file), &actual).expect("write");
            return;
        }
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/golden")
            .join(file);
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
        let route = RouteHandle::new(4.0, 500, 15);
        route.publish_new_program(crate::route::SPTS_PROGRAM_ID);
        route.set_track_specs(crate::route::SPTS_PROGRAM_ID, vec![video_spec(1)]);
        for seq in 1..=15u32 {
            route
                .add_segment(crate::route::SPTS_PROGRAM_ID, seg(seq, 4.0))
                .expect("add_segment");
        }
        let now = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let actual = render_mpd_at(&route, now).expect("renders");
        // A 15-segment x 4 s window => timeShiftBufferDepth = 60 s.
        assert!(
            actual.contains("timeShiftBufferDepth=\"PT1M\""),
            "a 60 s window must spell the balanced jiff xs:duration PT1M: {actual}"
        );
        let file = "dash_mpd_window60.golden";
        if let Ok(dir) = std::env::var("GOLDEN_BLESS") {
            std::fs::create_dir_all(&dir).expect("create golden dir");
            std::fs::write(std::path::Path::new(&dir).join(file), &actual).expect("write");
            return;
        }
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/golden")
            .join(file);
        let expected = std::fs::read_to_string(&path).expect("read golden");
        assert_eq!(actual, expected, "{file} differs from the golden output");
    }

    /// The deliberate >= 60 s wire change vs main, pinned against a golden
    /// **captured from `origin/main`** (`dash_mpd_window60_main.golden`): main
    /// prints `PT60S`, this crate prints the balanced `PT1M` — and the two
    /// golden files must differ by EXACTLY that one token, nothing else. This
    /// is the "pin the intended difference" gate: a future change that alters
    /// any other byte, or that silently reverts to `PT60S`, fails here.
    #[test]
    fn the_over_60s_window_differs_from_main_by_exactly_the_balanced_token() {
        // Render a live 60 s window and compare it to main's captured golden
        // (also check the two committed golden FILES agree) — so a code
        // regression to `PT60S` fails here, not just a golden edit.
        let route = RouteHandle::new(4.0, 500, 15);
        route.publish_new_program(crate::route::SPTS_PROGRAM_ID);
        route.set_track_specs(crate::route::SPTS_PROGRAM_ID, vec![video_spec(1)]);
        for seq in 1..=15u32 {
            route
                .add_segment(crate::route::SPTS_PROGRAM_ID, seg(seq, 4.0))
                .expect("add_segment");
        }
        let now = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let ours = render_mpd_at(&route, now).expect("renders");

        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden");
        let ours_file = std::fs::read_to_string(dir.join("dash_mpd_window60.golden"))
            .expect("read our window60 golden");
        let main = std::fs::read_to_string(dir.join("dash_mpd_window60_main.golden"))
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
            ours.replace(
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
        let route = RouteHandle::new(4.0, 500, 900);
        route.publish_new_program(crate::route::SPTS_PROGRAM_ID);
        route.set_track_specs(crate::route::SPTS_PROGRAM_ID, vec![video_spec(1)]);
        for seq in 1..=900u32 {
            route
                .add_segment(crate::route::SPTS_PROGRAM_ID, seg(seq, 4.0))
                .expect("add_segment");
        }
        let now = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let ours = render_mpd_at(&route, now).expect("renders");
        assert!(
            ours.contains("timeShiftBufferDepth=\"PT1H\""),
            "a 3600 s window must spell PT1H: {ours}"
        );
        let main = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/golden/dash_mpd_window3600_main.golden"),
        )
        .expect("read main's window3600 golden");
        assert!(
            main.contains("timeShiftBufferDepth=\"PT3600S\""),
            "main's golden must carry PT3600S: {main}"
        );
        assert_eq!(
            ours.replace(
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

    /// `xs_duration_secs` must REJECT NaN / infinite / negative / overflow with
    /// an error, never panic (the old `Duration::from_secs_f64` panicked and
    /// its `secs <= 0.0` guard let NaN through).
    #[test]
    fn xs_duration_secs_rejects_nan_inf_negative_and_overflow() {
        assert!(xs_duration_secs(f64::NAN).is_err(), "NaN must be rejected");
        assert!(
            xs_duration_secs(f64::INFINITY).is_err(),
            "+inf must be rejected"
        );
        assert!(
            xs_duration_secs(f64::NEG_INFINITY).is_err(),
            "-inf must be rejected"
        );
        assert!(xs_duration_secs(-1.0).is_err(), "negative must be rejected");
        assert!(xs_duration_secs(0.0).is_err(), "zero must be rejected");
        assert!(
            xs_duration_secs(f64::MAX).is_err(),
            "overflow must be rejected, not panic"
        );
        // A valid value still renders.
        assert_eq!(xs_duration_secs(4.0).unwrap(), "PT4S");
    }

    /// `xs_duration_secs_checked` (m6) must never silently 404 the manifest for
    /// an invalid duration: it renders `PT0S` and logs, and `debug_assert!`
    /// trips it in tests. Deleting the `debug_assert!(false, …)` makes this
    /// test pass silently instead of panicking.
    #[test]
    #[should_panic(expected = "invalid minimum_update_period")]
    fn xs_duration_secs_checked_debug_asserts_on_an_invalid_value() {
        let _ = xs_duration_secs_checked("minimum_update_period", f64::NAN);
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
        route.publish_new_program(crate::route::SPTS_PROGRAM_ID);
        route.set_track_specs(crate::route::SPTS_PROGRAM_ID, vec![video_spec(7)]);
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
        route.publish_new_program(crate::route::SPTS_PROGRAM_ID);
        route.set_track_specs(crate::route::SPTS_PROGRAM_ID, vec![video_spec(7)]);
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
    fn render_mpd_number_addressing_and_start_number_track_window() {
        let route = RouteHandle::new(4.0, 500, 2);
        route.publish_new_program(crate::route::SPTS_PROGRAM_ID);
        route.set_track_specs(crate::route::SPTS_PROGRAM_ID, vec![video_spec(1)]);
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
        route.publish_new_program(crate::route::SPTS_PROGRAM_ID);
        route.set_track_specs(crate::route::SPTS_PROGRAM_ID, vec![video_spec(1)]);
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
        route.publish_new_program(crate::route::SPTS_PROGRAM_ID);
        route.set_track_specs(
            crate::route::SPTS_PROGRAM_ID,
            vec![teletext_spec(1), video_spec(2)],
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
        let route = RouteHandle::new(4.0, 500, 4);
        route.publish_new_program(crate::route::SPTS_PROGRAM_ID);
        route.set_track_specs(
            crate::route::SPTS_PROGRAM_ID,
            vec![teletext_spec(1), teletext_spec(2)],
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
        let route = Arc::new(RouteHandle::new(4.0, 500, 4));
        route.publish_new_program(crate::route::SPTS_PROGRAM_ID);
        let resp = manifest(State(route)).await;
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn manifest_handler_200_with_dash_content_type() {
        let route = Arc::new(RouteHandle::new(4.0, 500, 4));
        route.publish_new_program(crate::route::SPTS_PROGRAM_ID);
        route.set_track_specs(crate::route::SPTS_PROGRAM_ID, vec![video_spec(1)]);
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
    /// `output::llhls`'s identical test for the mutation this guards
    /// (`http::resolve_route_program`'s `NotYetAnnounced` arm).
    #[tokio::test]
    async fn manifest_not_yet_announced_is_503_not_404() {
        let route = Arc::new(RouteHandle::new(4.0, 500, 4));
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
        let route = RouteHandle::new(4.0, 500, 4);
        route.publish_new_program(crate::route::SPTS_PROGRAM_ID);
        route.set_track_specs(crate::route::SPTS_PROGRAM_ID, vec![video_spec(1)]);
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
}
