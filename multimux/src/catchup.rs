//! Read-only reconstruction of a Media Playlist (RFC 8216 §4.3/§4.4) over
//! the DVR durable archive (`crate::dvr`) plus the live `Trunk`'s
//! still-unarchived tail — issue #900's catch-up / time-shift /
//! VOD-from-live serving.
//!
//! # Reading the archive, not re-caching it
//!
//! Per issue #746's hard design constraint (recorded on issue #900 too):
//! `MediaStore` was deleted and the `Trunk` is the single copy of *live*
//! data. This module never builds a second in-memory ring of segments —
//! every function here either scans the archive's on-disk period
//! files/indices fresh (the archive is durable storage; nothing here
//! caches it across requests) or reads the *existing* live window a
//! `hls_runtime::server::HlsOrigin` already maintains
//! ([`hls_runtime::server::HlsOrigin::closed_segments`], which reuses that
//! origin's own cursor rather than opening a second one — see that
//! method's own doc).
//!
//! # The straddle boundary (the reason this module exists)
//!
//! The archive and the live `Trunk` are different sources of the same
//! numbering scheme: `crate::dvr::DvrRecorder` persists each segment under
//! the exact `sequence_number`/`start_pts_ns` the live `Trunk` assigned it
//! (see `crate::dvr::IndexEntry`'s own doc). [`merge_segments`] is the one
//! place that fact is exploited: it concatenates the archive's segments
//! with only the live segments *not yet* archived (strictly greater than
//! the archive's highest sequence number), producing one ascending,
//! gap-free, duplicate-free sequence — never two disjoint lists a client
//! would have to stitch together itself.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;

use bytes::Bytes;
use hls_runtime::server::ClosedSegment;

use crate::dvr::{DvrConfig, IndexEntry};

/// Nanoseconds per second — used to convert `IndexEntry::duration_ns` to
/// the `f64` seconds `broadcast_hls::MediaSegment::duration` wants, and (as
/// `u64`) to convert [`apply_window`]'s `window_secs` into the same
/// nanosecond clock `start_pts_ns` uses.
const NANOS_PER_SEC_U64: u64 = 1_000_000_000;
const NANOS_PER_SEC: f64 = NANOS_PER_SEC_U64 as f64;

/// One archived segment, with enough metadata to render it into a
/// playlist ([`CatchupSegment`]) and locate its exact bytes on disk
/// ([`read_archived_bytes`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ArchivedSegment {
    pub seq: u32,
    pub start_pts_ns: u64,
    pub duration_secs: f64,
    pub discontinuous: bool,
    /// Which period file (`pN.<ext>`) this segment's bytes live in.
    pub period_num: u32,
    pub byte_offset: u64,
    pub byte_len: u64,
}

/// One segment in a rendered catch-up/VOD playlist, agnostic to whether
/// its bytes actually live in the archive or are still resident in the
/// live `Trunk` — [`crate::output::catchup`]'s resource route re-derives
/// that at fetch time; this shape only carries what a playlist needs to
/// render an `#EXTINF` entry.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct CatchupSegment {
    pub seq: u32,
    pub start_pts_ns: u64,
    pub duration_secs: f64,
    pub discontinuous: bool,
}

/// The archive directory for one route: `<archive_root>/<route_name>/` —
/// the exact layout `crate::dvr::DvrRecorder` writes (see that module's own
/// "On-disk layout" doc). `dvr` is expected to already be the
/// [`crate::route::RouteHandle::dvr_config`] accessor's `Some` (i.e.
/// `enabled == true`), but this function itself has no opinion on that —
/// it is a pure path join.
pub(crate) fn archive_dir(dvr: &DvrConfig, route_name: &str) -> PathBuf {
    Path::new(&dvr.archive_root).join(route_name)
}

/// List every period number with a readable index sidecar (`pN.idx`) in
/// `dir`, ascending. A period whose sidecar is missing/unreadable is
/// simply absent from the result (matching `crate::dvr::DvrRecorder`'s own
/// documented posture: a lost index makes that one period's data
/// unusable, not the whole archive).
pub(crate) fn list_period_nums(dir: &Path) -> Vec<u32> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut nums: Vec<u32> = entries
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name();
            let name = name.to_str()?.to_string();
            name.strip_prefix('p')?
                .strip_suffix(".idx")?
                .parse::<u32>()
                .ok()
        })
        .collect();
    nums.sort_unstable();
    nums
}

/// Read and parse one period's index sidecar into [`ArchivedSegment`]s,
/// in the order they were appended (== ascending `seq`, since
/// `crate::dvr::DvrRecorder::append_segment` only ever appends). A
/// missing/corrupt sidecar yields an empty vec (logged) rather than an
/// error — one period's lost data must not make every other period
/// unreadable.
///
/// # Why a cache (audit run 7, W3)
///
/// `GET /catchup.m3u8` reads and JSON-parses **every** `pN.idx` on every
/// request; a 48-hour archive of 30 s segments is tens of thousands of
/// entries re-parsed per playlist fetch, synchronously on a tokio worker.
/// A handful of concurrent unauthenticated requests pinned every worker and
/// stalled ingest for every route. The parsed index of a *finished* period
/// never changes, so it is cached keyed by the sidecar file's
/// `(mtime, len)` — any change (a still-open period being appended to, a
/// rewrite) invalidates just that period's entry. A cache hit is a mutex
/// lock and a couple of comparisons, not a file read.
pub(crate) fn read_period_segments(dir: &Path, period_num: u32) -> Arc<Vec<ArchivedSegment>> {
    let path = dir.join(format!("p{period_num}.idx"));
    let stamp = match std::fs::metadata(&path) {
        Ok(md) => (md.modified().ok(), md.len(), inode_of(&md)),
        Err(e) => {
            tracing::warn!(
                path = %path.display(),
                error = %e,
                "catch-up: could not stat period index"
            );
            return Arc::new(Vec::new());
        }
    };
    let stamp = PeriodStamp {
        mtime_ns: stamp.0,
        len: stamp.1,
        inode: stamp.2,
    };

    let cache = index_cache();
    if let Ok(mut guard) = cache.lock()
        && let Some(entry) = guard.entries.get(&path)
        && entry.stamp == stamp
    {
        // A cheap `Arc` bump — no `Vec` clone at all (issue #1083, item 4).
        let segments = Arc::clone(&entry.segments);
        // LRU touch.
        let mut entry = guard.entries.remove(&path).expect("just looked up");
        entry.last_used = guard.tick;
        guard.tick = guard.tick.wrapping_add(1);
        guard.entries.insert(path.clone(), entry);
        return segments;
    }

    let data = match std::fs::read(&path) {
        Ok(d) => d,
        Err(e) => {
            tracing::warn!(
                path = %path.display(),
                error = %e,
                "catch-up: could not read period index"
            );
            return Arc::new(Vec::new());
        }
    };
    let entries: Vec<IndexEntry> = match serde_json::from_slice(&data) {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!(
                path = %path.display(),
                error = %e,
                "catch-up: could not parse period index"
            );
            return Arc::new(Vec::new());
        }
    };
    let segments: Vec<ArchivedSegment> = entries
        .into_iter()
        .map(|e| ArchivedSegment {
            seq: e.seq,
            start_pts_ns: e.start_pts_ns,
            duration_secs: e.duration_ns as f64 / NANOS_PER_SEC,
            discontinuous: e.discontinuous,
            period_num,
            byte_offset: e.byte_offset,
            byte_len: e.byte_len,
        })
        .collect();

    let segments = Arc::new(segments);
    if let Ok(mut guard) = cache.lock() {
        let tick = guard.tick;
        guard.tick = tick.wrapping_add(1);
        guard.entries.insert(
            path,
            CacheEntry {
                stamp,
                segments: Arc::clone(&segments),
                last_used: tick,
            },
        );
        // LRU eviction (issue #1083, F): drop the least-recently-used entry
        // rather than clearing the whole cache.
        while guard.entries.len() > INDEX_CACHE_MAX_ENTRIES {
            let Some(oldest) = guard
                .entries
                .iter()
                .min_by_key(|(_, e)| e.last_used)
                .map(|(k, _)| k.clone())
            else {
                break;
            };
            guard.entries.remove(&oldest);
        }
    }
    segments
}

/// A file's inode number on Unix, or `0` elsewhere — part of the cache stamp
/// so a same-path, same-length, same-mtime rewrite that actually replaced the
/// file (a new inode) still invalidates (issue #1083, F).
fn inode_of(md: &std::fs::Metadata) -> u64 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        md.ino()
    }
    #[cfg(not(unix))]
    {
        let _ = md;
        0
    }
}

/// A period sidecar's identity for caching: mtime (nanosecond precision),
/// byte length, and inode. Any rewrite changes at least one — and the inode
/// catches a replace-in-place that happens to preserve length and mtime on a
/// coarse clock (issue #1083, F).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PeriodStamp {
    mtime_ns: Option<SystemTime>,
    len: u64,
    inode: u64,
}

struct CacheEntry {
    stamp: PeriodStamp,
    /// `Arc`, so a cache hit is a refcount bump rather than a `Vec` clone.
    segments: Arc<Vec<ArchivedSegment>>,
    /// Monotonic tick of the last use, for LRU eviction.
    last_used: u64,
}

/// Bounded so a long-lived process serving many routes cannot grow it
/// without limit; on overflow the least-recently-used entry is evicted.
const INDEX_CACHE_MAX_ENTRIES: usize = 4096;

#[derive(Default)]
struct IndexCache {
    entries: std::collections::HashMap<PathBuf, CacheEntry>,
    tick: u64,
}

fn index_cache() -> &'static Mutex<IndexCache> {
    static CACHE: OnceLock<Mutex<IndexCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(IndexCache::default()))
}

/// Drop cached indices whose sidecar no longer exists under `dir` — called
/// opportunistically so a deleted/rolled archive does not keep stale
/// entries resident. Correctness never depends on this.
fn prune_index_cache(dir: &Path) {
    if let Ok(mut guard) = index_cache().lock() {
        guard
            .entries
            .retain(|path, _| path.parent() != Some(dir) || path.exists());
    }
}

/// Every archived segment across every period file in `dir`, ascending by
/// sequence number. Periods are chronological by construction
/// (`crate::dvr::DvrRecorder` only ever opens a higher-numbered period
/// after closing the previous one), so concatenating period-by-period in
/// ascending period order already yields ascending sequence order — no
/// separate sort needed.
pub(crate) fn scan_archive(dir: &Path) -> Vec<ArchivedSegment> {
    prune_index_cache(dir);
    list_period_nums(dir)
        .into_iter()
        .flat_map(|n| {
            read_period_segments(dir, n)
                .iter()
                .copied()
                .collect::<Vec<_>>()
        })
        .collect()
}

/// Locate one archived segment's byte range by sequence number, for the
/// resource route ([`crate::output::catchup`]) to serve exactly the bytes
/// a rendered playlist referenced. `O(periods)` — reads every period's
/// index until found; acceptable for an occasional catch-up resource
/// fetch (unlike the live hot path, which never touches this module).
pub(crate) fn find_archived_segment(dir: &Path, seq: u32) -> Option<ArchivedSegment> {
    list_period_nums(dir).into_iter().find_map(|n| {
        read_period_segments(dir, n)
            .iter()
            .find(|s| s.seq == seq)
            .copied()
    })
}

/// Read one archived segment's exact bytes from its period container file.
///
/// `byte_offset`/`byte_len` come from an [`IndexEntry`] in the period's
/// `pN.idx` JSON sidecar — durable disk state, not something this process
/// controls the shape of. A power loss (or any other corruption) can leave
/// that JSON syntactically valid but numerically wrong, e.g. a `byte_len`
/// far larger than the container file ever was. Without a check, `vec![0u8;
/// byte_len as usize]` allocates whatever a corrupt sidecar claims *before*
/// `read_exact` ever gets a chance to fail — an unbounded allocation (OOM)
/// from disk-supplied JSON, the same class of trap the `dvr.rs`
/// `rebuild_index` truncated-`mdat` fix guards against. So the declared
/// range is bound against the file's real length first, and a corrupt entry
/// is rejected before any allocation.
pub(crate) fn read_archived_bytes(
    dir: &Path,
    ext: &str,
    period_num: u32,
    byte_offset: u64,
    byte_len: u64,
) -> Result<Bytes, ReadArchivedError> {
    let path = dir.join(format!("p{period_num}.{ext}"));
    let mut file = File::open(&path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            // The period was evicted between the index read and this open -
            // a race with retention. Report it as "gone" so the caller can
            // answer 404 rather than 500 (issue #1083, item 3).
            ReadArchivedError::Gone
        } else {
            ReadArchivedError::Other(format!("opening {}: {e}", path.display()))
        }
    })?;
    let file_len = file
        .metadata()
        .map_err(|e| ReadArchivedError::Other(format!("stat {}: {e}", path.display())))?
        .len();
    let end = byte_offset.checked_add(byte_len).ok_or_else(|| {
        ReadArchivedError::Other(format!(
            "index entry for {} overflows u64: offset {byte_offset} + len {byte_len}",
            path.display()
        ))
    })?;
    if end > file_len {
        return Err(ReadArchivedError::Other(format!(
            "index entry for {} claims range [{byte_offset}, {end}) but the file is only \
             {file_len} bytes - corrupt or truncated sidecar",
            path.display()
        )));
    }
    file.seek(SeekFrom::Start(byte_offset))
        .map_err(|e| ReadArchivedError::Other(format!("seeking {}: {e}", path.display())))?;
    let mut buf = vec![0u8; byte_len as usize];
    file.read_exact(&mut buf)
        .map_err(|e| ReadArchivedError::Other(format!("reading {}: {e}", path.display())))?;
    Ok(Bytes::from(buf))
}

/// Why [`read_archived_bytes`] failed. `Gone` is distinct so a caller can
/// answer `404` (the period was evicted between the index read and the
/// open) rather than `500` (a corrupt archive) -- issue #1083, item 3.
#[derive(Debug)]
pub(crate) enum ReadArchivedError {
    /// The period file no longer exists (evicted).
    Gone,
    /// Any other failure (corrupt index, truncation, I/O error).
    Other(String),
}

impl std::fmt::Display for ReadArchivedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReadArchivedError::Gone => write!(f, "archived period was evicted"),
            ReadArchivedError::Other(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for ReadArchivedError {}

/// Merge `archived` with the live `Trunk`'s still-unarchived closed tail
/// into ONE ascending, continuous sequence — the straddle fix issue #900
/// exists for. `live` is filtered to sequence numbers strictly greater
/// than `archived`'s highest (or every entry, if `archived` is empty): the
/// DVR recorder's pinning cursor (`crate::dvr::DvrRecorder`) guarantees
/// every segment the live `Trunk` ever closed is either already archived
/// or still resident in `live` — never both absent, and this filter is
/// what keeps it from ever appearing in both halves of the merge at once.
pub(crate) fn merge_segments(
    archived: &[ArchivedSegment],
    live: &[ClosedSegment],
) -> Vec<CatchupSegment> {
    let archive_max_seq = archived.last().map(|s| s.seq);
    let mut combined: Vec<CatchupSegment> = archived
        .iter()
        .map(|s| CatchupSegment {
            seq: s.seq,
            start_pts_ns: s.start_pts_ns,
            duration_secs: s.duration_secs,
            discontinuous: s.discontinuous,
        })
        .collect();
    combined.extend(live.iter().filter_map(|s| {
        let is_tail = match archive_max_seq {
            Some(max) => s.sequence_number > max,
            None => true,
        };
        is_tail.then_some(CatchupSegment {
            seq: s.sequence_number,
            start_pts_ns: s.start_ns,
            duration_secs: s.duration_secs,
            discontinuous: s.discontinuous,
        })
    }));
    combined
}

/// Restrict `combined` (ascending) to the trailing window covering
/// `window_secs` seconds before the last segment's start — the operator-
/// facing "catch-up window" (issue #900), using exactly the
/// `start_pts_ns`/`IndexEntry::start_pts_ns` clock that field's own doc
/// says is "what #900 uses for time-based seek". `None`, or `Some(0)`,
/// returns every segment unfiltered (the whole archive plus live tail —
/// the VOD-from-live shape).
pub(crate) fn apply_window(
    combined: &[CatchupSegment],
    window_secs: Option<u64>,
) -> Vec<CatchupSegment> {
    let Some(window_secs) = window_secs.filter(|&w| w > 0) else {
        return combined.to_vec();
    };
    let Some(edge_ns) = combined.last().map(|s| s.start_pts_ns) else {
        return Vec::new();
    };
    let window_ns = window_secs.saturating_mul(NANOS_PER_SEC_U64);
    let floor_ns = edge_ns.saturating_sub(window_ns);
    combined
        .iter()
        .copied()
        .filter(|s| s.start_pts_ns >= floor_ns)
        .collect()
}

/// Minimum `#EXT-X-TARGETDURATION` (RFC 8216 §4.3.3.1: a positive integer
/// number of seconds) — the floor used when `segments` is empty or every
/// duration rounds to zero.
const MIN_TARGET_DURATION_SECS: u32 = 1;

/// Render `segments` (already ordered/windowed by the caller) into a Media
/// Playlist whose segment URIs are `catchup/seg-{seq}.{ext}` (relative to
/// wherever the playlist itself is served — `crate::output::catchup`
/// mounts the resource route at exactly that path for every stream, so
/// this is correct regardless of which of that module's two playlist
/// endpoints call this).
///
/// `map_uri` is the `#EXT-X-MAP` URI to advertise (RFC 8216bis §4.4.4.5,
/// required for fMP4 — see `hls_runtime::server::Container`'s own doc);
/// `None` for the `MpegTs` container, which needs no map.
pub(crate) fn render_playlist(
    segments: &[CatchupSegment],
    ext: &str,
    map_uri: Option<&str>,
    playlist_type: broadcast_hls::PlaylistType,
    endlist: bool,
) -> std::result::Result<String, broadcast_hls::Error> {
    let target_duration = segments
        .iter()
        .map(|s| s.duration_secs)
        .fold(0.0_f64, f64::max)
        .ceil()
        .max(f64::from(MIN_TARGET_DURATION_SECS)) as u32;
    let media_sequence = segments
        .first()
        .map(|s| u64::from(s.seq))
        .unwrap_or(u64::from(MIN_TARGET_DURATION_SECS));
    let hls_segments: Vec<broadcast_hls::MediaSegment> = segments
        .iter()
        .map(|s| broadcast_hls::MediaSegment {
            uri: format!("catchup/seg-{}.{ext}", s.seq),
            // `duration_secs` is always `duration_ns as f64 / NANOS_PER_SEC`
            // (issue #1140): finite and non-negative for any real
            // segment duration.
            duration: broadcast_hls::DecimalSeconds::new(s.duration_secs)
                .expect("duration_ns / NANOS_PER_SEC is finite, >= 0"),
            discontinuous: s.discontinuous,
            ..Default::default()
        })
        .collect();
    let extra_tags = match map_uri {
        Some(uri) => vec![format!("#EXT-X-MAP:URI=\"{uri}\"")],
        None => Vec::new(),
    };
    let playlist = broadcast_hls::MediaPlaylist {
        target_duration,
        media_sequence,
        segments: hls_segments,
        endlist,
        extra_tags,
        playlist_type: Some(playlist_type),
        ..Default::default()
    };
    playlist.to_m3u8()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dvr::{ArchiveOverrunSerde, DvrRecorder};
    use media_plane::trunk::{SegmentEntry, Trunk, TrunkConfig};
    use std::num::NonZeroUsize;
    use std::time::Duration;

    fn nz(n: usize) -> NonZeroUsize {
        NonZeroUsize::new(n).expect("test capacity must be non-zero")
    }

    fn temp_dir() -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir =
            std::env::temp_dir().join(format!("multimux-catchup-{}-{}", std::process::id(), n));
        let _ = std::fs::create_dir_all(&dir);
        dir
    }

    fn cleanup(dir: &Path) {
        let _ = std::fs::remove_dir_all(dir);
    }

    fn closed(seq: u32, start_ns: u64, duration_secs: f64, discontinuous: bool) -> ClosedSegment {
        ClosedSegment::new(seq, start_ns, duration_secs, discontinuous)
    }

    // --- merge_segments: the straddle fix itself ---

    /// The core bite test for issue #900: an archive holding seq 1..=3 and
    /// a live window that (realistically — the live `HlsOrigin` window and
    /// the archive both drain the same `Trunk`, so they overlap) *also*
    /// still holds seq 1..=4 must merge into exactly ONE continuous
    /// sequence 1..=4, not five entries with seq 3 duplicated.
    ///
    /// MUTATION VERIFIED: changing this function's tail filter from
    /// `s.sequence_number > max` to `s.sequence_number >= max` makes this
    /// test's `assert_eq!(seqs, vec![1, 2, 3, 4])` fail —
    /// `left: [1, 2, 3, 3, 4], right: [1, 2, 3, 4]` — seq 3 appears twice
    /// because the live copy that duplicates an already-archived segment
    /// is no longer excluded. This is exactly the "two disjoint playlists"
    /// failure mode the issue calls out; a client stitching this into a
    /// playlist would see the same segment twice. Recompiled and re-run to
    /// confirm the failure, then reverted.
    #[test]
    fn merge_segments_excludes_archived_segments_from_live_tail() {
        let archived = vec![
            ArchivedSegment {
                seq: 1,
                start_pts_ns: 0,
                duration_secs: 2.0,
                discontinuous: false,
                period_num: 0,
                byte_offset: 0,
                byte_len: 10,
            },
            ArchivedSegment {
                seq: 2,
                start_pts_ns: 2_000_000_000,
                duration_secs: 2.0,
                discontinuous: false,
                period_num: 0,
                byte_offset: 10,
                byte_len: 10,
            },
            ArchivedSegment {
                seq: 3,
                start_pts_ns: 4_000_000_000,
                duration_secs: 2.0,
                discontinuous: false,
                period_num: 0,
                byte_offset: 20,
                byte_len: 10,
            },
        ];
        // The live window still has 1..=4 too — this is the realistic
        // shape (both the archive and the live window drain the SAME
        // Trunk; the archive lagging by one poll cycle is the norm, not
        // an edge case).
        let live = vec![
            closed(1, 0, 2.0, false),
            closed(2, 2_000_000_000, 2.0, false),
            closed(3, 4_000_000_000, 2.0, false),
            closed(4, 6_000_000_000, 2.0, false),
        ];

        let combined = merge_segments(&archived, &live);
        let seqs: Vec<u32> = combined.iter().map(|s| s.seq).collect();
        assert_eq!(
            seqs,
            vec![1, 2, 3, 4],
            "archive + live must merge into one continuous sequence, no duplicates"
        );
    }

    #[test]
    fn merge_segments_empty_archive_uses_every_live_segment() {
        let live = vec![
            closed(5, 0, 1.0, false),
            closed(6, 1_000_000_000, 1.0, true),
        ];
        let combined = merge_segments(&[], &live);
        let seqs: Vec<u32> = combined.iter().map(|s| s.seq).collect();
        assert_eq!(seqs, vec![5, 6]);
        assert!(combined[1].discontinuous);
    }

    // --- apply_window ---

    #[test]
    fn apply_window_keeps_only_the_trailing_seconds() {
        let combined = vec![
            CatchupSegment {
                seq: 1,
                start_pts_ns: 0,
                duration_secs: 2.0,
                discontinuous: false,
            },
            CatchupSegment {
                seq: 2,
                start_pts_ns: 10_000_000_000,
                duration_secs: 2.0,
                discontinuous: false,
            },
            CatchupSegment {
                seq: 3,
                start_pts_ns: 20_000_000_000,
                duration_secs: 2.0,
                discontinuous: false,
            },
        ];
        // Window of 5s before the last segment's start (20s) => floor 15s
        // => only segment 3 (20s) survives.
        let windowed = apply_window(&combined, Some(5));
        let seqs: Vec<u32> = windowed.iter().map(|s| s.seq).collect();
        assert_eq!(
            seqs,
            vec![3],
            "only the trailing 5s window must survive: {seqs:?}"
        );
    }

    #[test]
    fn apply_window_none_or_zero_returns_everything() {
        let combined = vec![CatchupSegment {
            seq: 1,
            start_pts_ns: 0,
            duration_secs: 2.0,
            discontinuous: false,
        }];
        assert_eq!(apply_window(&combined, None).len(), 1);
        assert_eq!(apply_window(&combined, Some(0)).len(), 1);
    }

    // --- scan_archive / read_archived_bytes against a REAL DvrRecorder
    //     fixture (not hand-crafted JSON) ---

    fn dummy_segment(seq: u32, byte: u8) -> SegmentEntry {
        SegmentEntry::new(
            bytes::Bytes::from(vec![byte; 24]),
            seq,
            Duration::from_secs(3),
            broadcast_common::Timestamp::from_nanos(u64::from(seq) * 3_000_000_000),
            transmux::SegmentMeta {
                discontinuous: seq == 2,
            },
        )
    }

    /// Records three real segments through the actual `DvrRecorder`
    /// (exactly the fixture `dvr.rs`'s own tests use), then proves
    /// `scan_archive`/`read_archived_bytes` recover byte-exact,
    /// metadata-exact segments purely by reading what `DvrRecorder`
    /// wrote to disk — never from any state shared in-process with the
    /// recorder.
    fn recorder_cfg(tmp: &Path) -> DvrConfig {
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

    #[test]
    fn scan_archive_recovers_real_recorder_output_byte_exact() {
        let tmp = temp_dir();
        let trunk = Trunk::new(TrunkConfig::new(nz(4), nz(4), nz(8), nz(4), nz(4)));
        let writer = trunk.segment_writer().expect("segment writer");
        let mut recorder =
            DvrRecorder::new("straddle".to_string(), recorder_cfg(&tmp), ".m4s", &trunk)
                .expect("recorder");

        let init = b"REAL_INIT";
        recorder.poll_and_persist(Some(init)).expect("poll init");
        for (seq, byte) in [(1u32, 0xAAu8), (2, 0xBB), (3, 0xCC)] {
            writer.publish_segment(dummy_segment(seq, byte)).unwrap();
        }
        recorder.poll_and_persist(Some(init)).expect("persist");

        let dir = archive_dir(&recorder_cfg(&tmp), "straddle");
        let archived = scan_archive(&dir);
        assert_eq!(archived.len(), 3);
        for (i, seg) in archived.iter().enumerate() {
            let seq = i as u32 + 1;
            assert_eq!(seg.seq, seq);
            assert_eq!(
                seg.duration_secs, 3.0,
                "seq {seq} duration must be real, not a shape"
            );
            assert_eq!(
                seg.discontinuous,
                seq == 2,
                "seq {seq} discontinuous bit must match what was published"
            );
            let expected_byte = match seq {
                1 => 0xAAu8,
                2 => 0xBB,
                3 => 0xCC,
                _ => unreachable!(),
            };
            let bytes =
                read_archived_bytes(&dir, "m4s", seg.period_num, seg.byte_offset, seg.byte_len)
                    .expect("read archived bytes");
            assert_eq!(
                bytes.as_ref(),
                vec![expected_byte; 24].as_slice(),
                "seq {seq} bytes must be byte-exact with what DvrRecorder wrote"
            );
        }

        cleanup(&tmp);
    }

    #[test]
    fn find_archived_segment_locates_the_right_period_and_range() {
        let tmp = temp_dir();
        let trunk = Trunk::new(TrunkConfig::new(nz(4), nz(4), nz(8), nz(4), nz(4)));
        let writer = trunk.segment_writer().expect("segment writer");
        let mut recorder = DvrRecorder::new("find".to_string(), recorder_cfg(&tmp), ".m4s", &trunk)
            .expect("recorder");
        let init = b"INIT";
        recorder.poll_and_persist(Some(init)).expect("poll init");
        writer.publish_segment(dummy_segment(1, 0x11)).unwrap();
        recorder.poll_and_persist(Some(init)).expect("persist");

        let dir = archive_dir(&recorder_cfg(&tmp), "find");
        let found = find_archived_segment(&dir, 1).expect("segment 1 must be found");
        assert_eq!(found.period_num, 0);
        let bytes = read_archived_bytes(
            &dir,
            "m4s",
            found.period_num,
            found.byte_offset,
            found.byte_len,
        )
        .expect("read bytes");
        assert_eq!(bytes.as_ref(), vec![0x11u8; 24].as_slice());

        assert!(find_archived_segment(&dir, 99).is_none());

        cleanup(&tmp);
    }

    /// Biting test: a `byte_len` a corrupt (but JSON-valid) `pN.idx` sidecar
    /// could claim — here, ~9.1 TB, far beyond any real period file but
    /// nowhere near overflowing `u64` on its own — must be rejected with a
    /// clear error *before* `read_archived_bytes` ever allocates a buffer
    /// for it, not discovered only once `read_exact` under-runs the real
    /// file.
    ///
    /// MUTATION VERIFIED: removing the `end > file_len` bounds check (so the
    /// function goes straight from `file.metadata()` to
    /// `vec![0u8; byte_len as usize]`) makes this test fail — not with the
    /// expected `Err`, but with the process itself attempting a ~9.1 TB
    /// allocation (an `AllocError`/OOM abort on most machines, rather than a
    /// graceful `Err`) before `expect_err` ever runs, i.e. exactly the
    /// unbounded-allocation defect this test exists to catch. Recompiled and
    /// re-run to confirm the mutated build no longer returns the bounds-check
    /// `Err` (it hangs/aborts attempting the allocation instead), then
    /// restored the guard.
    #[test]
    fn read_archived_bytes_rejects_a_byte_len_the_file_cannot_back() {
        let tmp = temp_dir();
        let trunk = Trunk::new(TrunkConfig::new(nz(4), nz(4), nz(8), nz(4), nz(4)));
        let writer = trunk.segment_writer().expect("segment writer");
        let mut recorder =
            DvrRecorder::new("corrupt".to_string(), recorder_cfg(&tmp), ".m4s", &trunk)
                .expect("recorder");
        let init = b"INIT";
        recorder.poll_and_persist(Some(init)).expect("poll init");
        writer.publish_segment(dummy_segment(1, 0x11)).unwrap();
        recorder.poll_and_persist(Some(init)).expect("persist");

        let dir = archive_dir(&recorder_cfg(&tmp), "corrupt");
        let found = find_archived_segment(&dir, 1).expect("segment 1 must be found");

        // Simulate a corrupt sidecar: claim a length the period file cannot
        // possibly back, but not so large it would overflow u64 on its own
        // (that's a separate, already-guarded case) — a plausible corrupt
        // value, not an adversarial one.
        let implausible_len = 9_100_000_000_000u64; // ~9.1 TB
        let err = read_archived_bytes(
            &dir,
            "m4s",
            found.period_num,
            found.byte_offset,
            implausible_len,
        )
        .expect_err("a byte_len the file cannot back must be rejected, not allocated");
        assert!(
            err.to_string().contains("corrupt or truncated sidecar"),
            "expected the bounds-check error, got: {err}"
        );

        cleanup(&tmp);
    }

    // --- render_playlist ---

    #[test]
    fn render_playlist_renders_real_segment_numbers_and_map() {
        let segments = vec![
            CatchupSegment {
                seq: 5,
                start_pts_ns: 0,
                duration_secs: 3.4,
                discontinuous: false,
            },
            CatchupSegment {
                seq: 6,
                start_pts_ns: 3_400_000_000,
                duration_secs: 3.4,
                discontinuous: true,
            },
        ];
        let body = render_playlist(
            &segments,
            "m4s",
            Some("init-1.mp4"),
            broadcast_hls::PlaylistType::Event,
            false,
        )
        .expect("generated URIs are valid");
        assert!(body.contains("#EXT-X-MEDIA-SEQUENCE:5"), "body: {body}");
        assert!(body.contains("#EXT-X-TARGETDURATION:4"), "body: {body}");
        assert!(body.contains("catchup/seg-5.m4s"), "body: {body}");
        assert!(body.contains("catchup/seg-6.m4s"), "body: {body}");
        assert!(
            body.contains("#EXT-X-MAP:URI=\"init-1.mp4\""),
            "body: {body}"
        );
        assert!(body.contains("#EXT-X-DISCONTINUITY\n"), "body: {body}");
        assert!(!body.contains("#EXT-X-ENDLIST"), "body: {body}");
        assert!(body.contains("#EXT-X-PLAYLIST-TYPE:EVENT"), "body: {body}");
    }

    #[test]
    fn render_playlist_vod_finished_emits_endlist_and_vod_type() {
        let segments = vec![CatchupSegment {
            seq: 1,
            start_pts_ns: 0,
            duration_secs: 2.0,
            discontinuous: false,
        }];
        let body = render_playlist(
            &segments,
            "ts",
            None,
            broadcast_hls::PlaylistType::Vod,
            true,
        )
        .expect("generated URIs are valid");
        assert!(body.contains("#EXT-X-ENDLIST"), "body: {body}");
        assert!(body.contains("#EXT-X-PLAYLIST-TYPE:VOD"), "body: {body}");
        assert!(
            !body.contains("#EXT-X-MAP"),
            "TS container must not advertise a map: {body}"
        );
    }

    // --- index cache (audit run 7, W3) ---

    /// `read_period_segments` caches a *finished* period's parse and serves
    /// the cached copy on a repeat call, but a sidecar whose contents change
    /// (a still-open period being appended to) must be re-read — a stale
    /// parse would render a playlist with the wrong segment list.
    ///
    /// Biting test: drop the `(mtime, len)` stamp comparison (always serve
    /// the cached entry) and the second assertion sees the stale 1-segment
    /// list, failing.
    #[test]
    fn period_index_cache_is_invalidated_when_the_sidecar_changes() {
        let dir = temp_dir();
        let idx = dir.join("p0.idx");

        let one = serde_json::to_vec(&vec![IndexEntry {
            seq: 7,
            start_pts_ns: 0,
            byte_offset: 0,
            byte_len: 4,
            duration_ns: 1_000_000_000,
            discontinuous: false,
        }])
        .unwrap();
        std::fs::write(&idx, &one).unwrap();
        let first = read_period_segments(&dir, 0);
        assert_eq!(
            first.iter().map(|s| s.seq).collect::<Vec<_>>(),
            vec![7],
            "the first read must reflect the on-disk sidecar"
        );

        // Rewrite with two entries. Bump the length so the stamp differs
        // even if the mtime clock is coarse.
        let two = serde_json::to_vec(&vec![
            IndexEntry {
                seq: 7,
                start_pts_ns: 0,
                byte_offset: 0,
                byte_len: 4,
                duration_ns: 1_000_000_000,
                discontinuous: false,
            },
            IndexEntry {
                seq: 8,
                start_pts_ns: 1_000_000_000,
                byte_offset: 4,
                byte_len: 4,
                duration_ns: 1_000_000_000,
                discontinuous: false,
            },
        ])
        .unwrap();
        std::fs::write(&idx, &two).unwrap();

        let second = read_period_segments(&dir, 0);
        assert_eq!(
            second.iter().map(|s| s.seq).collect::<Vec<_>>(),
            vec![7, 8],
            "a changed sidecar must invalidate the cache, not serve the stale parse"
        );

        cleanup(&dir);
    }

    /// A rewrite that keeps the SAME byte length (e.g. one `byte_len` value
    /// edited in place) must still invalidate the cache — a length-only or
    /// coarse-mtime key would serve the stale parse (issue #1083, F).
    ///
    /// Biting test: revert the stamp to `(mtime, len)` without the inode and
    /// drop the nanosecond/`len` sensitivity; a same-length in-place rewrite
    /// within one mtime tick would then serve the old entry. Here the stamp
    /// also covers `len`, so a same-length rewrite is caught by the inode
    /// when the file is replaced; this asserts the cache re-reads it
    /// regardless.
    #[test]
    fn period_index_cache_invalidates_a_same_length_rewrite() {
        let dir = temp_dir();
        let idx = dir.join("p0.idx");
        let entry = |seq: u32| {
            serde_json::to_vec(&vec![IndexEntry {
                seq,
                start_pts_ns: 0,
                byte_offset: 0,
                byte_len: 4,
                duration_ns: 1_000_000_000,
                discontinuous: false,
            }])
            .unwrap()
        };
        // Write a 1-digit seq, then rewrite with a different 1-digit seq —
        // identical byte length.
        let a = entry(7);
        std::fs::write(&idx, &a).unwrap();
        assert_eq!(
            read_period_segments(&dir, 0)
                .iter()
                .map(|s| s.seq)
                .collect::<Vec<_>>(),
            vec![7]
        );

        let b = entry(8);
        assert_eq!(a.len(), b.len(), "the rewrite must be the same length");
        // The DVR writes indexes write-then-rename (a NEW inode). Simulate
        // that AND restore the original mtime, so `(mtime_ns, len)` alone
        // cannot tell the files apart — only the inode can.
        let mtime_before = std::fs::metadata(&idx).unwrap().modified().unwrap();
        let tmp2 = dir.join("p0.idx.new");
        std::fs::write(&tmp2, &b).unwrap();
        std::fs::rename(&tmp2, &idx).unwrap();
        let f = std::fs::OpenOptions::new().write(true).open(&idx).unwrap();
        f.set_modified(mtime_before).unwrap();
        drop(f);

        assert_eq!(
            read_period_segments(&dir, 0)
                .iter()
                .map(|s| s.seq)
                .collect::<Vec<_>>(),
            vec![8],
            "a same-length rewrite must invalidate the cache"
        );
        cleanup(&dir);
    }

    /// Item 3: a period whose data file was evicted between the index read and
    /// the open must report `Gone` (mapped to a 404), not a generic error (a
    /// 500).
    ///
    /// Biting test: return `ReadArchivedError::Other` for a missing file and
    /// this fails.
    #[test]
    fn read_archived_bytes_reports_an_evicted_period_as_gone() {
        let dir = temp_dir();
        let missing = read_archived_bytes(&dir, "m4s", 0, 0, 4);
        assert!(
            matches!(missing, Err(ReadArchivedError::Gone)),
            "a missing period file must be Gone, got {missing:?}"
        );
        let _ = dir;
    }

    /// A missing sidecar yields an empty list and never poisons the cache —
    /// a later write of that same path is read fresh.
    #[test]
    fn period_index_cache_recovers_after_a_missing_then_written_sidecar() {
        let dir = temp_dir();
        let idx = dir.join("p3.idx");
        assert!(
            read_period_segments(&dir, 3).is_empty(),
            "no sidecar: empty"
        );

        std::fs::write(
            &idx,
            serde_json::to_vec(&vec![IndexEntry {
                seq: 42,
                start_pts_ns: 0,
                byte_offset: 0,
                byte_len: 4,
                duration_ns: 1_000_000_000,
                discontinuous: false,
            }])
            .unwrap(),
        )
        .unwrap();
        assert_eq!(
            read_period_segments(&dir, 3)
                .iter()
                .map(|s| s.seq)
                .collect::<Vec<_>>(),
            vec![42],
            "a sidecar written after a miss must be read"
        );

        cleanup(&dir);
    }

    #[test]
    fn render_playlist_empty_segments_uses_minimum_target_duration() {
        let body = render_playlist(&[], "m4s", None, broadcast_hls::PlaylistType::Event, false)
            .expect("empty playlist renders");
        assert!(body.contains("#EXT-X-TARGETDURATION:1"), "body: {body}");
    }
}
