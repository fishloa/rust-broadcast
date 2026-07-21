//! `LlDashOutput`: the low-latency DASH [`Output`] implementation (issue
//! #663 P4.2, #721) — renders `manifest-ll.mpd`, a **true chunked-transfer**
//! LL-DASH MPD (ISO/IEC 23009-1 + DASH-IF Low-Latency Live Interoperability,
//! "LL IOP") built from [`transmux::LlDashPackager`], whose `SegmentTemplate`
//! addresses **whole** segments (`$Number$`, the same `seg-{track}-{seq}.m4s`
//! filenames [`crate::output::dash`]'s `manifest.mpd` already uses) with
//! `@availabilityTimeOffset`/`availabilityTimeComplete="false"` signalling
//! that a segment's bytes start flowing before it nominally completes.
//!
//! # Distinct manifest name (not a mode flag on `manifest.mpd`)
//!
//! LL-DASH is served at its own path, `/manifest-ll.mpd`, rather than
//! branching `manifest.mpd` on a query flag — a route can enable both `dash`
//! and `ll_dash` simultaneously (a DVR-capable player fetches `manifest.mpd`;
//! a live/low-latency player fetches `manifest-ll.mpd`), and the two
//! [`Output`]s stay independently toggleable per [`crate::config::Route::outputs`]
//! like every other output pair.
//!
//! # True chunked-transfer delivery (not a parts-signalling fallback)
//!
//! A DASH `SegmentTemplate` addresses one URI per **whole** segment — there
//! is no per-chunk addressing in the MPD itself. Low latency instead comes
//! from serving that one URI **before** the segment is complete: the origin
//! opens an HTTP response as soon as the segment's first bytes exist and
//! keeps it open (HTTP chunked transfer-encoding), writing more bytes as the
//! shared [`crate::store::MediaStore`]'s live parts arrive, until the segment
//! closes. `crate::origin::resource`'s shared dynamic-file route implements
//! this (see its private `stream_in_progress_segment`): a `seg-{track}-{seq}.m4s`
//! request that doesn't yet resolve to a *closed* segment is retried against
//! the store's `part-{track}-{seq}.{idx}.m4s` entries — the exact same
//! partial-segment bytes [`crate::output::llhls`] already serves for
//! LL-HLS — concatenated in order as a streamed body, rather than 404ing.
//! Once the segment closes, later requests for the same URI resolve
//! immediately from the store's whole-segment bytes with a normal
//! `Content-Length` (the ordinary, non-streaming path every other output
//! already uses).
//!
//! Reusing the LL-HLS part bytes (rather than driving a second,
//! chunk-shaped [`transmux::LlSegmenter`] from raw samples) means this
//! design's CMAF chunks are each a bare `moof`+`mdat` (no leading `styp` on
//! the segment's first part — see [`transmux::ll_hls::PartInfo`]), whereas a
//! *closed* segment's bytes (served once complete) do carry the leading
//! `styp`/`ftyp`-adjacent segment-type box. This asymmetry is intentional:
//! reusing the already-produced, already-tested part bytes is far simpler
//! than standing up a second parallel segmenter fed from raw samples, and
//! the headless dash.js-LL acceptance test (`multimux/tests/lldash_dashjs.rs`)
//! is the arbiter of whether real LL-DASH clients tolerate it — see that
//! test's module docs for the validated result.
//!
//! # Why `availabilityTimeOffset` is genuinely non-zero here
//!
//! Unlike the discrete-parts design this module previously shipped (issue
//! #663 P4.2's first cut, which addressed individual `part-*.m4s` files and
//! could only honestly claim `availabilityTimeOffset="0"` since a part was
//! only ever exposed once complete), this module's whole-segment URI *does*
//! become partially available before the segment nominally completes: the
//! chunked-transfer response starts flowing as soon as the first part exists.
//! [`transmux::LlDashPackager::availability_time_offset`] (`segment_duration`
//! minus `chunk_duration`, DASH-IF LL IOP) is therefore a truthful figure,
//! not a fabricated one.
//!
//! # DVR window
//!
//! Because whole (closed) segments stay in the shared store's rolling
//! window exactly like [`crate::output::dash`]'s regular MPD, this design can
//! (and does) advertise a real `timeShiftBufferDepth` — unlike the old
//! parts-only design, which covered only the live edge.

use std::sync::Arc;

use axum::Router;
use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use broadcast_common::Package;
use ll_hls_runtime::server::DEFAULT_TRACK_ID;
use transmux::{Addressing, LlDashPackager, Media, Track, TrackSegments};

use crate::origin::resource::cors_preflight;
use crate::output::dash::{DASH_MANIFEST_CONTENT_TYPE, format_iso8601};
use crate::output::{Output, OutputKind};
use crate::store::MediaStore;

/// Filename this output serves its manifest at (`/{stream}/manifest-ll.mpd`)
/// — distinct from [`crate::output::dash`]'s `manifest.mpd` so a route can
/// enable both simultaneously (see the module docs).
pub const LL_DASH_MANIFEST_NAME: &str = "manifest-ll.mpd";

/// Heuristic target end-to-end latency, in units of [`MediaStore::part_target_ms`]
/// (`ServiceDescription/Latency@target`, ISO/IEC 23009-1 §5.13.2) — DASH-IF LL
/// IOP guidance targets a few chunk/part durations of glass-to-glass latency
/// to absorb normal jitter; not a literal spec-mandated constant, just a
/// documented, part-duration-derived default (never an unexplained magic
/// millisecond figure).
const LATENCY_TARGET_PART_MULTIPLE: u64 = 3;

/// The low-latency DASH [`Output`]: `manifest-ll.mpd` only. Init/segment
/// bytes are the origin's shared resource route (chunked-transfer while a
/// segment is in progress) — see the module docs.
pub struct LlDashOutput;

impl Output for LlDashOutput {
    fn kind(&self) -> OutputKind {
        OutputKind::LlDash
    }

    /// Routes (relative — mounted by the origin under `/{stream}/`):
    /// - `GET /manifest-ll.mpd` — the live LL-DASH MPD.
    fn manifest_routes(&self, store: Arc<MediaStore>) -> Router {
        Router::new()
            .route(
                &format!("/{LL_DASH_MANIFEST_NAME}"),
                get(manifest).options(cors_preflight),
            )
            .with_state(store)
    }
}

/// `GET /manifest-ll.mpd` — `503 Service Unavailable` until the route has
/// recorded its track specs, mirroring `crate::output::dash`'s `manifest.mpd`
/// handler.
async fn manifest(State(store): State<Arc<MediaStore>>) -> Response {
    match render_ll_dash_mpd(&store) {
        Some(body) => ([(header::CONTENT_TYPE, DASH_MANIFEST_CONTENT_TYPE)], body).into_response(),
        None => StatusCode::SERVICE_UNAVAILABLE.into_response(),
    }
}

/// Render the live LL-DASH MPD for `store`'s current window +
/// in-progress segment, via [`transmux::LlDashPackager`]. `None` if no track
/// specs have been recorded yet (mirrors `crate::output::dash::render_mpd`),
/// if the packager rejects the built [`Media`] (e.g. an opaque
/// [`transmux::CodecConfig::Data`] track with no derivable RFC 6381 codec
/// string), or if the store's configured part target exceeds its segment
/// target (nonsensical config — `LlDashPackager::new` rejects a chunk
/// duration longer than the segment it chunks).
fn render_ll_dash_mpd(store: &MediaStore) -> Option<String> {
    let mut specs = store.track_specs();
    if specs.is_empty() {
        return None;
    }
    // Single-rendition model (see `crate::output::dash`'s module docs):
    // describe exactly one Representation, `@id` forced to DEFAULT_TRACK_ID.
    let mut spec = specs.remove(0);
    spec.track_id = DEFAULT_TRACK_ID;
    let timescale = spec.timescale.max(1);

    // `$Number$` addresses whole segments (see module docs) -- startNumber
    // tracks the window's oldest retained segment, exactly like
    // `crate::output::dash::render_mpd`.
    let window = store.window_segments();
    let start_number = window
        .first()
        .map(|s| u64::from(s.segment_seq))
        .unwrap_or(1);

    let target_duration_secs = store.target_duration_secs();
    let nominal_duration_ticks =
        ((target_duration_secs * f64::from(timescale)).round() as u64).max(1);
    let part_target_ms = store.part_target_ms().max(1);
    let chunk_duration_secs = f64::from(part_target_ms) / 1000.0;
    let latency_target_ms =
        u32::try_from(u64::from(part_target_ms) * LATENCY_TARGET_PART_MULTIPLE).unwrap_or(u32::MAX);

    let media = Media::new(vec![Track::new(spec, Vec::new())], timescale);

    let mut packager = LlDashPackager::new(
        target_duration_secs,
        chunk_duration_secs,
        latency_target_ms,
        format_iso8601(store.created_at()),
    )
    .ok()?;
    packager.base.addressing = Addressing::Number;
    packager.base.start_number = start_number;
    // `$RepresentationID$` is substituted by the DASH *client*, not here
    // (real DASH template tokens) -- left literal so it resolves to "1"
    // (DEFAULT_TRACK_ID), matching the shared resource route's
    // `init-1.mp4`/`seg-1-<N>.m4s` filenames exactly (the same whole-segment
    // shape `crate::output::dash` uses -- see module docs for why LL-DASH no
    // longer needs its own filename scheme).
    packager.base.init_template = "init-$RepresentationID$.mp4".to_string();
    packager.base.media_template = "seg-$RepresentationID$-$Number$.m4s".to_string();
    // Tuned to the chunk/part interval, not the whole-segment target -- an
    // LL-DASH client should re-poll roughly as often as a new chunk can
    // appear.
    packager.base.minimum_update_period = Some(format!("PT{chunk_duration_secs}S"));
    // Unlike the old parts-only design (module docs), whole closed segments
    // stay in the store's rolling window, so a real DVR window can be
    // advertised -- same computation as `crate::output::dash::render_mpd`.
    let time_shift_buffer_depth_secs = target_duration_secs * (window.len().max(1) as f64);
    packager.base.time_shift_buffer_depth = Some(format!("PT{time_shift_buffer_depth_secs}S"));
    packager.base.segments = vec![TrackSegments {
        track_id: DEFAULT_TRACK_ID,
        durations: vec![nominal_duration_ticks],
    }];

    packager.package(&media).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use transmux::CodecConfig;
    use transmux::TrackSpec;
    use transmux::ll_hls::{PartInfo, SegmentInfo};

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

    fn part(seq: u32, idx: u32) -> PartInfo {
        PartInfo {
            bytes: vec![0x40 + idx as u8; 4],
            duration: 0.5,
            independent: idx == 0,
            segment_seq: seq,
            part_index: idx,
        }
    }

    fn seg(seq: u32, duration: f64) -> SegmentInfo {
        SegmentInfo {
            bytes: vec![seq as u8; 8],
            duration,
            segment_seq: seq,
            part_count: 2,
        }
    }

    #[test]
    fn render_ll_dash_mpd_none_without_track_specs() {
        let store = MediaStore::new(4.0, 500, 4);
        assert!(
            render_ll_dash_mpd(&store).is_none(),
            "no track specs recorded yet -> nothing to describe"
        );
    }

    #[test]
    fn render_ll_dash_mpd_valid_before_any_segment_closes() {
        let store = MediaStore::new(4.0, 500, 4);
        store.set_track_specs(vec![video_spec(7)]);
        let mpd = render_ll_dash_mpd(&store).expect("must render even with an empty window");
        assert!(mpd.contains("<MPD"));
        assert!(mpd.contains("type=\"dynamic\""));
    }

    #[test]
    fn render_ll_dash_mpd_carries_required_ll_dash_elements() {
        let store = MediaStore::new(4.0, 500, 4);
        store.set_track_specs(vec![video_spec(1)]);
        store.add_part(part(1, 0));
        let mpd = render_ll_dash_mpd(&store).unwrap();

        assert!(
            mpd.contains("availabilityTimeComplete=\"false\""),
            "availabilityTimeComplete must be present and false: {mpd}"
        );
        // True chunked design: availabilityTimeOffset = segment(4.0) -
        // chunk(0.5) = 3.5 -- genuinely non-zero (see module docs for why,
        // unlike the old parts-signalling design's honest "0").
        assert!(
            mpd.contains("availabilityTimeOffset=\"3.5\""),
            "availabilityTimeOffset must reflect segment-chunk duration: {mpd}"
        );
        assert!(
            mpd.contains("<ServiceDescription"),
            "ServiceDescription must be present: {mpd}"
        );
        assert!(mpd.contains("<Latency target="), "{mpd}");
        assert!(
            mpd.contains("PT0.5S"),
            "minimumUpdatePeriod must be tuned to the part/chunk target: {mpd}"
        );
    }

    #[test]
    fn render_ll_dash_mpd_addresses_whole_segments_not_parts() {
        let store = MediaStore::new(4.0, 500, 4);
        store.set_track_specs(vec![video_spec(1)]);
        store.add_part(part(5, 0));
        let mpd = render_ll_dash_mpd(&store).unwrap();

        assert!(
            mpd.contains("seg-$RepresentationID$-$Number$.m4s"),
            "media template must address whole segments (the true chunked-transfer \
             design serves parts internally, never in the MPD itself): {mpd}"
        );
        assert!(
            !mpd.contains("part-"),
            "no part-addressed URI should ever appear in the MPD: {mpd}"
        );
    }

    #[test]
    fn render_ll_dash_mpd_start_number_tracks_window() {
        let store = MediaStore::new(4.0, 500, 2);
        store.set_track_specs(vec![video_spec(1)]);
        store.add_segment(seg(1, 4.0));
        store.add_segment(seg(2, 4.0));
        store.add_segment(seg(3, 4.0)); // evicts seq 1 (window_segments == 2)

        let mpd = render_ll_dash_mpd(&store).unwrap();
        assert!(
            mpd.contains("startNumber=\"2\""),
            "startNumber must track the window's oldest retained segment_seq (2): {mpd}"
        );
    }

    #[test]
    fn render_ll_dash_mpd_carries_time_shift_buffer_depth() {
        // Unlike the old parts-only design, whole closed segments stay in
        // the window -- a real DVR window can be advertised.
        let store = MediaStore::new(2.0, 500, 4);
        store.set_track_specs(vec![video_spec(1)]);
        store.add_segment(seg(1, 2.0));
        let mpd = render_ll_dash_mpd(&store).unwrap();
        assert!(mpd.contains("timeShiftBufferDepth=\"PT2S\""), "{mpd}");
    }

    #[test]
    fn render_ll_dash_mpd_forces_representation_id_to_default_track() {
        let store = MediaStore::new(4.0, 500, 4);
        store.set_track_specs(vec![video_spec(7)]);
        store.add_part(part(1, 0));
        let mpd = render_ll_dash_mpd(&store).unwrap();
        assert!(
            mpd.contains(&format!("id=\"{DEFAULT_TRACK_ID}\"")),
            "Representation @id must be the DEFAULT_TRACK_ID, not the source's own \
             track_id (7): {mpd}"
        );
        assert!(!mpd.contains("id=\"7\""), "source track_id leaked: {mpd}");
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
        store.add_part(part(1, 0));
        let resp = manifest(State(store)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get(header::CONTENT_TYPE).unwrap(),
            DASH_MANIFEST_CONTENT_TYPE
        );
    }
}
