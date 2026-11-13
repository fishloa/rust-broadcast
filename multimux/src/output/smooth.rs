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
//! # Caching and the blocking pool
//!
//! The origin caches its plan and every packaged fragment (see
//! `smooth_runtime::server::SmoothOrigin`), and it lives as long as the
//! program does, so a request over an unchanged window is a lookup. A request
//! that misses demuxes and re-muxes, which is CPU-bound: this module runs that
//! work on the blocking pool (`tokio::task::spawn_blocking`, at most
//! four at once per program) and answers from the cache
//! afterwards, so a runtime worker never does it. The one exception is a
//! request that waited for the first segment of a program to exist: the wait
//! loop is the shared `http::resolve_blocking`, which resolves inline when the
//! wait ends, so that first plan is built on the worker that woke it.
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

use std::sync::{Arc, Weak};

use axum::Router;
use axum::extract::{OriginalUri, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use broadcast_common::Timestamp;
use bytes::Bytes;
use hls_runtime::server::{DEFAULT_TRACK_ID, HlsBody, HlsOrigin, HlsRequest};
use media_plane::egress::{AwaitPolicy, EgressResponse, ServedEgress};
use smooth_runtime::server::{SmoothBody, SmoothOrigin, SmoothOriginConfig, SmoothRequest};
use tokio::sync::Semaphore;

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
/// timeline, so a render that is not already cached runs on the **blocking
/// pool**, not a runtime worker.
async fn manifest(State(route): State<Arc<RouteHandle>>) -> Response {
    let serving = match http::resolve_route_program(&route) {
        Ok(serving) => serving,
        Err(resp) => return *resp,
    };
    let trunk = serving.trunk();
    let smooth = smooth_serving(&serving);
    if let Err(status) = prepare(&smooth, SmoothRequest::Manifest).await {
        return status.into_response();
    }
    let resp = http::resolve_blocking(
        &trunk,
        serving.warms(),
        &smooth.origin,
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
/// the origin's; re-demuxing and re-muxing are CPU-bound, so a fragment that
/// is not already cached is built on the blocking pool by [`prepare`], and the
/// answer is then a cache lookup.
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
    let smooth = smooth_serving(&serving);
    if let Err(status) = prepare(&smooth, SmoothRequest::Manifest).await {
        return status.into_response();
    }
    let kinds = smooth.origin.stream_keys();
    let Some(request) = SmoothRequest::parse(uri.path(), &kinds) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if let Err(status) = prepare(&smooth, request).await {
        return status.into_response();
    }
    let resp = http::resolve_blocking(
        &trunk,
        serving.warms(),
        &smooth.origin,
        request,
        BLOCKING_RELOAD_TIMEOUT,
        || (),
    )
    .await;
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

/// How many plan or fragment builds one program runs on the blocking pool at
/// once; a burst of cache misses queues here instead of filling the pool.
const MAX_CONCURRENT_BUILDS: usize = 4;

/// A program's Smooth origin and the bound on the builds it offloads.
pub(crate) struct SmoothServing {
    origin: Arc<SmoothOrigin>,
    builds: Arc<Semaphore>,
}

impl SmoothServing {
    /// The origin over `serving`'s `Trunk`, with its DASH-rendered window
    /// (which published segments it presents) and track set as its sources.
    /// They hold the program weakly: the program owns this origin, so a strong
    /// reference would keep both alive forever.
    fn new(serving: &Arc<ProgramServing>) -> Self {
        let window: Weak<ProgramServing> = Arc::downgrade(serving);
        let ll_hls = serving.ll_hls();
        let tracks: Weak<ProgramServing> = Arc::downgrade(serving);
        let origin = SmoothOrigin::new(SmoothOriginConfig {
            trunk: serving.trunk(),
            segments: Arc::new(move || {
                let Some(window) = window.upgrade() else {
                    return Vec::new();
                };
                window
                    .window_segments()
                    .into_iter()
                    .filter(|segment| !segment.gap)
                    .filter_map(|segment| {
                        segment_bytes(&ll_hls, segment.public_seq)
                            .map(|bytes| (segment.segment_seq, bytes))
                    })
                    .collect::<Vec<_>>()
            }),
            tracks: Arc::new(move || {
                tracks
                    .upgrade()
                    .map(|tracks| tracks.track_specs())
                    .unwrap_or_default()
            }),
        });
        Self {
            origin: Arc::new(origin),
            builds: Arc::new(Semaphore::new(MAX_CONCURRENT_BUILDS)),
        }
    }
}

/// The program's origin, holding the program's current init segment (the one
/// thing no `Trunk` ring holds); storing the init it already has is a no-op.
fn smooth_serving(serving: &Arc<ProgramServing>) -> Arc<SmoothServing> {
    let smooth = serving
        .smooth
        .get_or_init(|| Arc::new(SmoothServing::new(serving)));
    if let Some(init) = serving.init_bytes() {
        smooth.origin.set_init(init);
    }
    Arc::clone(smooth)
}

/// Make answering `request` a cache hit: when it is not one already, build
/// what it needs on the blocking pool. `Err` is the status to answer with
/// when the build could not run.
async fn prepare(smooth: &SmoothServing, request: SmoothRequest) -> Result<(), StatusCode> {
    if smooth.origin.is_warm(&request) {
        return Ok(());
    }
    let permit = Arc::clone(&smooth.builds)
        .acquire_owned()
        .await
        .map_err(|_| StatusCode::SERVICE_UNAVAILABLE)?;
    let origin = Arc::clone(&smooth.origin);
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        origin.prepare(request);
    })
    .await
    .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)
}
