//! `TsHlsOutput`: the classic MPEG-TS HLS [`crate::output::Output`]
//! implementation (issue #887) — a thin tokio+axum adapter over the same
//! sans-IO origin engine [`llhls`](crate::output::llhls) uses
//! ([`hls_runtime::server::HlsOrigin`]), except this route's `HlsOrigin` is
//! built with `.container(Container::MpegTs)` (see
//! `crate::route::ProgramServing::new`) rather than the default
//! `Container::Fmp4`: master/media playlists here reference whole `.ts`
//! media segments instead of fMP4 `.m4s` fragments, and the media playlist
//! carries no `#EXT-X-MAP` (RFC 8216bis §3.1.1's PAT/PMT-or-`EXT-X-MAP`
//! disjunction — a self-initialising `.ts` segment needs neither).
//!
//! The init/segment byte ranges these playlists reference are served by the
//! origin's *shared* resource route (`crate::origin::resource`), exactly like
//! [`llhls`](crate::output::llhls) — see that module's own docs and
//! `crate::output`'s module doc for why one shared route serves both
//! containers.
//!
//! Which container a route is built with is decided once, at
//! `crate::origin::serve_with_registry`'s route-construction time, from
//! whether [`crate::config::Route::outputs`] names
//! [`crate::output::OutputKind::TsHls`] — mutually exclusive with
//! `llhls`/`dash`/`ll_dash` on the same route
//! (`crate::config::Route::validate_standalone`).
//!
//! Master/media playlist tags are RFC 8216 §4.3.4 (`#EXT-X-STREAM-INF`) and
//! §4.3.3 (`#EXTM3U`/`#EXT-X-VERSION`/`#EXTINF`/`#EXT-X-ENDLIST`, rendered by
//! [`hls_runtime::server::master_playlist_m3u8`] for the master playlist and
//! `HlsOrigin`'s own renderer for the media playlist). No LL-HLS blocking-
//! reload query parameters are meaningful for classic HLS (there is no
//! `part_target_ms`/`.low_latency(..)` on a `ts_hls` route's `HlsOrigin` at
//! all — see `crate::route::ProgramServing::new`), but this route reuses the
//! same [`crate::output::llhls::BlockingReloadQuery`]/`resolve_blocking` wait
//! machinery as LL-HLS anyway: it degrades harmlessly to "no query params ->
//! render immediately" for a classic client, and reusing it (rather than a
//! parallel bespoke wait loop) is exactly the "one adapter, not one per
//! output" discipline `crate::http`'s own module doc requires.

use std::sync::Arc;

use axum::Router;
use axum::extract::{Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use hls_runtime::server::{DEFAULT_TRACK_ID, HlsBody, HlsRequest, master_playlist_m3u8};

use crate::http::{self, BLOCKING_RELOAD_TIMEOUT};
use crate::origin::resource::{BlockingRequestGuard, cors_preflight};
use crate::output::llhls::BlockingReloadQuery;
use crate::output::{Output, OutputKind};
use crate::route::RouteHandle;

const MEDIA_PLAYLIST_CONTENT_TYPE: &str = "application/vnd.apple.mpegurl";

/// Default media-playlist filename — mirrors
/// [`crate::output::llhls::DEFAULT_PLAYLIST_NAME`] (both `"media.m3u8"`);
/// kept as this module's own constant (rather than re-exporting the LL-HLS
/// one) since the two outputs are configured independently
/// (`crate::config::Config::playlist_name` applies to whichever single
/// fMP4-or-TS output is actually configured on a route — the two are
/// mutually exclusive, so there is never a name clash to resolve).
pub const DEFAULT_PLAYLIST_NAME: &str = "media.m3u8";

/// The classic TS-HLS [`Output`]: master/media playlists over a shared
/// [`RouteHandle`] whose `HlsOrigin` is configured for
/// [`hls_runtime::server::Container::MpegTs`] (see this module's own doc).
/// Init/segment byte ranges are the origin's shared resource route, not this
/// one — see the module docs.
pub struct TsHlsOutput {
    playlist_name: String,
}

impl Default for TsHlsOutput {
    fn default() -> Self {
        TsHlsOutput::new(DEFAULT_PLAYLIST_NAME)
    }
}

impl TsHlsOutput {
    /// Serves the media playlist at `/{playlist_name}` instead of the
    /// default `/media.m3u8`.
    pub fn new(playlist_name: impl Into<String>) -> Self {
        TsHlsOutput {
            playlist_name: playlist_name.into(),
        }
    }
}

/// The axum state for [`TsHlsOutput`]'s manifest routes — mirrors
/// [`crate::output::llhls::LlHlsState`]'s shape exactly.
#[derive(Clone)]
pub(crate) struct TsHlsState {
    route: Arc<RouteHandle>,
    playlist_name: String,
}

impl Output for TsHlsOutput {
    fn kind(&self) -> OutputKind {
        OutputKind::TsHls
    }

    /// Routes (relative — mounted by the origin under `/{stream}/`):
    /// - `GET /master.m3u8` — minimal single-variant master playlist.
    /// - `GET /{playlist_name}` — classic media playlist (`/media.m3u8`
    ///   unless [`TsHlsOutput::new`] configured a different name).
    fn manifest_routes(&self, route: Arc<RouteHandle>) -> Router {
        let state = TsHlsState {
            route,
            playlist_name: self.playlist_name.clone(),
        };
        Router::new()
            .route("/master.m3u8", get(master_playlist).options(cors_preflight))
            .route(
                &format!("/{}", self.playlist_name),
                get(media_playlist).options(cors_preflight),
            )
            .with_state(state)
    }
}

/// `GET /master.m3u8` — a minimal single-variant master playlist pointing at
/// this route's configured media-playlist filename.
pub(crate) async fn master_playlist(State(state): State<TsHlsState>) -> Response {
    (
        [(header::CONTENT_TYPE, MEDIA_PLAYLIST_CONTENT_TYPE)],
        master_playlist_m3u8(&state.playlist_name),
    )
        .into_response()
}

/// `GET /media.m3u8` — the classic media playlist for [`DEFAULT_TRACK_ID`].
/// See this module's own doc for why this reuses
/// [`crate::output::llhls::BlockingReloadQuery`]/`resolve_blocking` even
/// though a `ts_hls` route's `HlsOrigin` never enables LL-HLS.
pub(crate) async fn media_playlist(
    State(state): State<TsHlsState>,
    Query(q): Query<BlockingReloadQuery>,
) -> Response {
    let serving = match http::resolve_route_program(&state.route) {
        Ok(serving) => serving,
        Err(resp) => return *resp,
    };
    let trunk = serving.trunk();
    let ll_hls = serving.ll_hls();
    let request = HlsRequest::Playlist {
        track_id: DEFAULT_TRACK_ID,
        query: q.into(),
    };
    let resp = http::resolve_blocking(
        &trunk,
        ll_hls.as_ref(),
        request,
        BLOCKING_RELOAD_TIMEOUT,
        BlockingRequestGuard::new,
    )
    .await;
    http::into_response(resp, StatusCode::NOT_FOUND, |body| match body {
        HlsBody::Playlist(m) => {
            ([(header::CONTENT_TYPE, MEDIA_PLAYLIST_CONTENT_TYPE)], m).into_response()
        }
        HlsBody::Resource(_) => StatusCode::NOT_FOUND.into_response(),
        // `HlsBody` is `#[non_exhaustive]`; a future body variant this
        // playlist route doesn't understand is treated the same as a
        // resource body -- not found here.
        _ => StatusCode::NOT_FOUND.into_response(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::route::RouteHandle;
    use hls_runtime::server::Container;
    use transmux::ll_hls::SegmentInfo;

    /// A non-whole-second duration (4.2s, not 4.0s): `broadcast_hls`'s own
    /// `#EXT-X-VERSION` derivation only bumps the version for a duration that
    /// actually *renders* with a decimal point (RFC 8216 §8 row 3) — a whole
    /// `4.0` renders as bare `4` and triggers nothing. Needed so
    /// `media_playlist_version_is_broadcast_hls_derived_not_hardcoded` below
    /// has a real, non-trivial version to compare against.
    fn seg(seq: u32, byte: u8) -> SegmentInfo {
        SegmentInfo {
            bytes: vec![byte; 8],
            duration: 4.2,
            segment_seq: seq,
            part_count: 0,
        }
    }

    /// A populated classic-TS route: two closed segments. Publishes
    /// `SPTS_PROGRAM_ID` into the registry first (`publish_new_program`), so
    /// `media_playlist`'s `resolve_route_program` lookup sees `Found` — see
    /// `crate::output::llhls`'s identical test fixture for the same shape.
    fn make_route() -> Arc<RouteHandle> {
        let route = Arc::new(RouteHandle::new(4.0, 500, 4).with_container(Container::MpegTs));
        route.publish_new_program(crate::route::SPTS_PROGRAM_ID);
        route.add_segment(crate::route::SPTS_PROGRAM_ID, seg(1, 0x21));
        route.add_segment(crate::route::SPTS_PROGRAM_ID, seg(2, 0x22));
        route
    }

    async fn body_string(resp: Response) -> String {
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    fn state(route: Arc<RouteHandle>) -> TsHlsState {
        TsHlsState {
            route,
            playlist_name: DEFAULT_PLAYLIST_NAME.to_string(),
        }
    }

    #[tokio::test]
    async fn master_playlist_ok() {
        let route = make_route();
        let resp = master_playlist(State(state(route))).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;
        assert!(body.contains("#EXTM3U"));
        assert!(body.contains("#EXT-X-STREAM-INF"));
        assert!(body.contains("media.m3u8"));
    }

    /// The defining behaviour of this output (issue #887): served segments
    /// are `.ts`, and the media playlist carries no `#EXT-X-MAP` at all —
    /// `Container::MpegTs`'s segments are self-initialising (in-band
    /// PAT/PMT), so there is no init segment to reference.
    ///
    /// MUTATION VERIFIED: forcing `crate::route::ProgramServing::new` to
    /// always call `.container(Container::Fmp4)` regardless of the route's
    /// configured container (simulating the pre-#887 fMP4-only wiring) makes
    /// this test's `assert!(!body.contains("#EXT-X-MAP"))` fail: the rendered
    /// body contains `#EXT-X-MAP:URI="init-1.mp4"` (actual) where none was
    /// expected, and the segment URIs render as `seg-1-1.m4s`/`seg-1-2.m4s`
    /// (`.m4s`) instead of `.ts`, failing
    /// `assert!(body.contains("seg-1-1.ts"))` too. Recompiled and re-run to
    /// confirm both failures, then reverted.
    #[tokio::test]
    async fn media_playlist_serves_ts_segments_with_no_ext_x_map() {
        let route = make_route();
        let resp = media_playlist(State(state(route)), Query(BlockingReloadQuery::default())).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;
        assert!(body.contains("#EXTINF:"), "body: {body}");
        assert!(body.contains("seg-1-1.ts"), "body: {body}");
        assert!(body.contains("seg-1-2.ts"), "body: {body}");
        assert!(
            !body.contains("#EXT-X-MAP"),
            "a classic TS media playlist must never advertise an init segment: {body}"
        );
        assert!(
            !body.contains(".m4s"),
            "a classic TS route must never reference fMP4 segment filenames: {body}"
        );
    }

    /// The rendered `#EXT-X-VERSION` must be `broadcast-hls`'s own
    /// content-derived value (RFC 8216bis §8), never a value this crate
    /// chose ahead of time — proven by constructing an equivalent
    /// `broadcast_hls::MediaPlaylist` from the same segment data fed into
    /// `make_route` and asserting the two agree, rather than asserting a
    /// bare integer literal.
    #[tokio::test]
    async fn media_playlist_version_is_broadcast_hls_derived_not_hardcoded() {
        let route = make_route();
        let resp = media_playlist(State(state(route)), Query(BlockingReloadQuery::default())).await;
        let body = body_string(resp).await;
        let rendered_version: u8 = body
            .lines()
            .find_map(|l| l.strip_prefix("#EXT-X-VERSION:"))
            .expect("a rendered media playlist always carries #EXT-X-VERSION here")
            .parse()
            .expect("#EXT-X-VERSION value must be a valid integer");

        // Same shape `HlsOrigin::render_playlist` builds for this route's two
        // segments (floating-point `#EXTINF` durations, no low-latency/i-frame
        // tags) -- `broadcast_hls::MediaPlaylist::computed_version` is the one
        // and only source either side derives its version from.
        let equivalent = broadcast_hls::MediaPlaylist {
            target_duration: 4,
            media_sequence: 1,
            segments: vec![
                broadcast_hls::MediaSegment {
                    uri: "seg-1-1.ts".to_string(),
                    duration: 4.2,
                    ..Default::default()
                },
                broadcast_hls::MediaSegment {
                    uri: "seg-1-2.ts".to_string(),
                    duration: 4.2,
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        let expected_version = equivalent
            .computed_version()
            .expect("floating-point EXTINF durations trigger a real version floor");
        assert_eq!(
            rendered_version, expected_version,
            "the served playlist's #EXT-X-VERSION must equal broadcast-hls's own \
             content-derived value for equivalent segment data, not a value multimux chose"
        );
    }

    /// **Advertised == servable** (issue #887). Drives the SAME real axum
    /// router shape `crate::origin::router` assembles for a stream (the
    /// shared resource route merged with this output's manifest routes) —
    /// renders the playlist through it, extracts every `.ts` URI **from the
    /// rendered text** (not from test-internal knowledge of what was
    /// published), requests each one back through that same router, and
    /// asserts the served bytes are byte-identical to what was published via
    /// `RouteHandle::add_segment`. Proves the whole HTTP path end to end,
    /// not just that `HlsOrigin::resolve` agrees with itself.
    #[tokio::test]
    async fn advertised_ts_segment_is_exactly_what_was_published() {
        let route = make_route();
        let router = crate::origin::resource::router(route.clone())
            .merge(TsHlsOutput::default().manifest_routes(route));

        let playlist_req = axum::http::Request::builder()
            .method("GET")
            .uri("/media.m3u8")
            .body(axum::body::Body::empty())
            .unwrap();
        let playlist_resp = tower::ServiceExt::oneshot(router.clone(), playlist_req)
            .await
            .unwrap();
        assert_eq!(playlist_resp.status(), StatusCode::OK);
        let playlist = body_string(playlist_resp).await;

        let uris: Vec<&str> = playlist
            .lines()
            .filter(|l| !l.starts_with('#') && l.ends_with(".ts"))
            .collect();
        assert_eq!(
            uris.len(),
            2,
            "make_route() published exactly 2 segments: {playlist}"
        );

        for uri in uris {
            let req = axum::http::Request::builder()
                .method("GET")
                .uri(format!("/{uri}"))
                .body(axum::body::Body::empty())
                .unwrap();
            let resp = tower::ServiceExt::oneshot(router.clone(), req)
                .await
                .unwrap();
            assert_eq!(
                resp.status(),
                StatusCode::OK,
                "requesting advertised uri {uri:?}"
            );
            let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
                .await
                .unwrap();
            // `make_route()` published segment 1 as all-`0x21` bytes and
            // segment 2 as all-`0x22` -- matching which one this URI names
            // proves the served bytes are exactly what was published, not
            // merely "some 8 bytes".
            let expected_byte = if uri.contains("-1.ts") {
                0x21u8
            } else {
                0x22u8
            };
            assert_eq!(
                bytes.to_vec(),
                vec![expected_byte; 8],
                "served bytes for advertised uri {uri:?} must match what was published"
            );
        }
    }

    /// `manifest_routes`' own `OPTIONS` preflight — mirrors
    /// `crate::output::llhls`'s identical test.
    #[tokio::test]
    async fn options_preflight_returns_no_content() {
        let route = make_route();
        let router = TsHlsOutput::default().manifest_routes(route);
        let req = axum::http::Request::builder()
            .method("OPTIONS")
            .uri("/media.m3u8")
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = tower::ServiceExt::oneshot(router, req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    }

    /// MUTATION VERIFIED (mirrors `crate::output::llhls`'s identical test): a
    /// route with no program announced yet must be `503`, not `404`.
    #[tokio::test]
    async fn media_playlist_not_yet_announced_is_503_not_404() {
        let route = Arc::new(RouteHandle::new(4.0, 500, 4).with_container(Container::MpegTs));
        let resp = media_playlist(State(state(route)), Query(BlockingReloadQuery::default())).await;
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }
}
