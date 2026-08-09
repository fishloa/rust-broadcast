//! `CatchupOutput`: the HTTP surface for catch-up / time-shift /
//! VOD-from-live serving over the DVR durable archive (issue #900) — a
//! thin axum adapter over `crate::catchup`'s pure archive-scan/merge/
//! render logic, mounted and gated exactly like every other
//! [`crate::output::Output`] (so it shares the same output auth — Basic/
//! Digest/Bearer/Forwarded — every other output gets from
//! `crate::origin::router`'s `output_auth_gate`, rather than inventing a
//! second HTTP surface with its own auth path).
//!
//! Requires the route's DVR archive to be enabled
//! (`crate::config::Route::dvr.enabled`) — `crate::config::Route::validate_standalone`
//! rejects a `"catchup"` output configured without it, so a mounted
//! `CatchupOutput` in a validated config always has an archive to read.
//! The handlers below still check defensively (`RouteHandle::dvr_config`
//! returning `None`), in case that invariant is ever bypassed (e.g. the
//! runtime admin API adding a route from an unvalidated source).
//!
//! # Three routes, one archive
//!
//! - `GET /catchup.m3u8[?window_secs=N]` — a live-continuing playlist
//!   spanning the archive plus whatever the live `Trunk` has closed since
//!   the archive was last polled (the straddle boundary — see
//!   `crate::catchup`'s own module doc). `window_secs` bounds it to the
//!   trailing N seconds (the "catch-up window"); omitted, the whole
//!   archive plus live tail (RFC 8216 §4.4.3.5 `EVENT` semantics — segments
//!   only ever get appended, never removed, since nothing here evicts
//!   archived history the way a live rolling window does).
//! - `GET /vod/p{N}.m3u8` — exactly one archived period's segments,
//!   `#EXT-X-ENDLIST` and `PLAYLIST-TYPE:VOD` once that period is
//!   definitively finished (a later period exists on disk); otherwise the
//!   same still-growing shape as `catchup.m3u8`, restricted to that one
//!   period. This is "VOD-from-live": a finished recorded programme served
//!   as a complete, immutable asset.
//! - `GET /catchup/seg-{seq}.{ext}` — the resource route both playlists
//!   above reference: archived bytes read straight off disk when `seq` is
//!   in the archive, or (the still-unarchived tail) the same
//!   `hls_runtime::server::HlsOrigin` every other output resolves against,
//!   when it is not. One endpoint serves both sources — the client never
//!   needs to know which one held a given segment.
//!
//! Mounted under `/catchup*`/`/vod/*` (two-segment paths), never at the
//! same single-segment `/:file` shape `crate::origin::resource`'s shared
//! catch-all owns — axum's router cannot have two routes claim the exact
//! same wildcard segment, so this module deliberately nests one level
//! deeper instead of teaching the shared resource route a new filename
//! grammar.

use std::path::PathBuf;
use std::sync::Arc;

use axum::Router;
use axum::extract::{Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use broadcast_common::Timestamp;
use broadcast_hls::PlaylistType;
use hls_runtime::server::{Container, DEFAULT_TRACK_ID, HlsBody, HlsRequest};
use media_plane::egress::{AwaitPolicy, EgressResponse, ServedEgress};
use serde::Deserialize;

use crate::catchup;
use crate::http;
use crate::origin::resource::cors_preflight;
use crate::output::{Output, OutputKind};
use crate::route::RouteHandle;

const MEDIA_PLAYLIST_CONTENT_TYPE: &str = "application/vnd.apple.mpegurl";
const TS_SEGMENT_CONTENT_TYPE: &str = "video/mp2t";
const FMP4_SEGMENT_CONTENT_TYPE: &str = "video/mp4";

/// The catch-up/VOD-from-live [`Output`] — see the module docs.
#[derive(Debug, Default)]
pub struct CatchupOutput;

impl Output for CatchupOutput {
    fn kind(&self) -> OutputKind {
        OutputKind::Catchup
    }

    /// Routes (relative — mounted by the origin under `/{stream}/`):
    /// - `GET /catchup.m3u8`
    /// - `GET /vod/:period` (`period` is `p{N}.m3u8`)
    /// - `GET /catchup/:file` (`file` is `seg-{seq}.{ext}`)
    fn manifest_routes(&self, route: Arc<RouteHandle>) -> Router {
        Router::new()
            .route(
                "/catchup.m3u8",
                get(catchup_playlist).options(cors_preflight),
            )
            .route("/vod/:period", get(vod_playlist).options(cors_preflight))
            .route(
                "/catchup/:file",
                get(catchup_resource).options(cors_preflight),
            )
            .with_state(route)
    }
}

/// The dynamic-filename extension (without the leading `.`) for `route`'s
/// configured container — mirrors `crate::route::ProgramServing::new`'s own
/// mapping (`Container` is `#[non_exhaustive]`, hence the catch-all).
fn container_ext(container: Container) -> &'static str {
    match container {
        Container::MpegTs => "ts",
        Container::Fmp4 => "m4s",
        _ => "m4s",
    }
}

fn segment_content_type(ext: &str) -> &'static str {
    if ext == "ts" {
        TS_SEGMENT_CONTENT_TYPE
    } else {
        FMP4_SEGMENT_CONTENT_TYPE
    }
}

/// `#EXT-X-MAP` URI for `container`'s live init resource (served by the
/// shared resource route, `crate::origin::resource`), or `None` for
/// [`Container::MpegTs`] (no init resource exists — see
/// `hls_runtime::server::Container`'s own doc). Relative to this output's
/// own playlist paths, both of which are top-level under `/{stream}/`,
/// same as `init-{track}.mp4` itself.
fn map_uri(container: Container) -> Option<String> {
    matches!(container, Container::Fmp4).then(|| format!("init-{DEFAULT_TRACK_ID}.mp4"))
}

#[derive(Debug, Default, Deserialize)]
pub struct CatchupPlaylistQuery {
    /// Bound the catch-up window to this many trailing seconds of
    /// `crate::dvr::IndexEntry::start_pts_ns` — see
    /// `crate::catchup::apply_window`. Omitted or `0`: the whole archive
    /// plus the live tail.
    window_secs: Option<u64>,
}

/// `GET /catchup.m3u8` — see the module docs.
async fn catchup_playlist(
    State(route): State<Arc<RouteHandle>>,
    Query(q): Query<CatchupPlaylistQuery>,
) -> Response {
    let serving = match http::resolve_route_program(&route) {
        Ok(serving) => serving,
        Err(resp) => return *resp,
    };
    let Some(dvr) = route.dvr_config() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let ext = container_ext(route.container());
    let dir = catchup::archive_dir(dvr, route.name());
    let archived = catchup::scan_archive(&dir);
    let live = serving.ll_hls().closed_segments();
    let combined = catchup::merge_segments(&archived, &live);
    let windowed = catchup::apply_window(&combined, q.window_secs);
    let body = catchup::render_playlist(
        &windowed,
        ext,
        map_uri(route.container()).as_deref(),
        PlaylistType::Event,
        false,
    );
    ([(header::CONTENT_TYPE, MEDIA_PLAYLIST_CONTENT_TYPE)], body).into_response()
}

/// `GET /vod/p{N}.m3u8` — see the module docs.
async fn vod_playlist(
    State(route): State<Arc<RouteHandle>>,
    Path(period_file): Path<String>,
) -> Response {
    let Some(dvr) = route.dvr_config() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Some(period_num) = parse_period_filename(&period_file) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let ext = container_ext(route.container());
    let dir = catchup::archive_dir(dvr, route.name());
    let segments = catchup::read_period_segments(&dir, period_num);
    if segments.is_empty() {
        return StatusCode::NOT_FOUND.into_response();
    }
    // Definitively finished iff a later period exists on disk —
    // `crate::dvr::DvrRecorder::start_period` never opens period N+1 until
    // period N is closed, so this is exact, not a guess.
    let finished = catchup::list_period_nums(&dir)
        .iter()
        .any(|&n| n > period_num);
    let combined: Vec<catchup::CatchupSegment> = segments
        .iter()
        .map(|s| catchup::CatchupSegment {
            seq: s.seq,
            start_pts_ns: s.start_pts_ns,
            duration_secs: s.duration_secs,
            discontinuous: s.discontinuous,
        })
        .collect();
    let playlist_type = if finished {
        PlaylistType::Vod
    } else {
        PlaylistType::Event
    };
    let body = catchup::render_playlist(
        &combined,
        ext,
        map_uri(route.container()).as_deref(),
        playlist_type,
        finished,
    );
    ([(header::CONTENT_TYPE, MEDIA_PLAYLIST_CONTENT_TYPE)], body).into_response()
}

fn parse_period_filename(file: &str) -> Option<u32> {
    file.strip_prefix('p')?.strip_suffix(".m3u8")?.parse().ok()
}

/// `GET /catchup/seg-{seq}.{ext}` — archived bytes read straight from disk
/// when `seq` is archived; otherwise the live `Trunk`'s still-unarchived
/// tail, resolved through the exact same `HlsOrigin` the shared resource
/// route (`crate::origin::resource`) uses for `seg-{track}-{seq}.{ext}`.
/// This dispatch — not a second cache of anything — is what makes the
/// straddle boundary invisible to the client: one filename grammar, either
/// source.
async fn catchup_resource(
    State(route): State<Arc<RouteHandle>>,
    Path(file): Path<String>,
) -> Response {
    let ext = container_ext(route.container());
    let Some(seq) = parse_seg_filename(&file, ext) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Some(dvr) = route.dvr_config() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let dir: PathBuf = catchup::archive_dir(dvr, route.name());
    if let Some(seg) = catchup::find_archived_segment(&dir, seq) {
        return match catchup::read_archived_bytes(
            &dir,
            ext,
            seg.period_num,
            seg.byte_offset,
            seg.byte_len,
        ) {
            Ok(bytes) => {
                ([(header::CONTENT_TYPE, segment_content_type(ext))], bytes).into_response()
            }
            Err(e) => {
                tracing::error!(error = %e, seq, "catch-up: failed reading archived segment bytes");
                StatusCode::INTERNAL_SERVER_ERROR.into_response()
            }
        };
    }

    // Not archived (yet) — the still-live tail. A one-shot, non-blocking
    // resolve (deadline == now): the client only ever requests a segment a
    // playlist just told it exists, so there is nothing to wait for here —
    // unlike the live playlist's own blocking-reload semantics.
    let serving = match http::resolve_route_program(&route) {
        Ok(serving) => serving,
        Err(resp) => return *resp,
    };
    let ll_hls = serving.ll_hls();
    let now = Timestamp::from_nanos(0);
    let policy = AwaitPolicy::new(now);
    let request = HlsRequest::Resource {
        name: format!("seg-{DEFAULT_TRACK_ID}-{seq}.{ext}"),
    };
    match ll_hls.resolve(request, now, policy) {
        EgressResponse::Ready {
            body: HlsBody::Resource(bytes),
            ..
        } => ([(header::CONTENT_TYPE, segment_content_type(ext))], bytes).into_response(),
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

fn parse_seg_filename(file: &str, ext: &str) -> Option<u32> {
    let suffix = format!(".{ext}");
    file.strip_prefix("seg-")?
        .strip_suffix(&suffix)?
        .parse()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dvr::{ArchiveOverrunSerde, DvrConfig};
    use crate::route::SPTS_PROGRAM_ID;

    fn temp_dir() -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "multimux-catchup-output-{}-{}",
            std::process::id(),
            n
        ));
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    fn cleanup(dir: &std::path::Path) {
        let _ = std::fs::remove_dir_all(dir);
    }

    fn dvr_config(tmp: &std::path::Path) -> DvrConfig {
        DvrConfig {
            enabled: true,
            archive_root: tmp.to_string_lossy().to_string(),
            retention_periods: 10,
            retention_bytes: 0,
            period_duration_secs: 3600,
            overrun: ArchiveOverrunSerde::Gap,
            dvb_service_id: None,
        }
    }

    fn seg_bytes(seq: u32, byte: u8) -> transmux::ll_hls::SegmentInfo {
        transmux::ll_hls::SegmentInfo {
            bytes: vec![byte; 24],
            duration: 3.0,
            segment_seq: seq,
            part_count: 1,
        }
    }

    async fn body_string(resp: Response) -> String {
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    async fn body_bytes(resp: Response) -> Vec<u8> {
        axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec()
    }

    /// The end-to-end straddle bite test (issue #900's whole point): three
    /// segments are archived (drained via `drain_dvr`), a fourth is
    /// published to the live `Trunk` only — never drained to disk. The
    /// combined catch-up playlist must show all four, continuously, and
    /// BOTH the archived segments and the live-only segment must be
    /// fetchable byte-exact through the ONE `catchup/seg-*` endpoint.
    ///
    /// This would fail under two disjoint playlists (the exact failure
    /// mode #900 exists to prevent): a naive "archive playlist" alone
    /// would omit segment 4 entirely, and a naive "live playlist" alone
    /// would use `MEDIA-SEQUENCE` starting wherever the live window
    /// happens to begin, not the true first archived segment.
    #[tokio::test]
    async fn catchup_playlist_straddles_archive_and_live_tail_continuously() {
        let tmp = temp_dir();
        let route = Arc::new(
            RouteHandle::new(4.0, 500, 8)
                .with_name("straddle")
                .with_dvr(dvr_config(&tmp)),
        );
        route.publish_new_program(SPTS_PROGRAM_ID);
        route.set_init(SPTS_PROGRAM_ID, vec![0xAA; 4]);

        // Segments 1..=3: published, then drained to the archive.
        for (seq, byte) in [(1u32, 0x11u8), (2, 0x22), (3, 0x33)] {
            route.add_segment(SPTS_PROGRAM_ID, seg_bytes(seq, byte));
        }
        route.drain_dvr();

        // Segment 4: published, but NEVER drained -- lives only in the
        // live Trunk/HlsOrigin window.
        route.add_segment(SPTS_PROGRAM_ID, seg_bytes(4, 0x44));

        let resp =
            catchup_playlist(State(route.clone()), Query(CatchupPlaylistQuery::default())).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;
        assert!(body.contains("#EXT-X-MEDIA-SEQUENCE:1"), "body: {body}");
        for seq in 1..=4 {
            assert!(
                body.contains(&format!("catchup/seg-{seq}.m4s")),
                "seq {seq} missing from combined playlist: {body}"
            );
        }
        // No duplicate entries: exactly 4 EXTINF lines.
        assert_eq!(
            body.matches("#EXTINF").count(),
            4,
            "must show each segment exactly once: {body}"
        );

        // Archived segment: served straight from disk.
        let archived_resp =
            catchup_resource(State(route.clone()), Path("seg-2.m4s".to_string())).await;
        assert_eq!(archived_resp.status(), StatusCode::OK);
        assert_eq!(body_bytes(archived_resp).await, vec![0x22u8; 24]);

        // Live-only segment: served through the SAME endpoint, resolved
        // via the live HlsOrigin.
        let live_resp = catchup_resource(State(route), Path("seg-4.m4s".to_string())).await;
        assert_eq!(live_resp.status(), StatusCode::OK);
        assert_eq!(body_bytes(live_resp).await, vec![0x44u8; 24]);

        cleanup(&tmp);
    }

    #[tokio::test]
    async fn catchup_playlist_without_dvr_is_404() {
        let route = Arc::new(RouteHandle::new(4.0, 500, 8).with_name("no-dvr"));
        route.publish_new_program(SPTS_PROGRAM_ID);
        route.set_init(SPTS_PROGRAM_ID, vec![0xAA; 4]);
        let resp = catchup_playlist(State(route), Query(CatchupPlaylistQuery::default())).await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn window_secs_bounds_the_playlist() {
        let tmp = temp_dir();
        let route = Arc::new(
            RouteHandle::new(4.0, 500, 8)
                .with_name("windowed")
                .with_dvr(dvr_config(&tmp)),
        );
        route.publish_new_program(SPTS_PROGRAM_ID);
        route.set_init(SPTS_PROGRAM_ID, vec![0xAA; 4]);
        for (seq, byte) in [(1u32, 0x11u8), (2, 0x22), (3, 0x33)] {
            route.add_segment(SPTS_PROGRAM_ID, seg_bytes(seq, byte));
        }
        route.drain_dvr();

        // Segments start at 0s, 3s, 6s (each 3s long); the live edge is
        // segment 3's start (6s). A 2s window reaches back to 4s, which
        // excludes segment 2 (starts at 3s) and keeps only segment 3.
        let resp = catchup_playlist(
            State(route),
            Query(CatchupPlaylistQuery {
                window_secs: Some(2),
            }),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;
        assert!(body.contains("catchup/seg-3.m4s"), "body: {body}");
        assert!(!body.contains("catchup/seg-2.m4s"), "body: {body}");
        assert!(!body.contains("catchup/seg-1.m4s"), "body: {body}");
        assert_eq!(body.matches("#EXTINF").count(), 1, "body: {body}");
    }

    #[tokio::test]
    async fn vod_playlist_finished_period_has_endlist() {
        let tmp = temp_dir();
        let route = Arc::new(
            RouteHandle::new(4.0, 500, 8)
                .with_name("vod")
                .with_dvr(dvr_config(&tmp)),
        );
        route.publish_new_program(SPTS_PROGRAM_ID);
        route.set_init(SPTS_PROGRAM_ID, vec![0xAA; 4]);

        route.add_segment(SPTS_PROGRAM_ID, seg_bytes(1, 0x11));
        route.drain_dvr(); // opens + writes period 0 with init A

        // Changing the init rolls the period (crate::dvr::DvrRecorder's
        // mid-stream init-change rollover) — a real, reliable way to force
        // period 1 to open without waiting out `period_duration_secs`.
        route.set_init(SPTS_PROGRAM_ID, vec![0xBB; 4]);
        route.add_segment(SPTS_PROGRAM_ID, seg_bytes(2, 0x22));
        route.drain_dvr();

        let resp = vod_playlist(State(route.clone()), Path("p0.m3u8".to_string())).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;
        assert!(
            body.contains("#EXT-X-ENDLIST"),
            "period 0 has a successor (period 1) so it must be finished: {body}"
        );
        assert!(body.contains("#EXT-X-PLAYLIST-TYPE:VOD"), "body: {body}");

        cleanup(&tmp);
    }

    #[tokio::test]
    async fn vod_playlist_unknown_period_is_404() {
        let tmp = temp_dir();
        let route = Arc::new(
            RouteHandle::new(4.0, 500, 8)
                .with_name("vod-missing")
                .with_dvr(dvr_config(&tmp)),
        );
        route.publish_new_program(SPTS_PROGRAM_ID);
        let resp = vod_playlist(State(route), Path("p99.m3u8".to_string())).await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        cleanup(&tmp);
    }

    #[tokio::test]
    async fn catchup_resource_unmatched_filename_404() {
        let route = Arc::new(RouteHandle::new(4.0, 500, 8).with_name("bad-name"));
        let resp = catchup_resource(State(route), Path("not-a-segment.txt".to_string())).await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }
}
