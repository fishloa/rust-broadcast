//! The `Insufficient` vs `Unknown` contract, swept **exhaustively**.
//!
//! `Probe::Unknown` means "stop — more bytes will not help". For a real file of
//! a supported format, that is always a lie: reading the rest of the file
//! resolves it. So for every fixture below, **every** prefix length from 0 to
//! the whole file must probe as either `Insufficient` (read more) or
//! `Identified`/`Ambiguous` — never `Unknown`.
//!
//! # Why this test exists in this shape
//!
//! `tests/insufficient_contract.rs` checks the same contract at *hand-picked*
//! boundaries: one byte short of each prober's header minimum, plus a 64-byte
//! elementary-stream read. It passed while the contract was still broken at
//! more than two hundred other lengths, because the bug never lived at a
//! boundary someone thought to sample.
//!
//! Concretely, the following all returned `Unknown` — a real MP4, a real TS, a
//! real MP3 and a real H.264 stream, each telling a streaming caller to give
//! up — while the sampled test was green:
//!
//! ```text
//! fixtures/mp4/h264_high.mp4        lengths 33..=39, 41..=47
//! fixtures/mp4/cmaf/av_frag.mp4     21 lengths, including 1278..=1284, 1590..=1596
//! fixtures/container-probe/ts_midpacket_phase.ts   all 96 lengths 16..=111
//! fixtures/container-probe/audio.mp3              41 lengths (16..=48, 237..=240, 429..=432)
//! fixtures/container-probe/h264.annexb            32, 40
//! ```
//!
//! Two successive fix rounds patched this prober-by-prober and each round a
//! *different* prober was still wrong at a *different* length, because the
//! contract is decided independently in twelve places. A sampled guard cannot
//! catch that class. An exhaustive one cannot miss it: there is no boundary
//! left to pick wrongly.
//!
//! The sweep is bounded to keep it fast — see `SWEEP_LIMIT` — but it is dense
//! (every single length) over the region where the probers actually make
//! decisions, which is where every defect above lived.

use container_probe::{Format, Probe};
use std::fs;
use std::path::PathBuf;

/// Sweep every prefix length up to this many bytes.
///
/// Every prober reaches its verdict well inside this window: the widest
/// evidence any of them needs is a 208-byte-stride TS lattice at 8
/// confirmations plus a full phase search. Beyond it the answer no longer
/// changes with length, so sweeping further costs time and proves nothing.
/// The whole file is probed separately, below, so the far end is still covered.
const SWEEP_LIMIT: usize = 2048;

fn repo_path(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join(rel)
}

/// Whether the *complete* file carries enough evidence to be identified.
#[derive(PartialEq, Eq, Clone, Copy)]
enum WholeFile {
    /// The full file must probe `Identified` as its own format.
    Identifies,
    /// The full file is legitimately too short to reach any confidence tier,
    /// so `Insufficient` is the correct and honest answer even at EOF. It is
    /// still bound by the never-`Unknown` rule — "read more" is right, "stop"
    /// is not, and a caller concatenating captures would eventually resolve it.
    TooShortToConclude,
}

/// Every fixture in the repository that IS a real file of a format this crate
/// claims to detect, with the format it must ultimately resolve to.
///
/// A non-media file is deliberately absent: `Unknown` is the *correct* answer
/// there, and that direction is covered by `tests/corpus_sweep.rs`.
const REAL_FILES: &[(&str, Format, WholeFile)] = &[
    (
        "fixtures/ts/h264_aac.ts",
        Format::MpegTs,
        WholeFile::Identifies,
    ),
    (
        "fixtures/ts/france2.ts",
        Format::MpegTs,
        WholeFile::Identifies,
    ),
    // 188 bytes: exactly ONE TS packet. The weak tier needs 3 sync
    // confirmations and the strong tier 8, so one packet cannot reach either.
    // Reporting `Insufficient { need_at_least: 1504 }` (188 * 8) is correct;
    // claiming MpegTs from a single 0x47 is the false-positive this crate's
    // whole scoring model exists to prevent.
    (
        "fixtures/ts/scte35-real.ts",
        Format::MpegTs,
        WholeFile::TooShortToConclude,
    ),
    (
        "fixtures/ts/pcr-wrap.ts",
        Format::MpegTs,
        WholeFile::Identifies,
    ),
    // 376 bytes: two TS packets, one short of the 3-sync weak threshold.
    (
        "fixtures/mpeg-ts/af-pcr-stuffing.ts",
        Format::MpegTs,
        WholeFile::TooShortToConclude,
    ),
    (
        "fixtures/container-probe/ts_midpacket_phase.ts",
        Format::MpegTs,
        WholeFile::Identifies,
    ),
    (
        "fixtures/container-probe/m2ts_192.m2ts",
        Format::MpegTs,
        WholeFile::Identifies,
    ),
    (
        "fixtures/mp4/h264_high.mp4",
        Format::Isobmff,
        WholeFile::Identifies,
    ),
    (
        "fixtures/mp4/hevc_main.mp4",
        Format::Isobmff,
        WholeFile::Identifies,
    ),
    (
        "fixtures/mp4/cenc.mp4",
        Format::Isobmff,
        WholeFile::Identifies,
    ),
    (
        "fixtures/mp4/cmaf/av_frag.mp4",
        Format::Isobmff,
        WholeFile::Identifies,
    ),
    (
        "fixtures/mp4/progressive/av_prog.mp4",
        Format::Isobmff,
        WholeFile::Identifies,
    ),
    (
        "fixtures/mp4/frag/av1.frag.mp4",
        Format::Isobmff,
        WholeFile::Identifies,
    ),
    (
        "fixtures/mkv/h264_aac.mkv",
        Format::Matroska,
        WholeFile::Identifies,
    ),
    (
        "fixtures/mkv/vp9_opus.mkv",
        Format::Matroska,
        WholeFile::Identifies,
    ),
    (
        "fixtures/webm/vorbis.webm",
        Format::WebM,
        WholeFile::Identifies,
    ),
    (
        "fixtures/webm/vp9_opus.webm",
        Format::WebM,
        WholeFile::Identifies,
    ),
    (
        "fixtures/mxf/op1a_mpeg2_pcm.mxf",
        Format::Mxf,
        WholeFile::Identifies,
    ),
    (
        "fixtures/ps/h264_ac3.ps",
        Format::MpegPs,
        WholeFile::Identifies,
    ),
    ("fixtures/flv/av.flv", Format::Flv, WholeFile::Identifies),
    (
        "fixtures/container-probe/pcm_s16le.wav",
        Format::Wav,
        WholeFile::Identifies,
    ),
    (
        "fixtures/container-probe/opus.ogg",
        Format::Ogg,
        WholeFile::Identifies,
    ),
    (
        "fixtures/container-probe/video.asf",
        Format::Asf,
        WholeFile::Identifies,
    ),
    (
        "fixtures/container-probe/aac.adts",
        Format::AdtsAac,
        WholeFile::Identifies,
    ),
    (
        "fixtures/container-probe/audio.mp3",
        Format::Mp3,
        WholeFile::Identifies,
    ),
    (
        "fixtures/container-probe/h264.annexb",
        Format::AnnexB,
        WholeFile::Identifies,
    ),
];

/// No prefix of a real file of a supported format may probe `Unknown`.
///
/// Reports **every** offending length, not just the first: the failure mode
/// this guards is "a whole contiguous run of lengths is broken", and seeing one
/// length at a time turns one fix round into twenty.
#[test]
fn no_prefix_of_a_real_file_is_ever_unknown() {
    let mut failures: Vec<String> = Vec::new();
    let mut skipped: Vec<&str> = Vec::new();

    for (rel, _expected, _whole) in REAL_FILES {
        let path = repo_path(rel);
        let Ok(data) = fs::read(&path) else {
            skipped.push(rel);
            continue;
        };
        let sweep_to = data.len().min(SWEEP_LIMIT);
        let mut bad: Vec<usize> = Vec::new();
        for n in 0..=sweep_to {
            if matches!(container_probe::probe(&data[..n]), Probe::Unknown) {
                bad.push(n);
            }
        }
        if !bad.is_empty() {
            failures.push(format!(
                "  {rel}: {} of {} prefix lengths probe Unknown (a real file of a \
                 supported format telling the caller to stop): {}",
                bad.len(),
                sweep_to + 1,
                summarise_runs(&bad)
            ));
        }
    }

    assert!(
        skipped.is_empty(),
        "fixtures missing from the repository — the sweep must be exhausting, \
         not silently reduced. Missing: {skipped:?}"
    );
    assert!(
        failures.is_empty(),
        "Probe::Unknown means \"stop, more bytes will not help\", which is false for \
         every one of these:\n{}\n(skipped, not present: {:?})",
        failures.join("\n"),
        skipped
    );
}

/// A prefix may be undecided, but it must never be decided *wrongly*: no prefix
/// of a real file may be `Identified` as a format other than the file's own.
///
/// This is the other half of the contract. A caller that acts on `Identified`
/// hands the bytes to the wrong demuxer, which is worse than being told to
/// read more.
#[test]
fn no_prefix_of_a_real_file_is_identified_as_another_format() {
    let mut failures: Vec<String> = Vec::new();
    let mut checked = 0usize;

    for (rel, expected, _whole) in REAL_FILES {
        let path = repo_path(rel);
        let Ok(data) = fs::read(&path) else { continue };
        checked += 1;
        let sweep_to = data.len().min(SWEEP_LIMIT);
        let mut bad: Vec<(usize, Format)> = Vec::new();
        for n in 0..=sweep_to {
            if let Probe::Identified { format, .. } = container_probe::probe(&data[..n])
                && format != *expected
            {
                bad.push((n, format));
            }
        }
        if !bad.is_empty() {
            let (first_len, first_fmt) = bad[0];
            failures.push(format!(
                "  {rel} (really {}): {} prefix lengths identify as the wrong format, \
                 first at {first_len} bytes -> {}",
                expected.name(),
                bad.len(),
                first_fmt.name()
            ));
        }
    }

    assert!(
        checked == REAL_FILES.len(),
        "{} of {} fixtures were missing — the sweep must confirm every real file, \
         not a subset (checked {checked})",
        REAL_FILES.len() - checked,
        REAL_FILES.len()
    );
    assert!(
        failures.is_empty(),
        "a prefix identified as the wrong format sends the caller to the wrong \
         demuxer:\n{}",
        failures.join("\n")
    );
}

/// The whole file — past `SWEEP_LIMIT` — must resolve to the right format.
/// Keeps the bounded sweep above honest: a prober cannot satisfy it by simply
/// never concluding anything.
#[test]
fn every_real_file_identifies_in_full() {
    let mut failures: Vec<String> = Vec::new();
    let mut checked = 0usize;

    for (rel, expected, whole) in REAL_FILES {
        let Ok(data) = fs::read(repo_path(rel)) else {
            continue;
        };
        checked += 1;
        let got = container_probe::probe(&data);
        match (whole, &got) {
            (WholeFile::Identifies, Probe::Identified { format, .. }) if format == expected => {}
            // Documented as carrying too little evidence to conclude: the only
            // acceptable answer is "read more". Identifying it anyway would mean
            // the tier thresholds had been weakened, so that is a failure too.
            (WholeFile::TooShortToConclude, Probe::Insufficient { .. }) => {}
            _ => failures.push(format!(
                "  {rel}: expected {}, got {got:?}",
                match whole {
                    WholeFile::Identifies => expected.name(),
                    WholeFile::TooShortToConclude => "Insufficient (too short to conclude)",
                }
            )),
        }
    }

    assert!(
        checked == REAL_FILES.len(),
        "{} of {} fixtures were missing — the sweep must confirm every real file, \
         not a subset (checked {checked})",
        REAL_FILES.len() - checked,
        REAL_FILES.len()
    );
    assert!(
        failures.is_empty(),
        "a complete real file must identify as its own format:\n{}",
        failures.join("\n")
    );
}

/// Render `[33,34,35,41,42]` as `"33..=35, 41..=42"` so a failure shows the
/// contiguous broken *ranges* — which is how this defect actually presents —
/// rather than a wall of individual integers.
fn summarise_runs(sorted: &[usize]) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i < sorted.len() {
        let start = sorted[i];
        let mut end = start;
        while i + 1 < sorted.len() && sorted[i + 1] == end + 1 {
            i += 1;
            end = sorted[i];
        }
        out.push(if start == end {
            format!("{start}")
        } else {
            format!("{start}..={end}")
        });
        i += 1;
    }
    out.join(", ")
}
