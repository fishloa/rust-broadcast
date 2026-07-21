//! `DashOutput`: the DASH [`crate::output::Output`] implementation (issue
//! #663 P4) — renders `manifest.mpd` from the shared
//! [`crate::store::MediaStore`]'s window via `transmux::dash::DashPackager`.
//! Init/segment byte ranges are the origin's *shared* resource route
//! (`crate::origin::resource`) — the exact same bytes [`crate::output::llhls`]
//! serves; this module only renders the manifest (see `crate::output`
//! module docs for why the byte-serving is shared, not per-output).
//!
//! # Addressing (why `$Number$`, not `$Time$`)
//!
//! [`MediaStore`] names its closed segments `seg-{track}-{seq}.m4s`, where
//! `{seq}` is a plain monotonic sequence number (`SegmentInfo::segment_seq`)
//! — **not** a cumulative-duration timestamp. `SegmentTemplate`'s `$Time$`
//! substitution (ISO/IEC 23009-1 §5.3.9.4.4 / §5.3.9.6) is the segment's
//! *start time*, which would not match those filenames. `$Number$`
//! substitution is a literal, caller-chosen integer per segment (this
//! module sets [`DashPackager::start_number`] to the window's oldest
//! `segment_seq` and relies on `$Number$` counting up from there) — that
//! *is* `segment_seq`, so [`transmux::Addressing::Number`] is the only mode
//! that produces URIs the shared resource route actually resolves.
//!
//! # Single-rendition model
//!
//! Like [`crate::output::llhls`] (see `ll_hls_runtime::server::DEFAULT_TRACK_ID`'s
//! docs), exactly one `Representation` is described, using the *first*
//! [`transmux::TrackSpec`] [`crate::pipeline::run_pipeline`] recorded via
//! [`MediaStore::set_track_specs`] — but with its `track_id` forced to
//! [`DEFAULT_TRACK_ID`] so the DASH client's `$RepresentationID$`
//! substitution produces the same `init-1.mp4`/`seg-1-<N>.m4s` filenames the
//! shared resource route serves, regardless of the source's own track
//! numbering.
//!
//! # Scope: standard DASH only, LL-DASH is a follow-up
//!
//! This ships `type="dynamic"` DASH (`minimumUpdatePeriod`/
//! `timeShiftBufferDepth`/`availabilityStartTime`), **not** LL-DASH. Low-
//! latency DASH (`transmux::LlDashPackager`: `availabilityTimeOffset`,
//! `<ServiceDescription><Latency>`, chunked-transfer parts) needs
//! byte-range-addressable *chunks* within an in-progress segment; the
//! store's `part-{track}-{seq}.{idx}.m4s` files are shaped for LL-HLS's own
//! addressing (a whole extra fMP4 moof/mdat per part, not CMAF chunks sharing
//! one segment's byte range), so wiring `LlDashPackager` needs either a
//! second parallel chunk representation from the segmenter or a store-side
//! reshaping — a larger lift than P4's scope. Tracked as a follow-up (issue
//! #663 P4.2).

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::Router;
use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use broadcast_common::Package;
use ll_hls_runtime::server::DEFAULT_TRACK_ID;
use transmux::{Addressing, DashPackager, Media, Track, TrackSegments};

use crate::origin::resource::cors_preflight;
use crate::output::{Output, OutputKind};
use crate::store::MediaStore;

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
    fn manifest_routes(&self, store: Arc<MediaStore>) -> Router {
        Router::new()
            .route("/manifest.mpd", get(manifest).options(cors_preflight))
            .with_state(store)
    }
}

/// `GET /manifest.mpd` — `503 Service Unavailable` until the route has at
/// least recorded its track specs (`MediaStore::set_track_specs`, done once
/// at pipeline start) — before that, no `codecs`/`Representation` can be
/// described at all, unlike LL-HLS's playlist which can render (near-)empty.
async fn manifest(State(store): State<Arc<MediaStore>>) -> Response {
    match render_mpd(&store) {
        Some(body) => ([(header::CONTENT_TYPE, DASH_MANIFEST_CONTENT_TYPE)], body).into_response(),
        None => StatusCode::SERVICE_UNAVAILABLE.into_response(),
    }
}

/// Render the live MPD for `store`'s current window. `None` if no track
/// specs have been recorded yet ([`MediaStore::track_specs`] empty — nothing
/// to describe) or if [`DashPackager::package`] itself rejects the built
/// [`Media`] (e.g. an opaque [`transmux::CodecConfig::Data`] track with no
/// derivable RFC 6381 codec string).
fn render_mpd(store: &MediaStore) -> Option<String> {
    let mut specs = store.track_specs();
    if specs.is_empty() {
        return None;
    }
    // Single-rendition model (see module docs): describe exactly one
    // Representation, `@id` forced to DEFAULT_TRACK_ID.
    let mut spec = specs.remove(0);
    spec.track_id = DEFAULT_TRACK_ID;
    let timescale = spec.timescale.max(1);

    let window = store.window_segments();
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
    let target_duration_secs = store.target_duration_secs();
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
        availability_start_time: Some(format_iso8601(store.created_at())),
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
    use transmux::TrackSpec;
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
        let store = MediaStore::new(4.0, 500, 4);
        assert!(
            render_mpd(&store).is_none(),
            "no track specs recorded yet -> nothing to describe"
        );
    }

    #[test]
    fn render_mpd_valid_before_any_segment_closes() {
        // Track specs known, but the window is still empty (no segment has
        // closed yet) -- must still render a syntactically valid MPD (a
        // degenerate SegmentTemplate@duration=0), not None/panic.
        let store = MediaStore::new(4.0, 500, 4);
        store.set_track_specs(vec![video_spec(7)]);
        let mpd = render_mpd(&store).expect("must render even with an empty window");
        assert!(mpd.contains("<MPD"));
        assert!(mpd.contains("type=\"dynamic\""));
    }

    #[test]
    fn render_mpd_forces_representation_id_to_default_track() {
        // The source's own track_id (7) must NOT leak into the Representation
        // @id -- it must be forced to DEFAULT_TRACK_ID (1) so
        // $RepresentationID$ substitution matches the shared resource
        // route's init-1.mp4/seg-1-<N>.m4s filenames.
        let store = MediaStore::new(4.0, 500, 4);
        store.set_track_specs(vec![video_spec(7)]);
        store.add_segment(seg(1, 4.0));
        let mpd = render_mpd(&store).unwrap();
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
        let store = MediaStore::new(4.0, 500, 2);
        store.set_track_specs(vec![video_spec(1)]);
        store.add_segment(seg(1, 4.0));
        store.add_segment(seg(2, 4.0));
        store.add_segment(seg(3, 4.0)); // evicts seq 1 (window_segments == 2)

        let mpd = render_mpd(&store).unwrap();
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
        let store = MediaStore::new(2.0, 500, 4);
        store.set_track_specs(vec![video_spec(1)]);
        store.add_segment(seg(1, 2.0));
        let mpd = render_mpd(&store).unwrap();
        assert!(mpd.contains("availabilityStartTime="), "{mpd}");
        assert!(mpd.contains("minimumUpdatePeriod=\"PT2S\""), "{mpd}");
        assert!(mpd.contains("timeShiftBufferDepth=\"PT2S\""), "{mpd}");
    }

    #[tokio::test]
    async fn manifest_handler_503_before_track_specs_known() {
        let store = Arc::new(MediaStore::new(4.0, 500, 4));
        let resp = manifest(State(store)).await;
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn manifest_handler_200_with_dash_content_type() {
        let store = Arc::new(MediaStore::new(4.0, 500, 4));
        store.set_track_specs(vec![video_spec(1)]);
        store.add_segment(seg(1, 4.0));
        let resp = manifest(State(store)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get(header::CONTENT_TYPE).unwrap(),
            DASH_MANIFEST_CONTENT_TYPE
        );
    }
}
