//! `LlDashOutput`: the low-latency DASH [`Output`] implementation (issue #663
//! P4.2) — renders `manifest-ll.mpd`, an LL-DASH-**signalled** MPD (ISO/IEC
//! 23009-1 + DASH-IF Low-Latency Live Interoperability, "LL IOP") whose
//! `SegmentTemplate` addresses the shared [`MediaStore`]'s live **parts**
//! (`part-{track}-{seq}.{idx}.m4s` — the same LL-HLS-shaped partial-segment
//! bytes `crate::origin::resource` already serves for `ll_hls`), instead of
//! whole closed segments (`crate::output::dash`'s `manifest.mpd`).
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
//! # Scope: discrete-parts signalling, not true chunked-transfer LL-DASH
//!
//! Full LL-DASH (DASH-IF LL IOP's "chunked CMAF" mode, using
//! `transmux::LlDashPackager` and `transmux::LlSegmenter` together) splits
//! **one** segment into byte-range chunks delivered over a single HTTP
//! response via chunked transfer while the segment is still being produced —
//! `availabilityTimeOffset` then means "this segment's *bytes start flowing*
//! this many seconds before its nominal completion". Wiring that requires a
//! second, chunk-shaped segmenter output (`crate::output::dash`'s module doc
//! explains why the LL-HLS-shaped `part-*.m4s` files aren't such chunks) — a
//! larger lift than this issue's scope, tracked as a follow-up (issue #663
//! **P4.3**).
//!
//! This module instead takes the "acceptable" fallback: it re-addresses the
//! **existing** `part-*.m4s` files as the DASH-addressable unit. Concretely,
//! the LL-DASH `Representation`'s `SegmentTemplate` uses
//! [`transmux::Addressing::Number`] with:
//! - `@duration` = the real nominal **part** duration
//!   ([`MediaStore::part_target_ms`]), not the whole-segment target — so each
//!   numbered unit genuinely is one part, not a whole segment;
//! - `startNumber="0"` = the first live part's index (LL-HLS parts always
//!   restart numbering at 0 when a new segment opens — see
//!   `transmux::ll_hls`'s segmenter);
//! - a `media` template whose `$Number$` token substitutes the **part
//!   index**, with the *current in-progress segment's sequence number* baked
//!   in as plain literal text, refreshed on every manifest fetch (the MPD is
//!   `type="dynamic"`, never cached — see `manifest`): `` `part-$RepresentationID$-{seq}.$Number$.m4s` ``,
//!   which — once a real client substitutes `$RepresentationID$`/`$Number$`
//!   the same way it would for any other `SegmentTemplate` — resolves to
//!   exactly the same `part-{track}-{seq}.{idx}.m4s` filenames the shared
//!   resource route already serves for `ll_hls`.
//!
//! This intentionally covers **only the live edge** (the in-progress
//! segment's parts) — there is no `timeShiftBufferDepth` (old parts are not
//! retained long enough to offer a seek-back window; per ISO/IEC 23009-1
//! §5.3.1.2 Table 3, an absent `timeShiftBufferDepth` means "unknown", which
//! is the honest state here, not "unlimited"). A player wanting DVR seeks
//! back through `crate::output::dash`'s `manifest.mpd` instead — both can be
//! enabled on the same route.
//!
//! # Why `availabilityTimeOffset="0"`
//!
//! Unlike the "preferred" chunked-CMAF design, a `part-*.m4s` file is
//! produced **atomically** — the shared store never exposes a part's bytes
//! until the whole part is complete (`crate::origin::resource`'s blocking
//! preload-hint wait resolves only once `add_part` has run). So there is no
//! genuine *early*, partial availability to signal for an individual
//! numbered unit: `availabilityTimeOffset="0"` (present, per DASH-IF LL IOP,
//! but honestly zero) is the correct value here — the low-latency win in
//! this design comes entirely from the **small nominal segment duration**
//! (one part, not one whole segment), not from partial-segment delivery.
//! Fabricating a nonzero offset (e.g. reusing `transmux::LlDashPackager`'s
//! `segment_duration − chunk_duration` formula, which assumes the *different*
//! chunked-CMAF model) would misrepresent this origin's actual delivery
//! behaviour, so this module hand-rolls its own small XML injection instead
//! of reusing that packager.

use std::sync::Arc;

use axum::Router;
use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use broadcast_common::Package;
use ll_hls_runtime::server::DEFAULT_TRACK_ID;
use transmux::{Addressing, DashPackager, Media, Track, TrackSegments};

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

/// The low-latency DASH [`Output`]: `manifest-ll.mpd` only. Init/part bytes
/// are the origin's shared resource route — see the module docs.
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

/// Render the live LL-DASH MPD for `store`'s current in-progress segment's
/// parts. `None` if no track specs have been recorded yet (mirrors
/// `crate::output::dash::render_mpd`) or if [`DashPackager::package`] itself
/// rejects the built [`Media`].
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

    // The in-progress segment's sequence number -- baked as plain literal
    // text into the media template below (not a DASH token: it changes on
    // every manifest fetch, which is fine since this MPD is always
    // `type="dynamic"` and never cached).
    let (in_progress_seg_seq, _live_part_count) = store.latest_progress();

    let part_target_secs = f64::from(store.part_target_ms()) / 1000.0;
    let part_duration_ticks = ((part_target_secs * f64::from(timescale)).round() as u64).max(1);

    let media = Media::new(vec![Track::new(spec, Vec::new())], timescale);

    let mut packager = DashPackager {
        dynamic: true,
        addressing: Addressing::Number,
        start_number: 0,
        init_template: "init-$RepresentationID$.mp4".to_string(),
        // See the module docs: `{in_progress_seg_seq}` is plain literal text
        // baked in at render time; `$RepresentationID$`/`$Number$` are the
        // real DASH tokens a client substitutes, producing exactly the
        // `part-{track}-{seq}.{idx}.m4s` filenames the shared resource route
        // already serves.
        media_template: format!("part-$RepresentationID$-{in_progress_seg_seq}.$Number$.m4s"),
        availability_start_time: Some(format_iso8601(store.created_at())),
        // Tuned low (one part interval), not the whole-segment target -- the
        // LL-DASH client should re-poll roughly as often as a new part can
        // appear.
        minimum_update_period: Some(format!("PT{part_target_secs}S")),
        // No `time_shift_buffer_depth` / `suggested_presentation_delay`: see
        // the module docs (an absent value is spec-honest "unknown", not a
        // fabricated DVR window this origin cannot actually serve).
        segments: vec![TrackSegments {
            track_id: DEFAULT_TRACK_ID,
            durations: vec![part_duration_ticks],
        }],
        ..DashPackager::default()
    };

    let base_xml = packager.package(&media).ok()?;
    let latency_target_ms = u64::from(store.part_target_ms()) * LATENCY_TARGET_PART_MULTIPLE;
    Some(inject_ll_dash_signalling(&base_xml, latency_target_ms))
}

/// Post-process a rendered MPD to add the LL-DASH signalling this module's
/// design needs (see the module docs for why this doesn't reuse
/// `transmux::LlDashPackager`, whose `availabilityTimeOffset` formula assumes
/// the different chunked-CMAF model):
/// - `availabilityTimeOffset="0"` on every `<SegmentTemplate .../>`
///   (ISO/IEC 23009-1 §5.3.9.5.3) -- present, honestly zero (see module docs).
/// - A top-level `<ServiceDescription><Latency target="…"/></ServiceDescription>`
///   (§5.13.2), inserted as the first child of `<MPD>`.
fn inject_ll_dash_signalling(xml: &str, latency_target_ms: u64) -> String {
    let mut out = String::with_capacity(xml.len() + 256);
    for line in xml.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("<SegmentTemplate") && line.trim_end().ends_with("/>") {
            let end = line.trim_end();
            let head = &end[..end.len() - 2]; // strip "/>"
            out.push_str(head);
            out.push_str(" availabilityTimeOffset=\"0\"/>\n");
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }

    let Some(pos) = find_mpd_open_end(&out) else {
        return out;
    };
    let mut service = String::new();
    service.push_str("  <ServiceDescription id=\"0\">\n");
    service.push_str("    <Latency target=\"");
    service.push_str(&latency_target_ms.to_string());
    service.push_str("\"/>\n");
    service.push_str("  </ServiceDescription>\n");

    let mut with_sd = String::with_capacity(out.len() + service.len());
    with_sd.push_str(&out[..pos]);
    with_sd.push_str(&service);
    with_sd.push_str(&out[pos..]);
    with_sd
}

/// Byte offset just after the `>` (and its trailing newline) that closes the
/// `<MPD ...>` open tag -- mirrors `transmux::ll_dash`'s private helper of the
/// same shape.
fn find_mpd_open_end(xml: &str) -> Option<usize> {
    let start = xml.find("<MPD")?;
    let rel = xml[start..].find('>')?;
    Some(start + rel + 1 + 1) // +1 past '>', +1 past the trailing '\n'
}

#[cfg(test)]
mod tests {
    use super::*;
    use ll_hls_runtime::server::ResourceOutcome;
    use transmux::CodecConfig;
    use transmux::TrackSpec;
    use transmux::ll_hls::PartInfo;

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

    #[test]
    fn render_ll_dash_mpd_none_without_track_specs() {
        let store = MediaStore::new(4.0, 500, 4);
        assert!(
            render_ll_dash_mpd(&store).is_none(),
            "no track specs recorded yet -> nothing to describe"
        );
    }

    #[test]
    fn render_ll_dash_mpd_valid_before_any_part_lands() {
        let store = MediaStore::new(4.0, 500, 4);
        store.set_track_specs(vec![video_spec(7)]);
        let mpd = render_ll_dash_mpd(&store).expect("must render even with no live parts yet");
        assert!(mpd.contains("<MPD"));
        assert!(mpd.contains("type=\"dynamic\""));
    }

    #[test]
    fn render_ll_dash_mpd_carries_required_ll_dash_elements() {
        let store = MediaStore::new(4.0, 500, 4);
        store.set_track_specs(vec![video_spec(1)]);
        store.add_part(part(3, 0));
        store.add_part(part(3, 1));
        let mpd = render_ll_dash_mpd(&store).unwrap();

        assert!(
            mpd.contains("availabilityTimeOffset=\"0\""),
            "availabilityTimeOffset must be present: {mpd}"
        );
        assert!(
            mpd.contains("<ServiceDescription"),
            "ServiceDescription must be present: {mpd}"
        );
        assert!(mpd.contains("<Latency target="), "{mpd}");
        assert!(
            mpd.contains("PT0.5S"),
            "minimumUpdatePeriod must be tuned to the part target: {mpd}"
        );
    }

    #[test]
    fn render_ll_dash_mpd_media_template_names_real_part_files() {
        let store = MediaStore::new(4.0, 500, 4);
        store.set_track_specs(vec![video_spec(1)]);
        store.add_part(part(5, 0));
        store.add_part(part(5, 1));
        let mpd = render_ll_dash_mpd(&store).unwrap();

        assert!(
            mpd.contains("part-$RepresentationID$-5.$Number$.m4s"),
            "media template must bake the in-progress segment_seq (5) as \
             literal text, keeping $RepresentationID$/$Number$ as real DASH \
             tokens: {mpd}"
        );
        assert!(mpd.contains("startNumber=\"0\""), "{mpd}");

        // Substitute the tokens exactly like a real DASH client would
        // ($RepresentationID$ -> the forced DEFAULT_TRACK_ID, 1; $Number$ ->
        // startNumber, 0) and confirm the resolved filename is one the
        // shared store's live parts actually satisfy.
        let resolved = "part-1-5.0.m4s";
        assert!(
            matches!(
                store.resolve_resource(resolved),
                ResourceOutcome::Ready { .. }
            ),
            "resolved LL-DASH filename must be servable from the store: {resolved}"
        );
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
