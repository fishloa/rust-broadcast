//! `SmoothOutput`: the Smooth Streaming [`crate::output::Output`]
//! implementation — the HTTP adapter around `smooth_runtime::SmoothOrigin`,
//! which renders the MS-SSTR client Manifest and resolves fragment requests
//! from the route's shared `Trunk`.
//!
//! # What this module does, and does not
//!
//! The protocol lives in `smooth_runtime::server`: the Manifest model and its
//! XML writer, the chunk timeline, the fragment-URL grammar, stream identity
//! (`video`, `audio_2`, `text`, …) and each track's codec parameters all come
//! from there (through `broadcast-smooth` and `transmux`). This module is
//! axum routing plus the one response mapping every other output shares
//! (`crate::http::into_response`), so it never re-invents an MS-SSTR decision.
//!
//! # Route design
//!
//! The manifest is served at `GET /Manifest`. Fragment URLs are served via a
//! fallback route: axum's router cannot match the parenthesised Smooth path
//! segments (`QualityLevels(…)/Fragments(…)`) with literal routes, and the
//! shared resource route's `/:file` catch-all only matches the first segment,
//! so the fallback parses the original request URI.
//!
//! Fragments are the same bytes the shared resource route serves for
//! LL-HLS/DASH — the `Trunk` is the single copy.
//!
//! # `Cache-Control`
//!
//! The origin answers with a `media_plane::egress::CachePolicy` (`NoCache` for
//! a Manifest, `Immutable` for a fragment), but no route in this crate applies
//! that policy to the wire: the former hand-built handlers dropped it too.
//! Every response's `Cache-Control` comes from the router's own
//! `cache_control_for`, which classifies a `Manifest` and a
//! `QualityLevels(…)/Fragments(…)` path as "anything else" — `no-cache` for a
//! success and for an error alike. That is what these routes sent before and
//! what they send now (`tests/it/smooth_output.rs`'s
//! `smooth_responses_carry_no_cache` pins it).

use std::sync::Arc;

use axum::Router;
use axum::extract::{OriginalUri, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use broadcast_common::Timestamp;
use bytes::Bytes;
use hls_runtime::server::{DEFAULT_TRACK_ID, HlsBody, HlsOrigin, HlsRequest};
use media_plane::egress::{AwaitPolicy, EgressResponse, ServedEgress};
use smooth_runtime::server::{SmoothBody, SmoothOrigin, SmoothRequest};

use crate::http::{self, BLOCKING_RELOAD_TIMEOUT};
use crate::origin::resource::cors_preflight;
use crate::output::{Output, OutputKind};
use crate::route::{ProgramServing, RouteHandle};

/// `Content-Type` of a client Manifest document.
const MANIFEST_CONTENT_TYPE: &str = "text/xml";
/// `Content-Type` of a Smooth fragment (`moof`+`mdat` ISOBMFF bytes).
const FRAGMENT_CONTENT_TYPE: &str = "video/mp4";

/// The Smooth [`Output`]: a manifest plus fragment fallback, over the
/// shared [`RouteHandle`].
pub struct SmoothOutput;

impl Output for SmoothOutput {
    fn kind(&self) -> OutputKind {
        OutputKind::Smooth
    }

    /// Routes (relative — mounted by the origin under `/{stream}/`):
    /// - `GET /Manifest` — the Smooth client Manifest XML.
    /// - Fallback — catches multi-segment Smooth fragment URLs
    ///   (`QualityLevels(BITRATE)/Fragments(TYPE=START_TIME)`) that the
    ///   resource route's `/:file` catch-all cannot match (axum's
    ///   `/:file` only captures a single path segment).
    fn manifest_routes(&self, route: Arc<RouteHandle>) -> Router {
        Router::new()
            .route("/Manifest", get(manifest).options(cors_preflight))
            .fallback(get(fragment_fallback))
            .with_state(route)
    }
}

/// `GET /Manifest` — the origin's live Smooth client Manifest.
///
/// Rendering demuxes every window segment to derive each track's own `c`
/// timeline, so it runs on the **blocking pool**, not a runtime worker.
async fn manifest(State(route): State<Arc<RouteHandle>>) -> Response {
    let serving = match http::resolve_route_program(&route) {
        Ok(serving) => serving,
        Err(resp) => return *resp,
    };
    let trunk = serving.trunk();
    let origin = origin(&serving);
    let resp = http::resolve_blocking(
        &trunk,
        &origin,
        SmoothRequest::Manifest,
        BLOCKING_RELOAD_TIMEOUT,
        || (),
    )
    .await;
    http::into_response_with_message(
        resp,
        StatusCode::SERVICE_UNAVAILABLE,
        StatusCode::INTERNAL_SERVER_ERROR,
        |body| match body {
            SmoothBody::Manifest(xml) => {
                ([(header::CONTENT_TYPE, MANIFEST_CONTENT_TYPE)], xml).into_response()
            }
            _ => StatusCode::NOT_FOUND.into_response(),
        },
    )
}

/// Fallback route: catches Smooth fragment URLs which have the shape
/// `QualityLevels(BITRATE)/Fragments(TYPE=START_TIME)`. See the module docs
/// for why a fallback is needed (axum's `/:file` is single-segment only).
///
/// The request grammar, the stream it names and the fragment's bytes are all
/// the origin's; re-demuxing and re-muxing are CPU-bound and must not run on
/// a runtime worker, which [`http::resolve_blocking`] guarantees.
///
/// A fragment the origin's window does not hold (a start time older than its
/// first chunk or later than its last, or a segment that slid away while the
/// window was being read) answers `404`, exactly as an evicted HLS segment
/// does; `500` is reserved for a manifest the origin cannot use at all.
async fn fragment_fallback(State(route): State<Arc<RouteHandle>>, uri: OriginalUri) -> Response {
    let serving = match http::resolve_route_program(&route) {
        Ok(serving) => serving,
        Err(resp) => return *resp,
    };
    let trunk = serving.trunk();
    let origin = origin(&serving);
    let kinds = origin.stream_keys();
    let Some(request) = SmoothRequest::parse(uri.path(), &kinds) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let resp =
        http::resolve_blocking(&trunk, &origin, request, BLOCKING_RELOAD_TIMEOUT, || ()).await;
    http::into_response_with_message(
        resp,
        StatusCode::NOT_FOUND,
        StatusCode::INTERNAL_SERVER_ERROR,
        |body| match body {
            SmoothBody::Fragment(bytes) => {
                ([(header::CONTENT_TYPE, FRAGMENT_CONTENT_TYPE)], bytes).into_response()
            }
            _ => StatusCode::NOT_FOUND.into_response(),
        },
    )
}

/// The bytes the shared resource route serves for the segment numbered
/// `public_seq`: the `Trunk` is the single copy of the media, so this reads
/// that ring rather than keeping a second segment store.
fn segment_bytes(ll_hls: &HlsOrigin, public_seq: u64) -> Option<Bytes> {
    match ll_hls.resolve(
        HlsRequest::Resource {
            name: format!("seg-{DEFAULT_TRACK_ID}-{public_seq}.m4s"),
        },
        Timestamp::from_nanos(0),
        AwaitPolicy::new(Timestamp::from_nanos(u64::MAX)),
    ) {
        EgressResponse::Ready {
            body: HlsBody::Resource(bytes),
            ..
        } => Some(bytes),
        _ => None,
    }
}

/// The origin over this program's `Trunk`, carrying the program's init
/// segment (the one thing no `Trunk` ring holds) and its DASH-rendered
/// window (which published segments it presents) as the segment source.
fn origin(serving: &Arc<ProgramServing>) -> SmoothOrigin {
    let window = Arc::clone(serving);
    let ll_hls = serving.ll_hls();
    let tracks = Arc::clone(serving);
    let origin = SmoothOrigin::new(
        serving.trunk(),
        Arc::new(move || {
            window
                .window_segments()
                .into_iter()
                .filter(|segment| !segment.gap)
                .filter_map(|segment| {
                    segment_bytes(&ll_hls, segment.public_seq)
                        .map(|bytes| (segment.segment_seq, Arc::<[u8]>::from(bytes.as_ref())))
                })
                .collect::<Vec<_>>()
        }),
        Arc::new(move || tracks.track_specs()),
    );
    if let Some(init) = serving.init_bytes() {
        origin.set_init(Arc::<[u8]>::from(init.as_ref()));
    }
    origin
}
