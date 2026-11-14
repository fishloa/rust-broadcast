//! `LlDashOutput`: the low-latency DASH [`Output`] implementation. This module
//! only mounts the LL-DASH manifest route; the MPD itself is rendered by the
//! shared renderer in [`crate::output::dash`], asked for
//! `DashFlavour::LowLatency`.

#[cfg(test)]
use hls_runtime::server::DEFAULT_TRACK_ID;
use std::sync::Arc;
use std::time::SystemTime;
#[cfg(test)]
use transmux::TrackId;

use axum::Router;
use axum::extract::State;
use axum::response::Response;
use axum::routing::get;

use crate::origin::resource::cors_preflight;
use crate::output::dash::{DashFlavour, serve_mpd};
use crate::output::{Output, OutputKind};
use crate::route::RouteHandle;

/// Filename the LL-DASH manifest is served at (`/{stream}/manifest-ll.mpd`),
/// distinct from the standard flavour's `manifest.mpd` so one route can offer
/// both.
pub const LL_DASH_MANIFEST_NAME: &str = "manifest-ll.mpd";

/// The low-latency DASH [`Output`]: `manifest-ll.mpd` only. Init and media
/// bytes come from the origin's shared resource route.
pub struct LlDashOutput;

impl Output for LlDashOutput {
    fn kind(&self) -> OutputKind {
        OutputKind::LlDash
    }

    /// Routes (relative — mounted by the origin under `/{stream}/`):
    /// - `GET /manifest-ll.mpd` — the live LL-DASH MPD.
    fn manifest_routes(&self, route: Arc<RouteHandle>) -> Router {
        Router::new()
            .route(
                &format!("/{LL_DASH_MANIFEST_NAME}"),
                get(manifest).options(cors_preflight),
            )
            .with_state(route)
    }
}

/// `GET /manifest-ll.mpd` — the shared [`serve_mpd`] handler over the
/// low-latency flavour of the MPD renderer (`503` until a track is
/// representable).
async fn manifest(State(route): State<Arc<RouteHandle>>) -> Response {
    serve_mpd(route, DashFlavour::LowLatency).await
}

/// Render the live LL-DASH MPD for `route`, uncached.
#[cfg(test)]
fn render_ll_dash_mpd(route: &RouteHandle) -> Option<String> {
    crate::output::dash::render_ll_dash_mpd(route)
}

/// Render the LL-DASH MPD for `route` with an explicit `origin` for
/// `availabilityStartTime`, so a test can freeze the wall-clock value.
#[doc(hidden)]
pub fn render_ll_dash_mpd_at(route: &RouteHandle, origin: SystemTime) -> Option<String> {
    crate::output::dash::render_ll_dash_mpd_at(route, origin)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::dash::DASH_MANIFEST_CONTENT_TYPE;
    use crate::route::RouteConfig;
    use crate::testutil::video_spec;

    #[tokio::test]
    async fn ll_dash_continues_across_a_source_reconnect() {
        crate::testutil::assert_dash_continues_across_a_reconnect(
            Arc::new(LlDashOutput),
            "/cam/manifest-ll.mpd",
        )
        .await;
    }

    use axum::http::{StatusCode, header};

    use transmux::ll_hls::PartInfo;

    fn part(seq: u32, idx: u32) -> PartInfo {
        PartInfo::new(vec![0x40 + idx as u8; 4], 0.5, seq, idx).with_independent(idx == 0)
    }

    fn seg(seq: u32, duration: f64) -> transmux::ll_hls::SegmentInfo {
        transmux::ll_hls::SegmentInfo::new(vec![seq as u8; 8], duration, seq).with_part_count(2)
    }

    /// Every `initialization="..."` template in `mpd`, in document order.
    fn init_templates(mpd: &str) -> Vec<&str> {
        mpd.match_indices("initialization=\"")
            .map(|(at, key)| {
                let rest = &mpd[at + key.len()..];
                &rest[..rest.find('"').expect("closing quote")]
            })
            .collect()
    }

    /// `mpd` reduced to the MPD model both flavours share: the LL-only
    /// `ServiceDescription` dropped, and every per-segment availability
    /// attribute and the `minimumUpdatePeriod` value removed (each is a
    /// `name="value"` attribute, so the whole attribute goes). What remains is
    /// the Periods — their start/duration, init URL and startNumber — which a
    /// folded standard and a folded low-latency render must spell identically.
    fn without_flavour_fields(mpd: &str) -> String {
        let mut folded = mpd.to_string();
        if let Some(start) = mpd.find("<ServiceDescription") {
            let end = start
                + mpd[start..]
                    .find("</ServiceDescription>")
                    .expect("closing tag")
                + "</ServiceDescription>".len();
            folded = format!("{}{}", &mpd[..start], &mpd[end..]);
        }
        for name in LL_ONLY_ATTRIBUTES {
            folded = without_attribute(&folded, name);
        }
        folded
    }

    /// The attributes only the low-latency flavour writes on the MPD itself
    /// and on its `SegmentTemplate`s.
    const LL_ONLY_ATTRIBUTES: [&str; 3] = [
        "availabilityTimeOffset",
        "availabilityTimeComplete",
        "minimumUpdatePeriod",
    ];

    /// `mpd` with the `name="value"` attribute removed wherever it appears,
    /// and the blank line the dropped `ServiceDescription` leaves behind.
    fn without_attribute(mpd: &str, name: &str) -> String {
        let key = format!(" {name}=\"");
        let mut out = String::with_capacity(mpd.len());
        let mut rest = mpd;
        while let Some(at) = rest.find(&key) {
            out.push_str(&rest[..at]);
            match rest[at + key.len()..].find('"') {
                Some(close) => rest = &rest[at + key.len() + close + 1..],
                None => rest = "",
            }
        }
        out.push_str(rest);
        out.replace("\n  \n  <Period ", "\n  <Period ")
    }

    /// A gap placeholder: the source had no muxable track while this segment
    /// was cut, so it carries no bytes.
    fn gap_seg(seq: u32, duration: f64) -> transmux::ll_hls::SegmentInfo {
        {
            let mut s = seg(seq, duration);
            s.bytes = Vec::new();
            s
        }
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
        let now = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
        route
            .add_segment(crate::route::SPTS_PROGRAM_ID, seg(1, 4.0))
            .expect("add_segment");
        let first = publish_time_of(&render_ll_dash_mpd_at(&route, now).expect("renders"));
        assert_eq!(
            first,
            publish_time_of(&render_ll_dash_mpd_at(&route, now).expect("renders"))
        );
        route
            .add_segment(crate::route::SPTS_PROGRAM_ID, seg(2, 0.5))
            .expect("add_segment");
        let second = publish_time_of(&render_ll_dash_mpd_at(&route, now).expect("renders"));
        let (t1, t2): (jiff::Timestamp, jiff::Timestamp) = (
            first.parse().expect("xs:dateTime"),
            second.parse().expect("xs:dateTime"),
        );
        assert!(t2 > t1, "{second} must be later than {first}");
        assert_eq!(first, "2023-11-14T22:13:24Z");
        assert_eq!(second, "2023-11-14T22:13:24.5Z");
    }

    #[test]
    fn render_ll_dash_mpd_none_without_track_specs() {
        let route = RouteHandle::new(
            RouteConfig::default()
                .with_target_duration_secs(4.0)
                .with_part_target_ms(500)
                .with_window_segments(4),
        );
        assert!(
            render_ll_dash_mpd(&route).is_none(),
            "no track specs recorded yet -> nothing to describe"
        );
    }

    #[test]
    fn render_ll_dash_mpd_valid_before_any_segment_closes() {
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
        let mpd = render_ll_dash_mpd(&route).expect("must render even with an empty window");
        assert!(mpd.contains("<MPD"));
        assert!(mpd.contains("type=\"dynamic\""));
    }

    #[test]
    fn render_ll_dash_mpd_carries_required_ll_dash_elements() {
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
        route.add_part(crate::route::SPTS_PROGRAM_ID, part(1, 0));
        let mpd = render_ll_dash_mpd(&route).unwrap();

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
        route.add_part(crate::route::SPTS_PROGRAM_ID, part(5, 0));
        let mpd = render_ll_dash_mpd(&route).unwrap();

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
    fn render_ll_dash_mpd_start_number_is_fixed_at_first_segment() {
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

        let mpd = render_ll_dash_mpd(&route).unwrap();
        assert!(
            mpd.contains("startNumber=\"1\""),
            "startNumber must stay at the stream's first segment (1) after seq 1 left \
             the window; a sliding startNumber (2) made players 404 (issue #1058): {mpd}"
        );
        assert!(!mpd.contains("startNumber=\"2\""), "{mpd}");
    }

    #[test]
    fn render_ll_dash_mpd_carries_time_shift_buffer_depth() {
        // Unlike the old parts-only design, whole closed segments stay in
        // the window -- a real DVR window can be advertised.
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
        let mpd = render_ll_dash_mpd(&route).unwrap();
        assert!(mpd.contains("timeShiftBufferDepth=\"PT2S\""), "{mpd}");
    }

    /// Byte-for-byte golden of the whole LL-DASH MPD with `availabilityStartTime`
    /// frozen, proving the jiff migration (SP5) changes no wire byte.
    #[test]
    fn render_ll_dash_mpd_matches_frozen_now_golden() {
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
        let now = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
        let instance = route
            .serving(crate::route::SPTS_PROGRAM_ID)
            .unwrap()
            .ll_hls()
            .instance()
            .to_string();
        let actual = render_ll_dash_mpd_at(&route, now)
            .expect("renders")
            .replace(&instance, "INSTANCE");
        let file = "golden/dash/ll-dash-mpd.golden";
        if let Ok(dir) = std::env::var("GOLDEN_BLESS") {
            std::fs::create_dir_all(&dir).expect("create golden dir");
            std::fs::write(std::path::Path::new(&dir).join(file), &actual).expect("write");
            return;
        }
        let path = test_fixtures::path(file);
        let expected = std::fs::read_to_string(&path).expect("read golden");
        assert_eq!(actual, expected, "{file} differs from the golden output");
    }

    #[test]
    fn render_ll_dash_mpd_forces_representation_id_to_default_track() {
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
        route.add_part(crate::route::SPTS_PROGRAM_ID, part(1, 0));
        let mpd = render_ll_dash_mpd(&route).unwrap();
        assert!(
            mpd.contains(&format!("id=\"{DEFAULT_TRACK_ID}\"")),
            "Representation @id must be the DEFAULT_TRACK_ID, not the source's own \
             track_id (7): {mpd}"
        );
        assert!(!mpd.contains("id=\"7\""), "source track_id leaked: {mpd}");
    }

    /// Publishes its own program first (`publish_new_program`) — see
    /// `output::dash`'s identical test for why (isolates issue #776's "no
    /// representable track" 503 from issue #805's "not yet announced" 503).
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
        route.add_part(crate::route::SPTS_PROGRAM_ID, part(1, 0));
        let resp = manifest(State(route)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get(header::CONTENT_TYPE).unwrap(),
            DASH_MANIFEST_CONTENT_TYPE
        );
    }

    /// MUTATION VERIFIED (issue #805 task 4): a route with no program
    /// announced yet must answer `503`, not `404` — see
    /// `output::hls`'s identical test for the mutation this guards.
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

    /// Issue #2448 acceptance: a window holding a gap (the source dropped every
    /// track) and segments from two init generations must render the SAME
    /// Periods under both flavours — one Period ending where the gap begins,
    /// one per init generation, each naming its own versioned init URL — and
    /// the low-latency flavour must add only its LL fields on top.
    #[test]
    fn an_ll_dash_window_over_a_gap_and_a_codec_change_renders_the_standard_periods_plus_ll() {
        let program = crate::route::SPTS_PROGRAM_ID;
        let route = RouteHandle::new(
            RouteConfig::default()
                .with_target_duration_secs(4.0)
                .with_part_target_ms(500)
                .with_window_segments(8),
        );
        route.publish_new_program(program);
        route.set_init(program, vec![0xAA; 8]);
        route.set_track_specs(program, vec![video_spec(TrackId::new(1))]);
        for seq in 1..=3 {
            route
                .add_segment(program, seg(seq, 4.0))
                .expect("add_segment");
        }
        route.set_init(program, vec![0xBB; 8]);
        for seq in 4..=5 {
            route
                .add_segment(program, seg(seq, 4.0))
                .expect("add_segment");
        }
        route
            .add_segment(program, gap_seg(6, 4.0))
            .expect("add_segment");
        for seq in 7..=8 {
            route
                .add_segment(program, seg(seq, 4.0))
                .expect("add_segment");
        }

        let now = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
        let standard =
            crate::output::dash::render_mpd_at(&route, now).expect("standard flavour renders");
        let low_latency = render_ll_dash_mpd_at(&route, now).expect("low-latency flavour renders");

        assert_eq!(
            without_flavour_fields(&low_latency),
            without_flavour_fields(&standard),
            "the low-latency MPD must carry exactly the standard MPD's Periods: {low_latency}"
        );

        for (flavour, mpd) in [("standard", &standard), ("low-latency", &low_latency)] {
            assert_eq!(mpd.matches("<Period ").count(), 3, "{flavour}: {mpd}");
            assert!(
                mpd.contains("<Period id=\"0\" start=\"PT0S\" duration=\"PT12S\">"),
                "{flavour}: the first Period must be its init generation's, ending at the codec change: {mpd}"
            );
            assert!(
                mpd.contains("<Period id=\"1\" start=\"PT12S\" duration=\"PT8S\">"),
                "{flavour}: the second Period must end where the gap begins: {mpd}"
            );
            assert!(
                mpd.contains("<Period id=\"2\" start=\"PT24S\">"),
                "{flavour}: a gap ends a Period and the next segments open a new one: {mpd}"
            );
            let inits = init_templates(mpd);
            assert_eq!(inits.len(), 3, "{flavour}: {mpd}");
            assert_ne!(
                inits[0], inits[1],
                "{flavour}: the Periods either side of the codec change name their own init: {mpd}"
            );
            assert_eq!(
                inits[1], inits[2],
                "{flavour}: the Period after the gap stays on the second generation's init: {mpd}"
            );
            assert!(
                inits[0].ends_with("-1.mp4") && inits[1].ends_with("-2.mp4"),
                "{flavour}: {{RepresentationID}} then the instance and the generation the \
                 Period's segments were cut under: {mpd}"
            );
            assert!(
                mpd.contains("urn:scte:scte35:2013:bin"),
                "{flavour}: both flavours declare the SCTE-35 event stream: {mpd}"
            );
        }

        assert!(
            !standard.contains("<ServiceDescription") && !standard.contains("availabilityTime"),
            "the standard flavour carries none of the LL fields: {standard}"
        );
        assert!(
            low_latency.contains("<ServiceDescription") && low_latency.contains("<Latency target="),
            "{low_latency}"
        );
        assert!(
            low_latency.contains("availabilityTimeOffset=\"3.5\"")
                && low_latency.contains("availabilityTimeComplete=\"false\""),
            "{low_latency}"
        );
        assert!(
            low_latency.contains("minimumUpdatePeriod=\"PT0.5S\""),
            "minimumUpdatePeriod must follow the chunk interval: {low_latency}"
        );
    }
}
