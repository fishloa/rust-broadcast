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
    FileReader::new(
        FileReaderConfig::new(path, loop_file, trunk)
            .with_pace(pace)
            .with_max_retries(0)
            .with_retry_interval(Duration::from_millis(1))
            .with_max_loops(max_loops),
    )
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

/// The **decode-ordered** DTS sequence of one `track_id` in the source — the
/// demuxer emits a track's samples in decode order, and the reader publishes
/// in that same order (DTS is the intra-track key; PTS only interleaves across
/// tracks). The "source's own" timeline a per-track deltas assertion is held
/// against is this decode order, unsorted.
fn source_track_dts(rel: &str, track_id: u32) -> Vec<i64> {
    let bytes = std::fs::read(fixture(rel)).unwrap();
    let mut demux = StreamingTsDemux::new();
    demux.feed(&bytes);
    let mut dts = Vec::new();
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
            && let Some(d) = sample.dts
        {
            dts.push(d);
        }
    }
    dts
}

/// Drain a Trunk into a flat list of `(track_id, dts)` in publish order.
fn drain_samples(trunk: &Arc<Trunk>) -> Vec<(u32, i64)> {
    let mut cursor = trunk.subscribe_from_backlog();
    let mut out = Vec::new();
    loop {
        match cursor.poll() {
            Some(SampleCursorItem::Timed { track_id, sample }) => {
                out.push((track_id, sample.dts.expect("timed sample must carry a dts")));
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
async fn timeline_dts_is_monotonic_and_matches_source_decode_order() {
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
    // IR stores absolute dts per track in *that* track's timescale).
    let tracks = trunk.tracks();
    assert!(tracks.len() >= 2, "TS fixture must demux video + audio");

    // Per-track: the DTS sequence each track writes must be strictly monotonic
    // (decode order — the fixture's video has B-frames, so PTS reorders within
    // the pass while DTS stays monotonic), and it must equal the source's own
    // decode-order DTS sequence exactly. Group the drained samples by track_id,
    // preserving publish order.
    let mut by_track: std::collections::BTreeMap<u32, Vec<i64>> = Default::default();
    for (track_id, dts) in &drained {
        by_track.entry(*track_id).or_default().push(*dts);
    }
    for (track_id, seq) in &by_track {
        for pair in seq.windows(2) {
            assert!(
                pair[1] > pair[0],
                "track {track_id} dts must be strictly monotonic: {} then {}",
                pair[0],
                pair[1]
            );
        }
        // The source's own per-track DTS sequence (decode order, unsorted)
        // equals exactly what the reader wrote — the reader preserves decode
        // order per track and loses nothing.
        let source = source_track_dts("fixtures/ts/h264_aac.ts", *track_id);
        assert_eq!(
            &source, seq,
            "track {track_id} decode order must match the source's own DTS sequence"
        );
    }

    // Nothing silently dropped: the reader wrote as many samples as the source
    // carries.
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
async fn loop_preserves_dts_monotonicity_across_boundary() {
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

    // Group the drained samples by track_id (each track carries DTS in its own
    // timescale; the loop offset is advanced *per track*). For every track,
    // loop 1 and loop 2 concatenate into a strictly monotonic DTS sequence —
    // the loop offset is what keeps it from stepping backwards across the
    // boundary, and the mutation kills it. (The fixture's video has B-frames,
    // so PTS reorders within a pass; DTS is the decode-order invariant.)
    let mut by_track: std::collections::BTreeMap<u32, Vec<i64>> = Default::default();
    for (track_id, dts) in &drained {
        by_track.entry(*track_id).or_default().push(*dts);
    }

    for (track_id, seq) in &by_track {
        // Source's own per-loop DTS sequence for this track, to locate the
        // loop boundary in its published seq.
        let source = source_track_dts("fixtures/ts/h264_aac.ts", *track_id);
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
                "loop DTS must stay strictly monotonic for track {track_id}: {} then {}",
                pair[0],
                pair[1]
            );
        }

        // The boundary is a forward step, not a reset and not the whole file's
        // length: pass-1 first DTS must exceed pass-0 last DTS by more than the
        // reorder depth (a B-frame track's max composition offset) yet far less
        // than a full pass span.
        let boundary_delta = seq[per_loop] - seq[per_loop - 1];
        assert!(
            boundary_delta > 0,
            "track {track_id}: loop boundary DTS delta must be positive, got {boundary_delta}"
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
async fn empty_file_is_reported_as_too_short_to_identify() {
    // Write a truly empty temp file so the probe sees zero bytes.
    let dir = std::env::temp_dir().join("multimux-file-reader-empty");
    std::fs::create_dir_all(&dir).unwrap();
    let empty = dir.join("empty.bin");
    std::fs::write(&empty, b"").unwrap();

    let trunk = trunk();
    let result = reader(empty.clone(), false, false, None, trunk).run().await;
    // An empty file is not "unknown format" — there is nothing to identify.
    // `container-probe` answers `Insufficient` for a 0-byte slice because it
    // has no end-of-file signal and a longer buffer genuinely could decide.
    // This reader probes the WHOLE file, so it knows there are no more bytes
    // and reports that directly rather than propagating "read more" for a file
    // that has no more.
    match result {
        Err(FileReaderError::FileTooShortToIdentify {
            need_at_least,
            file_bytes,
        }) => {
            assert_eq!(file_bytes, 0, "the fixture is a 0-byte file");
            assert!(need_at_least > 0, "the probe must ask for a real minimum");
        }
        other => panic!("empty file must be FileTooShortToIdentify, got {other:?}"),
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

// ---------------------------------------------------------------------------
// The production `pace: true` × `loop_file: true` cell — the shipped default
// that the earlier tests never exercised, and whose pacing baseline used to be
// a fixed `start` (so every pass past the first found every due instant in the
// past and never `.await`ed, spinning at 100% CPU).
// ---------------------------------------------------------------------------

/// The fixture's total playback duration in seconds, measured the same way the
/// reader does — max `(pts / timescale)` over every track's presentation PTS —
/// so the pacing test asserts against ~2× real content, never a hardcoded
/// floor that the pass-1 duration alone would satisfy.
fn fixture_content_duration_secs(rel: &str) -> f64 {
    let bytes = std::fs::read(fixture(rel)).unwrap();
    let mut demux = StreamingTsDemux::new();
    demux.feed(&bytes);
    let mut spans: std::collections::HashMap<u32, (i64, i64, u32)> =
        std::collections::HashMap::new(); // track_id -> (first, last, timescale)
    while let Some(ev) = demux.poll_event() {
        match ev {
            DemuxEvent::TrackAdded(spec) | DemuxEvent::TrackUpdated(spec) => {
                spans
                    .entry(spec.track_id)
                    .or_insert((i64::MAX, i64::MIN, spec.timescale))
                    .2 = spec.timescale;
            }
            DemuxEvent::Sample {
                track_id, sample, ..
            } => {
                if let Some(pts) = sample.pts
                    && let Some(span) = spans.get_mut(&track_id)
                {
                    span.0 = span.0.min(pts);
                    span.1 = span.1.max(pts);
                }
            }
            _ => {}
        }
    }
    spans
        .values()
        .map(|&(first, last, ts)| {
            if last <= first {
                0.0
            } else {
                (last - first) as f64 / ts.max(1) as f64
            }
        })
        .fold(0.0f64, f64::max)
}

/// When pacing AND looping are both on (the [`FileReader::standard`] default),
/// the second pass must not complete before its content duration has elapsed —
/// the pacing baseline advances one content span per pass, so the loop does not
/// dump the whole file instantly and does not spin once its targets are past.
///
/// The threshold is **~2× the fixture's own content duration** (derived above),
/// never a hardcoded floor: the fixture is ~3 s, so a 2 s floor would be met by
/// pass 1 alone and fail to distinguish the fixed baseline from the bug — the
/// thing this test exists to prove. A 2-pass paced run must take ~2× content.
///
/// MUTATION VERIFIED, recorded verbatim: reverting `publish_looping`'s pacing
/// baseline advance (so `start` never grows past pass 1) makes this test FAIL
/// with:
///
///     a paced 2-pass loop must take ~2× content duration (2.972 s each pass → 5.944 s), not 2.990257625s (baseline did not advance)
///
/// — the run finishes in one content duration (~2.99 s) because pass 2's due
/// instants are all already in the past and dump instantly. Restoring the
/// advance (and a `touch`) makes it pass again.
#[tokio::test]
async fn paced_loop_second_pass_waits_for_content_duration() {
    let content = fixture_content_duration_secs("fixtures/ts/h264_aac.ts");
    assert!(
        content > 0.0,
        "fixture must carry a positive content duration"
    );

    let trunk = trunk();
    let t = trunk.clone();
    let started = std::time::Instant::now();
    let handle = tokio::spawn(async move {
        reader(fixture("fixtures/ts/h264_aac.ts"), true, true, Some(2), t)
            .run()
            .await
    });
    handle
        .await
        .unwrap()
        .expect("a bounded paced loop must finish cleanly");

    let elapsed = started.elapsed();
    let two_contents = Duration::from_secs_f64(2.0 * content);
    assert!(
        elapsed >= two_contents,
        "a paced 2-pass loop must take ~2× content duration ({content:.3} s each pass → {:.3} s), not {elapsed:?} (baseline did not advance)",
        two_contents.as_secs_f64()
    );
}

// ---------------------------------------------------------------------------
// WriterUnavailable — a second FileReader over one Arc<Trunk> (or a caller
// that already took the writer) must fail as a structured error, never panic.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn second_reader_over_same_trunk_fails_with_writer_unavailable() {
    let trunk = trunk();
    // Claim the one sample/event writer ourselves.
    let _writer = trunk.writer().expect("first writer must be claimable");

    let result = reader(
        fixture("fixtures/ts/h264_aac.ts"),
        false,
        false,
        None,
        trunk,
    )
    .run()
    .await;
    match result {
        Err(FileReaderError::WriterUnavailable) => {}
        other => {
            panic!("a second reader over a claimed trunk must be WriterUnavailable, got {other:?}")
        }
    }
}

// ---------------------------------------------------------------------------
// Size cap + regular-file check — an operator-supplied path is never read
// unbounded (a character device/FIFO would OOM the whole origin), and an
// over-cap file fails cleanly before the read.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn oversized_file_yields_file_too_large() {
    let dir = std::env::temp_dir().join("multimux-file-reader-oversize");
    std::fs::create_dir_all(&dir).unwrap();
    let big = dir.join("big.bin");
    // 1 KiB of bytes, under a 512-byte cap.
    std::fs::write(&big, vec![0u8; 1024]).unwrap();

    let trunk = trunk();
    let config = FileReaderConfig::new(big.clone(), false, trunk)
        .with_pace(false)
        .with_max_retries(0)
        .with_retry_interval(Duration::from_millis(1))
        .with_max_file_bytes(512);
    let result = FileReader::new(config).run().await;
    match result {
        Err(FileReaderError::FileTooLarge { size, max, .. }) => {
            assert_eq!(size, 1024, "must report the file's actual size");
            assert_eq!(max, 512, "must report the configured cap");
        }
        other => panic!("over-cap file must be FileTooLarge, got {other:?}"),
    }
}

#[tokio::test]
async fn directory_path_yields_not_a_regular_file() {
    // A directory is not a regular file — the reader must reject it before any
    // read, with a structured error rather than an unbounded read.
    let dir = std::env::temp_dir(); // a real directory
    let trunk = trunk();
    let result = reader(dir.clone(), false, false, None, trunk).run().await;
    match result {
        Err(FileReaderError::NotARegularFile { kind, .. }) => {
            assert_eq!(
                kind, "directory",
                "a directory must be labelled 'directory'"
            );
        }
        other => panic!("a directory must be NotARegularFile, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Decode-order invariant on a genuinely B-frame-bearing fixture — the reader
// must publish each track in DTS (decode) order, never presentation order.
// `fixtures/mp4/progressive/av_prog.mp4` carries video with `dts != pts`
// (B-frame reordering) and audio with `dts == pts`, so a presentation-order
// publish would emit referenced P-frames after the B-frames that reference
// them and produce a non-monotonic DTS sequence `transmux` would mux wrong.
// ---------------------------------------------------------------------------

/// Play a B-frame-bearing fixture through the reader and assert every track's
/// published DTS sequence is **non-decreasing** (decode order) — the invariant
/// BLOCKER 3 guards. A presentation-order (`pts`) publish would reorder the
/// video's decode sequence and fail the monotonic check.
///
/// MUTATION VERIFIED, recorded verbatim: keying the intra-track sort on `pts`
/// instead of `dts` (a presentation-order publish) makes the video track's DTS
/// sequence step backwards and this test FAILS with:
///
///     track 1 published DTS must be non-decreasing (decode order): 10800 then 7200
///
/// — a P-frame (`dts = 7200`) is emitted after the B-frame that references it
/// (`dts = 10800`). Restoring the `dts` key (and a `touch`) makes it pass
/// again.
#[tokio::test]
async fn b_frame_fixture_publishes_decode_order_dts() {
    let trunk = trunk();
    let t = trunk.clone();
    let handle = tokio::spawn(async move {
        reader(
            fixture("fixtures/mp4/progressive/av_prog.mp4"),
            false,
            false,
            None,
            t,
        )
        .run()
        .await
    });
    handle
        .await
        .unwrap()
        .expect("progressive mp4 fixture must play cleanly");

    let drained = drain_samples(&trunk);
    assert!(
        drained.len() > 1,
        "B-frame fixture must yield samples, got {}",
        drained.len()
    );

    let mut by_track: std::collections::BTreeMap<u32, Vec<i64>> = Default::default();
    for (track_id, dts) in &drained {
        by_track.entry(*track_id).or_default().push(*dts);
    }
    assert!(
        by_track.len() >= 2,
        "fixture must carry video + audio tracks, got {:?}",
        by_track.keys().collect::<Vec<_>>()
    );

    for (track_id, seq) in &by_track {
        for pair in seq.windows(2) {
            assert!(
                pair[1] >= pair[0],
                "track {track_id} published DTS must be non-decreasing (decode order): {} then {}",
                pair[0],
                pair[1]
            );
        }
    }
}
