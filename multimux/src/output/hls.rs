//! Shared helpers for the HLS [`crate::output::Output`] implementations
//! (`llhls` and `ts_hls`). Each behaviour lives once here; the two output
//! modules are thin adapters over these helpers, parameterised by what
//! actually differs (segment container and the LL-HLS-only
//! parts/preload/iframe machinery).

use std::sync::Arc;

use axum::Router;
use axum::extract::{Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use hls_runtime::server::{BlockingQuery, DEFAULT_TRACK_ID, HlsBody, HlsRequest, SkipDirective};
use serde::Deserialize;

use crate::http;
use crate::origin::resource::{BlockingRequestGuard, cors_preflight};
use crate::output::{Output, OutputKind};
use crate::route::{MAX_SUBTITLE_RENDITIONS, RouteHandle, subtitle_playlist_uri};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum HlsFlavour {
    LowLatency,
    Segmented,
}

pub(crate) async fn media_playlist_inner(
    route: Arc<RouteHandle>,
    flavour: HlsFlavour,
    q: BlockingReloadQuery,
) -> Response {
    let started = tokio::time::Instant::now();
    let serving = if flavour == HlsFlavour::LowLatency {
        match http::resolve_route_program_held(&route).await {
            Ok(s) => s,
            Err(resp) => return *resp,
        }
    } else {
        match http::resolve_route_program(&route) {
            Ok(s) => s,
            Err(resp) => return *resp,
        }
    };
    let trunk = serving.trunk();
    let origin = serving.ll_hls();
    let request = HlsRequest::Playlist {
        track_id: DEFAULT_TRACK_ID,
        query: q.into(),
    };
    let priority = origin.http_priority(&request);
    let playlist_timeout = hls_runtime::server::playlist_reload_timeout(&origin);
    let remaining = playlist_timeout.saturating_sub(started.elapsed());
    let (resp, timed_out) = http::resolve_blocking_timed(
        &trunk,
        serving.warms(),
        &origin,
        request,
        remaining,
        BlockingRequestGuard::new,
    )
    .await;
    http::into_playlist_response(resp, timed_out, |body| match body {
        HlsBody::Playlist(m) => {
            let mut resp =
                ([(header::CONTENT_TYPE, http::PLAYLIST_CONTENT_TYPE)], m).into_response();
            if flavour == HlsFlavour::LowLatency
                && let Ok(value) = header::HeaderValue::from_str(&format!("u={}", priority.urgency))
            {
                resp.headers_mut().insert("priority", value);
            }
            resp
        }
        HlsBody::Resource(_) => StatusCode::NOT_FOUND.into_response(),
        _ => StatusCode::NOT_FOUND.into_response(),
    })
}

/// Default media-playlist filename (issue #663 "configurable `playlist_name`")
/// — every pre-existing `HlsOutput::default()`/`OutputKind::build()` call
/// site keeps serving `/media.m3u8` unchanged.
pub const DEFAULT_PLAYLIST_NAME: &str = "media.m3u8";

/// Fixed filename for the I-frame (trick-play) playlist served by
/// [`HlsOutput`] (issue #1413 P3). Not configurable: the master playlist's
/// `#EXT-X-I-FRAME-STREAM-INF` `URI` references this name verbatim.
pub const IFRAME_PLAYLIST_NAME: &str = "iframe.m3u8";

/// The LL-HLS [`Output`]: master/media playlists over a shared
/// [`RouteHandle`]. Init/segment/part byte ranges are the origin's shared
/// resource route, not this one — see the module docs.
///
/// [`Self::low_latency`] and [`Self::segmented`] serve the media playlist under a caller-chosen filename
/// (`crate::config::Config::playlist_name`) — `master.m3u8` always points at
/// whichever name this instance was built with. [`Default`] (and therefore
/// [`OutputKind::build`]) uses [`DEFAULT_PLAYLIST_NAME`].
pub struct HlsOutput {
    flavour: HlsFlavour,
    playlist_name: String,
}

impl Default for HlsOutput {
    fn default() -> Self {
        HlsOutput::low_latency(DEFAULT_PLAYLIST_NAME)
    }
}

impl HlsOutput {
    /// Serves the media playlist at `/{playlist_name}` instead of the
    /// default `/media.m3u8` (`master.m3u8`'s `#EXT-X-STREAM-INF` reference
    /// follows suit — see `master_playlist`).
    pub(crate) fn new(flavour: HlsFlavour, playlist_name: impl Into<String>) -> Self {
        HlsOutput {
            flavour,
            playlist_name: playlist_name.into(),
        }
    }
    /// The LL-HLS output (`llhls` in a route config): fMP4 parts with
    /// blocking reload, plus the audio, subtitle and I-frame playlists, its
    /// media playlist served as `playlist_name`.
    pub fn low_latency(playlist_name: impl Into<String>) -> Self {
        Self::new(HlsFlavour::LowLatency, playlist_name)
    }
    /// The classic TS-HLS output (`ts_hls` in a route config): whole MPEG-TS
    /// segments, no parts, its media playlist served as `playlist_name`.
    pub fn segmented(playlist_name: impl Into<String>) -> Self {
        Self::new(HlsFlavour::Segmented, playlist_name)
    }
}

/// The axum state for [`HlsOutput`]'s manifest routes: the shared `route`
/// plus this instance's configured media-playlist filename (needed by
/// [`master_playlist`] to render the correct `#EXT-X-STREAM-INF` reference,
/// and by [`HlsOutput::manifest_routes`] to mount [`media_playlist`] under
/// the right path).
#[derive(Clone)]
pub(crate) struct HlsState {
    route: Arc<RouteHandle>,
    playlist_name: String,
    flavour: HlsFlavour,
    audio_index: Option<usize>,
    subtitle_slot: Option<usize>,
}

impl Output for HlsOutput {
    fn kind(&self) -> OutputKind {
        match self.flavour {
            HlsFlavour::LowLatency => OutputKind::LlHls,
            HlsFlavour::Segmented => OutputKind::TsHls,
        }
    }

    /// Routes (relative — mounted by the origin under `/{stream}/`):
    /// - `GET /master.m3u8` — minimal single-variant master playlist.
    /// - `GET /{playlist_name}` — LL-HLS media playlist, blocking-reload
    ///   aware (`/media.m3u8` unless [`HlsOutput::low_latency`] or [`HlsOutput::segmented`] configured a
    ///   different name).
    fn manifest_routes(&self, route: Arc<RouteHandle>) -> Router {
        let state = HlsState {
            route: Arc::clone(&route),
            playlist_name: self.playlist_name.clone(),
            flavour: self.flavour,
            audio_index: None,
            subtitle_slot: None,
        };
        let mut router = Router::new()
            .route("/master.m3u8", get(master_playlist).options(cors_preflight))
            .route(
                &format!("/{}", self.playlist_name),
                get(media_playlist).options(cors_preflight),
            )
            .with_state(state.clone());

        if self.flavour == HlsFlavour::LowLatency {
            let iframe_state = state.clone();
            router = router.route(
                &format!("/{}", IFRAME_PLAYLIST_NAME),
                get(iframe_playlist)
                    .options(cors_preflight)
                    .with_state(iframe_state),
            );
        }

        if self.flavour == HlsFlavour::LowLatency {
            // Serve one audio media playlist per declared rendition at its
            // declared URI (issue #1428 slice 2b). Each gets its own state
            // carrying the rendition index so the handler resolves the sibling
            // origin.
            let split = !route.audio_rendition_sources().is_empty();
            for (idx, decl) in route.audio_renditions().iter().enumerate() {
                if !split {
                    break;
                }
                let uri = decl.uri.clone();
                if uri.is_empty() {
                    continue;
                }
                let st = HlsState {
                    route: Arc::clone(&route),
                    playlist_name: self.playlist_name.clone(),
                    flavour: self.flavour,
                    audio_index: Some(idx),
                    subtitle_slot: None,
                };
                router = router.route(
                    &format!("/{}", uri),
                    get(audio_playlist).options(cors_preflight).with_state(st),
                );
            }
        } // close audio flavour guard

        // One route per subtitle slot (issue #1428): the renditions are
        // learned from the stream, after this router exists, so the slots'
        // URIs are fixed and an unused slot answers 404.
        if self.flavour == HlsFlavour::LowLatency
            && route.subtitles_enabled()
            && route.container() == hls_runtime::server::Container::Fmp4
        {
            for slot in 0..MAX_SUBTITLE_RENDITIONS {
                let st = HlsState {
                    route: Arc::clone(&route),
                    playlist_name: self.playlist_name.clone(),
                    flavour: self.flavour,
                    audio_index: None,
                    subtitle_slot: Some(slot),
                };
                router = router.route(
                    &format!("/{}", subtitle_playlist_uri(slot)),
                    get(subtitle_playlist)
                        .options(cors_preflight)
                        .with_state(st),
                );
            }
        }
        router
    }
}

/// `GET /master.m3u8` — a minimal single-variant master playlist pointing at
/// this route's configured media-playlist filename.
pub(crate) async fn master_playlist(State(state): State<HlsState>) -> Response {
    if state.flavour == HlsFlavour::LowLatency {
        http::master_playlist_response(&state.route, |origin| {
            origin.master_playlist_with_iframes(&state.playlist_name, IFRAME_PLAYLIST_NAME)
        })
    } else {
        http::master_playlist_response(&state.route, |origin| {
            origin.master_playlist(&state.playlist_name)
        })
    }
}

/// Blocking playlist reload query parameters (RFC 8216bis §6.2.5.2), as
/// deserialized from the HTTP query string — the wire-format counterpart of
/// [`hls_runtime::server::BlockingQuery`], which `media_playlist` maps
/// this into before handing off to the sans-IO engine.
#[derive(Debug, Default, Deserialize)]
pub struct BlockingReloadQuery {
    /// The Media Sequence Number the client already has, plus one — the
    /// origin should not respond until a segment/part beyond this is ready.
    #[serde(rename = "_HLS_msn")]
    pub hls_msn: Option<u64>,
    /// The part index (within `_HLS_msn`) the client is waiting for.
    #[serde(rename = "_HLS_part")]
    pub hls_part: Option<u32>,
    /// The `_HLS_skip` Delivery Directive (RFC 8216bis §6.2.5.1).
    #[serde(rename = "_HLS_skip")]
    pub hls_skip: Option<String>,
    /// The `_HLS_primary` / `_HLS_primary_id` value (RFC 8216bis Appendix D.4).
    #[serde(rename = "_HLS_primary")]
    pub hls_primary: Option<String>,
}

impl From<BlockingReloadQuery> for BlockingQuery {
    fn from(q: BlockingReloadQuery) -> Self {
        let hls_skip = match q.hls_skip.as_deref() {
            Some("YES") => Some(SkipDirective::Yes),
            Some("v2") => Some(SkipDirective::V2),
            _ => None,
        };
        BlockingQuery {
            hls_msn: q.hls_msn,
            hls_part: q.hls_part,
            hls_skip,
            hls_primary: q.hls_primary,
        }
    }
}

/// `GET /media.m3u8` — the LL-HLS media playlist for [`DEFAULT_TRACK_ID`],
/// blocking on `_HLS_msn`/`_HLS_part` when present (via
/// [`crate::http::resolve_blocking`]).
///
/// RFC 8216bis §6.2.5.2 abuse-prevention (SHOULD/MUST): `_HLS_part` without
/// `_HLS_msn` is meaningless (a part is only addressable relative to a
/// segment) and a `_HLS_msn` unreasonably far beyond the current live edge is
/// either a broken client or abuse — both are rejected with `400 Bad
/// Request` immediately (`HlsOrigin::resolve` answers `BadRequest`, which
/// [`http::into_response`] maps to `400` with no wait at all), rather than
/// blocking to the blocking-reload timeout and returning `200` regardless.
pub(crate) async fn media_playlist(
    State(state): State<HlsState>,
    Query(q): Query<BlockingReloadQuery>,
) -> Response {
    crate::output::hls::media_playlist_inner(state.route.clone(), state.flavour, q).await
}

/// `GET /{audio uri}` — the LL-HLS media playlist for one audio rendition
/// (issue #1428 slice 2b), served from its sibling `HlsOrigin` with blocking
/// reload support identical to the video playlist.
pub(crate) async fn audio_playlist(
    State(state): State<HlsState>,
    Query(q): Query<BlockingReloadQuery>,
) -> Response {
    let serving = match http::resolve_route_program_held(&state.route).await {
        Ok(serving) => serving,
        Err(resp) => return *resp,
    };
    let idx = state.audio_index.unwrap_or(0);
    let Some(audio) = serving.audio_servings().get(idx) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    sibling_playlist(
        serving.warms(),
        &audio.trunk,
        &audio.origin,
        audio.track_id,
        q,
    )
    .await
}

/// `GET /subs-{slot}.m3u8` — the WebVTT media playlist of one subtitle
/// rendition (issue #1428), 404 until the stream has shown that page.
pub(crate) async fn subtitle_playlist(
    State(state): State<HlsState>,
    Query(q): Query<BlockingReloadQuery>,
) -> Response {
    let serving = match http::resolve_route_program_held(&state.route).await {
        Ok(serving) => serving,
        Err(resp) => return *resp,
    };
    let slot = state.subtitle_slot.unwrap_or(0);
    let Some(subtitle) = serving.subtitle_servings().into_iter().nth(slot) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    sibling_playlist(
        serving.warms(),
        &subtitle.trunk,
        &subtitle.origin,
        subtitle.track_id,
        q,
    )
    .await
}

async fn sibling_playlist(
    warms: &tokio::sync::Semaphore,
    trunk: &Arc<media_plane::Trunk>,
    origin: &Arc<hls_runtime::server::HlsOrigin>,
    track_id: transmux::TrackId,
    q: BlockingReloadQuery,
) -> Response {
    let request = HlsRequest::Playlist {
        track_id,
        query: q.into(),
    };
    let priority = origin.http_priority(&request);
    let (resp, timed_out) = http::resolve_blocking_timed(
        trunk,
        warms,
        origin,
        request,
        hls_runtime::server::playlist_reload_timeout(origin),
        BlockingRequestGuard::new,
    )
    .await;
    http::into_playlist_response(resp, timed_out, |body| match body {
        HlsBody::Playlist(m) => {
            let mut resp =
                ([(header::CONTENT_TYPE, http::PLAYLIST_CONTENT_TYPE)], m).into_response();
            if let Ok(value) = header::HeaderValue::from_str(&format!("u={}", priority.urgency)) {
                resp.headers_mut().insert("priority", value);
            }
            resp
        }
        HlsBody::Resource(_) => StatusCode::NOT_FOUND.into_response(),
        _ => StatusCode::NOT_FOUND.into_response(),
    })
}

/// `GET /iframe.m3u8` — the LL-HLS I-frame (trick-play) playlist for
/// [`DEFAULT_TRACK_ID`], blocking on `_HLS_msn` when present (via
/// [`crate::http::resolve_blocking`]). Carries no parts, so `_HLS_part` is
/// rejected with `400 Bad Request` by the origin.
pub(crate) async fn iframe_playlist(
    State(state): State<HlsState>,
    Query(q): Query<BlockingReloadQuery>,
) -> Response {
    let serving = match http::resolve_route_program_held(&state.route).await {
        Ok(serving) => serving,
        Err(resp) => return *resp,
    };
    let trunk = serving.trunk();
    let ll_hls = serving.ll_hls();
    let request = HlsRequest::IFramePlaylist {
        track_id: DEFAULT_TRACK_ID,
        query: q.into(),
        media_playlist_name: state.playlist_name.clone(),
    };
    let priority = ll_hls.http_priority(&request);
    let (resp, timed_out) = http::resolve_blocking_timed(
        &trunk,
        serving.warms(),
        &ll_hls,
        request,
        hls_runtime::server::playlist_reload_timeout(&ll_hls),
        BlockingRequestGuard::new,
    )
    .await;
    http::into_playlist_response(resp, timed_out, |body| match body {
        HlsBody::Playlist(m) => {
            let mut resp =
                ([(header::CONTENT_TYPE, http::PLAYLIST_CONTENT_TYPE)], m).into_response();
            if let Ok(value) = header::HeaderValue::from_str(&format!("u={}", priority.urgency)) {
                resp.headers_mut().insert("priority", value);
            }
            resp
        }
        HlsBody::Resource(_) => StatusCode::NOT_FOUND.into_response(),
        _ => StatusCode::NOT_FOUND.into_response(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::route::RouteHandle;
    use crate::testutil::body_string;
    use std::time::Duration;
    use transmux::ll_hls::{PartInfo, SegmentInfo};

    async fn playlist_text(app: &axum::Router) -> String {
        let (status, body) = crate::testutil::get_bytes(app, "/cam/media.m3u8").await;
        assert_eq!(status, StatusCode::OK);
        String::from_utf8(body).expect("utf8")
    }

    fn segment_number(uri: &str) -> u64 {
        let stem = uri.strip_suffix(".m4s").expect("segment uri");
        stem.rsplit('-')
            .next()
            .expect("number")
            .parse()
            .expect("number")
    }

    async fn every_listed_resource_is_served(
        app: &axum::Router,
        playlist: &broadcast_hls::MediaPlaylist,
    ) {
        let mut uris: Vec<String> = Vec::new();
        for segment in &playlist.segments {
            uris.push(segment.uri.clone());
            uris.extend(segment.parts.iter().map(|p| p.uri.clone()));
            if let Some(map) = &segment.map {
                uris.push(map.uri.clone());
            }
        }
        for uri in uris {
            let (status, _) = crate::testutil::get_bytes(app, &format!("/cam/{uri}")).await;
            assert_eq!(status, StatusCode::OK, "{uri} is listed but not served");
        }
    }

    async fn validator_findings(
        app: &axum::Router,
        tag: &str,
    ) -> Vec<test_fixtures::process::msv::Finding> {
        let scratch =
            std::env::temp_dir().join(format!("llhls-reconnect-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&scratch);
        std::fs::create_dir_all(&scratch).expect("scratch");
        let text = playlist_text(app).await;
        std::fs::write(scratch.join("media.m3u8"), &text).expect("playlist");
        for line in text.lines() {
            let uri = line
                .split_once("URI=\"")
                .and_then(|(_, rest)| rest.split('"').next())
                .or_else(|| (!line.starts_with('#') && !line.is_empty()).then_some(line));
            if let Some(uri) = uri {
                let (status, body) = crate::testutil::get_bytes(app, &format!("/cam/{uri}")).await;
                if status == StatusCode::OK {
                    std::fs::write(scratch.join(uri), body).expect("resource");
                }
            }
        }
        let report = test_fixtures::process::msv::run_report(
            &scratch,
            "media.m3u8",
            Duration::from_secs(60),
        );
        let findings = test_fixtures::process::msv::findings(&report);
        for f in &findings {
            eprintln!("VALIDATOR[{tag}]: {} level={} {}", f.0, f.1, f.2);
        }
        let _ = std::fs::remove_dir_all(&scratch);
        findings
    }

    /// Apple's `mediastreamvalidator` raises no finding at its MUST level for
    /// the playlist after a reconnect that it did not already raise before.
    /// Skips loudly where the tool is not on `PATH`.
    #[tokio::test]
    async fn mediastreamvalidator_accepts_the_playlist_across_a_source_reconnect() {
        if !test_fixtures::process::msv::available() {
            eprintln!(
                "SKIP mediastreamvalidator_accepts_the_playlist_across_a_source_reconnect: `mediastreamvalidator` not on PATH"
            );
            return;
        }
        let route = std::sync::Arc::new(RouteHandle::new(1.0, 500, 8));
        let app = crate::testutil::app_with(&route, std::sync::Arc::new(HlsOutput::default()));
        let first = crate::testutil::ingest_run(&route, 8).await;
        let before = validator_findings(&app, "before").await;
        crate::source::release_route(&first, &route);
        let _second = crate::testutil::ingest_run(&route, 8).await;
        let after = validator_findings(&app, "after").await;
        let new_must: Vec<_> = after
            .iter()
            .filter(|f| f.1 <= 1 && !before.iter().any(|b| b.2 == f.2))
            .collect();
        assert!(new_must.is_empty(), "new MUST-level findings: {new_must:?}");
    }

    /// A source reconnect keeps the media playlist valid (RFC 8216bis section
    /// 6.2.1/6.2.2 in `private/specs-verified/hls/ietf/`): the Media Sequence
    /// Number only grows and numbers the segments implicitly, the first
    /// segment of the new run carries `EXT-X-DISCONTINUITY`, the
    /// Discontinuity Sequence does not decrease, and everything listed is
    /// served.
    #[tokio::test]
    async fn the_media_playlist_stays_valid_across_a_source_reconnect() {
        let route = std::sync::Arc::new(RouteHandle::new(1.0, 500, 8));
        let app = crate::testutil::app_with(&route, std::sync::Arc::new(HlsOutput::default()));
        let first = crate::testutil::ingest_run(&route, 8).await;
        let before_text = playlist_text(&app).await;
        let before = broadcast_hls::MediaPlaylist::parse(&before_text).expect("parses");

        crate::source::release_route(&first, &route);
        let _second = crate::testutil::ingest_run(&route, 8).await;
        let after_text = playlist_text(&app).await;
        let after = broadcast_hls::MediaPlaylist::parse(&after_text).expect("parses");

        for (name, text) in [("before", &before_text), ("after", &after_text)] {
            let mut report = media_doctor::Report::new();
            media_doctor::check_hls_playlist(text, &mut report);
            assert!(
                report.findings().is_empty(),
                "{name}: {:?}\n{text}",
                report.findings()
            );
        }

        let seen = before
            .segments
            .iter()
            .map(|s| segment_number(&s.uri))
            .max()
            .expect("segments");
        for (playlist, label) in [(&before, "before"), (&after, "after")] {
            for (index, segment) in playlist.segments.iter().enumerate() {
                assert_eq!(
                    segment_number(&segment.uri),
                    playlist.media_sequence + index as u64,
                    "{label}: segment {index} is not numbered MEDIA-SEQUENCE + index"
                );
            }
        }
        assert!(
            after.media_sequence > seen,
            "the Media Sequence Number must not go back to a number a client already saw"
        );
        assert!(
            after.segments[0].discontinuous,
            "the new run opens with a discontinuity"
        );
        assert!(
            after.discontinuity_sequence >= before.discontinuity_sequence,
            "DISCONTINUITY-SEQUENCE decreased"
        );
        assert!(after.segments[1..].iter().all(|s| !s.discontinuous));
        every_listed_resource_is_served(&app, &after).await;

        crate::source::release_route(&_second, &route);
        let _third = crate::testutil::ingest_run(&route, 8).await;
        let third =
            broadcast_hls::MediaPlaylist::parse(&playlist_text(&app).await).expect("parses");
        assert!(third.media_sequence > after.media_sequence + after.segments.len() as u64 - 1);
        assert!(third.segments[0].discontinuous);
        assert!(
            third.discontinuity_sequence > after.discontinuity_sequence,
            "the first reconnect's discontinuity rolled off: DISCONTINUITY-SEQUENCE must count it"
        );
        every_listed_resource_is_served(&app, &third).await;
    }

    fn part(seq: u32, idx: u32) -> PartInfo {
        PartInfo {
            bytes: vec![0x10 + idx as u8; 4],
            duration: 0.5,
            independent: idx == 0,
            segment_seq: seq,
            part_index: idx,
        }
    }

    fn seg(seq: u32) -> SegmentInfo {
        SegmentInfo {
            bytes: vec![0x20 + seq as u8; 8],
            duration: 4.0,
            segment_seq: seq,
            part_count: 2,
        }
    }

    /// A populated route: a closed segment 1, plus two live parts of
    /// in-progress segment 2 -- so the live edge is `(2, 2)`. Publishes
    /// `SPTS_PROGRAM_ID` into the registry first (`publish_new_program`) so
    /// `media_playlist`'s `resolve_route_program` lookup (issue #805 tasks
    /// 3/6) sees `Found`, exactly as a real driver-backed route's
    /// `crate::source::report_driver_progress` call does by the time it
    /// serves anything -- and so there is a `ProgramServing` bundle for
    /// `set_init`/`add_segment`/`add_part` to write into at all.
    fn make_route() -> Arc<RouteHandle> {
        let route = Arc::new(RouteHandle::new(4.0, 500, 4));
        route.publish_new_program(crate::route::SPTS_PROGRAM_ID);
        route.set_init(crate::route::SPTS_PROGRAM_ID, vec![0xAA; 8]);
        route
            .add_segment(crate::route::SPTS_PROGRAM_ID, seg(1))
            .expect("add_segment");
        route.add_part(crate::route::SPTS_PROGRAM_ID, part(2, 0));
        route.add_part(crate::route::SPTS_PROGRAM_ID, part(2, 1));
        route
    }

    /// Wraps `route` into an [`HlsState`] using the default playlist name —
    /// the shape the handlers under test actually receive as `State`.
    fn state(route: Arc<RouteHandle>) -> HlsState {
        HlsState {
            route,
            playlist_name: DEFAULT_PLAYLIST_NAME.to_string(),
            flavour: HlsFlavour::LowLatency,
            audio_index: None,
            subtitle_slot: None,
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

    /// r09-W17 (#1089): the master playlist's `BANDWIDTH` is the measured
    /// peak segment bitrate, not a fixed 5 Mb/s. `make_route`'s one closed
    /// segment is 8 bytes over 4.0 s = 64 bits / 4 s = 16 b/s.
    #[tokio::test]
    async fn master_playlist_advertises_the_measured_peak_bandwidth() {
        let route = make_route();
        let resp = master_playlist(State(state(route))).await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            body_string(resp).await,
            "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=16,AVERAGE-BANDWIDTH=16\nmedia.m3u8\n"
        );
    }

    /// Biting test (issue #663 "configurable `playlist_name`"): a
    /// non-default playlist name must appear in the master playlist's
    /// `#EXT-X-STREAM-INF` reference, and the default name must not.
    #[tokio::test]
    async fn master_playlist_points_at_configured_playlist_name() {
        let route = make_route();
        let resp = master_playlist(State(HlsState {
            route,
            playlist_name: "index.m3u8".to_string(),
            flavour: HlsFlavour::LowLatency,
            audio_index: None,
            subtitle_slot: None,
        }))
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;
        assert!(body.contains("index.m3u8"), "body: {body}");
        assert!(!body.contains("media.m3u8"), "body: {body}");
    }

    #[test]
    fn blocking_reload_query_carries_hls_primary() {
        let q = BlockingReloadQuery {
            hls_primary: Some("primary-1".to_string()),
            ..BlockingReloadQuery::default()
        };
        let bq: BlockingQuery = q.into();
        assert_eq!(bq.hls_primary.as_deref(), Some("primary-1"));
    }

    #[tokio::test]
    async fn media_playlist_hls_primary_does_not_change_rendered_playlist() {
        let route = make_route();
        let base = media_playlist(
            State(state(route.clone())),
            Query(BlockingReloadQuery {
                hls_primary: None,
                ..BlockingReloadQuery::default()
            }),
        )
        .await;
        let with_primary = media_playlist(
            State(state(route)),
            Query(BlockingReloadQuery {
                hls_primary: Some("primary-1".to_string()),
                ..BlockingReloadQuery::default()
            }),
        )
        .await;
        assert_eq!(body_string(base).await, body_string(with_primary).await);
    }

    #[tokio::test]
    async fn media_playlist_no_query_renders_now() {
        let route = make_route();
        let resp = media_playlist(State(state(route)), Query(BlockingReloadQuery::default())).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;
        assert!(body.contains("#EXT-X-PART"), "body: {body}");
    }

    #[tokio::test]
    async fn media_playlist_already_satisfied_blocking_request_resolves_immediately() {
        // Live edge is (2, 2): asking for msn=1 (an earlier segment) is
        // already satisfied and must not wait.
        let route = make_route();
        let resp = media_playlist(
            State(state(route)),
            Query(BlockingReloadQuery {
                hls_msn: Some(1),
                hls_part: Some(0),
                hls_skip: None,
                hls_primary: None,
            }),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn media_playlist_already_satisfied_same_msn_lower_part() {
        // in_progress_seg_seq == msn and live parts(2) > part(1): satisfied.
        let route = make_route();
        let resp = media_playlist(
            State(state(route)),
            Query(BlockingReloadQuery {
                hls_msn: Some(2),
                hls_part: Some(1),
                hls_skip: None,
                hls_primary: None,
            }),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn media_playlist_msn_only_waits_for_closed_segment_not_just_open_parts() {
        // make_route()'s segment 2 is OPEN with 2 live parts but not yet
        // CLOSED. RFC 8216bis §6.2.5.2: a bare `_HLS_msn=2` (no `_HLS_part`)
        // must wait for segment 2 to actually close, not resolve merely
        // because it has live parts.
        let route = make_route();
        let route_for_task = route.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(80)).await;
            route_for_task
                .add_segment(crate::route::SPTS_PROGRAM_ID, seg(2))
                .expect("add_segment"); // closes segment 2
        });

        let started = std::time::Instant::now();
        let resp = media_playlist(
            State(state(route)),
            Query(BlockingReloadQuery {
                hls_msn: Some(2),
                hls_part: None,
                hls_skip: None,
                hls_primary: None,
            }),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(
            started.elapsed() >= Duration::from_millis(70),
            "must have waited for segment 2 to close, not returned as soon as \
             it had live parts: elapsed {:?}",
            started.elapsed()
        );
        let body = body_string(resp).await;
        assert!(
            body.lines()
                .any(|l| l.starts_with("seg-1-") && l.ends_with("-2.m4s")),
            "resolved playlist must show segment 2 as a closed, fetchable segment: {body}"
        );
    }

    #[tokio::test]
    async fn media_playlist_far_future_msn_rejected_400_fast() {
        // Live edge is (2, 2). A `_HLS_msn` 1000 ahead of it is not a
        // legitimate blocking-reload request (RFC 8216bis §6.2.5.2 abuse
        // prevention) — it must 400 immediately, not consume the full
        // BLOCKING_RELOAD_TIMEOUT before giving up.
        let route = make_route();
        let started = std::time::Instant::now();
        let resp = media_playlist(
            State(state(route)),
            Query(BlockingReloadQuery {
                hls_msn: Some(1002),
                hls_part: None,
                hls_skip: None,
                hls_primary: None,
            }),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "must reject promptly, not block out the 5s timeout: {:?}",
            started.elapsed()
        );
    }

    #[tokio::test]
    async fn media_playlist_msn_within_bound_still_blocks_normally() {
        // Sanity check for the abuse-bound change: a legitimate
        // just-ahead-of-live-edge msn must still work as before (block, then
        // resolve), not get swept up by the new bound check.
        let route = make_route(); // (2, 2)
        let route_for_task = route.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            route_for_task.add_part(crate::route::SPTS_PROGRAM_ID, part(2, 2));
        });
        let resp = media_playlist(
            State(state(route)),
            Query(BlockingReloadQuery {
                hls_msn: Some(2),
                hls_part: Some(2),
                hls_skip: None,
                hls_primary: None,
            }),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn media_playlist_part_without_msn_rejected_400() {
        // RFC 8216bis §6.2.5.2: `_HLS_part` without `_HLS_msn` is
        // meaningless (a part is only addressable relative to a segment).
        let route = make_route();
        let resp = media_playlist(
            State(state(route)),
            Query(BlockingReloadQuery {
                hls_msn: None,
                hls_part: Some(0),
                hls_skip: None,
                hls_primary: None,
            }),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    /// MUTATION VERIFIED (issue #805 task 4): a route with no program
    /// announced yet (never called `publish_new_program`/`publish_program`)
    /// must answer `503 Service Unavailable` — a wait/not-ready signal — not
    /// `404 Not Found`. Changing `http::resolve_route_program`'s
    /// `ProgramResolution::NotYetAnnounced` arm to `Err(StatusCode::NOT_FOUND.into_response())`
    /// makes this test fail: `assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE)`
    /// fails, comparing actual `404 Not Found` against expected
    /// `503 Service Unavailable` — collapsing "not yet announced" into "gone"
    /// would make a route mid-connect indistinguishable from one that will
    /// never exist. Recompiled and re-run to confirm the failure, then
    /// reverted.
    #[tokio::test]
    async fn media_playlist_not_yet_announced_is_503_not_404() {
        // A bare route: never `publish_new_program`/`publish_program`d, so
        // its registry is empty -- exactly the window between a driver-backed
        // route connecting and its first `SessionEvent::NewProgram`.
        let route = Arc::new(RouteHandle::new(4.0, 500, 4));
        let resp = media_playlist(State(state(route)), Query(BlockingReloadQuery::default())).await;
        assert_eq!(
            resp.status(),
            StatusCode::SERVICE_UNAVAILABLE,
            "a route with no program announced yet must be 503 (not ready), not 404 (gone)"
        );
    }

    /// `manifest_routes`' own `OPTIONS` preflight (the `Access-Control-*`
    /// header *values* are the origin's shared `add_response_headers`
    /// middleware's job now — see `crate::origin::mod`'s tests — but the
    /// route itself, and that it 204s rather than 404ing, is this output's
    /// responsibility).
    #[tokio::test]
    async fn options_preflight_returns_no_content() {
        let route = make_route();
        let router = HlsOutput::default().manifest_routes(route);
        let req = axum::http::Request::builder()
            .method("OPTIONS")
            .uri("/media.m3u8")
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = tower::ServiceExt::oneshot(router, req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    }
}
#[cfg(test)]
mod ts_tests {
    use super::*;
    use crate::route::RouteHandle;
    use crate::testutil::body_string;
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
    /// `crate::output::hls`'s identical test fixture for the same shape.
    fn make_route() -> Arc<RouteHandle> {
        let route = Arc::new(RouteHandle::new(4.0, 500, 4).with_container(Container::MpegTs));
        route.publish_new_program(crate::route::SPTS_PROGRAM_ID);
        route
            .add_segment(crate::route::SPTS_PROGRAM_ID, seg(1, 0x21))
            .expect("add_segment");
        route
            .add_segment(crate::route::SPTS_PROGRAM_ID, seg(2, 0x22))
            .expect("add_segment");
        route
    }

    fn state(route: Arc<RouteHandle>) -> HlsState {
        HlsState {
            route,
            playlist_name: super::DEFAULT_PLAYLIST_NAME.to_string(),
            flavour: super::HlsFlavour::Segmented,
            audio_index: None,
            subtitle_slot: None,
        }
    }

    /// r09-W17 (#1089): a TS-HLS route has no init segment, so its master's
    /// `CODECS` can only come from the track specs the TS demux recovered —
    /// the same sync `report_driver_progress` does. Real TS fixture; oracle:
    /// `MP4Box -info` on the same capture's fMP4 remux reads
    /// `avc1.4D400D` / `mp4a.40.2` (`fixtures/hls/cmaf-fmp4`).
    #[tokio::test]
    async fn master_playlist_carries_codecs_from_the_ts_track_specs() {
        let ts = std::fs::read(test_fixtures::path("ts/h264-aac.ts")).expect("h264-aac.ts fixture");
        let media = transmux::TsDemux::new().demux(&ts).expect("demux");
        let specs: Vec<transmux::TrackSpec> = media.tracks.into_iter().map(|t| t.spec).collect();

        let route = make_route();
        route.set_track_specs(crate::route::SPTS_PROGRAM_ID, specs);
        let resp = master_playlist(State(state(route))).await;
        assert_eq!(resp.status(), StatusCode::OK);
        // Segment 1/2: 8 bytes in 4.2 s -> ceil(64 / 4.2) = 16 b/s.
        assert_eq!(
            body_string(resp).await,
            "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=16,AVERAGE-BANDWIDTH=15,CODECS=\"avc1.4D400D,mp4a.40.2\",RESOLUTION=320x240,FRAME-RATE=25.000\nmedia.m3u8\n"
        );
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
        for n in [1, 2] {
            assert!(
                body.lines()
                    .any(|l| l.starts_with("seg-1-") && l.ends_with(&format!("-{n}.ts"))),
                "body: {body}"
            );
        }
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
                    duration: broadcast_hls::DecimalSeconds::new(4.2).unwrap(),
                    ..Default::default()
                },
                broadcast_hls::MediaSegment {
                    uri: "seg-1-2.ts".to_string(),
                    duration: broadcast_hls::DecimalSeconds::new(4.2).unwrap(),
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
            .merge(HlsOutput::default().manifest_routes(route));

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
    /// `crate::output::hls`'s identical test.
    #[tokio::test]
    async fn options_preflight_returns_no_content() {
        let route = make_route();
        let router = HlsOutput::default().manifest_routes(route);
        let req = axum::http::Request::builder()
            .method("OPTIONS")
            .uri("/media.m3u8")
            .body(axum::body::Body::empty())
            .unwrap();
        let resp = tower::ServiceExt::oneshot(router, req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    }

    /// MUTATION VERIFIED (mirrors `crate::output::hls`'s identical test): a
    /// route with no program announced yet must be `503`, not `404`.
    #[tokio::test]
    async fn media_playlist_not_yet_announced_is_503_not_404() {
        let route = Arc::new(RouteHandle::new(4.0, 500, 4).with_container(Container::MpegTs));
        let resp = media_playlist(State(state(route)), Query(BlockingReloadQuery::default())).await;
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }
}
