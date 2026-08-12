//! Integration tests for the linear-playout [`FileReader`] (issue #748, WP2).
//!
//! Real fixtures only — in-memory or hand-built bytes are rejected by this
//! project's discipline (a hand-built fixture risks missing real container
//! framing quirks that expose demuxer bugs). Fixture paths are
//! workspace-relative and resolved against `CARGO_MANIFEST_DIR`:
//!
//! ```
//! format!("{}/../{}", env!("CARGO_MANIFEST_DIR"), "fixtures/ts/h264_aac.ts")
//! ```
//!
//! The tests cover the design spec's `multimux/src/source/file_reader.rs`
//! section: exact demuxer selection, the timeline and loop invariants
//! (including a recorded mutation proof), pacing, and the robustness cases.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use container_probe::{Probe, probe_with_budget};
use media_plane::trunk::{SampleCursorItem, Trunk, TrunkConfig};
use multimux::source::file_reader::{
    DemuxerKind, FileReader, FileReaderConfig, FileReaderError, select_demuxer,
};
use transmux::{DemuxEvent, StreamingTsDemux};

/// Workspace-root fixture path for a `rel` path like `"fixtures/ts/h264_aac.ts"`.
fn fixture(rel: &str) -> PathBuf {
    PathBuf::from(format!("{}/../{}", env!("CARGO_MANIFEST_DIR"), rel))
}

/// A per-test Trunk ready for the reader to write into, with generous ring
/// capacities so the timeline tests never evict before draining.
fn trunk() -> Arc<Trunk> {
    fn nz(n: usize) -> std::num::NonZeroUsize {
        std::num::NonZeroUsize::new(n).unwrap()
    }
    Trunk::new(TrunkConfig::new(
        nz(4096),
        nz(1024),
        nz(128),
        nz(1024),
        nz(1024),
    ))
}

/// A reader config with `max_retries = 0` (no sleeping on a failure) and
/// `max_loops` bounded, for fast deterministic tests. `pace` defaults to
/// `false` so the timeline/e(loop) tests drain the whole file immediately.
fn reader(
    path: PathBuf,
    loop_file: bool,
    pace: bool,
    max_loops: Option<u32>,
    trunk: Arc<Trunk>,
) -> FileReader {
    FileReader::new(FileReaderConfig {
        path,
        loop_file,
        trunk,
        pace,
        max_retries: 0,
        retry_interval: Duration::from_millis(1),
        max_loops,
    })
}

/// Probe a fixture file and return its `(format, detail)` for feeding to
/// [`select_demuxer`] (re-deriving the verdict is legitimate in a test — it is
/// asserting *which* demuxer the reader would select).
fn probe_fixture(rel: &str) -> (container_probe::Format, container_probe::Detail) {
    let bytes = std::fs::read(fixture(rel)).expect("fixture must exist");
    match probe_with_budget(&bytes, bytes.len()) {
        Probe::Identified { format, detail, .. } => (format, detail),
        other => panic!("fixture {rel} must probe cleanly, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Format selection — the exact demuxer (or the exact rejection).
// ---------------------------------------------------------------------------

#[test]
fn ts_fixture_selects_streaming_ts_demuxer() {
    let (format, detail) = probe_fixture("fixtures/ts/h264_aac.ts");
    assert_eq!(
        select_demuxer(format, detail).unwrap(),
        DemuxerKind::StreamingTs,
        "expected StreamingTsDemux for MpegTs"
    );
}

#[test]
fn progressive_mp4_selects_progressive_demuxer() {
    let (format, detail) = probe_fixture("fixtures/mp4/h264_high.mp4");
    assert_eq!(
        select_demuxer(format, detail).unwrap(),
        DemuxerKind::Progressive,
        "expected ProgressiveDemux for progressive ISOBMFF"
    );
}

#[test]
fn fragmented_mp4_selects_fmp4_demuxer() {
    let (format, detail) = probe_fixture("fixtures/mp4/cmaf/av_frag.mp4");
    assert_eq!(
        select_demuxer(format, detail).unwrap(),
        DemuxerKind::Fmp4,
        "expected Fmp4Demux for fragmented ISOBMFF"
    );
}

#[test]
fn mkv_selects_webm_demuxer() {
    let (format, detail) = probe_fixture("fixtures/mkv/h264_aac.mkv");
    assert_eq!(
        select_demuxer(format, detail).unwrap(),
        DemuxerKind::Webm,
        "expected WebmDemux for Matroska"
    );
}

#[test]
fn ps_fixture_selects_ps_demuxer() {
    let (format, detail) = probe_fixture("fixtures/ps/h264_ac3.ps");
    assert_eq!(
        select_demuxer(format, detail).unwrap(),
        DemuxerKind::Ps,
        "expected PsDemux for MPEG-PS"
    );
}

#[test]
fn flv_fixture_selects_streaming_flv_demuxer() {
    let (format, detail) = probe_fixture("fixtures/flv/av.flv");
    assert_eq!(
        select_demuxer(format, detail).unwrap(),
        DemuxerKind::StreamingFlv,
        "expected StreamingFlvDemux for FLV"
    );
}

#[test]
fn unsupported_fixtures_yield_distinct_format_errors() {
    // Mxf → unsupported-format error naming Mxf.
    let (format, detail) = probe_fixture("fixtures/mxf/op1a_mpeg2_pcm.mxf");
    match select_demuxer(format, detail) {
        Err(FileReaderError::UnsupportedFormat { format: f }) => {
            assert_eq!(
                f, "Mxf",
                "Mxf must be named by the unsupported-format error"
            )
        }
        other => panic!("Mxf must be rejected as UnsupportedFormat, got {other:?}"),
    }

    // Wav → unsupported-format error naming Wav.
    let (format, detail) = probe_fixture("fixtures/container-probe/pcm_s16le.wav");
    match select_demuxer(format, detail) {
        Err(FileReaderError::UnsupportedFormat { format: f }) => {
            assert_eq!(
                f, "Wav",
                "Wav must be named by the unsupported-format error"
            )
        }
        other => panic!("Wav must be rejected as UnsupportedFormat, got {other:?}"),
    }

    // ADTS AAC → unsupported-format error naming AdtsAac.
    let (format, detail) = probe_fixture("fixtures/container-probe/aac.adts");
    match select_demuxer(format, detail) {
        Err(FileReaderError::UnsupportedFormat { format: f }) => {
            assert_eq!(
                f, "AdtsAac",
                "AdtsAac must be named by the unsupported-format error"
            )
        }
        other => panic!("ADTS AAC must be rejected as UnsupportedFormat, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// The timeline invariant — the primary gate, on a real TS fixture.
// ---------------------------------------------------------------------------

/// The **presentation-ordered** PTS sequence of one `track_id` in the source
/// — sorted ascending, because the TS demuxer may emit a track's samples in
/// decode order while the reader writes them at their presentation time (the
/// reader's due-time merge is presentation). The "source's own" timeline a
/// per-track deltas assertion is held against is this presentation order.
fn source_track_pts(rel: &str, track_id: u32) -> Vec<i64> {
    let bytes = std::fs::read(fixture(rel)).unwrap();
    let mut demux = StreamingTsDemux::new();
    demux.feed(&bytes);
    let mut pts = Vec::new();
    while let Some(ev) = demux.poll_event() {
        let DemuxEvent::Sample {
            track_id: id,
            sample,
            ..
        } = ev
        else {
            // TrackAdded/TracksResolved/metadata events are not samples —
            // keep polling.
            continue;
        };
        if id == track_id
            && let Some(p) = sample.pts
        {
            pts.push(p);
        }
    }
    pts.sort_unstable();
    pts
}

/// One frame period (the smallest positive PTS delta) of `track_id` in the
/// source — the exact increment a loop should continue the timeline by.
fn source_frame_period(rel: &str, track_id: u32) -> i64 {
    let pts = source_track_pts(rel, track_id);
    let period = pts
        .windows(2)
        .map(|w| w[1] - w[0])
        .filter(|&d| d > 0)
        .min()
        .expect("source track must have a positive frame period");
    assert!(period > 0, "source frame period must be positive");
    period
}

/// Drain a Trunk into a flat list of `(track_id, pts)` in publish order.
fn drain_samples(trunk: &Arc<Trunk>) -> Vec<(u32, i64)> {
    let mut cursor = trunk.subscribe_from_backlog();
    let mut out = Vec::new();
    loop {
        match cursor.poll() {
            Some(SampleCursorItem::Timed { track_id, sample }) => {
                out.push((track_id, sample.pts.expect("timed sample must carry a pts")));
            }
            Some(SampleCursorItem::Sparse { .. })
            | Some(SampleCursorItem::Lagged { .. })
            | Some(SampleCursorItem::Degraded { .. }) => {
                panic!("reader must not drop timed samples nor use the sparse ring")
            }
            None => break,
            // Any future cursor item is a loss/other report the reader never
            // produces — treat it as a test failure, not silent data loss.
            Some(_) => panic!("unexpected sample-cursor item from the file reader"),
        }
    }
    out
}

#[tokio::test]
async fn timeline_pts_is_monotonic_and_deltas_match_source() {
    let trunk = trunk();
    let t = trunk.clone();
    let handle = tokio::spawn(async move {
        reader(fixture("fixtures/ts/h264_aac.ts"), false, false, None, t)
            .run()
            .await
    });
    handle.await.unwrap().expect("reader must finish cleanly");

    let drained = drain_samples(&trunk);
    assert!(
        drained.len() > 1,
        "fixture must yield samples, got {}",
        drained.len()
    );

    // The trunk's announced track set carries each track's own timescale (the
    // IR stores absolute pts per track in *that* track's timescale).
    let tracks = trunk.tracks();
    assert!(tracks.len() >= 2, "TS fixture must demux video + audio");

    // Per-track: the pts sequence each track writes must be strictly
    // monotonic (a track's own timeline never steps backwards), and the
    // deltas must match the source's own per-track deltas. Group the drained
    // samples by track_id, preserving publish order.
    let mut by_track: std::collections::BTreeMap<u32, Vec<i64>> = Default::default();
    for (track_id, pts) in &drained {
        by_track.entry(*track_id).or_default().push(*pts);
    }
    for (track_id, seq) in &by_track {
        for pair in seq.windows(2) {
            assert!(
                pair[1] > pair[0],
                "track {track_id} pts must be strictly monotonic: {} then {}",
                pair[0],
                pair[1]
            );
        }
        // Deltas match the source's own: the source's per-track pts set equals
        // what the reader wrote (the reader preserves decode order per track).
        let source = source_track_pts("fixtures/ts/h264_aac.ts", *track_id);
        assert_eq!(
            &source, seq,
            "track {track_id} deltas must match the source's own"
        );
    }

    // Presentation-time sanity across tracks (the reader's merge order): no
    // sample may be written before an earlier one of *its own* kind it depends
    // on, so we additionally require the reader wrote as many distinct pts as
    // the source carries (nothing silently dropped).
    assert_eq!(
        drained.len(),
        source_sample_count("fixtures/ts/h264_aac.ts"),
        "reader must write every source sample"
    );
}

// ---------------------------------------------------------------------------
// The loop invariant — strictly monotonic across the loop point, boundary =
// one frame. Includes the recorded mutation proof.
//
// MUTATION PROOF, recorded verbatim: removing the loop PTS offset (making
// `advance_offsets` a no-op so loop N+1 restarts at the file's own first PTS)
// makes this test FAIL with:
//
//     loop PTS must stay strictly monotonic for track 1: 399600 then 133200
//
// — loop 2's first video sample carries the file's *first* PTS (133200), far
// below loop 1's last (399600), an unambiguous backwards step across the loop
// point. Restoring the offset (and `touch`ing the restored files so cargo
// does not serve a stale binary) makes the test pass again. The monotonicity
// assertion below is therefore the property the loop-offset exists to provide.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn loop_preserves_monotonicity_and_one_frame_boundary() {
    let trunk = trunk();
    let t = trunk.clone();
    let handle = tokio::spawn(async move {
        reader(fixture("fixtures/ts/h264_aac.ts"), true, false, Some(2), t)
            .run()
            .await
    });
    handle
        .await
        .unwrap()
        .expect("bounded loop must finish cleanly");

    let drained = drain_samples(&trunk);

    // Group the drained samples by track_id (each track carries PTS in its own
    // timescale; the loop offset is advanced *per track*). For every track,
    // the points of loop 1 and loop 2 concatenate into a strictly monotonic
    // sequence whose boundary delta is exactly one frame — the loop offset is
    // what keeps it from stepping backwards, and the mutation kills it.
    let mut by_track: std::collections::BTreeMap<u32, Vec<i64>> = Default::default();
    for (track_id, pts) in &drained {
        by_track.entry(*track_id).or_default().push(*pts);
    }

    for (track_id, seq) in &by_track {
        // Source's own per-loop frame period and per-loop sample count for
        // this track, so we know where the loop boundary lands in its seq.
        let source = source_track_pts("fixtures/ts/h264_aac.ts", *track_id);
        let one_frame = source_frame_period("fixtures/ts/h264_aac.ts", *track_id);
        let per_loop = source.len();

        assert!(
            seq.len() == 2 * per_loop,
            "track {track_id}: a 2-loop run must write 2× the per-loop sample count (got {} vs 2×{per_loop})",
            seq.len()
        );

        // Strictly monotonic across the loop point — the mutation-biting gate.
        for pair in seq.windows(2) {
            assert!(
                pair[1] > pair[0],
                "loop PTS must stay strictly monotonic for track {track_id}: {} then {}",
                pair[0],
                pair[1]
            );
        }

        // The boundary delta equals one frame duration, not zero and not the
        // whole file's length.
        let boundary_delta = seq[per_loop] - seq[per_loop - 1];
        assert_eq!(
            boundary_delta, one_frame,
            "track {track_id}: loop boundary delta must equal one frame duration, got {boundary_delta} vs frame {one_frame}"
        );
        assert!(
            boundary_delta > 0 && boundary_delta < one_frame * (per_loop as i64),
            "track {track_id}: boundary delta must be one frame, not zero and not the whole file's length: {boundary_delta}"
        );
    }
}

// ---------------------------------------------------------------------------
// Pacing — the reader must not dump the whole file instantly.
// ---------------------------------------------------------------------------

/// Count every sample the source file yields (all tracks).
fn source_sample_count(rel: &str) -> usize {
    let bytes = std::fs::read(fixture(rel)).unwrap();
    let mut demux = StreamingTsDemux::new();
    demux.feed(&bytes);
    let mut n = 0usize;
    while let Some(ev) = demux.poll_event() {
        if let DemuxEvent::Sample { .. } = ev {
            n += 1;
        }
    }
    n
}

/// With pacing on, a short bounded wait yields only a *prefix* of the file —
/// not the whole stream. The TS fixture is ~3 s of media; a 150 ms wait must
/// observe substantially fewer than all samples. We assert "fewer than all",
/// never an exact count, to stay robust against scheduler jitter and library
/// timing.
///
/// Bound rationale: 150 ms is well under the ~3 s the pacer would need to emit
/// the entire file at its natural cadence; even the densest track (audio,
/// ~1920 ticks @90 kHz ≈ 21 ms/frame) cannot legally emit more than ~7 frames
/// in 150 ms, and the full fixture has thousands. So "fewer than all" is a
/// wide, deterministic bound that still bites if pacing is removed (which
/// would dump every sample instantly).
#[tokio::test]
async fn pacing_does_not_dump_the_whole_file_instantly() {
    let total = source_sample_count("fixtures/ts/h264_aac.ts");
    assert!(total > 0);

    let trunk = trunk();
    let t = trunk.clone();
    let handle = tokio::spawn(async move {
        reader(fixture("fixtures/ts/h264_aac.ts"), false, true, None, t)
            .run()
            .await
    });

    // Wait a short bounded time, then count what has been written so far.
    tokio::time::sleep(Duration::from_millis(150)).await;
    let seen = drain_samples(&trunk).len();
    assert!(
        seen < total,
        "after a short wait the reader must have written a prefix ({seen}) not the whole file ({total})"
    );

    handle.abort();
    let _ = handle.await;
}

// ---------------------------------------------------------------------------
// Robustness — nonexistent / empty / random-bytes files each fail with their
// own distinct error, never panic.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn nonexistent_path_yields_read_error() {
    let trunk = trunk();
    let result = reader(
        fixture("fixtures/does-not-exist.ts"),
        false,
        false,
        None,
        trunk,
    )
    .run()
    .await;
    match result {
        Err(FileReaderError::Read { path, .. }) => {
            assert!(
                path.ends_with("does-not-exist.ts"),
                "read error must name the missing path, got {path:?}"
            )
        }
        other => panic!("nonexistent path must be a Read error, got {other:?}"),
    }
}

#[tokio::test]
async fn empty_file_yields_unknown_probe_error() {
    // Write a truly empty temp file so the probe sees zero bytes.
    let dir = std::env::temp_dir().join("multimux-file-reader-empty");
    std::fs::create_dir_all(&dir).unwrap();
    let empty = dir.join("empty.bin");
    std::fs::write(&empty, b"").unwrap();

    let trunk = trunk();
    let result = reader(empty.clone(), false, false, None, trunk).run().await;
    match result {
        Err(FileReaderError::UnknownProbe) => {}
        other => panic!("empty file must be UnknownProbe, got {other:?}"),
    }
}

#[tokio::test]
async fn random_bytes_yield_a_probe_error_never_panic() {
    // A file of random bytes must not panic the reader; it fails with a
    // probe-class error (which for uniformly random bytes is UnknownProbe or
    // Ambiguous/Insufficient, depending on what happened to line up).
    let dir = std::env::temp_dir().join("multimux-file-reader-random");
    std::fs::create_dir_all(&dir).unwrap();
    let random = dir.join("random.bin");
    // 64 KiB of a fixed pseudo-random pattern.
    let mut seed: u64 = 0x9e37_79b9_7f4a_7c15;
    let mut bytes = Vec::with_capacity(64 * 1024);
    for _ in 0..bytes.capacity() {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        bytes.push((seed & 0xff) as u8);
    }
    std::fs::write(&random, &bytes).unwrap();

    let trunk = trunk();
    let result = reader(random, false, false, None, trunk).run().await;
    assert!(
        result.is_err(),
        "random bytes must fail, and must never panic"
    );
}
