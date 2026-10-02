//! `SmoothOutput`: the Smooth Streaming [`crate::output::Output`]
//! implementation (issue #742) — renders an MS-SSTR client Manifest XML
//! plus fragment responses from the shared [`crate::route::RouteHandle`]'s
//! `Trunk`-drained window.
//!
//! # Architecture
//!
//! Smooth Streaming ([MS-SSTR]) serves two kinds of response:
//! - a **client Manifest** (`/<route>/Manifest`) — the
//!   `SmoothStreamingMedia` XML describing available tracks and fragment
//!   timelines (one `StreamIndex` per track, `QualityLevel`, `c` entries);
//! - **fragment** requests in the Smooth URI shape
//!   (`QualityLevels({bitrate})/Fragments({type}={start time})`) — each
//!   returns the self-contained fMP4 segment bytes the `Trunk` already
//!   holds (the same `styp`+`moof`+`mdat` bytes every other output shares).
//!
//! # Route design
//!
//! The manifest is served at `GET /Manifest`. Fragment URLs are served via
//! a fallback route: axum's router cannot match the parenthesised Smooth
//! path segments (`QualityLevels(…)/Fragments(…)`) with literal routes, and
//! the shared resource route's `/:file` catch-all only matches the first
//! segment — so this output's fallback catches multi-segment paths by
//! inspecting the original request URI. It serves the exact same segment
//! bytes through the route's `HlsOrigin` resource path.
//!
//! Fragments are the same bytes the shared resource route serves for
//! LL-HLS/DASH — the `Trunk` is the single copy; this module only maps
//! Smooth time-addressed URLs to the same segments.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::{OriginalUri, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use broadcast_common::{Timestamp, Unpackage};
use hls_runtime::server::{DEFAULT_TRACK_ID, HlsBody, HlsRequest};
use media_plane::egress::{AwaitPolicy, CachePolicy, EgressResponse, ServedEgress};
use transmux::CodecConfig;
use transmux::smooth::SMOOTH_TIMESCALE;
use transmux::{Fmp4Demux, SmoothPackager, SmoothStreamType};

use crate::http::{self, BLOCKING_RELOAD_TIMEOUT};
use crate::origin::resource::cors_preflight;
use crate::output::{Output, OutputKind};
use crate::route::{DashWindowSegment, ProgramServing, RouteHandle};

const MANIFEST_CONTENT_TYPE: &str = "text/xml";

/// FourCC code for H.264 video (MS-SSTR §2.2.2.5).
const FOURCC_H264: &str = "H264";
/// FourCC code for AAC audio (MS-SSTR §2.2.2.5).
const FOURCC_AACL: &str = "AACL";
/// Smooth manifest major version.
const MAJOR_VERSION: u32 = 2;
/// Smooth manifest minor version.
const MINOR_VERSION: u32 = 0;

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
        let state = SmoothState {
            route: route.clone(),
            fragment_cache: Arc::new(Mutex::new(HashMap::new())),
            layout_cache: Arc::new(Mutex::new(HashMap::new())),
        };
        Router::new()
            .route("/Manifest", get(manifest).options(cors_preflight))
            .fallback(get(fragment_fallback))
            .with_state(state)
    }
}

/// Axum state for the Smooth manifest + fragment routes.
#[derive(Clone)]
struct SmoothState {
    route: Arc<RouteHandle>,
    /// Built per-track fragments, keyed by `(segment_seq, track)`. A muxed
    /// segment is fetched (and re-demuxed) at most once per requested track,
    /// so a client re-fetching the same fragment does no CPU work (audit
    /// W13d). Bounded by the live window's segment count times the track
    /// count (small); it is cleared whenever it exceeds
    /// [`FRAGMENT_CACHE_MAX_ENTRIES`].
    fragment_cache: Arc<Mutex<HashMap<FragmentCacheKey, Arc<Vec<u8>>>>>,
    /// Rendered manifests, keyed by the window's signature (each track's chunk
    /// starts) so a manifest is only re-rendered when the window actually
    /// changes, not on every poll.
    layout_cache: Arc<Mutex<HashMap<LayoutSignature, Arc<String>>>>,
}

/// Upper bound on cached fragments before the cache is cleared — the window
/// is small, so this only guards against unbounded growth from a very large
/// `window_segments` or a long-lived process.
const FRAGMENT_CACHE_MAX_ENTRIES: usize = 4096;

/// Upper bound on cached rendered manifests (one per distinct window layout).
const LAYOUT_CACHE_MAX_ENTRIES: usize = 64;

/// `GET /Manifest` — renders the Smooth live client Manifest XML.
///
/// Building the manifest demuxes every window segment to derive each track's
/// own `c` timeline, so it runs on the **blocking pool**, not a runtime
/// worker.
async fn manifest(State(state): State<SmoothState>) -> Response {
    let serving = match http::resolve_route_program(&state.route) {
        Ok(serving) => serving,
        Err(resp) => return *resp,
    };
    let route = Arc::clone(&state.route);
    let cache = Arc::clone(&state.layout_cache);
    let built = tokio::task::spawn_blocking(move || {
        let layout = build_window_layout(&route, &serving)?;
        let signature = layout.signature();
        if let Some(hit) = crate::lock::lock(&cache).get(&signature).map(Arc::clone) {
            return Some((*hit).clone());
        }
        let body = render_manifest(&layout);
        let mut guard = crate::lock::lock(&cache);
        if guard.len() >= LAYOUT_CACHE_MAX_ENTRIES {
            guard.clear();
        }
        guard.insert(signature, Arc::new(body.clone()));
        Some(body)
    })
    .await;
    match built {
        Ok(Some(body)) => ([(header::CONTENT_TYPE, MANIFEST_CONTENT_TYPE)], body).into_response(),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "Smooth manifest: blocking task failed");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// Fallback route: catches Smooth fragment URLs which have the shape
/// `QualityLevels(BITRATE)/Fragments(TYPE=START_TIME)`. See the module docs
/// for why a fallback is needed (axum's `/:file` is single-segment only).
///
/// The muxed program segment is resolved through the (sans-IO, sync)
/// [`SmoothFragmentOrigin`], then re-packaged into the requested
/// StreamIndex's own track on the **blocking pool** (audit W13d): the demux
/// (`Fmp4Demux`) and re-mux (`SmoothPackager`) are CPU-bound and must not run
/// on a tokio worker. The result is cached by `(seq, track key)` so a client
/// re-fetching the same fragment (or the many fragments of one segment) does
/// not re-demux the whole programme each time.
async fn fragment_fallback(State(state): State<SmoothState>, uri: OriginalUri) -> Response {
    let full_path = uri.path();
    let request = match parse_smooth_fragment_request(full_path) {
        Some(r) => r,
        None => return StatusCode::NOT_FOUND.into_response(),
    };

    let serving = match http::resolve_route_program(&state.route) {
        Ok(serving) => serving,
        Err(resp) => return *resp,
    };

    // Map the requested (absolute) track-timeline start to a chunk, using the
    // same per-track timeline `render_manifest` renders — so the fragment's
    // `tfxd` is exactly the `c@t` the client saw.
    let route = Arc::clone(&state.route);
    let serving_for_layout = serving.clone();
    let request_for_layout = request;
    let located = tokio::task::spawn_blocking(move || {
        let layout = build_window_layout(&route, &serving_for_layout)?;
        // Select the track whose advertised `Bitrate` equals the requested
        // `QualityLevels(N)` value; ties broken by manifest order (the first
        // match, since `find` returns the first).
        let mut ordinal = 0u32;
        let mut chosen: Option<(&TrackChunks, u32)> = None;
        for (_, bitrate, p, chunks) in &layout.tracks {
            if p.stream_type != request_for_layout.stream_type.name() {
                continue;
            }
            ordinal = ordinal.saturating_add(1);
            if *bitrate == request_for_layout.quality_bitrate && chosen.is_none() {
                chosen = Some((chunks, ordinal));
            }
        }
        let (chunks, ordinal) = chosen?;
        let start_ticks = request_for_layout.start_time;
        let segment_seq = chunks.segment_for_start(start_ticks)?;
        Some((segment_seq, start_ticks, ordinal))
    })
    .await
    .ok()
    .flatten();
    let Some((segment_seq, start_ticks, ordinal)) = located else {
        return StatusCode::NOT_FOUND.into_response();
    };

    let trunk = serving.trunk();
    let origin = SmoothFragmentOrigin {
        serving: serving.clone(),
    };
    let resp = http::resolve_blocking(
        &trunk,
        &origin,
        SegmentRequest { segment_seq },
        BLOCKING_RELOAD_TIMEOUT,
        || (),
    )
    .await;
    let segment = match resp {
        media_plane::egress::EgressResponse::Ready { body, .. } => body,
        _ => return StatusCode::NOT_FOUND.into_response(),
    };

    // The remaining work (demux + re-mux) is CPU-bound: run it off the worker.
    let init = state.route.init_bytes(crate::route::SPTS_PROGRAM_ID);
    let cache = Arc::clone(&state.fragment_cache);
    let key = FragmentCacheKey::new(start_ticks, &request);
    let built = tokio::task::spawn_blocking(move || {
        if let Some(hit) = crate::lock::lock(&cache).get(&key) {
            return Some(Arc::clone(hit));
        }
        let fragment = build_track_fragment(
            init.as_deref(),
            &segment,
            &request,
            start_ticks,
            segment_seq,
            ordinal,
        )?;
        let fragment = Arc::new(fragment);
        let mut guard = crate::lock::lock(&cache);
        if guard.len() >= FRAGMENT_CACHE_MAX_ENTRIES {
            guard.clear();
        }
        guard.insert(key, Arc::clone(&fragment));
        Some(fragment)
    })
    .await;
    match built {
        Ok(Some(fragment)) => {
            ([(header::CONTENT_TYPE, "video/mp4")], (*fragment).clone()).into_response()
        }
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            tracing::error!(error = %e, "Smooth fragment: blocking task failed");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// One track's fragment timeline: per window segment, the chunk's absolute
/// start and duration in [`SMOOTH_TIMESCALE`] ticks, in the **track's own**
/// sample timeline (not the muxed segment's).
///
/// Derived by demuxing every window segment and rescaling each sample's
/// absolute decode time from the track's media timescale to 10 MHz with
/// **integer cumulative** arithmetic: `c@t` is `round(first_sample_ticks *
/// 10^7 / ts)` and `c@d` is the difference of consecutive chunk starts, so
/// rounding never accumulates (issue #1083, W13 follow-up). The start is the
/// track's own absolute decode time (its `tfdt`), so it is stable as the
/// window slides, and audio keeps its real first-sample offset rather than
/// being snapped to the video segment start.
#[derive(Debug, Clone)]
struct TrackChunks {
    /// Parallel to the window: `(segment_seq, start_ticks, duration_ticks)`.
    chunks: Vec<ChunkTiming>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ChunkTiming {
    segment_seq: u32,
    start_ticks: u64,
    duration_ticks: u64,
}

impl TrackChunks {
    /// The segment whose chunk starts at `start_ticks`, if any.
    fn segment_for_start(&self, start_ticks: u64) -> Option<u32> {
        self.chunks
            .iter()
            .find(|c| c.start_ticks == start_ticks)
            .map(|c| c.segment_seq)
    }
}

/// Rescale a media-timescale tick count to [`SMOOTH_TIMESCALE`] with
/// round-to-nearest, using `u128` so the multiply cannot overflow for any
/// realistic timeline.
fn to_smooth_ticks(ticks: u64, timescale: u32) -> u64 {
    let ts = u128::from(timescale.max(1));
    let num = u128::from(ticks) * u128::from(SMOOTH_TIMESCALE);
    // round-to-nearest
    let rounded = (num + ts / 2) / ts;
    u64::try_from(rounded).unwrap_or(u64::MAX)
}

/// Build one track's chunk timeline over `window` by demuxing each segment's
/// `init + bytes` and reading the matching track's absolute sample times.
///
/// `init` is the program's init segment (`None` before it is published).
/// `spec` identifies the track by its position among the `TYPE`-matching
/// tracks. Returns `None` if no window segment carries the track yet.
fn track_chunks(
    init: Option<&[u8]>,
    window: &[DashWindowSegment],
    segments: &std::collections::HashMap<u32, bytes::Bytes>,
    stream_type: SmoothStreamType,
    quality_index: u32,
) -> Option<TrackChunks> {
    let init = init?;
    let mut chunks: Vec<ChunkTiming> = Vec::with_capacity(window.len());
    for seg in window {
        let Some(bytes) = segments.get(&seg.segment_seq) else {
            continue;
        };
        let mut combined = Vec::with_capacity(init.len() + bytes.len());
        combined.extend_from_slice(init);
        combined.extend_from_slice(bytes);
        let Some(media) = Fmp4Demux::new().unpackage(&combined).ok() else {
            continue;
        };
        let Some(track) = select_track(&media, stream_type, quality_index) else {
            continue;
        };
        let ts = track.spec.timescale.max(1);
        let Some(first) = track.samples.first() else {
            continue;
        };
        let start_media_ticks = u64::try_from(first.dts.unwrap_or(0).max(0)).unwrap_or(0);
        // The first chunk's start is the track's own absolute rescaled
        // first-sample decode time — a property of that segment alone, so it
        // is stable as the window slides. Every later chunk's start is derived
        // cumulatively (`start(n) = start(n-1) + d(n-1)`) so the timeline never
        // drifts, and `tfxd(n)+d(n) == tfxd(n+1)` holds to the tick.
        let absolute_start = to_smooth_ticks(start_media_ticks, ts);
        let chain_start = chunks
            .last()
            .map(|c| c.start_ticks.saturating_add(c.duration_ticks))
            .unwrap_or(absolute_start);
        // The duration is exactly the sum of the fragment's own `trun` sample
        // durations: build the fragment with the same packager the fragment
        // handler uses and take its `duration`, so `Σtrun == c@d` holds by
        // construction whatever rounding the packager applies per sample.
        let duration_ticks = SmoothPackager {
            target_duration_secs: SINGLE_SEGMENT_FRAGMENT_TARGET_SECS,
        }
        .package_track_fragment(track, seg.segment_seq, chain_start)
        .map(|f| f.duration)
        .unwrap_or(0);
        chunks.push(ChunkTiming {
            segment_seq: seg.segment_seq,
            start_ticks: chain_start,
            duration_ticks,
        });
    }
    if chunks.is_empty() {
        return None;
    }
    Some(TrackChunks { chunks })
}

/// A resolved snapshot of one program's Smooth window: the segment bytes for
/// every window sequence number, and the per-track chunk timeline for each
/// advertised StreamIndex. Built once per (window signature) request and
/// cached, so the expensive demux happens once, not per fragment request.
struct WindowLayout {
    /// Per advertised track, in `specs` order (video/audio filtered and with
    /// an unsupported codec already dropped): `(quality_index, Bitrate,
    /// StreamIndex params, TrackChunks)`. `Bitrate` is the value advertised
    /// in `QualityLevel@Bitrate` and is what a fragment URL's
    /// `QualityLevels(N)` carries back.
    tracks: Vec<(u32, u64, SmoothCodecParams, TrackChunks)>,
}

/// Demux every window segment once and build the per-track chunk timeline for
/// each representable track. `None` until an init segment and at least one
/// segment are available.
fn build_window_layout(route: &RouteHandle, serving: &ProgramServing) -> Option<WindowLayout> {
    let specs = route.track_specs(crate::route::SPTS_PROGRAM_ID);
    if specs.is_empty() {
        return None;
    }
    let window = route.window_segments(crate::route::SPTS_PROGRAM_ID);
    if window.is_empty() {
        return None;
    }
    let init = route.init_bytes(crate::route::SPTS_PROGRAM_ID)?;
    let ll_hls = serving.ll_hls();
    // Resolve each window segment's muxed bytes once.
    let mut segments: std::collections::HashMap<u32, bytes::Bytes> =
        std::collections::HashMap::with_capacity(window.len());
    for seg in &window {
        let name = format!("seg-{DEFAULT_TRACK_ID}-{}.m4s", seg.segment_seq);
        if let EgressResponse::Ready {
            body: HlsBody::Resource(bytes),
            ..
        } = ll_hls.resolve(
            HlsRequest::Resource { name },
            Timestamp::from_nanos(0),
            AwaitPolicy::new(Timestamp::from_nanos(u64::MAX)),
        ) {
            segments.insert(seg.segment_seq, bytes);
        }
    }
    if segments.is_empty() {
        return None;
    }

    let mut tracks = Vec::new();
    let mut video_ordinal: u32 = 0;
    let mut audio_ordinal: u32 = 0;
    for spec in &specs {
        let Some(params) = smooth_codec_params(&spec.config) else {
            tracing::warn!(
                track_id = spec.track_id,
                "Smooth output: track's codec has no MS-SSTR representation; omitting its StreamIndex rather than advertising a wrong FourCC"
            );
            continue;
        };
        let quality_index = if params.stream_type == "audio" {
            audio_ordinal = audio_ordinal.saturating_add(1);
            audio_ordinal
        } else {
            video_ordinal = video_ordinal.saturating_add(1);
            video_ordinal
        };
        let stream_type = match params.stream_type {
            "audio" => SmoothStreamType::Audio,
            _ => SmoothStreamType::Video,
        };
        let Some(chunks) =
            track_chunks(Some(&init), &window, &segments, stream_type, quality_index)
        else {
            continue;
        };
        let bitrate = advertised_bitrate(&params);
        tracks.push((quality_index, bitrate, params, chunks));
    }
    if tracks.is_empty() {
        return None;
    }
    Some(WindowLayout { tracks })
}

/// The cache key for one re-packaged per-track Smooth fragment: the chunk's
/// **absolute** start (the value baked into the fragment's `tfxd`) plus the
/// requested StreamIndex identity. Keyed by the absolute start, not the
/// window-relative request time, so a cache hit can never serve a fragment
/// whose `tfxd` was computed for a different window position. The stream type
/// is keyed by its `name()` (a `&'static str`) since `transmux::SmoothStreamType`
/// is not `Hash`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct FragmentCacheKey {
    start_ticks: u64,
    stream_type: &'static str,
    quality_bitrate: u64,
    /// The requested start time from the URL — included so a stale request
    /// (a client asking for a time no longer in the window) cannot hit a
    /// fragment built for the same absolute start under a different request.
    requested_start: u64,
}

impl FragmentCacheKey {
    fn new(start_ticks: u64, request: &SmoothFragmentRequest) -> Self {
        Self {
            start_ticks,
            stream_type: request.stream_type.name(),
            quality_bitrate: request.quality_bitrate,
            requested_start: request.start_time,
        }
    }
}

/// A stable signature of a [`WindowLayout`]'s per-track timelines: if
/// unchanged, a re-render produces identical XML, so it can be served from
/// cache.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct LayoutSignature(Vec<(u32, u64, u64)>);

impl WindowLayout {
    /// The signature: each track's `(quality_index, first_start, last_end)`.
    fn signature(&self) -> LayoutSignature {
        LayoutSignature(
            self.tracks
                .iter()
                .map(|(q, _, _, c)| {
                    let first = c.chunks.first().map(|x| x.start_ticks).unwrap_or(0);
                    let last = c
                        .chunks
                        .last()
                        .map(|x| x.start_ticks.saturating_add(x.duration_ticks))
                        .unwrap_or(0);
                    (*q, first, last)
                })
                .collect(),
        )
    }
}

/// A request to [`SmoothFragmentOrigin`] for the muxed programme segment with
/// sequence number `segment_seq`.
#[derive(Debug, Clone, Copy)]
struct SegmentRequest {
    segment_seq: u32,
}

/// Build the per-track Smooth fragment for `segment` (the muxed programme
/// segment bytes) — demux, select the requested track, re-mux one fragment
/// whose `tfxd` equals the manifest `c@t` for `request.start_time`.
///
/// Pure and synchronous: the caller runs it on the blocking pool. Returns
/// `None` when the segment does not carry a track matching the request (an
/// audio request on a video-only route, an unpublished init, an unsupported
/// codec), which the handler maps to a 404.
#[allow(clippy::too_many_arguments)]
fn build_track_fragment(
    init: Option<&[u8]>,
    segment: &[u8],
    request: &SmoothFragmentRequest,
    start_ticks: u64,
    segment_seq: u32,
    ordinal: u32,
) -> Option<Vec<u8>> {
    let init = init?;
    let mut combined = Vec::with_capacity(init.len() + segment.len());
    combined.extend_from_slice(init);
    combined.extend_from_slice(segment);

    let media = Fmp4Demux::new().unpackage(&combined).ok()?;
    // The requested track: the `TYPE`-matching tracks are the manifest's
    // advertised ones, in the same order `render_manifest` walks `specs`, so
    // `ordinal` is the track's 1-based position among the `TYPE`-matching
    // tracks in manifest order (the URL's bitrate selected it above).
    let wanted = select_track(&media, request.stream_type, ordinal)?;
    // One segment is one fragment, so the packager must not re-cut it — set
    // the target duration above any single segment (audit W13a).
    let packager = SmoothPackager {
        target_duration_secs: SINGLE_SEGMENT_FRAGMENT_TARGET_SECS,
    };
    // The fragment's absolute time is the chunk's **absolute** track-timeline
    // start (`start_ticks`), which is exactly the manifest's `c@t` for this
    // chunk and does not change as the window slides. The chunk sequence
    // number is the segment's, so `mfhd.sequence_number` is monotonic rather
    // than a constant track ordinal.
    let fragment = packager
        .package_track_fragment(wanted, segment_seq, start_ticks)
        .ok()?;
    if fragment.data.is_empty() {
        return None;
    }
    Some(fragment.data)
}

/// Target fragment duration above which a live Smooth segment is never cut:
/// set far above any real segment so one `Trunk` segment maps to exactly one
/// Smooth fragment (audit W13a). The value is a whole number of seconds
/// because [`SmoothPackager::target_duration_secs`] is an integer.
const SINGLE_SEGMENT_FRAGMENT_TARGET_SECS: u32 = 3600;

/// Select the `quality_index`-th (1-based) track of `stream_type` from the
/// demuxed media, in document order — the same order `render_manifest`
/// advertises them.
fn select_track(
    media: &transmux::Media,
    stream_type: SmoothStreamType,
    quality_index: u32,
) -> Option<&transmux::Track> {
    let index = usize::try_from(quality_index.checked_sub(1)?).ok()?;
    media
        .tracks
        .iter()
        .filter(|t| track_stream_type(&t.spec.config) == Some(stream_type))
        .nth(index)
}

/// Internal ServedEgress for fragment resolution: fetches the muxed
/// programme segment for a sequence number from the shared HlsOrigin.
struct SmoothFragmentOrigin {
    serving: Arc<ProgramServing>,
}

/// One Smooth fragment request: which StreamIndex (`stream_type`), which
/// `QualityLevel` (`quality_index`, its 1-based position among the manifest's
/// advertised tracks) and which fragment start time (in
/// [`SMOOTH_TIMESCALE`] ticks), parsed from the URL shape
/// `QualityLevels(BITRATE)/Fragments(TYPE=START_TIME)` (MS-SSTR §2.2.3).
///
/// The `QualityLevels(N)` value is a stable per-track index (see
/// `render_manifest`) rather than a real bitrate, so two tracks of the same
/// `TYPE` are addressed distinctly — the pre-fix handler ignored it entirely
/// and could not tell two audio streams apart (audit W13d).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SmoothFragmentRequest {
    stream_type: SmoothStreamType,
    /// The `QualityLevels(N)` value from the fragment URL — the advertised
    /// `QualityLevel@Bitrate`, used to select the track (ties by manifest
    /// order).
    quality_bitrate: u64,
    start_time: u64,
}

impl ServedEgress for SmoothFragmentOrigin {
    type Request = SegmentRequest;
    type Body = Vec<u8>;

    fn resolve(
        &self,
        request: SegmentRequest,
        _now: Timestamp,
        _await_policy: AwaitPolicy,
    ) -> EgressResponse<Vec<u8>> {
        // The muxed programme segment for this sequence number, straight from
        // the shared HlsOrigin — the per-track re-packaging happens later, on
        // the blocking pool (see `fragment_fallback`).
        let ll_hls = self.serving.ll_hls();
        let filename = format!("seg-{DEFAULT_TRACK_ID}-{}.m4s", request.segment_seq);
        let now = Timestamp::from_nanos(0);
        let deadline = Timestamp::from_nanos(u64::MAX);
        match ll_hls.resolve(
            HlsRequest::Resource { name: filename },
            now,
            AwaitPolicy::new(deadline),
        ) {
            EgressResponse::Ready {
                body: HlsBody::Resource(bytes),
                ..
            } => EgressResponse::Ready {
                body: bytes.to_vec(),
                cache: CachePolicy::NoCache,
            },
            _ => EgressResponse::NotFound,
        }
    }
}

/// Parse a Smooth fragment request from a full request path like
/// `/cam/QualityLevels(1)/Fragments(video=0)` — the StreamIndex `TYPE`
/// (MS-SSTR §2.2.3: `video`/`audio`, matching [`SmoothStreamType::name`]),
/// the `QualityLevels` index, and the fragment start time in
/// [`SMOOTH_TIMESCALE`] ticks. `None` for an unrecognised `TYPE` token or a
/// missing/non-numeric start time or index.
fn parse_smooth_fragment_request(full_path: &str) -> Option<SmoothFragmentRequest> {
    let ql_pos = full_path.find("QualityLevels(")?;
    let after_ql = &full_path[ql_pos + "QualityLevels(".len()..];
    // The `QualityLevels(N)` index, up to its closing paren.
    let close_paren = after_ql.find(')')?;
    let quality_bitrate = after_ql[..close_paren].parse().ok()?;
    let after_close = &after_ql[close_paren + 1..];
    // Now we need "/Fragments(TYPE=START_TIME)"
    let frag_open = after_close.find("/Fragments(")?;
    let inside = &after_close[frag_open + "/Fragments(".len()..];
    let eq_pos = inside.find('=')?;
    let type_str = &inside[..eq_pos];
    let stream_type = match type_str {
        "video" => SmoothStreamType::Video,
        "audio" => SmoothStreamType::Audio,
        _ => return None,
    };
    let start_str = &inside[eq_pos + 1..];
    let start_str = start_str.trim_end_matches(')');
    Some(SmoothFragmentRequest {
        stream_type,
        quality_bitrate,
        start_time: start_str.parse().ok()?,
    })
}

/// The Smooth StreamIndex `TYPE` a track's codec maps to, or `None` for a
/// codec MS-SSTR / this crate's `transmux::smooth` packager cannot describe.
///
/// Only AVC video and AAC audio: `SmoothPackager`'s own `resolve_codec`
/// rejects everything else with `Error::UnsupportedCodec`, and "HEVC" is not
/// in this repo's MS-SSTR transcription (`transmux/docs/smooth/ms-sstr.md`
/// lists `H264`/`AACL`/`AACH` only). Advertising HEVC here made every HEVC
/// fragment request 404 while the manifest still listed it (audit W13c), so
/// HEVC is dropped — a track Smooth cannot actually serve is not advertised.
fn track_stream_type(config: &CodecConfig) -> Option<SmoothStreamType> {
    match config {
        CodecConfig::Avc { .. } => Some(SmoothStreamType::Video),
        CodecConfig::Aac { .. } => Some(SmoothStreamType::Audio),
        _ => None,
    }
}

/// Render the Smooth client Manifest XML from a resolved [`WindowLayout`].
///
/// Each `StreamIndex` gets its **own** `c` timeline (MS-SSTR §2.2.2.6): the
/// per-track chunk starts/durations from [`TrackChunks`], so an audio
/// `StreamIndex` reflects AAC's own frame timing rather than the video
/// segment's. Every `t` is written (absolute, track timeline) so a client
/// never has to accumulate `d`.
fn render_manifest(layout: &WindowLayout) -> String {
    // The presentation duration is the longest track's last chunk end.
    let total_duration: u64 = layout
        .tracks
        .iter()
        .filter_map(|(_, _, _, t)| t.chunks.last())
        .map(|c| c.start_ticks.saturating_add(c.duration_ticks))
        .max()
        .unwrap_or(0);

    let mut xml = String::new();
    xml.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    xml.push_str(&format!(
        "<SmoothStreamingMedia MajorVersion=\"{MAJOR_VERSION}\" MinorVersion=\"{MINOR_VERSION}\" Duration=\"{total_duration}\" TimeScale=\"{SMOOTH_TIMESCALE}\" IsLive=\"true\" LookAheadFragmentCount=\"0\" DVRWindowLength=\"{total_duration}\">\n",
    ));

    for (quality_index, bitrate, params, chunks) in &layout.tracks {
        let url = format!(
            "QualityLevels({{bitrate}})/Fragments({}={{start time}})",
            params.stream_type
        );
        // `Bitrate` is the real advertised value (see `advertised_bitrate`),
        // not the quality ordinal — a client uses it for ABR, and it is the
        // value the fragment URL's `QualityLevels(N)` carries back.
        xml.push_str("  <StreamIndex");
        xml.push_str(&format!(" Type=\"{}\"", params.stream_type));
        xml.push_str(" Subtype=\"\"");
        xml.push_str(&format!(" Chunks=\"{}\"", chunks.chunks.len()));
        xml.push_str(" QualityLevels=\"1\"");
        xml.push_str(&format!(" Url=\"{url}\">\n"));

        xml.push_str("    <QualityLevel");
        xml.push_str(&format!(" Index=\"0\" Bitrate=\"{bitrate}\""));
        xml.push_str(&format!(" FourCC=\"{}\"", params.fourcc));
        if let Some(w) = params.max_width {
            xml.push_str(&format!(" MaxWidth=\"{w}\""));
        }
        if let Some(h) = params.max_height {
            xml.push_str(&format!(" MaxHeight=\"{h}\""));
        }
        if let Some(sr) = params.sampling_rate {
            xml.push_str(&format!(" SamplingRate=\"{sr}\""));
        }
        if let Some(ch) = params.channels {
            xml.push_str(&format!(" Channels=\"{ch}\""));
        }
        if params.stream_type == "audio" {
            xml.push_str(" BitsPerSample=\"16\" AudioTag=\"255\"");
        }
        xml.push_str(&format!(
            " CodecPrivateData=\"{}\"/>\n",
            params.codec_private_data
        ));

        // One `c` per chunk: every `t` explicit (absolute, track timeline),
        // `d` the chunk's own duration, `n` the ordinal.
        for (i, c) in chunks.chunks.iter().enumerate() {
            xml.push_str(&format!(
                "    <c n=\"{i}\" t=\"{}\" d=\"{}\"/>\n",
                c.start_ticks, c.duration_ticks
            ));
        }
        // `quality_index` is used only for the fragment URL selection; the
        // ordinal is not advertised as a bitrate.
        let _ = quality_index;

        xml.push_str("  </StreamIndex>\n");
    }

    xml.push_str("</SmoothStreamingMedia>\n");
    xml
}

/// A real `QualityLevel@Bitrate` for `params`, in bits per second, from the
/// codec's nominal rate. Smooth's `Bitrate` is advisory ABR metadata; a
/// documented per-codec estimate is used because the live window carries no
/// per-track byte count at manifest-render time.
fn advertised_bitrate(params: &SmoothCodecParams) -> u64 {
    match params.stream_type {
        "audio" => {
            // AAC-LC nominal: sampling_rate * channels * 16-bit, a standard
            // upper bound; at least 64 kbps.
            let sr = u64::from(params.sampling_rate.unwrap_or(48_000));
            let ch = u64::from(params.channels.unwrap_or(2));
            sr.saturating_mul(ch)
                .saturating_mul(16)
                .clamp(64_000, params_max_audio_bitrate())
        }
        _ => {
            // Video: a resolution-scaled estimate, clamped to a sane band.
            let w = u64::from(params.max_width.unwrap_or(1280));
            let h = u64::from(params.max_height.unwrap_or(720));
            w.saturating_mul(h)
                .saturating_mul(4)
                .clamp(500_000, 20_000_000)
        }
    }
}

/// Upper clamp for the audio bitrate estimate (320 kbps stereo AAC).
fn params_max_audio_bitrate() -> u64 {
    320_000
}

/// Codec parameters resolved for the Smooth manifest.
struct SmoothCodecParams {
    stream_type: &'static str,
    fourcc: &'static str,
    max_width: Option<u32>,
    max_height: Option<u32>,
    sampling_rate: Option<u32>,
    channels: Option<u16>,
    /// `CodecPrivateData` — the hex-encoded decoder initialisation data a
    /// Smooth client needs before it can decode anything (MS-SSTR §2.2.2.5).
    ///
    /// For H.264 this is the parameter sets in Annex-B form (each NAL
    /// preceded by the 4-byte start code `00 00 00 01`); for AAC it is the
    /// `AudioSpecificConfig`. Empty only when the codec is one this packager
    /// cannot describe — a client seeing an empty value cannot initialise a
    /// decoder, which is why this used to be a hard defect (issue #934).
    codec_private_data: String,
}

/// Concatenate parameter-set NALs in Annex-B form and hex-encode them.
fn annex_b_hex(nals: &[&[u8]]) -> String {
    const START_CODE: [u8; 4] = [0x00, 0x00, 0x00, 0x01];
    let mut out = Vec::new();
    for nal in nals {
        out.extend_from_slice(&START_CODE);
        out.extend_from_slice(nal);
    }
    broadcast_common::hex::hex_encode(&out).to_uppercase()
}

/// Resolve Smooth codec parameters from a [`CodecConfig`], or `None` for a
/// codec MS-SSTR cannot describe (anything other than H.264 video or AAC
/// audio — see `transmux::smooth`'s `resolve_codec`, the authoritative
/// per-track analogue of this function). Returning `None` lets the caller
/// omit the track rather than advertise it under a fabricated `H264` FourCC
/// (audit run 7, W13).
fn smooth_codec_params(config: &CodecConfig) -> Option<SmoothCodecParams> {
    match config {
        CodecConfig::Avc {
            config,
            width,
            height,
        } => {
            let rec = &config.config;
            let nals: Vec<&[u8]> = rec
                .sps
                .iter()
                .map(|s| s.0.as_slice())
                .chain(rec.pps.iter().map(|p| p.0.as_slice()))
                .collect();
            Some(SmoothCodecParams {
                stream_type: "video",
                fourcc: FOURCC_H264,
                max_width: Some(u32::from(*width)),
                max_height: Some(u32::from(*height)),
                sampling_rate: None,
                channels: None,
                codec_private_data: annex_b_hex(&nals),
            })
        }
        CodecConfig::Hevc {
            config: _,
            width: _,
            height: _,
        } => None,
        CodecConfig::Aac {
            esds,
            sample_rate,
            channel_count,
            ..
        } => Some(SmoothCodecParams {
            stream_type: "audio",
            fourcc: FOURCC_AACL,
            max_width: None,
            max_height: None,
            sampling_rate: Some(*sample_rate),
            channels: Some(*channel_count),
            // AAC's CodecPrivateData is the AudioSpecificConfig verbatim —
            // no Annex-B framing. It lives in the esds' DecoderSpecificInfo
            // (ISO/IEC 14496-1 §7.2.6.7).
            codec_private_data: esds
                .es_descriptor
                .decoder_config
                .as_ref()
                .and_then(|dc| dc.decoder_specific_info.as_ref())
                .map(|dsi| broadcast_common::hex::hex_encode(&dsi.data).to_uppercase())
                .unwrap_or_default(),
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use transmux::avc_config::{AVCConfigurationBox, AVCDecoderConfigurationRecord};
    use transmux::nalu_types::{AvcPps, AvcSps};

    /// `CodecPrivateData` must carry the H.264 parameter sets in Annex-B form,
    /// hex-encoded. It shipped empty (issue #934), which leaves a Smooth client
    /// unable to initialise a decoder — the manifest looked structurally right
    /// and was unusable.
    #[test]
    fn avc_codec_private_data_carries_annex_b_parameter_sets() {
        let sps = vec![0x67, 0x42, 0xC0, 0x1E];
        let pps = vec![0x68, 0xCE, 0x3C, 0x80];
        let cfg = CodecConfig::Avc {
            config: AVCConfigurationBox::new(AVCDecoderConfigurationRecord {
                configuration_version: 1,
                profile_indication: 0x42,
                profile_compatibility: 0xC0,
                level_indication: 0x1E,
                length_size_minus_one: 3,
                sps: vec![AvcSps(sps.clone())],
                pps: vec![AvcPps(pps.clone())],
                chroma_format: None,
                bit_depth_luma_minus8: None,
                bit_depth_chroma_minus8: None,
                sps_ext: Vec::new(),
            }),
            width: 1920,
            height: 1080,
        };

        let params = smooth_codec_params(&cfg).expect("AVC is a Smooth-representable codec");

        assert!(
            !params.codec_private_data.is_empty(),
            "CodecPrivateData must not be empty — a client cannot decode without it"
        );
        // Each parameter set is preceded by the 4-byte Annex-B start code.
        assert_eq!(
            params.codec_private_data,
            "0000000167 42C01E0000000168CE3C80".replace(' ', ""),
            "expected start-code-prefixed SPS then PPS, hex-encoded"
        );
    }

    /// Audit run 7, W13: a codec MS-SSTR cannot describe resolves to `None`
    /// (so the manifest omits its StreamIndex) rather than being advertised
    /// under a fabricated `H264` FourCC with empty `CodecPrivateData`.
    #[test]
    fn unsupported_codec_has_no_smooth_params() {
        let cfg = CodecConfig::Subtitle {
            format: transmux::ir::SubtitleFormat::WebVtt,
        };
        assert!(
            smooth_codec_params(&cfg).is_none(),
            "a WebVTT subtitle track is not a Smooth codec and must not be advertised"
        );
        assert_eq!(
            track_stream_type(&cfg),
            None,
            "only AVC/HEVC video and AAC audio map to a Smooth StreamIndex TYPE"
        );
    }

    /// The supported codecs map to their StreamIndex `TYPE`.
    #[test]
    fn supported_codec_stream_types() {
        assert_eq!(
            track_stream_type(&CodecConfig::Subtitle {
                format: transmux::ir::SubtitleFormat::WebVtt,
            }),
            None
        );
        // AVC → video; the mapping is what the manifest's `Url` template and
        // the fragment handler's per-track selection both key on.
        assert_eq!(track_stream_type(&avc_cfg()), Some(SmoothStreamType::Video));
    }

    /// Audit run 7, W13c: HEVC is **not** a Smooth codec this crate serves
    /// (`transmux::smooth`'s `resolve_codec` rejects everything but AVC/AAC,
    /// and MS-SSTR's transcription lists only `H264`/`AACL`/`AACH`). It must
    /// resolve to `None` on both sides — otherwise the manifest advertised a
    /// "HEVC" StreamIndex whose every fragment request 404s.
    #[test]
    fn hevc_is_not_a_smooth_codec() {
        use broadcast_common::Parse;
        let record = transmux::HEVCDecoderConfigurationRecord::parse(&hevc_config_body())
            .expect("valid minimal hvcc");
        let cfg = CodecConfig::Hevc {
            config: transmux::HEVCConfigurationBox::new(record),
            width: 1280,
            height: 720,
        };
        assert!(
            smooth_codec_params(&cfg).is_none(),
            "HEVC has no MS-SSTR representation here; it must not be advertised"
        );
        assert_eq!(
            track_stream_type(&cfg),
            None,
            "HEVC must not map to a Smooth StreamIndex TYPE"
        );
    }

    /// A minimal, valid `HEVCDecoderConfigurationRecord` body (ISO/IEC
    /// 14496-15 §8.3.3).
    fn hevc_config_body() -> Vec<u8> {
        let mut body = vec![0x01, 0x01];
        body.extend_from_slice(&[0u8; 4]);
        body.extend_from_slice(&[0u8; 6]);
        body.push(93);
        body.extend_from_slice(&[0xF0, 0x00]);
        body.push(0xFC);
        body.push(0xFD);
        body.push(0xF8);
        body.push(0xF8);
        body.extend_from_slice(&[0u8; 2]);
        body.push(0x03);
        body.push(0x01);
        body.push(0x80 | 32);
        body.extend_from_slice(&[0x00, 0x01]);
        let vps = [0x40u8, 0x01, 0x0C, 0x01, 0xFF];
        body.extend_from_slice(&(vps.len() as u16).to_be_bytes());
        body.extend_from_slice(&vps);
        body
    }

    /// The AVC config the `CodecPrivateData` test above builds.
    fn avc_cfg() -> CodecConfig {
        let sps = vec![0x67, 0x42, 0xC0, 0x1E];
        let pps = vec![0x68, 0xCE, 0x3C, 0x80];
        CodecConfig::Avc {
            config: AVCConfigurationBox::new(AVCDecoderConfigurationRecord {
                configuration_version: 1,
                profile_indication: 0x42,
                profile_compatibility: 0xC0,
                level_indication: 0x1E,
                length_size_minus_one: 3,
                sps: vec![AvcSps(sps)],
                pps: vec![AvcPps(pps)],
                chroma_format: None,
                bit_depth_luma_minus8: None,
                bit_depth_chroma_minus8: None,
                sps_ext: Vec::new(),
            }),
            width: 1920,
            height: 1080,
        }
    }
}
