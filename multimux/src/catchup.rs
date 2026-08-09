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
pub(crate) fn read_period_segments(dir: &Path, period_num: u32) -> Vec<ArchivedSegment> {
    let path = dir.join(format!("p{period_num}.idx"));
    let data = match std::fs::read(&path) {
        Ok(d) => d,
        Err(e) => {
            tracing::warn!(
                path = %path.display(),
                error = %e,
                "catch-up: could not read period index"
            );
            return Vec::new();
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
            return Vec::new();
        }
    };
    entries
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
        .collect()
}

/// Every archived segment across every period file in `dir`, ascending by
/// sequence number. Periods are chronological by construction
/// (`crate::dvr::DvrRecorder` only ever opens a higher-numbered period
/// after closing the previous one), so concatenating period-by-period in
/// ascending period order already yields ascending sequence order — no
/// separate sort needed.
pub(crate) fn scan_archive(dir: &Path) -> Vec<ArchivedSegment> {
    list_period_nums(dir)
        .into_iter()
        .flat_map(|n| read_period_segments(dir, n))
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
            .into_iter()
            .find(|s| s.seq == seq)
    })
}

/// Read one archived segment's exact bytes from its period container file.
pub(crate) fn read_archived_bytes(
    dir: &Path,
    ext: &str,
    period_num: u32,
    byte_offset: u64,
    byte_len: u64,
) -> Result<Bytes, String> {
    let path = dir.join(format!("p{period_num}.{ext}"));
    let mut file = File::open(&path).map_err(|e| format!("opening {}: {e}", path.display()))?;
    file.seek(SeekFrom::Start(byte_offset))
        .map_err(|e| format!("seeking {}: {e}", path.display()))?;
    let mut buf = vec![0u8; byte_len as usize];
    file.read_exact(&mut buf)
        .map_err(|e| format!("reading {}: {e}", path.display()))?;
    Ok(Bytes::from(buf))
}

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
) -> String {
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
            duration: s.duration_secs,
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
            writer.publish_segment(dummy_segment(seq, byte));
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
        writer.publish_segment(dummy_segment(1, 0x11));
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
        );
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
        );
        assert!(body.contains("#EXT-X-ENDLIST"), "body: {body}");
        assert!(body.contains("#EXT-X-PLAYLIST-TYPE:VOD"), "body: {body}");
        assert!(
            !body.contains("#EXT-X-MAP"),
            "TS container must not advertise a map: {body}"
        );
    }

    #[test]
    fn render_playlist_empty_segments_uses_minimum_target_duration() {
        let body = render_playlist(&[], "m4s", None, broadcast_hls::PlaylistType::Event, false);
        assert!(body.contains("#EXT-X-TARGETDURATION:1"), "body: {body}");
    }
}
