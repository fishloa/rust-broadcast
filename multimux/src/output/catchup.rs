//! `CatchupOutput`: the HTTP surface for catch-up / time-shift /
//! VOD-from-live serving over the DVR archive (issue #900), a thin axum
//! adapter over [`dvr_runtime`]'s archive reading and merge planning,
//! mounted and gated exactly like every other [`crate::output::Output`] (so
//! it shares the output auth every other output gets from
//! `crate::origin::router`'s `output_auth_gate`).
//!
//! Requires the route's DVR archive to be enabled
//! (`crate::config::Route::dvr.enabled`) — `crate::config::Route::validate_standalone`
//! rejects a `"catchup"` output configured without it. The handlers still
//! check defensively (`RouteHandle::catchup_archive` returning `None`), in
//! case that invariant is ever bypassed.
//!
//! # Three routes, one archive
//!
//! - `GET /catchup.m3u8[?window_secs=N]` — a live-continuing playlist
//!   spanning the archive plus whatever the live `Trunk` has closed since the
//!   archive was last polled ([`dvr_runtime::read::merge`]). `window_secs`
//!   bounds it to the trailing N seconds.
//! - `GET /vod/p{N}.m3u8` — exactly one archived period's segments,
//!   `#EXT-X-ENDLIST` and `PLAYLIST-TYPE:VOD` once that period is
//!   definitively finished (a later period holds segments); otherwise the
//!   same still-growing shape, restricted to that one period.
//! - `GET /catchup/seg-{seq}.{ext}` — the resource route both playlists
//!   reference: archived bytes read off disk when `seq` is in the archive,
//!   or the still-unarchived tail from the same
//!   `hls_runtime::server::HlsOrigin` every other output resolves against.
//!   `GET /catchup/init-p{N}.mp4` serves an archived period's init, the byte
//!   range its playlist's `EXT-X-MAP` names.
//!
//! Every archive read runs on the blocking pool under the route's scan
//! semaphore, through the route's own [`dvr_runtime::fs::FsArchive`] of the
//! requested program.
//!
//! # Selecting a program
//!
//! On a multi-program (MPTS) route every program records into its own
//! archive (see `crate::dvr`). Each of the routes above takes an optional
//! `program` query parameter naming the program by its MPEG-2
//! `program_number` (the DVB `service_id`, `?program=1538`); absent (or `0`,
//! never a service's `program_number`) is the default program, exactly as on
//! a single-program route. Every URI a non-default program's playlists list
//! carries the same parameter, and the init of its still-live tail is served
//! under `catchup/` too (`catchup/init-….mp4?program=1538`), since the
//! route's shared resource route serves the default program only. A program
//! the route neither serves nor found archived answers as a route without an
//! archive does; a program with archived segments but no live program (gone
//! after a replace, or not yet announced since a restart) is served from the
//! archive alone.
//!
//! Mounted under `/catchup*`/`/vod/*` (two-segment paths), never at the
//! same single-segment `/:file` shape `crate::origin::resource`'s shared
//! catch-all owns.

use std::sync::Arc;
use std::time::Duration;
#[cfg(test)]
use transmux::TrackId;

use axum::Router;
use axum::extract::{Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use broadcast_common::Timestamp;
use broadcast_hls::PlaylistType;
use dvr_runtime::PeriodNum;
use dvr_runtime::fs::FsArchive;
use dvr_runtime::read::{ArchiveScan, CatchupSegment, InitRef, LiveSegment, ReadError};
use hls_runtime::server::{DEFAULT_TRACK_ID, Generation, HlsBody, HlsRequest};
use media_plane::egress::{AwaitPolicy, EgressResponse, ServedEgress};
use parking_lot::Mutex;
use serde::Deserialize;
use transmux::Container;

use crate::dvr::ProgramArchive;
use crate::http;
use crate::origin::resource::cors_preflight;
use crate::output::{Output, OutputKind};
use crate::route::RouteHandle;

/// The catch-up/VOD-from-live [`Output`] — see the module docs.
#[derive(Debug, Default)]
pub struct CatchupOutput;

impl Output for CatchupOutput {
    fn kind(&self) -> OutputKind {
        OutputKind::Catchup
    }

    /// Routes (relative — mounted by the origin under `/{stream}/`):
    /// - `GET /catchup.m3u8`
    /// - `GET /vod/{period}` (`period` is `p{N}.m3u8`)
    /// - `GET /catchup/{file}` (`file` is `seg-{seq}.{ext}` or `init-p{N}.mp4`)
    fn manifest_routes(&self, route: Arc<RouteHandle>) -> Router {
        Router::new()
            .route(
                "/catchup.m3u8",
                get(catchup_playlist).options(cors_preflight),
            )
            .route("/vod/{period}", get(vod_playlist).options(cors_preflight))
            .route(
                "/catchup/{file}",
                get(catchup_resource).options(cors_preflight),
            )
            .with_state(route)
    }
}

/// Minimum `#EXT-X-TARGETDURATION` (RFC 8216 §4.3.3.1: a positive integer
/// number of seconds), used when `segments` is empty or every duration
/// rounds to zero.
const MIN_TARGET_DURATION_SECS: u32 = 1;

/// The `#EXT-X-MEDIA-SEQUENCE` of an empty playlist (RFC 8216 §4.3.3.2: the
/// first segment's number is 0 when the tag is absent).
const DEFAULT_MEDIA_SEQUENCE: u64 = 0;

/// Render `segments` (already ordered and windowed) into a Media Playlist
/// whose segment URIs are `catchup/seg-{seq}.{ext}`, relative to where both
/// playlist endpoints are served.
///
/// `init_uri` maps a segment's [`InitRef`] to the `#EXT-X-MAP` URI to
/// advertise (RFC 8216bis §4.4.4.5, required for fMP4); `None` for the
/// `MpegTs` container. Every segment carries its own run's map:
/// `broadcast-hls` renders it where it changes, so an archived run is never
/// decoded with a later run's init.
///
/// `playlist_type` is `None` for a playlist that may lose leading segments:
/// RFC 8216 §6.2.2 forbids `EXT-X-PLAYLIST-TYPE` on a playlist that removes
/// segments.
fn render_playlist(
    segments: &[CatchupSegment<Generation>],
    ext: &str,
    program: ProgramSelector,
    init_uri: Option<&dyn Fn(InitRef<Generation>) -> String>,
    playlist_type: Option<PlaylistType>,
    endlist: bool,
) -> Result<String, broadcast_hls::Error> {
    let target_duration = segments
        .iter()
        .map(|s| s.duration.as_secs_f64())
        .fold(0.0_f64, f64::max)
        .ceil()
        .clamp(f64::from(MIN_TARGET_DURATION_SECS), f64::from(u32::MAX))
        as u32;
    let media_sequence = segments
        .first()
        .map(|s| u64::from(s.seq.value()))
        .unwrap_or(DEFAULT_MEDIA_SEQUENCE);
    let mut hls_segments: Vec<broadcast_hls::MediaSegment> = segments
        .iter()
        .map(|s| broadcast_hls::MediaSegment {
            uri: format!("catchup/seg-{}.{ext}{}", s.seq, program.uri_query()),
            duration: broadcast_hls::DecimalSeconds::from_duration(s.duration),
            discontinuous: s.discontinuous,
            ..Default::default()
        })
        .collect();
    if let Some(init_uri) = init_uri {
        for (hls, seg) in hls_segments.iter_mut().zip(segments) {
            if let Some(init) = seg.init {
                hls.map = Some(broadcast_hls::MapTag {
                    uri: init_uri(init),
                    byte_range: None,
                    extra_attrs: Vec::new(),
                });
            }
        }
    }
    let playlist = broadcast_hls::MediaPlaylist {
        target_duration,
        msn: broadcast_hls::MediaSequence::new(media_sequence),
        segments: hls_segments,
        endlist,
        playlist_type,
        ..Default::default()
    };
    playlist.to_m3u8()
}

/// The dynamic-filename extension (without the leading `.`) for `route`'s
/// configured container.
fn container_ext(container: Container) -> &'static str {
    match container {
        Container::MpegTs => "ts",
        _ => "m4s",
    }
}

/// The `#EXT-X-MAP` URI of an [`InitRef`], relative to the catch-up
/// playlists (which sit at `/{stream}/`): an archived period's init is served
/// by this module's own resource route (`catchup/init-p{N}.mp4`), a segment
/// still in the live origin names the origin's versioned init resource
/// (`HlsOrigin::init_name`).
///
/// `shared` says whether the route's shared resource route serves the live
/// program (it serves the default program id only): when it does not, a live
/// init is named under `catchup/` with the program's query.
fn init_uri_of(
    ll_hls: Option<&hls_runtime::server::HlsOrigin>,
    shared: bool,
    program: ProgramSelector,
    init: InitRef<Generation>,
) -> String {
    let query = program.uri_query();
    let live = |name: String| {
        if shared {
            name
        } else {
            format!("catchup/{name}{query}")
        }
    };
    match init {
        InitRef::Live(generation) => live(ll_hls.map_or_else(
            || format!("init-{DEFAULT_TRACK_ID}.mp4"),
            |o| o.init_name(DEFAULT_TRACK_ID, generation),
        )),
        InitRef::Archived { period, .. } => format!("catchup/init-p{period}.mp4{query}"),
        _ => live(format!("init-{DEFAULT_TRACK_ID}.mp4")),
    }
}

/// The program a catch-up request names (see
/// [the module docs](self#selecting-a-program)).
#[derive(Debug, Clone, Copy, Default, Deserialize)]
pub struct ProgramSelector {
    /// The program's `program_number`; omitted or `0` is the default program.
    program: Option<u16>,
}

impl ProgramSelector {
    /// The `program_number` named, if any (`0` names none).
    fn program_number(self) -> Option<u16> {
        self.program.filter(|&n| n != 0)
    }

    /// The archive this request reads on `route`: the default one with no
    /// program, or for the route's `dvb_service_id` (an alias of the
    /// default), `<route>@<program_number>` otherwise.
    fn archive(self, route: &RouteHandle) -> ProgramArchive {
        let service = route.dvr_config().and_then(|config| config.dvb_service_id);
        match self.program_number() {
            None => ProgramArchive::Default,
            Some(program_number) if Some(program_number) == service => ProgramArchive::Default,
            Some(program_number) => ProgramArchive::Program(program_number),
        }
    }

    fn is_default(self) -> bool {
        self.program_number().is_none()
    }

    /// The query string every URI of this program's playlists carries.
    fn uri_query(self) -> String {
        self.program_number()
            .map_or_else(String::new, |n| format!("?program={n}"))
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
    /// Bound the catch-up window to this many trailing seconds (see
    /// [`dvr_runtime::read::window`]). Omitted or `0`: the whole archive
    /// plus the live tail.
    window_secs: Option<u64>,
    /// The program's `program_number`; see [`ProgramSelector`].
    program: Option<u16>,
}

/// Log what a scan could not read; the readable periods are still served.
fn report_scan(route: &RouteHandle, scan: &ArchiveScan) {
    for (period, error) in &scan.unreadable {
        tracing::warn!(
            route = %route.name(),
            period = period.value(),
            %error,
            "catch-up: skipping an unreadable archive period"
        );
    }
    for conflict in &scan.conflicts {
        tracing::warn!(
            route = %route.name(),
            seq = conflict.seq.value(),
            kept = conflict.kept.value(),
            dropped = conflict.dropped.value(),
            "catch-up: two archive periods claim one sequence number; serving the earlier"
        );
    }
}

/// Scan the route's archive on the blocking pool under a scan permit.
async fn scan_archive(
    route: &RouteHandle,
    archive: Arc<Mutex<FsArchive>>,
) -> Result<ArchiveScan, ScanError> {
    let scan = run_scan(route, move || archive.lock().scan()).await??;
    report_scan(route, &scan);
    Ok(scan)
}

/// `GET /catchup.m3u8` — see the module docs.
async fn catchup_playlist(
    State(route): State<Arc<RouteHandle>>,
    Query(q): Query<CatchupPlaylistQuery>,
) -> Response {
    let program = ProgramSelector { program: q.program };
    let live_serving = http::resolution_response(route.resolve_archive(program.archive(&route)));
    let (Some(dvr), Some(archive)) = (
        route.dvr_config(),
        route.catchup_archive(program.archive(&route)),
    ) else {
        return match live_serving {
            Ok(_) => StatusCode::NOT_FOUND.into_response(),
            Err(resp) => *resp,
        };
    };
    let ext = container_ext(route.container());
    let archived = match scan_archive(&route, archive).await {
        Ok(scan) => scan.segments,
        Err(e) => {
            tracing::error!(error = %e, "catch-up: archive scan failed");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    // No live program (gone after a replace, or not yet announced since a
    // restart): the archive alone, as the VOD route serves it.
    let serving = match live_serving {
        Ok(serving) => Some(serving),
        Err(_) if !archived.is_empty() => None,
        Err(resp) => return *resp,
    };
    // The live tail in the numbers the origin's playlist and the archive use
    // (the origin's offset plus the `Trunk`'s number, audit r07-C5, #1083).
    let ll_hls = serving.as_ref().map(|serving| serving.ll_hls());
    let live: Vec<LiveSegment<Generation>> = ll_hls
        .iter()
        .flat_map(|ll_hls| {
            let offset = ll_hls.media_sequence_offset();
            ll_hls.closed_segments().into_iter().filter_map(move |s| {
                let public =
                    u32::try_from(offset.checked_add(u64::from(s.sequence_number.value()))?)
                        .ok()?;
                Some(
                    LiveSegment::new(
                        media_plane::ids::SegmentSeq::new(public),
                        Timestamp::from_nanos(s.start.0),
                        s.duration,
                        s.init_generation,
                    )
                    .with_discontinuous(s.discontinuous),
                )
            })
        })
        .collect();
    let combined = dvr_runtime::read::merge(&archived, &live);
    let windowed = dvr_runtime::read::window(&combined, q.window_secs.map(Duration::from_secs));
    // Audit run 7, W11: `EVENT` only when nothing can remove the playlist's
    // leading segments (RFC 8216 §6.2.2).
    let removes_leading = q.window_secs.filter(|&w| w > 0).is_some() || dvr.retention_active();
    let playlist_type = if removes_leading {
        None
    } else {
        Some(PlaylistType::Event)
    };
    let shared = program.is_default() && serving.as_ref().is_some_and(|s| s.is_default_program());
    let init_of = |init| init_uri_of(ll_hls.as_deref(), shared, program, init);
    let Ok(body) = render_playlist(
        &windowed,
        ext,
        program,
        matches!(route.container(), Container::Fmp4).then_some(&init_of as &dyn Fn(_) -> _),
        playlist_type,
        false,
    ) else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    (
        [(header::CONTENT_TYPE, crate::http::PLAYLIST_CONTENT_TYPE)],
        body,
    )
        .into_response()
}

/// `GET /vod/p{N}.m3u8` — see the module docs.
async fn vod_playlist(
    State(route): State<Arc<RouteHandle>>,
    Path(period_file): Path<String>,
    Query(program): Query<ProgramSelector>,
) -> Response {
    let Some(archive) = route.catchup_archive(program.archive(&route)) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Some(period) = parse_period_filename(&period_file).map(PeriodNum::new) else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let ext = container_ext(route.container());
    let scan = match scan_archive(&route, archive).await {
        Ok(scan) => scan,
        Err(e) => {
            tracing::error!(error = %e, "catch-up: period read task failed");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };
    let segments: Vec<CatchupSegment<Generation>> = scan
        .segments
        .iter()
        .filter(|s| s.period == period)
        .map(|s| {
            CatchupSegment::new(s.seq, s.start, s.duration)
                .with_discontinuous(s.discontinuous)
                .with_init(s.init.map(|span| InitRef::Archived {
                    period: s.period,
                    span,
                }))
        })
        .collect();
    if segments.is_empty() {
        return StatusCode::NOT_FOUND.into_response();
    }
    // A period is never appended to once a later period holds a segment: the
    // writer closes it with `EXT-X-ENDLIST` in the same plan that writes the
    // later period's first segment.
    let finished = scan.segments.iter().any(|s| s.period > period);
    let playlist_type = if finished {
        PlaylistType::Vod
    } else {
        PlaylistType::Event
    };
    let init_of = |init| init_uri_of(None, false, program, init);
    let Ok(body) = render_playlist(
        &segments,
        ext,
        program,
        matches!(route.container(), Container::Fmp4).then_some(&init_of as &dyn Fn(_) -> _),
        Some(playlist_type),
        finished,
    ) else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    (
        [(header::CONTENT_TYPE, crate::http::PLAYLIST_CONTENT_TYPE)],
        body,
    )
        .into_response()
}

/// Bounds how many archive reads of one route run on the blocking pool at
/// once, held by all three catch-up paths. Sized well below the blocking
/// pool's own ceiling so the DVR recorder's polls always find a thread.
const MAX_CONCURRENT_SCANS: usize = 4;

/// The route's own scan pool, so one route's slow archive never holds the
/// permits another route's viewers wait for.
fn scan_permit(route: &RouteHandle) -> &Arc<tokio::sync::Semaphore> {
    route
        .catchup_scans()
        .get_or_init(|| Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_SCANS)))
}

#[derive(Debug, thiserror::Error)]
enum ScanError {
    #[error("scan pool closed")]
    Closed,
    #[error(transparent)]
    Task(#[from] tokio::task::JoinError),
    #[error("archive cannot be read: {0}")]
    Read(#[from] ReadError),
}

/// Run `scan` on the blocking pool under a scan permit. The permit moves into
/// the blocking task, so a request dropped mid-scan (a client disconnect)
/// does not free the slot while the scan it bounds is still running.
async fn run_scan<T: Send + 'static>(
    route: &RouteHandle,
    scan: impl FnOnce() -> T + Send + 'static,
) -> Result<T, ScanError> {
    run_scan_on(scan_permit(route), scan).await
}

async fn run_scan_on<T: Send + 'static>(
    pool: &Arc<tokio::sync::Semaphore>,
    scan: impl FnOnce() -> T + Send + 'static,
) -> Result<T, ScanError> {
    let permit = Arc::clone(pool)
        .acquire_owned()
        .await
        .map_err(|_| ScanError::Closed)?;
    Ok(tokio::task::spawn_blocking(move || {
        let _permit = permit;
        scan()
    })
    .await?)
}

fn parse_period_filename(file: &str) -> Option<u32> {
    file.strip_prefix('p')?.strip_suffix(".m3u8")?.parse().ok()
}

/// Answer an archive read: the bytes, `404` for an evicted period, `500`
/// otherwise.
fn archive_read_response(
    route: &RouteHandle,
    name: &str,
    read: Result<Option<Result<bytes::Bytes, ReadError>>, ScanError>,
) -> Response {
    match read {
        Ok(Some(Ok(bytes))) => (
            [(
                header::CONTENT_TYPE,
                crate::origin::resource::resource_content_type(route.container(), name),
            )],
            bytes,
        )
            .into_response(),
        Ok(None | Some(Err(ReadError::Gone { .. }))) => StatusCode::NOT_FOUND.into_response(),
        Ok(Some(Err(e))) => {
            tracing::error!(error = %e, name, "catch-up: failed reading archived bytes");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, name, "catch-up: archive read task failed");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

/// `GET /catchup/seg-{seq}.{ext}` — archived bytes read from disk when `seq`
/// is archived; otherwise the live `Trunk`'s still-unarchived tail, resolved
/// through the same `HlsOrigin` the shared resource route uses for
/// `seg-{track}-{seq}.{ext}`. One filename grammar, either source.
async fn catchup_resource(
    State(route): State<Arc<RouteHandle>>,
    Path(file): Path<String>,
    Query(program): Query<ProgramSelector>,
) -> Response {
    let ext = container_ext(route.container());
    if let Some(period) = parse_init_filename(&file) {
        return catchup_init(&route, program.archive(&route), PeriodNum::new(period)).await;
    }
    if file.starts_with(LIVE_INIT_PREFIX) {
        return live_resource(&route, program.archive(&route), file);
    }
    let Some(seq) = parse_seg_filename(&file, ext) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Some(archive) = route.catchup_archive(program.archive(&route)) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let found = run_scan(&route, move || {
        let mut archive = archive.lock();
        match archive.find(media_plane::ids::SegmentSeq::new(seq)) {
            Ok(found) => found.map(|segment| archive.read(&segment)),
            Err(error) => Some(Err(error)),
        }
    })
    .await;
    if !matches!(found, Ok(None)) {
        return archive_read_response(&route, &file, found);
    }

    // Not archived (yet): the still-live tail.
    live_resource(
        &route,
        program.archive(&route),
        format!("seg-{DEFAULT_TRACK_ID}-{seq}.{ext}"),
    )
}

/// The prefix of the live origin's init resource names a catch-up playlist
/// lists under `catchup/` when the shared resource route does not serve its
/// program.
const LIVE_INIT_PREFIX: &str = "init-";

/// `live_name` from `program`'s live origin. A one-shot, non-blocking
/// resolve: the client only requests a resource a playlist just listed.
fn live_resource(route: &RouteHandle, program: ProgramArchive, live_name: String) -> Response {
    let serving = match http::resolution_response(route.resolve_archive(program)) {
        Ok(serving) => serving,
        Err(resp) => return *resp,
    };
    let ll_hls = serving.ll_hls();
    let now = Timestamp::from_nanos(0);
    let policy = AwaitPolicy::new(now);
    let request = HlsRequest::Resource {
        name: live_name.clone(),
    };
    match ll_hls.resolve(request, now, policy) {
        EgressResponse::Ready {
            body: HlsBody::Resource(bytes),
            ..
        } => (
            [(
                header::CONTENT_TYPE,
                crate::origin::resource::resource_content_type(route.container(), &live_name),
            )],
            bytes,
        )
            .into_response(),
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

/// `GET /catchup/init-p{N}.mp4`: the init of archive period `N`, the byte
/// range its playlist's `EXT-X-MAP` names. `404` for a TS route, an unknown
/// or evicted period, or one with no segment yet.
async fn catchup_init(route: &RouteHandle, program: ProgramArchive, period: PeriodNum) -> Response {
    if crate::dvr::archive_container(route.container()) != dvr_runtime::ArchiveContainer::Fmp4 {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Some(archive) = route.catchup_archive(program) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let read = run_scan(route, move || {
        let mut archive = archive.lock();
        let scan = match archive.scan() {
            Ok(scan) => scan,
            Err(error) => return Some(Err(error)),
        };
        let span = scan
            .segments
            .iter()
            .find(|s| s.period == period)
            .and_then(|s| s.init)?;
        Some(archive.read_init(period, span))
    })
    .await;
    archive_read_response(route, &format!("init-p{period}.mp4"), read)
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
    use crate::route::RouteConfig;
    use crate::route::SPTS_PROGRAM_ID;
    use crate::testutil::body_string;
    use std::path::PathBuf;

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

    /// Take every permit from the shared scan semaphore, waiting for permits
    /// held by concurrently running tests.
    async fn scan_permit_wait_all(route: &RouteHandle) -> tokio::sync::SemaphorePermit<'_> {
        let n = u32::try_from(MAX_CONCURRENT_SCANS).expect("pool size fits u32");
        tokio::time::timeout(
            std::time::Duration::from_secs(30),
            scan_permit(route).acquire_many(n),
        )
        .await
        .expect("timed out waiting for every scan permit")
        .expect("scan semaphore is never closed")
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
        transmux::ll_hls::SegmentInfo::new(vec![byte; 24], Duration::from_secs(3), seq)
            .with_part_count(1)
    }

    async fn body_bytes(resp: Response) -> Vec<u8> {
        axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec()
    }

    // --- W3 (audit run 7): DVR persist through the async advance_route path ---

    /// `advance_route`'s DVR drain persists a published segment byte-exact to
    /// the period file, and the period's `pN.m3u8` describes exactly that byte
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
        use media_plane::ingress::{HandshakePolicy, IngestDriver};
        use media_plane::ts_session::{TsIngestSession, test_support::build_ts_bytes};
        use std::num::NonZeroUsize;

        // The init prelude of this synthetic track: 652 bytes before the init
        // segment wrote display geometry (Phase 4a-1) plus 19 for the `colr`
        // nclx box its SPS now earns (the SPS signals BT.709 colour): box
        // header 8 + colour_type 4 + three u16 code points 6 + full-range byte 1.
        const INIT_LEN: usize = 652 + 19;

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("current-thread runtime");
        rt.block_on(async {
            let tmp = temp_dir();
            let route = Arc::new(
                RouteHandle::new(
                    RouteConfig::default()
                        .with_target_duration(std::time::Duration::from_secs(1))
                        .with_part_target(std::time::Duration::from_millis(250))
                        .with_window_segments(64),
                )
                .with_name("w3")
                .with_dvr(dvr_config(&tmp)),
            );
            route.publish_new_program(SPTS_PROGRAM_ID);
            const SEG_LEN: usize = 4096;
            route
                .add_segment(
                    SPTS_PROGRAM_ID,
                    transmux::ll_hls::SegmentInfo::new(
                        vec![0x5A; SEG_LEN],
                        Duration::from_secs(3),
                        1,
                    )
                    .with_part_count(1),
                )
                .expect("add_segment");

            let nz = |n: usize| NonZeroUsize::new(n).unwrap();
            let mut driver = IngestDriver::new(
                TsIngestSession::new(),
                media_plane::IngestConfig::new(HandshakePolicy::establish_by(
                    broadcast_common::Timestamp::from_nanos(u64::MAX),
                ))
                .with_trunk(
                    media_plane::trunk::TrunkConfig::default()
                        .with_timed_capacity(nz(8))
                        .with_sparse_capacity(nz(8))
                        .with_segment_capacity(nz(64))
                        .with_event_capacity(nz(8))
                        .with_part_capacity(nz(8)),
                ),
            );
            let mut progress = DriverProgress::new();
            driver.feed(
                &build_ts_bytes(TrackId::new(1), 0xAB, 90),
                broadcast_common::Timestamp::ZERO,
            );

            crate::source::advance_route(&driver, &route, &mut progress).await;

            // The drain persisted the segment: the init prelude is INIT_LEN bytes
            // (a real fMP4 init for the synthetic track), then SEG_LEN bytes
            // of 0x5A.
            let period = tmp.join("w3").join("p0.m4s");
            let bytes = std::fs::read(&period).expect("period file must exist");
            assert_eq!(
                bytes.len(),
                INIT_LEN + SEG_LEN,
                "period file must hold init + the one segment"
            );
            assert!(
                bytes[..INIT_LEN].windows(4).any(|w| w == b"colr"),
                "the extra 19 init bytes are the colr nclx box"
            );
            assert!(
                bytes[INIT_LEN..].iter().all(|&b| b == 0x5A),
                "the segment's bytes must be persisted byte-exact"
            );

            // The period playlist describes exactly that range, and the init
            // range its `EXT-X-MAP` names.
            let scan = crate::testutil::scan_archive(&tmp, "w3", Container::Fmp4);
            assert_eq!(scan.segments.len(), 1, "one segment must be indexed");
            let segment = scan.segments[0];
            assert_eq!(segment.seq.value(), 1);
            assert_eq!(segment.span.offset, INIT_LEN as u64);
            assert_eq!(segment.span.len.get(), SEG_LEN as u64);
            let init = segment.init.expect("an fMP4 segment names its init");
            assert_eq!((init.offset, init.len.get()), (0, INIT_LEN as u64));

            cleanup(&tmp);
        });
    }

    /// A scan whose request is dropped mid-scan (a client disconnect) keeps
    /// its permit until the blocking task itself ends, so the bound on
    /// concurrent scans holds for the work actually running.
    #[tokio::test]
    async fn a_dropped_scan_request_keeps_its_permit_until_the_scan_ends() {
        let pool = Arc::new(tokio::sync::Semaphore::new(1));
        let (release, hold) = std::sync::mpsc::channel::<()>();
        let (started_tx, started) = tokio::sync::oneshot::channel::<()>();
        let request = tokio::spawn({
            let pool = Arc::clone(&pool);
            async move {
                run_scan_on(&pool, move || {
                    let _ = started_tx.send(());
                    let _ = hold.recv();
                })
                .await
            }
        });
        started.await.expect("the scan started");
        request.abort();
        let _ = request.await;
        assert_eq!(
            pool.available_permits(),
            0,
            "the permit was released while the blocking scan was still running"
        );

        release.send(()).expect("scan still waiting");
        let _permit = tokio::time::timeout(std::time::Duration::from_secs(5), pool.acquire())
            .await
            .expect("the permit returns when the scan ends")
            .expect("pool open");
    }

    /// One route's exhausted scan pool does not delay another route's scan.
    #[tokio::test]
    async fn one_routes_exhausted_scan_pool_does_not_delay_another_route() {
        let busy = Arc::new(RouteHandle::new(
            RouteConfig::default()
                .with_target_duration(std::time::Duration::from_secs(1))
                .with_part_target(std::time::Duration::from_millis(250))
                .with_window_segments(8),
        ));
        let idle = Arc::new(RouteHandle::new(
            RouteConfig::default()
                .with_target_duration(std::time::Duration::from_secs(1))
                .with_part_target(std::time::Duration::from_millis(250))
                .with_window_segments(8),
        ));
        let _held = scan_permit_wait_all(&busy).await;
        assert_eq!(scan_permit(&busy).available_permits(), 0);
        let blocked = tokio::time::timeout(
            std::time::Duration::from_millis(100),
            run_scan(&busy, || ()),
        )
        .await;
        assert!(blocked.is_err(), "the busy route's scan must wait");
        let done = tokio::time::timeout(std::time::Duration::from_secs(5), run_scan(&idle, || 7))
            .await
            .expect("the other route's scan is not delayed")
            .expect("scan runs");
        assert_eq!(done, 7);
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
            RouteHandle::new(
                RouteConfig::default()
                    .with_target_duration(std::time::Duration::from_secs(1))
                    .with_part_target(std::time::Duration::from_millis(250))
                    .with_window_segments(8),
            )
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
        let held = scan_permit_wait_all(&route).await;
        assert_eq!(scan_permit(&route).available_permits(), 0);

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
            vod_playlist(
                State(Arc::clone(&route)),
                Path("p0.m3u8".into()),
                Query(ProgramSelector::default())
            )
        );
        must_wait!(
            "catchup_resource",
            catchup_resource(
                State(Arc::clone(&route)),
                Path("seg-1.m4s".into()),
                Query(ProgramSelector::default())
            )
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

    fn map_uris(body: &str) -> Vec<String> {
        body.lines()
            .filter_map(|l| l.strip_prefix("#EXT-X-MAP:URI=\""))
            .filter_map(|l| l.split('"').next())
            .map(str::to_owned)
            .collect()
    }

    fn program(program_number: u16) -> Query<ProgramSelector> {
        Query(ProgramSelector {
            program: Some(program_number),
        })
    }

    fn playlist_query(program_number: u16) -> Query<CatchupPlaylistQuery> {
        Query(CatchupPlaylistQuery {
            window_secs: None,
            program: Some(program_number),
        })
    }

    fn mpts_route(tmp: &std::path::Path, name: &str) -> RouteHandle {
        RouteHandle::new(
            RouteConfig::default()
                .with_target_duration(std::time::Duration::from_secs(4))
                .with_part_target(std::time::Duration::from_millis(500))
                .with_window_segments(8),
        )
        .with_name(name)
        .with_dvr(DvrConfig {
            dvb_service_id: Some(0x0601),
            ..dvr_config(tmp)
        })
    }

    /// #3950: a non-default program's archive is read through
    /// `?program=<program_number>`, every URI its playlists list carries the
    /// parameter (the live tail's init included, served under `catchup/`),
    /// and the default program's URIs stay exactly as on a single-program
    /// route.
    #[tokio::test]
    async fn a_second_program_is_read_through_the_program_query() {
        let tmp = temp_dir();
        let route = Arc::new(mpts_route(&tmp, "mpts"));
        let second = media_plane::ProgramId(1);
        route.publish_new_service(SPTS_PROGRAM_ID, 0x0601);
        route.publish_new_service(second, 0x0602);
        route.set_init(SPTS_PROGRAM_ID, vec![0xAA; 4]);
        route.set_init(second, vec![0xBB; 4]);
        for seq in 1..=2 {
            route
                .add_segment(SPTS_PROGRAM_ID, seg_bytes(seq, 0x10))
                .expect("add_segment");
            route
                .add_segment(second, seg_bytes(seq, 0x20))
                .expect("add_segment");
        }
        route.drain_dvr().await;
        route
            .add_segment(second, seg_bytes(3, 0x23))
            .expect("add_segment");

        let resp = catchup_playlist(State(Arc::clone(&route)), playlist_query(1538)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;
        let segments: Vec<&str> = body
            .lines()
            .filter(|l| l.starts_with("catchup/seg-"))
            .collect();
        assert_eq!(
            segments,
            [
                "catchup/seg-1.m4s?program=1538",
                "catchup/seg-2.m4s?program=1538",
                "catchup/seg-3.m4s?program=1538"
            ],
            "{body}"
        );
        let maps = map_uris(&body);
        assert_eq!(maps.len(), 2, "{body}");
        assert_eq!(maps[0], "catchup/init-p0.mp4?program=1538");
        let live_init = maps[1]
            .strip_prefix("catchup/")
            .and_then(|m| m.strip_suffix("?program=1538"))
            .expect("the live init is served under catchup/ for program 1538");

        for (seq, byte) in [(1, 0x20u8), (2, 0x20), (3, 0x23)] {
            let resp = catchup_resource(
                State(Arc::clone(&route)),
                Path(format!("seg-{seq}.m4s")),
                program(1538),
            )
            .await;
            assert_eq!(resp.status(), StatusCode::OK, "seg-{seq}");
            assert_eq!(body_bytes(resp).await, vec![byte; 24], "seg-{seq}");
        }
        for file in ["init-p0.mp4".to_string(), live_init.to_string()] {
            let resp =
                catchup_resource(State(Arc::clone(&route)), Path(file.clone()), program(1538))
                    .await;
            assert_eq!(resp.status(), StatusCode::OK, "{file}");
            assert_eq!(body_bytes(resp).await, vec![0xBB; 4], "{file}");
        }
        let vod = vod_playlist(
            State(Arc::clone(&route)),
            Path("p0.m3u8".to_string()),
            program(1538),
        )
        .await;
        assert_eq!(vod.status(), StatusCode::OK);
        assert!(
            body_string(vod)
                .await
                .contains("catchup/seg-1.m4s?program=1538")
        );

        let default = body_string(
            catchup_playlist(
                State(Arc::clone(&route)),
                Query(CatchupPlaylistQuery::default()),
            )
            .await,
        )
        .await;
        let segments: Vec<&str> = default
            .lines()
            .filter(|l| l.starts_with("catchup/seg-"))
            .collect();
        assert_eq!(segments, ["catchup/seg-1.m4s", "catchup/seg-2.m4s"]);
        assert_eq!(map_uris(&default), ["catchup/init-p0.mp4"]);
        let resp = catchup_resource(
            State(Arc::clone(&route)),
            Path("seg-2.m4s".to_string()),
            Query(ProgramSelector::default()),
        )
        .await;
        assert_eq!(body_bytes(resp).await, vec![0x10; 24]);

        // `?program=<dvb_service_id>` aliases the default archive.
        let alias =
            body_string(catchup_playlist(State(Arc::clone(&route)), playlist_query(1537)).await)
                .await;
        let segments: Vec<&str> = alias
            .lines()
            .filter(|l| l.starts_with("catchup/seg-"))
            .collect();
        assert_eq!(
            segments,
            [
                "catchup/seg-1.m4s?program=1537",
                "catchup/seg-2.m4s?program=1537"
            ]
        );
        let resp = catchup_resource(
            State(Arc::clone(&route)),
            Path("seg-2.m4s".to_string()),
            program(1537),
        )
        .await;
        assert_eq!(body_bytes(resp).await, vec![0x10; 24]);

        let unknown = catchup_playlist(State(Arc::clone(&route)), playlist_query(7)).await;
        assert_eq!(unknown.status(), StatusCode::NOT_FOUND);
        let unknown = vod_playlist(
            State(Arc::clone(&route)),
            Path("p0.m3u8".into()),
            program(7),
        )
        .await;
        assert_eq!(unknown.status(), StatusCode::NOT_FOUND);
        cleanup(&tmp);
    }

    /// #3950: a request naming a `program_number` the route neither serves
    /// nor found archived builds no archive reader, so arbitrary query
    /// values cannot grow the route's reader map.
    #[tokio::test]
    async fn an_unknown_program_builds_no_archive_reader() {
        let tmp = temp_dir();
        let route = Arc::new(mpts_route(&tmp, "unknown"));
        route.publish_new_service(SPTS_PROGRAM_ID, 0x0601);
        route.set_init(SPTS_PROGRAM_ID, vec![0xAA; 4]);
        route
            .add_segment(SPTS_PROGRAM_ID, seg_bytes(1, 0x10))
            .expect("add_segment");
        route.drain_dvr().await;
        assert_eq!(route.catchup_archive_count(), 0);
        for program_number in [7u16, 1538, 65535] {
            let resp =
                catchup_playlist(State(Arc::clone(&route)), playlist_query(program_number)).await;
            assert_eq!(resp.status(), StatusCode::NOT_FOUND);
            let resp = catchup_resource(
                State(Arc::clone(&route)),
                Path("seg-1.m4s".into()),
                program(program_number),
            )
            .await;
            assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        }
        assert_eq!(route.catchup_archive_count(), 0);
        let resp = catchup_playlist(State(Arc::clone(&route)), Query(Default::default())).await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(route.catchup_archive_count(), 1);
        cleanup(&tmp);
    }

    /// #3950: a program with archived segments but no live program (gone
    /// after a replace, or found on disk at a restart and not yet announced)
    /// serves its catch-up playlist from the archive alone, as the VOD route
    /// does, rather than `404`.
    #[tokio::test]
    async fn an_archived_program_without_a_live_one_is_served_from_the_archive() {
        let tmp = temp_dir();
        {
            let route = mpts_route(&tmp, "gone");
            route.publish_new_service(SPTS_PROGRAM_ID, 0x0601);
            route.publish_new_service(media_plane::ProgramId(1), 0x0602);
            route.set_init(media_plane::ProgramId(1), vec![0xBB; 4]);
            route
                .add_segment(media_plane::ProgramId(1), seg_bytes(1, 0x20))
                .expect("add_segment");
            route.drain_dvr().await;
        }
        let floors = crate::dvr::archive_seq_floors(&dvr_config(&tmp), "gone", Container::Fmp4)
            .await
            .expect("floors");
        let restarted = Arc::new(mpts_route(&tmp, "gone").with_archive_floors(floors));
        restarted.publish_new_service(SPTS_PROGRAM_ID, 0x0601);
        let resp = catchup_playlist(State(Arc::clone(&restarted)), playlist_query(1538)).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;
        let segments: Vec<&str> = body
            .lines()
            .filter(|l| l.starts_with("catchup/seg-"))
            .collect();
        assert_eq!(segments, ["catchup/seg-1.m4s?program=1538"], "{body}");
        assert_eq!(map_uris(&body), ["catchup/init-p0.mp4?program=1538"]);
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
            RouteHandle::new(
                RouteConfig::default()
                    .with_target_duration(std::time::Duration::from_secs(4))
                    .with_part_target(std::time::Duration::from_millis(500))
                    .with_window_segments(8),
            )
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
        let archived_resp = catchup_resource(
            State(route.clone()),
            Path("seg-2.m4s".to_string()),
            Query(ProgramSelector::default()),
        )
        .await;
        assert_eq!(archived_resp.status(), StatusCode::OK);
        // Apple HLS authoring spec 10.4: an fMP4 `.m4s` segment is
        // `video/iso.segment`, from the archive AND from the live window.
        assert_eq!(
            archived_resp.headers().get(header::CONTENT_TYPE).unwrap(),
            "video/iso.segment"
        );
        assert_eq!(body_bytes(archived_resp).await, vec![0x22u8; 24]);

        // Live-only segment: served through the SAME endpoint, resolved
        // via the live HlsOrigin.
        let live_resp = catchup_resource(
            State(route),
            Path("seg-4.m4s".to_string()),
            Query(ProgramSelector::default()),
        )
        .await;
        assert_eq!(live_resp.status(), StatusCode::OK);
        assert_eq!(
            live_resp.headers().get(header::CONTENT_TYPE).unwrap(),
            "video/iso.segment"
        );
        assert_eq!(body_bytes(live_resp).await, vec![0x44u8; 24]);

        cleanup(&tmp);
    }

    #[tokio::test]
    async fn catchup_playlist_without_dvr_is_404() {
        let route = Arc::new(
            RouteHandle::new(
                RouteConfig::default()
                    .with_target_duration(std::time::Duration::from_secs(4))
                    .with_part_target(std::time::Duration::from_millis(500))
                    .with_window_segments(8),
            )
            .with_name("no-dvr"),
        );
        route.publish_new_program(SPTS_PROGRAM_ID);
        route.set_init(SPTS_PROGRAM_ID, vec![0xAA; 4]);
        let resp = catchup_playlist(State(route), Query(CatchupPlaylistQuery::default())).await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn window_secs_bounds_the_playlist() {
        let tmp = temp_dir();
        let route = Arc::new(
            RouteHandle::new(
                RouteConfig::default()
                    .with_target_duration(std::time::Duration::from_secs(4))
                    .with_part_target(std::time::Duration::from_millis(500))
                    .with_window_segments(8),
            )
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
                program: None,
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
            RouteHandle::new(
                RouteConfig::default()
                    .with_target_duration(std::time::Duration::from_secs(4))
                    .with_part_target(std::time::Duration::from_millis(500))
                    .with_window_segments(8),
            )
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
            RouteHandle::new(
                RouteConfig::default()
                    .with_target_duration(std::time::Duration::from_secs(4))
                    .with_part_target(std::time::Duration::from_millis(500))
                    .with_window_segments(8),
            )
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
            RouteHandle::new(
                RouteConfig::default()
                    .with_target_duration(std::time::Duration::from_secs(4))
                    .with_part_target(std::time::Duration::from_millis(500))
                    .with_window_segments(8),
            )
            .with_name("vod")
            .with_dvr(dvr_config(&tmp)),
        );
        route.publish_new_program(SPTS_PROGRAM_ID);
        route.set_init(SPTS_PROGRAM_ID, vec![0xAA; 4]);

        route
            .add_segment(SPTS_PROGRAM_ID, seg_bytes(1, 0x11))
            .expect("add_segment");
        route.drain_dvr().await; // opens + writes period 0 with init A

        // Changing the init rolls the period (the DVR writer's
        // mid-stream init-change rollover) — a real, reliable way to force
        // period 1 to open without waiting out `period_duration_secs`.
        route.set_init(SPTS_PROGRAM_ID, vec![0xBB; 4]);
        route
            .add_segment(SPTS_PROGRAM_ID, seg_bytes(2, 0x22))
            .expect("add_segment");
        route.drain_dvr().await;

        let resp = vod_playlist(
            State(route.clone()),
            Path("p0.m3u8".to_string()),
            Query(ProgramSelector::default()),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;
        assert!(
            body.contains("#EXT-X-ENDLIST"),
            "period 0 has a successor (period 1) so it must be finished: {body}"
        );
        assert!(body.contains("#EXT-X-PLAYLIST-TYPE:VOD"), "body: {body}");

        // The latest period has no successor, so it is still growing: no
        // ENDLIST (and it stays EVENT-shaped).
        let latest = vod_playlist(
            State(Arc::clone(&route)),
            Path("p1.m3u8".to_string()),
            Query(ProgramSelector::default()),
        )
        .await;
        assert_eq!(latest.status(), StatusCode::OK);
        let body = body_string(latest).await;
        assert!(
            !body.contains("#EXT-X-ENDLIST"),
            "the latest period has no successor, so it must not be finished: {body}"
        );
        assert!(body.contains("#EXT-X-PLAYLIST-TYPE:EVENT"), "body: {body}");

        for (period, init) in [("init-p0.mp4", [0xAA; 4]), ("init-p1.mp4", [0xBB; 4])] {
            let resp = catchup_resource(
                State(Arc::clone(&route)),
                Path(period.to_string()),
                Query(ProgramSelector::default()),
            )
            .await;
            assert_eq!(resp.status(), StatusCode::OK, "{period}");
            assert_eq!(body_bytes(resp).await, init, "{period} serves its own init");
        }

        cleanup(&tmp);
    }

    /// `window_secs` alone decides the EVENT claim when retention removes
    /// nothing: any positive bound removes leading segments (no claim), and
    /// `0` means "no bound" (the head only grows, so EVENT is accurate).
    #[tokio::test]
    async fn window_secs_without_retention_controls_the_event_claim() {
        let tmp = temp_dir();
        let dvr = DvrConfig {
            enabled: true,
            archive_root: tmp.to_string_lossy().to_string(),
            retention_periods: 0,
            retention_bytes: 0,
            ..DvrConfig::default()
        };
        let route = Arc::new(
            RouteHandle::new(
                RouteConfig::default()
                    .with_target_duration(std::time::Duration::from_secs(4))
                    .with_part_target(std::time::Duration::from_millis(500))
                    .with_window_segments(8),
            )
            .with_name("window-no-retention")
            .with_dvr(dvr),
        );
        route.publish_new_program(SPTS_PROGRAM_ID);
        route.set_init(SPTS_PROGRAM_ID, vec![0xAA; 4]);
        for (seq, byte) in [(1u32, 0x11u8), (2, 0x22), (3, 0x33)] {
            route
                .add_segment(SPTS_PROGRAM_ID, seg_bytes(seq, byte))
                .expect("add_segment");
        }
        route.drain_dvr().await;

        let bounded = catchup_playlist(
            State(Arc::clone(&route)),
            Query(CatchupPlaylistQuery {
                window_secs: Some(2),
                program: None,
            }),
        )
        .await;
        assert_eq!(bounded.status(), StatusCode::OK);
        let body = body_string(bounded).await;
        assert!(
            !body.contains("#EXT-X-PLAYLIST-TYPE"),
            "a positive window removes leading segments: {body}"
        );

        let unbounded = catchup_playlist(
            State(route),
            Query(CatchupPlaylistQuery {
                window_secs: Some(0),
                program: None,
            }),
        )
        .await;
        assert_eq!(unbounded.status(), StatusCode::OK);
        let body = body_string(unbounded).await;
        assert!(
            body.contains("#EXT-X-PLAYLIST-TYPE:EVENT"),
            "window_secs=0 means no bound, so the head only grows: {body}"
        );

        cleanup(&tmp);
    }

    /// An init request names an archived period: a period the archive does
    /// not hold, or any period of a TS route (whose archive has no init), is
    /// 404, not an empty 200.
    #[tokio::test]
    async fn an_init_request_for_a_period_without_an_init_is_404() {
        let tmp = temp_dir();
        crate::testutil::record_archive_run(&tmp, "no-init", Container::Fmp4, 0, &[(1, 0x11)]);
        crate::testutil::record_archive_run(&tmp, "ts-init", Container::MpegTs, 0, &[(1, 0x11)]);
        let route_for = |name: &str, container: Container| {
            Arc::new(
                RouteHandle::new(
                    RouteConfig::default()
                        .with_target_duration(std::time::Duration::from_secs(4))
                        .with_part_target(std::time::Duration::from_millis(500))
                        .with_window_segments(8)
                        .with_container(container),
                )
                .with_name(name)
                .with_dvr(dvr_config(&tmp)),
            )
        };
        let fmp4 = route_for("no-init", Container::Fmp4);
        let resp = catchup_resource(
            State(Arc::clone(&fmp4)),
            Path("init-p0.mp4".to_string()),
            Query(ProgramSelector::default()),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(body_bytes(resp).await, b"INIT");
        let resp = catchup_resource(
            State(fmp4),
            Path("init-p1.mp4".to_string()),
            Query(ProgramSelector::default()),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND, "no period 1");
        let ts = route_for("ts-init", Container::MpegTs);
        let resp = catchup_resource(
            State(ts),
            Path("init-p0.mp4".to_string()),
            Query(ProgramSelector::default()),
        )
        .await;
        assert_eq!(
            resp.status(),
            StatusCode::NOT_FOUND,
            "a TS archive has no init"
        );
        cleanup(&tmp);
    }

    /// Each route owns one archive reader, shared by every catch-up request,
    /// and dropping the route releases it with its parsed-playlist cache (the
    /// process-global cache this replaces stayed resident).
    #[test]
    fn a_dropped_route_releases_its_archive_reader() {
        let tmp = temp_dir();
        crate::testutil::record_archive_run(&tmp, "cam", Container::Fmp4, 0, &[(1, 0x11)]);
        let route = RouteHandle::new(RouteConfig::default())
            .with_name("cam")
            .with_dvr(dvr_config(&tmp));
        let archive = route
            .catchup_archive(ProgramArchive::Default)
            .expect("DVR is on");
        assert!(Arc::ptr_eq(
            &archive,
            &route
                .catchup_archive(ProgramArchive::Default)
                .expect("DVR is on")
        ));
        assert_eq!(archive.lock().scan().expect("scan").segments.len(), 1);
        assert_eq!(archive.lock().cached_periods(), 1);
        let released = Arc::downgrade(&archive);
        drop(archive);
        drop(route);
        assert!(
            released.upgrade().is_none(),
            "the reader outlived its route"
        );
        assert!(
            RouteHandle::new(RouteConfig::default())
                .with_name("cam")
                .catchup_archive(ProgramArchive::Default)
                .is_none(),
            "a route without DVR has no archive"
        );
        cleanup(&tmp);
    }

    /// A TS route's catch-up segment URIs carry the `.ts` extension, not the
    /// fMP4 `.m4s`.
    #[tokio::test]
    async fn a_ts_route_advertises_ts_segment_names() {
        assert_eq!(container_ext(Container::MpegTs), "ts");
        assert_eq!(container_ext(Container::Fmp4), "m4s");

        let tmp = temp_dir();
        let route = Arc::new(
            RouteHandle::new(
                RouteConfig::default()
                    .with_target_duration(std::time::Duration::from_secs(4))
                    .with_part_target(std::time::Duration::from_millis(500))
                    .with_window_segments(8)
                    .with_container(Container::MpegTs),
            )
            .with_name("ts-catchup")
            .with_dvr(dvr_config(&tmp)),
        );
        route.publish_new_program(SPTS_PROGRAM_ID);
        route
            .add_segment(SPTS_PROGRAM_ID, seg_bytes(1, 0x11))
            .expect("add_segment");
        route.drain_dvr().await;

        let resp = catchup_playlist(
            State(route),
            Query(CatchupPlaylistQuery {
                window_secs: None,
                program: None,
            }),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_string(resp).await;
        assert!(body.contains("catchup/seg-1.ts"), "body: {body}");
        assert!(!body.contains(".m4s"), "body: {body}");
        cleanup(&tmp);
    }

    /// `pNNN.m3u8` parses to its own period number, and any other name is
    /// refused (period 0 would otherwise serve the wrong segments).
    #[test]
    fn period_filenames_parse_to_their_own_number() {
        assert_eq!(parse_period_filename("p7.m3u8"), Some(7));
        assert_eq!(parse_period_filename("p0.m3u8"), Some(0));
        assert_eq!(parse_period_filename("x.m3u8"), None);
        assert_eq!(parse_period_filename("p7.m4s"), None);
    }

    #[tokio::test]
    async fn vod_playlist_unknown_period_is_404() {
        let tmp = temp_dir();
        let route = Arc::new(
            RouteHandle::new(
                RouteConfig::default()
                    .with_target_duration(std::time::Duration::from_secs(4))
                    .with_part_target(std::time::Duration::from_millis(500))
                    .with_window_segments(8),
            )
            .with_name("vod-missing")
            .with_dvr(dvr_config(&tmp)),
        );
        route.publish_new_program(SPTS_PROGRAM_ID);
        let resp = vod_playlist(
            State(route),
            Path("p99.m3u8".to_string()),
            Query(ProgramSelector::default()),
        )
        .await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
        cleanup(&tmp);
    }

    #[tokio::test]
    async fn catchup_resource_unmatched_filename_404() {
        let route = Arc::new(
            RouteHandle::new(
                RouteConfig::default()
                    .with_target_duration(std::time::Duration::from_secs(4))
                    .with_part_target(std::time::Duration::from_millis(500))
                    .with_window_segments(8),
            )
            .with_name("bad-name"),
        );
        let resp = catchup_resource(
            State(route),
            Path("not-a-segment.txt".to_string()),
            Query(ProgramSelector::default()),
        )
        .await;
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
            RouteHandle::new(
                RouteConfig::default()
                    .with_target_duration(std::time::Duration::from_secs(1))
                    .with_part_target(std::time::Duration::from_millis(250))
                    .with_window_segments(8),
            )
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
        let archived = crate::testutil::scan_archive(&tmp, "c5", Container::Fmp4).segments;
        assert_eq!(
            archived
                .iter()
                .map(|s| (s.period.value(), s.seq.value()))
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
            let resp = catchup_resource(
                State(route.clone()),
                Path(format!("init-p{period}.mp4")),
                Query(ProgramSelector::default()),
            )
            .await;
            assert_eq!(resp.status(), StatusCode::OK, "init-p{period}");
            assert_eq!(body_bytes(resp).await, vec![init; 4], "init-p{period}");
        }
        let missing = catchup_resource(
            State(route.clone()),
            Path("init-p9.mp4".into()),
            Query(ProgramSelector::default()),
        )
        .await;
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);

        // Number 3 is the old run's open segment: skipped, never reissued.
        for (n, expected) in [(1u32, 0x11u8), (2, 0x12), (4, 0x21), (5, 0x22)] {
            let resp = catchup_resource(
                State(route.clone()),
                Path(format!("seg-{n}.m4s")),
                Query(ProgramSelector::default()),
            )
            .await;
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
            RouteHandle::new(
                RouteConfig::default()
                    .with_target_duration(std::time::Duration::from_secs(1))
                    .with_part_target(std::time::Duration::from_millis(250))
                    .with_window_segments(8),
            )
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
            crate::testutil::scan_archive(&tmp, "c5-live", Container::Fmp4)
                .segments
                .iter()
                .map(|s| s.seq.value())
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
        // The live tail's init is the live origin's versioned one: the
        // generation the tail was closed under (the reconnect seeded the
        // previous init as generation 1, so 0xBB is generation 2).
        let live_init = playlist
            .lines()
            .filter_map(|l| l.strip_prefix("#EXT-X-MAP:URI=\""))
            .map(|l| l.trim_end_matches('"'))
            .next_back()
            .expect("a map");
        assert!(
            live_init.starts_with("init-1-") && live_init.ends_with("-2.mp4"),
            "{live_init}"
        );
        let origin = route.ll_hls(SPTS_PROGRAM_ID).expect("origin");
        let init = origin.init_name(TrackId::new(1), hls_runtime::server::Generation::new(2));
        assert_eq!(live_init, init);
        let now = Timestamp::from_nanos(0);
        match origin.resolve(
            HlsRequest::Resource {
                name: live_init.to_string(),
            },
            now,
            AwaitPolicy::new(now),
        ) {
            EgressResponse::Ready {
                body: HlsBody::Resource(bytes),
                ..
            } => assert_eq!(&bytes[..], [0xBB; 4], "the tail's own init"),
            _ => panic!("the live tail's init is not served"),
        }

        // And the tail bytes are the live ones, under the playlist's numbers.
        for (n, expected) in [(4u32, 0x21u8), (5, 0x22)] {
            let resp = catchup_resource(
                State(route.clone()),
                Path(format!("seg-{n}.m4s")),
                Query(ProgramSelector::default()),
            )
            .await;
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

    /// The validator's own `-t` timeout, seconds.
    const VALIDATOR_TIMEOUT_SECS: u64 = 3;
    /// Margin added to `-t` to form the hard kill deadline.
    const VALIDATOR_MARGIN: std::time::Duration = std::time::Duration::from_secs(30);

    fn validator_unavailable() -> Option<&'static str> {
        if !cfg!(target_os = "macos") {
            return Some("`mediastreamvalidator` is macOS-only and this is not macOS");
        }
        if test_fixtures::process::msv::available() {
            None
        } else {
            Some("`mediastreamvalidator` is not on PATH (Additional Tools for Xcode)")
        }
    }

    /// MUST-level (requirement level 1) findings in the validator's JSON.
    fn validator_errors(dir: &std::path::Path, entry: &str) -> Vec<String> {
        let json = test_fixtures::process::msv::MsvRun::new(dir, entry)
            .timeout(std::time::Duration::from_secs(VALIDATOR_TIMEOUT_SECS))
            .deadline(std::time::Duration::from_secs(VALIDATOR_TIMEOUT_SECS) + VALIDATOR_MARGIN)
            .run();
        let compact: String = json
            .to_string()
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
        let fixtures = test_fixtures::path("hls/cmaf-fmp4");
        let read = |name: &str| std::fs::read(fixtures.join(name)).expect("fixture");
        let init_a = read("init.mp4");
        // A different init for the second run: the same, plus a trailing `free`
        // box (valid ISOBMFF, byte-wise different).
        let mut init_b = init_a.clone();
        init_b.extend_from_slice(&[0, 0, 0, 8, b'f', b'r', b'e', b'e']);

        let tmp = temp_dir();
        let route = Arc::new(
            RouteHandle::new(
                RouteConfig::default()
                    .with_target_duration(std::time::Duration::from_secs(1))
                    .with_part_target(std::time::Duration::from_millis(250))
                    .with_window_segments(8),
            )
            .with_name("valid")
            .with_dvr(dvr_config(&tmp)),
        );
        let info = |seq: u32, name: &str| {
            transmux::ll_hls::SegmentInfo::new(read(name), Duration::from_secs(1), seq)
                .with_part_count(1)
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
            let resp = catchup_resource(
                State(route.clone()),
                Path(name.to_string()),
                Query(ProgramSelector::default()),
            )
            .await;
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

    fn archived_seq_periods(root: &std::path::Path, name: &str) -> Vec<(u32, u32)> {
        crate::testutil::scan_archive(root, name, Container::Fmp4)
            .segments
            .iter()
            .map(|s| (s.seq.value(), s.period.value()))
            .collect()
    }

    #[tokio::test]
    async fn a_segment_closed_under_the_old_init_is_archived_with_the_old_init() {
        let root = temp_dir();
        let route = Arc::new(
            RouteHandle::new(
                RouteConfig::default()
                    .with_target_duration(std::time::Duration::from_secs(2))
                    .with_part_target(std::time::Duration::from_millis(500))
                    .with_window_segments(8),
            )
            .with_name("r")
            .with_dvr(dvr_config(&root)),
        );
        route.publish_new_program(SPTS_PROGRAM_ID);
        route.set_init(SPTS_PROGRAM_ID, b"INIT_A".to_vec());
        route
            .add_segment(SPTS_PROGRAM_ID, seg_bytes(1, 0xA1))
            .expect("add");
        route.drain_dvr().await;

        route
            .add_segment(SPTS_PROGRAM_ID, seg_bytes(2, 0xA2))
            .expect("add");
        route.set_init(SPTS_PROGRAM_ID, b"INIT_B".to_vec());
        route
            .add_segment(SPTS_PROGRAM_ID, seg_bytes(3, 0xB3))
            .expect("add");
        route.drain_dvr().await;

        assert_eq!(
            archived_seq_periods(&root, "r"),
            vec![(1, 0), (2, 0), (3, 1)],
            "segment 2 was closed under INIT_A and belongs in the INIT_A period"
        );
        cleanup(&root);
    }

    #[tokio::test]
    async fn segments_closed_before_a_source_reconnect_are_archived() {
        let root = temp_dir();
        let route = Arc::new(
            RouteHandle::new(
                RouteConfig::default()
                    .with_target_duration(std::time::Duration::from_secs(2))
                    .with_part_target(std::time::Duration::from_millis(500))
                    .with_window_segments(8),
            )
            .with_name("r")
            .with_dvr(dvr_config(&root)),
        );
        let first = route.publish_new_program(SPTS_PROGRAM_ID);
        route.set_init(SPTS_PROGRAM_ID, b"INIT_A".to_vec());
        route
            .add_segment(SPTS_PROGRAM_ID, seg_bytes(1, 0xA1))
            .expect("add");
        route.drain_dvr().await;
        route
            .add_segment(SPTS_PROGRAM_ID, seg_bytes(2, 0xA2))
            .expect("add");

        route.release_program(SPTS_PROGRAM_ID, &first);
        route.publish_new_program(SPTS_PROGRAM_ID);
        route.set_init(SPTS_PROGRAM_ID, b"INIT_A".to_vec());
        route
            .add_segment(SPTS_PROGRAM_ID, seg_bytes(1, 0xB1))
            .expect("add");
        route.drain_dvr().await;

        let seqs: Vec<u32> = archived_seq_periods(&root, "r")
            .iter()
            .map(|s| s.0)
            .collect();
        assert_eq!(
            seqs,
            vec![1, 2, 4],
            "segment 2 was closed on the old Trunk before the reconnect"
        );
        cleanup(&root);
    }

    #[tokio::test]
    async fn the_archive_timeline_does_not_go_backwards_across_a_source_reconnect() {
        let root = temp_dir();
        let route = Arc::new(
            RouteHandle::new(
                RouteConfig::default()
                    .with_target_duration(std::time::Duration::from_secs(2))
                    .with_part_target(std::time::Duration::from_millis(500))
                    .with_window_segments(8),
            )
            .with_name("r")
            .with_dvr(dvr_config(&root)),
        );
        let first = route.publish_new_program(SPTS_PROGRAM_ID);
        route.set_init(SPTS_PROGRAM_ID, b"INIT_A".to_vec());
        route
            .add_segment(SPTS_PROGRAM_ID, seg_bytes(1, 0xA1))
            .expect("add");
        route
            .add_segment(SPTS_PROGRAM_ID, seg_bytes(2, 0xA2))
            .expect("add");
        route.drain_dvr().await;

        route.release_program(SPTS_PROGRAM_ID, &first);
        route.publish_new_program(SPTS_PROGRAM_ID);
        route.set_init(SPTS_PROGRAM_ID, b"INIT_A".to_vec());
        route
            .add_segment(SPTS_PROGRAM_ID, seg_bytes(1, 0xB1))
            .expect("add");
        route.drain_dvr().await;

        let starts: Vec<u64> = crate::testutil::scan_archive(&root, "r", Container::Fmp4)
            .segments
            .iter()
            .map(|s| s.start.as_nanos())
            .collect();
        assert!(
            starts.windows(2).all(|w| w[0] < w[1]),
            "an archived segment start must increase across the reconnect: {starts:?}"
        );
        cleanup(&root);
    }
}
