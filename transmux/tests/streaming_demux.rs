//! `StreamingTsDemux` gate (issue #555): chunk-boundary equivalence against
//! the batch `TsDemux`, event-surface coverage, bounded-memory eager polling,
//! and `finish()` trailing-flush semantics.
//!
//! Every fixture here is a real, committed broadcast capture (never
//! hand-built bytes): `fixtures/ts/h264_aac.ts`, `fixtures/ts/dolby/*.ts`
//! (AC-3/E-AC-3), `fixtures/ts/m6-single.ts` (real DVB mux — subtitle/
//! teletext data tracks, zero PCRs, one codec PID with no packets in this
//! excerpt), and `fixtures/ts/france-pcr-discontinuity.ts` (a genuine PCR
//! discontinuity). `m6-single.ts` lives in `fixtures/ts/` (not
//! `dvb-si/tests/fixtures/`) in this checkout — `transmux/tests/ir_timing.rs`
//! already reads it from that path.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use broadcast_common::Unpackage;
use transmux::TsDemux;
use transmux::media::{Media, PcrSample, Track};
use transmux::pipeline::CodecConfig;
use transmux::ts_demux::{DemuxEvent, StreamingTsDemux};

// ── Fixture loading ─────────────────────────────────────────────────────────

fn fixtures_ts_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/ts")
}

fn read(path: &Path) -> Vec<u8> {
    std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn dolby_files() -> Vec<PathBuf> {
    let dir = fixtures_ts_dir().join("dolby");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("{}: {e}", dir.display()))
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("ts"))
        .collect();
    files.sort();
    assert!(
        !files.is_empty(),
        "fixtures/ts/dolby must contain at least one .ts file"
    );
    files
}

fn equivalence_fixture_files() -> Vec<PathBuf> {
    let mut files = vec![
        fixtures_ts_dir().join("h264_aac.ts"),
        fixtures_ts_dir().join("m6-single.ts"),
        fixtures_ts_dir().join("france-pcr-discontinuity.ts"),
    ];
    files.extend(dolby_files());
    files
}

// ── Fold a StreamingTsDemux event stream into a Media (mirrors TsDemux::demux) ─

fn assemble(mut demux: StreamingTsDemux) -> Media {
    let mut tracks: Vec<Track> = Vec::new();
    let mut index_by_id: BTreeMap<u32, usize> = BTreeMap::new();
    let mut pcr: Vec<PcrSample> = Vec::new();
    while let Some(event) = demux.poll_event() {
        match event {
            DemuxEvent::TrackAdded(track) => {
                index_by_id.insert(track.spec.track_id, tracks.len());
                tracks.push(track);
            }
            DemuxEvent::TrackUpdated(track) => {
                if let Some(&i) = index_by_id.get(&track.spec.track_id) {
                    let samples = std::mem::take(&mut tracks[i].samples);
                    tracks[i] = track;
                    tracks[i].samples = samples;
                }
            }
            DemuxEvent::Sample { track_id, sample } => {
                if let Some(&i) = index_by_id.get(&track_id) {
                    tracks[i].samples.push(sample);
                }
            }
            DemuxEvent::Pcr(sample) => pcr.push(sample),
            DemuxEvent::Discontinuity { .. } => {}
            _ => {}
        }
    }
    Media::new(tracks, 90_000).with_pcr(pcr)
}

fn feed_in_chunks(data: &[u8], chunk_size: usize) -> Media {
    let mut demux = StreamingTsDemux::new();
    for chunk in data.chunks(chunk_size.max(1)) {
        demux.feed(chunk);
    }
    demux.finish();
    assemble(demux)
}

/// Deep-equality assertion between two `Media` values built via different
/// paths. `pcr` compares via `PcrSample`'s own `PartialEq`; everything else
/// (tracks/samples/codec configs/source_timing/composition offsets) compares
/// via the complete `Debug` dump of `Media::tracks` — every nested type here
/// (`Sample`, `TrackSpec`, `CodecConfig` and its box types) is derived-Debug
/// plain data, so any byte, timestamp, or config divergence between the two
/// paths surfaces as a string mismatch. `Media`/`Track`/`Sample` do not
/// implement `PartialEq` themselves (only their leaf field types do), so this
/// is the practical whole-value equality check available from outside the
/// crate's own internals.
fn assert_media_eq(label: &str, chunked: &Media, oracle: &Media) {
    assert_eq!(
        chunked.movie_timescale, oracle.movie_timescale,
        "{label}: movie_timescale"
    );
    assert_eq!(chunked.pcr, oracle.pcr, "{label}: pcr timeline");
    assert_eq!(
        chunked.tracks.len(),
        oracle.tracks.len(),
        "{label}: track count"
    );
    let chunked_dump = format!("{:#?}", chunked.tracks);
    let oracle_dump = format!("{:#?}", oracle.tracks);
    assert_eq!(
        chunked_dump, oracle_dump,
        "{label}: tracks differ (full Debug dump — see the diff above)"
    );
}

const CHUNK_SIZES: [usize; 8] = [1, 7, 100, 187, 188, 189, 1024, 65536];

/// Test 1 (headline gate) — for every fixture, for every chunk size, feeding
/// the file through `StreamingTsDemux` in that chunk size must reconstruct a
/// `Media` byte-identical (all tracks, all sample bytes, all source_timing
/// values, all pcr entries) to the one-shot batch `TsDemux::demux()` result.
/// Chunk size 1 (byte-at-a-time) is included.
#[test]
fn chunk_boundary_equivalence_matches_batch() {
    for path in equivalence_fixture_files() {
        let data = read(&path);
        assert_eq!(
            data.len() % 188,
            0,
            "{}: must be whole 188-byte TS packets",
            path.display()
        );
        let oracle: Media = TsDemux::new()
            .unpackage(&data)
            .unwrap_or_else(|e| panic!("{}: batch demux: {e}", path.display()));

        for &size in &CHUNK_SIZES {
            let chunked = feed_in_chunks(&data, size);
            assert_media_eq(
                &format!("{} @ chunk_size={size}", path.display()),
                &chunked,
                &oracle,
            );
        }
    }
}

// ── Test 2 — event-surface coverage ──────────────────────────────────────────

#[test]
fn m6_single_track_added_covers_every_live_pid_incl_data_tracks() {
    let data = read(&fixtures_ts_dir().join("m6-single.ts"));

    let mut demux = StreamingTsDemux::new();
    demux.feed(&data);
    demux.finish();
    let mut added: Vec<Track> = Vec::new();
    while let Some(event) = demux.poll_event() {
        if let DemuxEvent::TrackAdded(track) = event {
            added.push(track);
        }
    }

    assert!(
        !added.is_empty(),
        "m6-single.ts must produce at least one TrackAdded event"
    );
    assert!(
        added
            .iter()
            .any(|t| matches!(t.spec.config, CodecConfig::Data { .. })),
        "TrackAdded must cover at least one opaque Data (stream_type 0x06) track"
    );

    // Every track the batch wrapper (built on the very same engine) settles
    // on must have had exactly one corresponding TrackAdded fire.
    let batch: Media = TsDemux::new()
        .unpackage(&data)
        .expect("batch demux of m6-single.ts");
    assert_eq!(
        added.len(),
        batch.tracks.len(),
        "TrackAdded count must equal the final track count"
    );
}

#[test]
fn france_discontinuity_fixture_fires_discontinuity_event() {
    let data = read(&fixtures_ts_dir().join("france-pcr-discontinuity.ts"));

    let mut demux = StreamingTsDemux::new();
    demux.feed(&data);
    demux.finish();
    let mut saw_discontinuity = false;
    while let Some(event) = demux.poll_event() {
        if matches!(event, DemuxEvent::Discontinuity { .. }) {
            saw_discontinuity = true;
        }
    }
    assert!(
        saw_discontinuity,
        "france-pcr-discontinuity.ts must produce at least one Discontinuity event"
    );
}

#[test]
fn h264_aac_pcr_event_count_matches_batch_media_pcr_len() {
    let data = read(&fixtures_ts_dir().join("h264_aac.ts"));
    let batch: Media = TsDemux::new()
        .unpackage(&data)
        .expect("batch demux of h264_aac.ts");
    assert!(!batch.pcr.is_empty(), "h264_aac.ts must carry PCR");

    let mut demux = StreamingTsDemux::new();
    demux.feed(&data);
    demux.finish();
    let mut pcr_events = 0usize;
    while let Some(event) = demux.poll_event() {
        if matches!(event, DemuxEvent::Pcr(_)) {
            pcr_events += 1;
        }
    }
    assert_eq!(
        pcr_events,
        batch.pcr.len(),
        "Pcr event count must equal batch Media.pcr.len()"
    );
}

// ── Test 3 — bounded-memory smoke: eager polling across a doubled input ─────

/// Feed `h264_aac.ts` twice back-to-back (188-byte chunks), draining events
/// after *every* `feed()` call (not just at the end). Continuity across the
/// concatenation seam is not required to be seamless (a fresh PAT/PMT/PES
/// cycle restarts mid-stream) — only that no `Sample` event is lost when the
/// caller polls eagerly, and that memory does not require draining solely at
/// `finish()`: total sample count must be exactly 2x a single pass.
#[test]
fn bounded_memory_smoke_double_feed_eager_poll() {
    let data = read(&fixtures_ts_dir().join("h264_aac.ts"));
    let mut doubled = data.clone();
    doubled.extend_from_slice(&data);
    assert_eq!(doubled.len() % 188, 0);

    let single: Media = TsDemux::new()
        .unpackage(&data)
        .expect("single-pass batch demux");
    let single_sample_count: usize = single.tracks.iter().map(|t| t.samples.len()).sum();
    assert!(single_sample_count > 0, "single pass must yield samples");

    let mut demux = StreamingTsDemux::new();
    let mut sample_count = 0usize;
    for chunk in doubled.chunks(188) {
        demux.feed(chunk);
        while let Some(event) = demux.poll_event() {
            if matches!(event, DemuxEvent::Sample { .. }) {
                sample_count += 1;
            }
        }
    }
    demux.finish();
    while let Some(event) = demux.poll_event() {
        if matches!(event, DemuxEvent::Sample { .. }) {
            sample_count += 1;
        }
    }

    assert_eq!(
        sample_count,
        single_sample_count * 2,
        "doubled input, eagerly polled after every feed() call, must yield exactly 2x the samples"
    );
}

// ── Test 4 — finish() flushes the trailing partial access unit ─────────────

/// Truncate `h264_aac.ts` short of EOF (dropping the tail so a live track's
/// one-behind pending sample is never resolved by a following access unit)
/// and assert: before `finish()`, strictly fewer samples than the full
/// (finish()-included) demux of that same truncated input; after `finish()`,
/// exactly the remainder — i.e. the trailing partial access unit appears
/// only once `finish()` is called.
#[test]
fn finish_flushes_trailing_partial_access_unit_only_at_finish() {
    let data = read(&fixtures_ts_dir().join("h264_aac.ts"));
    let truncate_at = data.len() - (5 * 188);
    let truncated = data[..truncate_at].to_vec();
    assert_eq!(truncated.len() % 188, 0);

    // Oracle: the batch wrapper always calls finish() internally, so this is
    // "what a fully-flushed demux of this exact truncated input looks like".
    let oracle: Media = TsDemux::new()
        .unpackage(&truncated)
        .expect("batch demux of truncated input");
    let oracle_total: usize = oracle.tracks.iter().map(|t| t.samples.len()).sum();
    assert!(oracle_total > 0, "truncated input must still yield samples");

    let mut demux = StreamingTsDemux::new();
    demux.feed(&truncated);
    let mut before_count = 0usize;
    while let Some(event) = demux.poll_event() {
        if matches!(event, DemuxEvent::Sample { .. }) {
            before_count += 1;
        }
    }
    assert!(
        before_count < oracle_total,
        "before finish(), the trailing partial access unit must NOT yet be emitted \
         (before={before_count}, full-oracle-total={oracle_total})"
    );

    demux.finish();
    let mut after_count = 0usize;
    while let Some(event) = demux.poll_event() {
        if matches!(event, DemuxEvent::Sample { .. }) {
            after_count += 1;
        }
    }
    assert!(
        after_count > 0,
        "finish() must flush at least one trailing sample"
    );
    assert_eq!(
        before_count + after_count,
        oracle_total,
        "finish() must flush exactly the remaining trailing samples, no more, no less"
    );
}
