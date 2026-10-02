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

/// The `#EXT-X-MAP` URI of an [`catchup::InitRef`], relative to the catch-up
/// playlists (which sit at `/{stream}/`): an archived period's init is served
/// by this module's own resource route (`catchup/init-p{N}.mp4`, the bytes at
/// the head of that period's file — the init of the run that wrote it), a
/// segment still in the live origin names the origin's versioned init
/// resource (`HlsOrigin::init_name`).
fn init_uri_of(ll_hls: Option<&hls_runtime::server::HlsOrigin>, init: catchup::InitRef) -> String {
    match init {
        catchup::InitRef::Archived(period) => format!("catchup/init-p{period}.mp4"),
        catchup::InitRef::Live(generation) => ll_hls.map_or_else(
            || format!("init-{DEFAULT_TRACK_ID}.mp4"),
            |o| o.init_name(DEFAULT_TRACK_ID, generation),
        ),
    }
}

/// `catchup/init-p{N}.mp4` -> `N`.
fn parse_init_filename(file: &str) -> Option<u32> {
    file.strip_prefix("init-p")?
        .strip_suffix(".mp4")?
        .parse()
        .ok()
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
    // Audit run 7, W3: `scan_archive` reads and JSON-parses every `pN.idx`
    // on this route's archive — real, synchronous filesystem I/O. Run it on
    // the blocking pool so a catch-up request can never pin a runtime worker
    // (which would stall ingest for every route), and answer 500 if the pool
    // is gone (shutdown) rather than panicking.
    let scan_dir = dir.clone();
    // Bound concurrent archive scans (issue #1083, F): the blocking pool is
    // shared with the DVR persist, so an unauthenticated catch-up flood must
    // not be able to occupy every blocking thread and delay a route's
    // recording (whose `StallIngest` pin would then force-expire at 30 s).
    let _permit = scan_permit().acquire().await;
    let archived = match tokio::task::spawn_blocking(move || catchup::scan_archive(&scan_dir)).await
    {
        Ok(a) => a,
        Err(e) => {
            tracing::error!(error = %e, "catch-up: archive scan task failed");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    // The live tail in the numbers the origin's playlist and resource names
    // use (its Media Sequence Number = offset + the `Trunk`'s number), the
    // same numbers the archive indexes under (audit r07-C5, #1083): without
    // the offset a reconnect's restarted `Trunk` numbers collide with the
    // archive's, and the live tail is filtered out of the merge.
    let ll_hls = serving.ll_hls();
    let offset = ll_hls.media_sequence_offset();
    let live: Vec<_> = ll_hls
        .closed_segments()
        .into_iter()
        .filter_map(|s| {
            let public = u32::try_from(offset.checked_add(u64::from(s.sequence_number))?).ok()?;
            Some(hls_runtime::server::ClosedSegment::new(
                public,
                s.start_ns,
                s.duration_secs,
                s.discontinuous,
            ))
        })
        .collect();
    let combined = catchup::merge_segments(&archived, &live);
    let windowed = catchup::apply_window(&combined, q.window_secs);
    // Audit run 7, W11: this playlist is `EVENT`-shaped only when nothing
    // can remove its leading segments — a `window_secs` bound
    // (`apply_window`) or DVR retention (which evicts whole leading
    // periods) both do, and RFC 8216 §6.2.2 forbids PLAYLIST-TYPE on a
    // playlist that may remove segments ("no value of that tag allows
    // Media Segments to be removed"). With neither active the head only
    // ever grows, so `EVENT` is accurate and worth advertising (it lets a
    // player stop reloading once `#EXT-X-ENDLIST` cannot appear).
    let removes_leading = q.window_secs.filter(|&w| w > 0).is_some() || dvr.retention_active();
    let playlist_type = if removes_leading {
        None
    } else {
        Some(PlaylistType::Event)
    };
    let init_of = |init| init_uri_of(Some(&ll_hls), init);
    let Ok(body) = catchup::render_playlist(
        &windowed,
        ext,
        matches!(route.container(), Container::Fmp4).then_some(&init_of as &dyn Fn(_) -> _),
        playlist_type,
        false,
    ) else {
        // A field the renderer would have to quote carries a character
        // forbidden there (audit BH-W7, issue #1111) — unreachable for
        // this route's own generated URIs, but never silently mangled.
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
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
    // Audit run 7, W3: the index read/parse and the directory listing are
    // synchronous filesystem I/O — off the runtime worker, and bounded by the
    // same scan semaphore as the archive-wide scan (item 4).
    let _permit = scan_permit().acquire().await;
    let read_dir = dir.clone();
    let (segments, period_nums) = match tokio::task::spawn_blocking(move || {
        (
            catchup::read_period_segments(&read_dir, period_num),
            catchup::list_period_nums(&read_dir),
        )
    })
    .await
    {
        Ok(v) => v,
        Err(e) => {
            tracing::error!(error = %e, "catch-up: period read task failed");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    if segments.is_empty() {
        return StatusCode::NOT_FOUND.into_response();
    }
    // Definitively finished iff a later period exists on disk —
    // `crate::dvr::DvrRecorder::start_period` never opens period N+1 until
    // period N is closed, so this is exact, not a guess.
    let finished = period_nums.iter().any(|&n| n > period_num);
    let combined: Vec<catchup::CatchupSegment> = segments
        .iter()
        .map(|s| catchup::CatchupSegment {
            seq: s.seq,
            start_pts_ns: s.start_pts_ns,
            duration_secs: s.duration_secs,
            discontinuous: s.discontinuous,
            init: Some(catchup::InitRef::Archived(s.period_num)),
        })
        .collect();
    // One period's segments only ever grow within the period (retention
    // evicts whole periods, which a request for a still-listed period
    // cannot observe), so the live branch is a genuine EVENT playlist and
    // the tag is accurate — see `catchup::render_playlist`'s own doc.
    let playlist_type = if finished {
        PlaylistType::Vod
    } else {
        PlaylistType::Event
    };
    let init_of = |init| init_uri_of(None, init);
    let Ok(body) = catchup::render_playlist(
        &combined,
        ext,
        matches!(route.container(), Container::Fmp4).then_some(&init_of as &dyn Fn(_) -> _),
        Some(playlist_type),
        finished,
    ) else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    ([(header::CONTENT_TYPE, MEDIA_PLAYLIST_CONTENT_TYPE)], body).into_response()
}

/// Bounds how many archive *scans* run on the blocking pool at once — held
/// by all three catch-up paths (`catchup_playlist`'s `scan_archive`,
/// `vod_playlist`'s period read, and `catchup_resource`'s
/// `find_archived_segment`), each of which reads every `pN.idx` until it
/// finds what it wants. Sized well below the blocking pool's own ceiling so
/// ordinary DVR persists always find a thread.
fn scan_permit() -> &'static tokio::sync::Semaphore {
    static SEM: std::sync::OnceLock<tokio::sync::Semaphore> = std::sync::OnceLock::new();
    /// Conservative ceiling: the DVR persist must always be able to run.
    const MAX_CONCURRENT_SCANS: usize = 4;
    SEM.get_or_init(|| tokio::sync::Semaphore::new(MAX_CONCURRENT_SCANS))
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
    if let Some(period) = parse_init_filename(&file) {
        return catchup_init(&route, ext, period).await;
    }
    let Some(seq) = parse_seg_filename(&file, ext) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Some(dvr) = route.dvr_config() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let dir: PathBuf = catchup::archive_dir(dvr, route.name());
    // Audit run 7, W3: locating the segment reads every period index until a
    // match, then `read_archived_bytes` reads the whole segment — both
    // synchronous disk I/O, so both run on the blocking pool, bounded by the
    // same scan semaphore (item 4).
    let _permit = scan_permit().acquire().await;
    let find_dir = dir.clone();
    let found =
        match tokio::task::spawn_blocking(move || catchup::find_archived_segment(&find_dir, seq))
            .await
        {
            Ok(v) => v,
            Err(e) => {
                tracing::error!(error = %e, seq, "catch-up: archive lookup task failed");
                return StatusCode::INTERNAL_SERVER_ERROR.into_response();
            }
        };
    if let Some(seg) = found {
        let read_dir = dir.clone();
        let read = tokio::task::spawn_blocking(move || {
            catchup::read_archived_bytes(
                &read_dir,
                ext,
                seg.period_num,
                seg.byte_offset,
                seg.byte_len,
            )
        })
        .await;
        return match read {
            Ok(Ok(bytes)) => {
                ([(header::CONTENT_TYPE, segment_content_type(ext))], bytes).into_response()
            }
            Ok(Err(catchup::ReadArchivedError::Gone)) => {
                // The period was evicted between the index read and the
                // open (a race with retention) — not found, not an error
                // (issue #1083, item 3).
                StatusCode::NOT_FOUND.into_response()
            }
            Ok(Err(e)) => {
                tracing::error!(error = %e, seq, "catch-up: failed reading archived segment bytes");
                StatusCode::INTERNAL_SERVER_ERROR.into_response()
            }
            Err(e) => {
                tracing::error!(error = %e, seq, "catch-up: segment read task failed");
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

/// `GET /catchup/init-p{N}.mp4`: the init segment of the run that wrote
/// archive period `N` — the bytes at the head of `pN.m4s`, up to the first
/// indexed segment. `404` for a TS route, an unknown/evicted period or one
/// with no segment yet.
async fn catchup_init(route: &RouteHandle, ext: &'static str, period: u32) -> Response {
    let Some(dvr) = route.dvr_config() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if ext != "m4s" {
        return StatusCode::NOT_FOUND.into_response();
    }
    let dir: PathBuf = catchup::archive_dir(dvr, route.name());
    let _permit = scan_permit().acquire().await;
    let read = tokio::task::spawn_blocking(move || {
        let init_len = catchup::read_period_segments(&dir, period)
            .first()
            .map(|s| s.byte_offset)?;
        Some(catchup::read_archived_bytes(&dir, ext, period, 0, init_len))
    })
    .await;
    match read {
        Ok(Some(Ok(bytes))) if !bytes.is_empty() => {
            ([(header::CONTENT_TYPE, segment_content_type(ext))], bytes).into_response()
        }
        Ok(None | Some(Ok(_)) | Some(Err(catchup::ReadArchivedError::Gone))) => {
            StatusCode::NOT_FOUND.into_response()
        }
        Ok(Some(Err(e))) => {
            tracing::error!(error = %e, period, "catch-up: failed reading an archived init");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, period, "catch-up: init read task failed");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
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

    /// Take every permit from the shared scan semaphore, returning them so
    /// the caller can release.
    async fn scan_permit_wait_all() -> Vec<tokio::sync::SemaphorePermit<'static>> {
        let mut held = Vec::new();
        while let Ok(p) = scan_permit().try_acquire() {
            held.push(p);
        }
        held
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

    // --- W3 (audit run 7): DVR persist through the async advance_route path ---

    /// `advance_route`'s DVR drain persists a published segment byte-exact to
    /// the period file, and the period's `pN.idx` describes exactly that byte
    /// range, through the async path (which dispatches the synchronous
    /// `write_all`/`fs::rename` to the blocking pool).
    ///
    /// This asserts the *bytes*, not the dispatch: whether the persist runs
    /// inline or on `spawn_blocking` is a structural property of
    /// `poll_dvr_blocking` (it is called only from within `spawn_blocking`),
    /// not something a timing assertion here could observe reliably. The
    /// init-prelude length (652) is asserted literally because it is the
    /// real, deterministic length of this synthetic track's init segment —
    /// a genuine expected value, not a magic number.
    #[test]
    fn dvr_persist_through_advance_route_writes_bytes_exact() {
        use crate::source::DriverProgress;
        use crate::source::ts_program::{TsIngestSession, test_support::build_ts_bytes};
        use media_plane::DEFAULT_MAX_PROGRAMS;
        use media_plane::ingress::{HandshakePolicy, IngestDriver};
        use std::num::NonZeroUsize;

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime");
        rt.block_on(async {
            let tmp = temp_dir();
            let route = Arc::new(
                RouteHandle::new(1.0, 250, 64)
                    .with_name("w3")
                    .with_dvr(dvr_config(&tmp)),
            );
            route.publish_new_program(SPTS_PROGRAM_ID);
            route.set_init(SPTS_PROGRAM_ID, vec![0xAA; 4]);
            const SEG_LEN: usize = 4096;
            route
                .add_segment(
                    SPTS_PROGRAM_ID,
                    transmux::ll_hls::SegmentInfo {
                        bytes: vec![0x5A; SEG_LEN],
                        duration: 3.0,
                        segment_seq: 1,
                        part_count: 1,
                    },
                )
                .expect("add_segment");

            let nz = |n: usize| NonZeroUsize::new(n).unwrap();
            let mut driver = IngestDriver::new(
                TsIngestSession::new(),
                media_plane::trunk::TrunkConfig::new(nz(8), nz(8), nz(64), nz(8), nz(8)),
                HandshakePolicy::establish_by(broadcast_common::Timestamp::from_nanos(u64::MAX)),
                DEFAULT_MAX_PROGRAMS,
            );
            let mut progress = DriverProgress::new();
            driver.feed(
                &build_ts_bytes(1, 0xAB, 90),
                broadcast_common::Timestamp::ZERO,
            );

            crate::source::advance_route(&driver, &route, &mut progress).await;

            // The drain persisted the segment: the init prelude is 652 bytes
            // (a real fMP4 init for the synthetic track), then SEG_LEN bytes
            // of 0x5A.
            let period = tmp.join("w3").join("p0.m4s");
            let bytes = std::fs::read(&period).expect("period file must exist");
            assert_eq!(
                bytes.len(),
                652 + SEG_LEN,
                "period file must hold init + the one segment"
            );
            assert!(
                bytes[652..].iter().all(|&b| b == 0x5A),
                "the segment's bytes must be persisted byte-exact"
            );

            // The index describes exactly that range.
            let idx = std::fs::read_to_string(tmp.join("w3").join("p0.idx")).expect("index");
            let entries: Vec<serde_json::Value> = serde_json::from_str(&idx).expect("index JSON");
            assert_eq!(entries.len(), 1, "one segment must be indexed");
            assert_eq!(entries[0]["seq"], serde_json::json!(1));
            assert_eq!(entries[0]["byte_offset"], serde_json::json!(652));
            assert_eq!(entries[0]["byte_len"], serde_json::json!(SEG_LEN as u64));

            cleanup(&tmp);
        });
    }

    // --- item 4: every catch-up scan path is bounded by the scan semaphore ---

    /// With every permit held, each of the three catch-up scan paths must
    /// WAIT (not proceed) until one is released — proving all three acquire
    /// the same shared semaphore rather than only `catchup_playlist`.
    ///
    /// Biting test: remove the `scan_permit().acquire()` from `vod_playlist`
    /// / `catchup_resource` and those paths ignore the exhausted pool (the
    /// bounded wait returns a response instead of timing out).
    #[tokio::test]
    async fn all_three_catchup_scan_paths_share_the_scan_semaphore() {
        let tmp = temp_dir();
        let route = Arc::new(
            RouteHandle::new(1.0, 250, 8)
                .with_name("semp")
                .with_dvr(dvr_config(&tmp)),
        );
        route.publish_new_program(SPTS_PROGRAM_ID);
        route.set_init(SPTS_PROGRAM_ID, vec![0xAA; 4]);
        for (seq, byte) in [(1u32, 0x11u8), (2, 0x22)] {
            route
                .add_segment(SPTS_PROGRAM_ID, seg_bytes(seq, byte))
                .expect("add_segment");
        }
        route.drain_dvr().await;

        // Exhaust the pool.
        let held = scan_permit_wait_all().await;
        assert_eq!(held.len(), 4, "the scan pool has 4 permits");

        // Each path must not complete while no permit is free. Each is
        // driven in its own owned task so the borrow of `route` is explicit.
        macro_rules! must_wait {
            ($label:expr, $fut:expr) => {
                assert!(
                    tokio::time::timeout(std::time::Duration::from_millis(100), $fut)
                        .await
                        .is_err(),
                    "{} must wait for a free permit",
                    $label
                );
            };
        }
        must_wait!(
            "catchup_playlist",
            catchup_playlist(State(Arc::clone(&route)), Query(Default::default()))
        );
        must_wait!(
            "vod_playlist",
            vod_playlist(State(Arc::clone(&route)), Path("p0.m3u8".into()))
        );
        must_wait!(
            "catchup_resource",
            catchup_resource(State(Arc::clone(&route)), Path("seg-1.m4s".into()))
        );

        // Release — a path can now proceed and return a response.
        drop(held);
        let resp = catchup_playlist(State(Arc::clone(&route)), Query(Default::default())).await;
        assert!(
            resp.status().is_success() || resp.status().is_server_error(),
            "with a permit free the playlist path must complete, got {}",
            resp.status()
        );
        cleanup(&tmp);
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
            route
                .add_segment(SPTS_PROGRAM_ID, seg_bytes(seq, byte))
                .expect("add_segment");
        }
        route.drain_dvr().await;

        // Segment 4: published, but NEVER drained -- lives only in the
        // live Trunk/HlsOrigin window.
        route
            .add_segment(SPTS_PROGRAM_ID, seg_bytes(4, 0x44))
            .expect("add_segment");

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
            route
                .add_segment(SPTS_PROGRAM_ID, seg_bytes(seq, byte))
                .expect("add_segment");
        }
        route.drain_dvr().await;

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
        // Audit run 7, W11: a windowed playlist removes leading segments,
        // so it must NOT claim EXT-X-PLAYLIST-TYPE (RFC 8216 §6.2.2).
        assert!(
            !body.contains("#EXT-X-PLAYLIST-TYPE"),
            "a windowed playlist must omit PLAYLIST-TYPE: {body}"
        );
    }

    /// Audit W11: with neither `window_secs` nor retention removing
    /// segments, the live `/catchup.m3u8` is a genuine `EVENT` playlist and
    /// claims it. (A *validated* route always has retention, so this shape
    /// only arises from a directly-built `DvrConfig` — the branch must still
    /// be correct, not dead.)
    #[tokio::test]
    async fn catchup_playlist_claims_event_without_window_or_retention() {
        let tmp = temp_dir();
        let dvr = DvrConfig {
            enabled: true,
            archive_root: tmp.to_string_lossy().to_string(),
            retention_periods: 0,
            retention_bytes: 0,
            ..DvrConfig::default()
        };
        let route = Arc::new(
            RouteHandle::new(4.0, 500, 8)
                .with_name("event")
                .with_dvr(dvr),
        );
        route.publish_new_program(SPTS_PROGRAM_ID);
        route.set_init(SPTS_PROGRAM_ID, vec![0xAA; 4]);
        route
            .add_segment(SPTS_PROGRAM_ID, seg_bytes(1, 0x11))
            .expect("add_segment");
        route.drain_dvr().await;

        let resp = catchup_playlist(State(route), Query(CatchupPlaylistQuery::default())).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;
        assert!(
            body.contains("#EXT-X-PLAYLIST-TYPE:EVENT"),
            "a live playlist that never removes segments claims EVENT: {body}"
        );
        cleanup(&tmp);
    }

    #[tokio::test]
    async fn catchup_playlist_omits_playlist_type_when_retention_is_active() {
        let tmp = temp_dir();
        // `dvr_config` enables count-based retention (retention_periods:
        // 10), so leading periods (and therefore segments) can be evicted.
        let route = Arc::new(
            RouteHandle::new(4.0, 500, 8)
                .with_name("retained")
                .with_dvr(dvr_config(&tmp)),
        );
        route.publish_new_program(SPTS_PROGRAM_ID);
        route.set_init(SPTS_PROGRAM_ID, vec![0xAA; 4]);
        route
            .add_segment(SPTS_PROGRAM_ID, seg_bytes(1, 0x11))
            .expect("add_segment");
        route.drain_dvr().await;

        let resp = catchup_playlist(State(route), Query(CatchupPlaylistQuery::default())).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;
        assert!(
            !body.contains("#EXT-X-PLAYLIST-TYPE"),
            "retention-evicting archive must omit PLAYLIST-TYPE: {body}"
        );
        cleanup(&tmp);
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

        route
            .add_segment(SPTS_PROGRAM_ID, seg_bytes(1, 0x11))
            .expect("add_segment");
        route.drain_dvr().await; // opens + writes period 0 with init A

        // Changing the init rolls the period (crate::dvr::DvrRecorder's
        // mid-stream init-change rollover) — a real, reliable way to force
        // period 1 to open without waiting out `period_duration_secs`.
        route.set_init(SPTS_PROGRAM_ID, vec![0xBB; 4]);
        route
            .add_segment(SPTS_PROGRAM_ID, seg_bytes(2, 0x22))
            .expect("add_segment");
        route.drain_dvr().await;

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

    /// Audit r07-C5 (#1083), end to end: after a source reconnect the archive
    /// and the live tail share one numbering. The reconnected `Trunk` numbers
    /// its segments from 1 again; the playlist, the archive index and the
    /// by-number fetch must all use the origin's continued numbers, so the
    /// catch-up playlist lists every segment exactly once and each number
    /// serves its own run's bytes (the old code filtered the new run out of
    /// the merge and served the old run's bytes for a reused number).
    #[tokio::test]
    async fn reconnect_keeps_archive_and_live_tail_in_one_numbering() {
        let tmp = temp_dir();
        let route = Arc::new(
            RouteHandle::new(1.0, 250, 8)
                .with_name("c5")
                .with_dvr(dvr_config(&tmp)),
        );
        let first = route.publish_new_program(SPTS_PROGRAM_ID);
        route.set_init(SPTS_PROGRAM_ID, vec![0xAA; 4]);
        for (seq, byte) in [(1u32, 0x11u8), (2, 0x12)] {
            route
                .add_segment(SPTS_PROGRAM_ID, seg_bytes(seq, byte))
                .expect("add_segment");
        }
        route.drain_dvr().await;

        // The source reconnects: a fresh Trunk, numbering from 1 again.
        route.release_program(SPTS_PROGRAM_ID, &first);
        route.publish_new_program(SPTS_PROGRAM_ID);
        route.set_init(SPTS_PROGRAM_ID, vec![0xBB; 4]);
        for (seq, byte) in [(1u32, 0x21u8), (2, 0x22)] {
            route
                .add_segment(SPTS_PROGRAM_ID, seg_bytes(seq, byte))
                .expect("add_segment");
        }
        route.drain_dvr().await;

        // Both runs are really in the archive (not just served from the live
        // window): two periods, four indexed segments.
        let archived = catchup::scan_archive(&tmp.join("c5"));
        assert_eq!(
            archived
                .iter()
                .map(|s| (s.period_num, s.seq))
                .collect::<Vec<_>>(),
            vec![(0, 1), (0, 2), (1, 4), (1, 5)]
        );

        let playlist = body_string(
            catchup_playlist(State(route.clone()), Query(CatchupPlaylistQuery::default())).await,
        )
        .await;
        let listed: Vec<&str> = playlist
            .lines()
            .filter(|l| l.starts_with("catchup/seg-"))
            .collect();
        assert_eq!(
            listed,
            vec![
                "catchup/seg-1.m4s",
                "catchup/seg-2.m4s",
                "catchup/seg-4.m4s",
                "catchup/seg-5.m4s"
            ],
            "{playlist}"
        );

        // Each run decodes with ITS OWN init: the playlist carries one
        // `EXT-X-MAP` per run, after the `EXT-X-DISCONTINUITY` that opens the
        // second one, and each map serves that run's init bytes (the head of
        // the period file the run wrote).
        let lines: Vec<&str> = playlist.lines().collect();
        let at = |needle: &str| {
            lines
                .iter()
                .position(|l| *l == needle)
                .unwrap_or_else(|| panic!("{needle} missing from {playlist}"))
        };
        let (map0, map1) = (
            at("#EXT-X-MAP:URI=\"catchup/init-p0.mp4\""),
            at("#EXT-X-MAP:URI=\"catchup/init-p1.mp4\""),
        );
        assert!(map0 < at("catchup/seg-1.m4s"), "{playlist}");
        assert!(
            at("catchup/seg-2.m4s") < at("#EXT-X-DISCONTINUITY")
                && at("#EXT-X-DISCONTINUITY") < map1
                && map1 < at("catchup/seg-4.m4s"),
            "{playlist}"
        );
        for (period, init) in [(0u32, 0xAAu8), (1, 0xBB)] {
            let resp =
                catchup_resource(State(route.clone()), Path(format!("init-p{period}.mp4"))).await;
            assert_eq!(resp.status(), StatusCode::OK, "init-p{period}");
            assert_eq!(body_bytes(resp).await, vec![init; 4], "init-p{period}");
        }
        let missing = catchup_resource(State(route.clone()), Path("init-p9.mp4".into())).await;
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);

        // Number 3 is the old run's open segment: skipped, never reissued.
        for (n, expected) in [(1u32, 0x11u8), (2, 0x12), (4, 0x21), (5, 0x22)] {
            let resp = catchup_resource(State(route.clone()), Path(format!("seg-{n}.m4s"))).await;
            assert_eq!(resp.status(), StatusCode::OK, "seg-{n}");
            let bytes = body_bytes(resp).await;
            assert!(
                bytes.iter().all(|&b| b == expected),
                "seg-{n} must serve its own run's bytes ({expected:#x}), got {:#x?}",
                &bytes[..4.min(bytes.len())]
            );
        }
        cleanup(&tmp);
    }

    /// Audit r07-C5 (#1083): the live tail — segments the reconnected run has
    /// produced but the recorder has NOT archived yet — must be offset-mapped
    /// onto the playlist's numbers too. Without the mapping the tail keeps the
    /// restarted `Trunk` numbers (1, 2), which the merge filters out as "not
    /// above the archive" (it holds 1..=2), so the tail vanishes.
    #[tokio::test]
    async fn the_unarchived_live_tail_is_offset_mapped_after_a_reconnect() {
        let tmp = temp_dir();
        let route = Arc::new(
            RouteHandle::new(1.0, 250, 8)
                .with_name("c5-live")
                .with_dvr(dvr_config(&tmp)),
        );
        let first = route.publish_new_program(SPTS_PROGRAM_ID);
        route.set_init(SPTS_PROGRAM_ID, vec![0xAA; 4]);
        for (seq, byte) in [(1u32, 0x11u8), (2, 0x12)] {
            route
                .add_segment(SPTS_PROGRAM_ID, seg_bytes(seq, byte))
                .expect("add_segment");
        }
        route.drain_dvr().await;

        // Reconnect; the new run's segments are NOT drained into the archive.
        route.release_program(SPTS_PROGRAM_ID, &first);
        route.publish_new_program(SPTS_PROGRAM_ID);
        route.set_init(SPTS_PROGRAM_ID, vec![0xBB; 4]);
        for (seq, byte) in [(1u32, 0x21u8), (2, 0x22)] {
            route
                .add_segment(SPTS_PROGRAM_ID, seg_bytes(seq, byte))
                .expect("add_segment");
        }
        assert_eq!(
            catchup::scan_archive(&tmp.join("c5-live"))
                .iter()
                .map(|s| s.seq)
                .collect::<Vec<_>>(),
            vec![1, 2],
            "premise: only run 1 is archived"
        );

        let playlist = body_string(
            catchup_playlist(State(route.clone()), Query(CatchupPlaylistQuery::default())).await,
        )
        .await;
        let listed: Vec<&str> = playlist
            .lines()
            .filter(|l| l.starts_with("catchup/seg-"))
            .collect();
        assert_eq!(
            listed,
            vec![
                "catchup/seg-1.m4s",
                "catchup/seg-2.m4s",
                "catchup/seg-4.m4s",
                "catchup/seg-5.m4s"
            ],
            "{playlist}"
        );
        // The live tail's init is the live origin's versioned one.
        let live_init = playlist
            .lines()
            .filter_map(|l| l.strip_prefix("#EXT-X-MAP:URI=\""))
            .map(|l| l.trim_end_matches('"'))
            .next_back()
            .expect("a map");
        assert!(
            live_init.starts_with("init-1-") && live_init.ends_with("-1.mp4"),
            "{live_init}"
        );
        let init = route
            .ll_hls(SPTS_PROGRAM_ID)
            .expect("origin")
            .init_name(1, 1);
        assert_eq!(live_init, init);

        // And the tail bytes are the live ones, under the playlist's numbers.
        for (n, expected) in [(4u32, 0x21u8), (5, 0x22)] {
            let resp = catchup_resource(State(route.clone()), Path(format!("seg-{n}.m4s"))).await;
            assert_eq!(resp.status(), StatusCode::OK, "seg-{n}");
            assert!(
                body_bytes(resp).await.iter().all(|&b| b == expected),
                "seg-{n}"
            );
        }
        cleanup(&tmp);
    }

    // --- independent oracle: Apple's `mediastreamvalidator` over the served
    //     catch-up playlist and the files it names ---

    fn validator_unavailable() -> Option<&'static str> {
        if !cfg!(target_os = "macos") {
            return Some("`mediastreamvalidator` is macOS-only and this is not macOS");
        }
        let present = std::process::Command::new("mediastreamvalidator")
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success());
        (!present).then_some("`mediastreamvalidator` is not on PATH (Additional Tools for Xcode)")
    }

    /// MUST-level (requirement level 1) findings in the validator's JSON.
    fn validator_errors(dir: &std::path::Path, entry: &str) -> Vec<String> {
        let out = dir.join("out.json");
        let status = std::process::Command::new("mediastreamvalidator")
            .current_dir(dir)
            .args(["--quiet", "-t", "3", "-O"])
            .arg(&out)
            .arg(entry)
            .status()
            .expect("run mediastreamvalidator");
        assert!(status.success(), "validator exit {status}");
        let compact: String = std::fs::read_to_string(&out)
            .expect("validator json")
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect();
        let mut errors = Vec::new();
        let key = "\"errorRequirementLevel\":1";
        let mut from = 0;
        while let Some(at) = compact[from..].find(key) {
            let at = from + at;
            let open = compact[..at].rfind('{').expect("message object start");
            let close = at + compact[at..].find('}').expect("message object end") + 1;
            errors.push(compact[open..close].to_string());
            from = close;
        }
        if compact.contains("\"parseFailed\":true") {
            errors.push("parseFailed".to_string());
        }
        errors
    }

    /// The served catch-up playlist of two runs with DIFFERENT inits, over real
    /// CMAF media, carries a per-run `EXT-X-MAP` and is accepted by Apple's
    /// validator (which also fetches every file the playlist names through the
    /// paths this handler serves them under).
    #[tokio::test]
    async fn two_runs_with_different_inits_validate_with_mediastreamvalidator() {
        if let Some(why) = validator_unavailable() {
            eprintln!("SKIP catch-up validator oracle: {why}; no-op result, not coverage.");
            return;
        }
        let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../hls-runtime/tests/fixtures/cmaf-fmp4");
        let read = |name: &str| std::fs::read(fixtures.join(name)).expect("fixture");
        let init_a = read("init.mp4");
        // A different init for the second run: the same, plus a trailing `free`
        // box (valid ISOBMFF, byte-wise different).
        let mut init_b = init_a.clone();
        init_b.extend_from_slice(&[0, 0, 0, 8, b'f', b'r', b'e', b'e']);

        let tmp = temp_dir();
        let route = Arc::new(
            RouteHandle::new(1.0, 250, 8)
                .with_name("valid")
                .with_dvr(dvr_config(&tmp)),
        );
        let info = |seq: u32, name: &str| transmux::ll_hls::SegmentInfo {
            bytes: read(name),
            duration: 1.0,
            segment_seq: seq,
            part_count: 1,
        };
        let first = route.publish_new_program(SPTS_PROGRAM_ID);
        route.set_init(SPTS_PROGRAM_ID, init_a.clone());
        route
            .add_segment(SPTS_PROGRAM_ID, info(1, "index0.m4s"))
            .unwrap();
        route.drain_dvr().await;
        route.release_program(SPTS_PROGRAM_ID, &first);
        route.publish_new_program(SPTS_PROGRAM_ID);
        route.set_init(SPTS_PROGRAM_ID, init_b.clone());
        route
            .add_segment(SPTS_PROGRAM_ID, info(1, "index1.m4s"))
            .unwrap();
        route
            .add_segment(SPTS_PROGRAM_ID, info(2, "index2.m4s"))
            .unwrap();
        route.drain_dvr().await;

        let playlist = body_string(
            catchup_playlist(State(route.clone()), Query(CatchupPlaylistQuery::default())).await,
        )
        .await;
        let maps: Vec<&str> = playlist
            .lines()
            .filter(|l| l.starts_with("#EXT-X-MAP"))
            .collect();
        assert_eq!(
            maps,
            vec![
                "#EXT-X-MAP:URI=\"catchup/init-p0.mp4\"",
                "#EXT-X-MAP:URI=\"catchup/init-p1.mp4\""
            ],
            "{playlist}"
        );

        let dir = tmp.join("served");
        std::fs::create_dir_all(dir.join("catchup")).unwrap();
        std::fs::write(dir.join("catchup.m3u8"), &playlist).unwrap();
        let mut served_inits = Vec::new();
        for name in [
            "init-p0.mp4",
            "init-p1.mp4",
            "seg-1.m4s",
            "seg-3.m4s",
            "seg-4.m4s",
        ] {
            let resp = catchup_resource(State(route.clone()), Path(name.to_string())).await;
            assert_eq!(resp.status(), StatusCode::OK, "{name}");
            let bytes = body_bytes(resp).await;
            if name.starts_with("init-") {
                served_inits.push(bytes.clone());
            }
            std::fs::write(dir.join("catchup").join(name), bytes).unwrap();
        }
        assert_eq!(served_inits, vec![init_a, init_b], "each run's own init");

        let errors = validator_errors(&dir, "catchup.m3u8");
        assert!(
            errors.is_empty(),
            "validator MUST-level findings: {errors:#?}"
        );
        cleanup(&tmp);
    }
}
