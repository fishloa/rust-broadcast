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

use std::sync::Arc;

use axum::Router;
use axum::extract::{OriginalUri, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use broadcast_common::Timestamp;
use hls_runtime::server::{DEFAULT_TRACK_ID, HlsBody, HlsRequest};
use media_plane::egress::{AwaitPolicy, CachePolicy, EgressResponse, ServedEgress};
use transmux::CodecConfig;
use transmux::smooth::SMOOTH_TIMESCALE;

use crate::http::{self, BLOCKING_RELOAD_TIMEOUT};
use crate::origin::resource::cors_preflight;
use crate::output::{Output, OutputKind};
use crate::route::{ProgramServing, RouteHandle};

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
}

/// `GET /Manifest` — renders the Smooth live client Manifest XML.
async fn manifest(State(state): State<SmoothState>) -> Response {
    let serving = match http::resolve_route_program(&state.route) {
        Ok(serving) => serving,
        Err(resp) => return *resp,
    };
    let trunk = serving.trunk();
    let origin = SmoothManifestOrigin {
        route: state.route.clone(),
    };
    let resp = http::resolve_blocking(&trunk, &origin, (), BLOCKING_RELOAD_TIMEOUT, || ()).await;
    http::into_response(resp, StatusCode::SERVICE_UNAVAILABLE, |body| {
        ([(header::CONTENT_TYPE, MANIFEST_CONTENT_TYPE)], body).into_response()
    })
}

/// The Smooth manifest [`ServedEgress`]: renders the XML from the route's
/// current track specs and window — never answers [`EgressResponse::Await`].
struct SmoothManifestOrigin {
    route: Arc<RouteHandle>,
}

impl ServedEgress for SmoothManifestOrigin {
    type Request = ();
    type Body = String;

    fn resolve(
        &self,
        _request: (),
        _now: Timestamp,
        _await_policy: AwaitPolicy,
    ) -> EgressResponse<String> {
        match render_manifest(&self.route) {
            Some(body) => EgressResponse::Ready {
                body,
                cache: CachePolicy::NoCache,
            },
            None => EgressResponse::NotFound,
        }
    }
}

/// Fallback route: catches Smooth fragment URLs which have the shape
/// `QualityLevels(BITRATE)/Fragments(TYPE=START_TIME)`. See the module docs
/// for why a fallback is needed (axum's `/:file` is single-segment only).
async fn fragment_fallback(State(state): State<SmoothState>, uri: OriginalUri) -> Response {
    let full_path = uri.path();
    let start_time = match parse_smooth_fragment_start_time(full_path) {
        Some(t) => t,
        None => return StatusCode::NOT_FOUND.into_response(),
    };

    let serving = match http::resolve_route_program(&state.route) {
        Ok(serving) => serving,
        Err(resp) => return *resp,
    };
    let trunk = serving.trunk();
    let origin = SmoothFragmentOrigin {
        route: state.route.clone(),
        serving,
    };
    let resp =
        http::resolve_blocking(&trunk, &origin, start_time, BLOCKING_RELOAD_TIMEOUT, || ()).await;
    http::into_response(resp, StatusCode::NOT_FOUND, |body| {
        ([(header::CONTENT_TYPE, "video/mp4")], body).into_response()
    })
}

/// Internal ServedEgress for fragment resolution.
struct SmoothFragmentOrigin {
    route: Arc<RouteHandle>,
    serving: Arc<ProgramServing>,
}

impl ServedEgress for SmoothFragmentOrigin {
    type Request = u64;
    type Body = Vec<u8>;

    fn resolve(
        &self,
        start_time: u64,
        _now: Timestamp,
        _await_policy: AwaitPolicy,
    ) -> EgressResponse<Vec<u8>> {
        let window = self.route.window_segments(crate::route::SPTS_PROGRAM_ID);
        if window.is_empty() {
            return EgressResponse::NotFound;
        }

        let mut cumulative_smooth = 0u64;
        let mut target_seq: Option<u32> = None;
        for seg in &window {
            let dur_smooth = (seg.duration_secs * SMOOTH_TIMESCALE as f64).round() as u64;
            if cumulative_smooth == start_time {
                target_seq = Some(seg.segment_seq);
                break;
            }
            cumulative_smooth += dur_smooth;
        }
        let segment_seq = match target_seq {
            Some(seq) => seq,
            None => return EgressResponse::NotFound,
        };

        let ll_hls = self.serving.ll_hls();
        let filename = format!("seg-{DEFAULT_TRACK_ID}-{segment_seq}.m4s");
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

/// Parse a Smooth fragment start time from a full request path like
/// `/cam/QualityLevels(1000000)/Fragments(video=0)`.
fn parse_smooth_fragment_start_time(full_path: &str) -> Option<u64> {
    let ql_pos = full_path.find("QualityLevels(")?;
    let after_ql = &full_path[ql_pos + "QualityLevels(".len()..];
    // Skip past the bitrate digits and closing paren
    let close_paren = after_ql.find(')')?;
    let after_close = &after_ql[close_paren + 1..];
    // Now we need "/Fragments(TYPE=START_TIME)"
    let frag_open = after_close.find("/Fragments(")?;
    let inside = &after_close[frag_open + "/Fragments(".len()..];
    let eq_pos = inside.find('=')?;
    let start_str = &inside[eq_pos + 1..];
    let start_str = start_str.trim_end_matches(')');
    start_str.parse().ok()
}

/// Render the Smooth client Manifest XML from the route's current track
/// specs and closed-segment window. `None` if no tracks are recorded yet.
fn render_manifest(route: &RouteHandle) -> Option<String> {
    let specs = route.track_specs(crate::route::SPTS_PROGRAM_ID);
    if specs.is_empty() {
        return None;
    }
    let window = route.window_segments(crate::route::SPTS_PROGRAM_ID);

    let total_duration: u64 = window
        .iter()
        .map(|s| (s.duration_secs * SMOOTH_TIMESCALE as f64).round() as u64)
        .sum();

    let mut xml = String::new();
    xml.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    xml.push_str(&format!(
        "<SmoothStreamingMedia MajorVersion=\"{MAJOR_VERSION}\" MinorVersion=\"{MINOR_VERSION}\" Duration=\"{total_duration}\" TimeScale=\"{SMOOTH_TIMESCALE}\" IsLive=\"true\" LookAheadFragmentCount=\"0\" DVRWindowLength=\"{total_duration}\">\n",
    ));

    for spec in &specs {
        let params = smooth_codec_params(&spec.config);

        let url = format!(
            "QualityLevels({{bitrate}})/Fragments({}={{start time}})",
            params.stream_type
        );

        let chunk_count = window.len();

        xml.push_str("  <StreamIndex");
        xml.push_str(&format!(" Type=\"{}\"", params.stream_type));
        xml.push_str(" Subtype=\"\"");
        xml.push_str(&format!(" Chunks=\"{chunk_count}\""));
        xml.push_str(" QualityLevels=\"1\"");
        xml.push_str(&format!(" Url=\"{url}\">\n"));

        xml.push_str("    <QualityLevel");
        xml.push_str(" Index=\"0\" Bitrate=\"0\"");
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
        xml.push_str(" CodecPrivateData=\"\"/>\n");

        let mut cumulative_smooth = 0u64;
        for (i, seg) in window.iter().enumerate() {
            let dur_smooth = (seg.duration_secs * SMOOTH_TIMESCALE as f64).round() as u64;
            xml.push_str("    <c");
            if i == 0 {
                xml.push_str(&format!(" t=\"{cumulative_smooth}\""));
            }
            xml.push_str(&format!(" d=\"{dur_smooth}\" n=\"{i}\"/>\n"));
            cumulative_smooth += dur_smooth;
        }

        xml.push_str("  </StreamIndex>\n");
    }

    xml.push_str("</SmoothStreamingMedia>\n");
    Some(xml)
}

/// Codec parameters resolved for the Smooth manifest.
struct SmoothCodecParams {
    stream_type: &'static str,
    fourcc: &'static str,
    max_width: Option<u32>,
    max_height: Option<u32>,
    sampling_rate: Option<u32>,
    channels: Option<u16>,
}

/// Resolve Smooth codec parameters from a [`CodecConfig`].
fn smooth_codec_params(config: &CodecConfig) -> SmoothCodecParams {
    match config {
        CodecConfig::Avc { width, height, .. } => SmoothCodecParams {
            stream_type: "video",
            fourcc: FOURCC_H264,
            max_width: Some(u32::from(*width)),
            max_height: Some(u32::from(*height)),
            sampling_rate: None,
            channels: None,
        },
        CodecConfig::Hevc { width, height, .. } => SmoothCodecParams {
            stream_type: "video",
            fourcc: "HEVC",
            max_width: Some(u32::from(*width)),
            max_height: Some(u32::from(*height)),
            sampling_rate: None,
            channels: None,
        },
        CodecConfig::Aac {
            sample_rate,
            channel_count,
            ..
        } => SmoothCodecParams {
            stream_type: "audio",
            fourcc: FOURCC_AACL,
            max_width: None,
            max_height: None,
            sampling_rate: Some(*sample_rate),
            channels: Some(*channel_count),
        },
        _ => SmoothCodecParams {
            stream_type: "video",
            fourcc: FOURCC_H264,
            max_width: None,
            max_height: None,
            sampling_rate: None,
            channels: None,
        },
    }
}
