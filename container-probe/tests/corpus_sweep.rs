//! Corpus sweep — the institutionalised false-positive/ambiguity guard (WP2,
//! section B).
//!
//! Walks the whole repo media corpus and asserts each file's verdict matches
//! its extension. This is the permanent version of the sweep that caught every
//! WP1 defect: a file identified as the WRONG format, or returning
//! `Ambiguous`, or failing to identify at all, fails the test and names every
//! offender. `Insufficient` is tolerated ONLY for the files on the documented
//! `EXPECTED_INSUFFICIENT` allowlist (each holds fewer than the 3 whole packets
//! a weak lattice needs); any other non-identification is a miss and fails.
//!
//! An earlier revision only *printed* misses. That made the sweep unable to
//! fail on a prober that stopped firing entirely — breaking the ISOBMFF prober
//! left it green with 74 silently-printed lines. Silence must not read as
//! success.
//!
//! `.ts` is deliberately excluded from the extension map because it is
//! ambiguous (TypeScript `.d.ts` under `multimux/tests/assets/node_modules/`);
//! it is handled separately under the TS fixture directories only.

use container_probe::{Detail, Format, Probe};
use std::fs;
use std::path::{Path, PathBuf};

/// Directories to walk, relative to the workspace root.
const WALK_DIRS: &[&str] = &[
    "fixtures/",
    "transmux/tests/fixtures/",
    "st377-1/tests/fixtures/",
    "ts-fix/tests/fixtures/",
    "media-doctor/tests/",
];

/// The extension -> expected `Format` map. `.ts` is excluded (ambiguous).
fn extension_format(ext: &str) -> Option<Format> {
    match ext {
        "m2ts" | "mts" => Some(Format::MpegTs),
        "mp4" | "m4s" => Some(Format::Isobmff),
        "mkv" => Some(Format::Matroska),
        "webm" => Some(Format::WebM),
        "mxf" => Some(Format::Mxf),
        "ps" => Some(Format::MpegPs),
        "flv" => Some(Format::Flv),
        "wav" => Some(Format::Wav),
        "ogg" => Some(Format::Ogg),
        "asf" | "wmv" => Some(Format::Asf),
        "adts" | "aac" => Some(Format::AdtsAac),
        "mp3" => Some(Format::Mp3),
        "annexb" | "h264" | "264" => Some(Format::AnnexB),
        _ => None,
    }
}

/// The subset of fixture dirs where `.ts` unambiguously means an MPEG-2 TS
/// stream (as opposed to TypeScript). Any `.ts` outside these is skipped.
const TS_DIRS: &[&str] = &[
    "fixtures/ts/",
    "fixtures/mpeg-ts/",
    "fixtures/dvb-si/",
    "fixtures/mpeg-pes/",
];

/// Skip files smaller than this — too small for any prober to conclude.
const MIN_FILE_SIZE: u64 = 64;

/// Whether a workspace-relative path is under one of the TS fixture dirs.
///
/// `rel` is ALREADY relative to the workspace root — the caller strips it. An
/// earlier revision took `root` and stripped a second time, which always failed
/// and fell back to `""`, making this return `false` for every path. The effect
/// was silent and total: every `.ts` file was dropped from the sweep, so the
/// crate's most important format contributed nothing to it, and the count of
/// files reporting `Insufficient` read as 0 when six genuinely do. A sweep that
/// silently skips its subject looks identical to a sweep that passes.
fn is_under_ts_dir(rel: &Path) -> bool {
    let rel_str = rel.to_string_lossy();
    TS_DIRS.iter().any(|d| rel_str.starts_with(d))
}

/// The workspace root: two up from `CARGO_MANIFEST_DIR`.
fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crate dir has a parent")
        .to_path_buf()
}

/// Recursively collect all files under a directory.
fn collect_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("read_dir") {
        let entry = entry.expect("entry");
        let path = entry.path();
        if path.is_dir() {
            collect_files(&path, out);
        } else if path.is_file() {
            out.push(path);
        }
    }
}

#[inline]
fn extension(p: &Path) -> String {
    p.extension()
        .map(|e| e.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Files that legitimately cannot be concluded from, with the reason.
///
/// A TS lattice needs `TS_CONFIRM_FOR_WEAK` (3) contiguous confirmations, so a
/// file holding fewer than 3 whole 188-byte packets — under 564 bytes — cannot
/// reach even the weak tier. These are complete files on disk, so the
/// `Insufficient` they return can never be satisfied; that is the honest
/// verdict, not a gap. See `fixture_ts.rs`'s
/// `tiny_ts_fixtures_below_the_weak_threshold_are_insufficient`.
///
/// This is an explicit allowlist rather than a size heuristic so that a NEW
/// file failing to identify is a test failure, not a silently tolerated line of
/// output.
const EXPECTED_INSUFFICIENT: &[&str] = &[
    "fixtures/ts/emsg-pid4.ts",                  // 188 B — 1 packet
    "fixtures/ts/scte35-balanced.ts",            // 188 B — 1 packet
    "fixtures/ts/scte35-real.ts",                // 188 B — 1 packet
    "fixtures/ts/scte35-unbalanced.ts",          // 188 B — 1 packet
    "fixtures/mpeg-ts/af-pcr-stuffing.ts",       // 376 B — 2 packets
    "fixtures/mpeg-pes/pes-pts-dts-stuffing.ts", // 376 B — 2 packets
];

/// Whether `rel` is on the documented too-small allowlist.
fn is_expected_insufficient(rel: &str) -> bool {
    EXPECTED_INSUFFICIENT.contains(&rel)
}

#[test]
fn corpus_sweep() {
    let root = workspace_root();
    let mut files: Vec<PathBuf> = Vec::new();
    for d in WALK_DIRS {
        let dir = root.join(d);
        if dir.is_dir() {
            collect_files(&dir, &mut files);
        }
    }

    let mut scanned = 0usize;
    let mut identified = 0usize;
    let mut insufficient = 0usize;
    let mut false_positives: Vec<String> = Vec::new();
    let mut ambiguous: Vec<String> = Vec::new();
    let mut misses: Vec<String> = Vec::new();

    for path in &files {
        let rel = path
            .strip_prefix(&root)
            .expect("walk stays within workspace");
        let rel_str = rel.to_string_lossy();
        let ext = extension(path);

        // Determine the expected format.
        let expected = if let Some(f) = extension_format(ext.as_str()) {
            Some(f)
        } else if ext == "ts" && is_under_ts_dir(rel) {
            Some(Format::MpegTs)
        } else {
            None // not a media extension the sweep knows
        };
        let Some(expected) = expected else {
            continue;
        };

        let Ok(data) = fs::read(path) else {
            continue;
        };
        if data.len() < MIN_FILE_SIZE as usize {
            continue;
        }

        scanned += 1;
        let p = container_probe::probe(&data);

        match p {
            Probe::Identified { format, detail, .. } => {
                // Check it is the expected container (for TS also confirm the
                // detail is actually a TS lattice).
                let is_t = format == expected;
                let ts_ok = format != Format::MpegTs || matches!(detail, Detail::Ts { .. });
                if is_t && ts_ok {
                    identified += 1;
                } else {
                    false_positives.push(format!(
                        "{rel_str}: expected {expected:?}, identified {:?}",
                        format
                    ));
                }
            }
            Probe::Insufficient { .. } => {
                if is_expected_insufficient(&rel_str) {
                    insufficient += 1;
                } else {
                    // A file large enough to conclude from that still asks for
                    // more bytes is a prober gap, not a tolerable outcome.
                    misses.push(format!(
                        "{rel_str} ({} bytes, expected {expected:?}): Insufficient, and it is \
                         not on the documented too-small allowlist",
                        data.len()
                    ));
                }
            }
            Probe::Ambiguous { candidates } => {
                ambiguous.push(format!(
                    "{rel_str}: ambiguous between {:?}",
                    candidates.iter().map(|c| c.format).collect::<Vec<_>>()
                ));
            }
            Probe::Unknown => {
                // The sweep only considers files whose extension names a format
                // this crate detects, so `Unknown` here is always a miss — the
                // prober for `expected` failed to fire on a real file of that
                // format.
                misses.push(format!(
                    "{rel_str} ({} bytes): Unknown, expected {expected:?}",
                    data.len()
                ));
            }
            _ => {}
        }
    }

    println!(
        "corpus sweep: {scanned} scanned, {identified} identified, \
         {insufficient} insufficient (allowlisted), {} missed, {} false positives, {} ambiguous",
        misses.len(),
        false_positives.len(),
        ambiguous.len()
    );

    assert!(
        false_positives.is_empty(),
        "files identified as the WRONG format:\n{}",
        false_positives.join("\n")
    );
    assert!(
        ambiguous.is_empty(),
        "files returning Ambiguous:\n{}",
        ambiguous.join("\n")
    );
    // Without this, a prober that stopped firing entirely would leave the sweep
    // green — it would merely print a line per file. Silence must not read as
    // success.
    assert!(
        misses.is_empty(),
        "files that failed to identify:\n{}",
        misses.join("\n")
    );
    assert_eq!(
        insufficient,
        EXPECTED_INSUFFICIENT.len(),
        "every allowlisted too-small file must actually be reached and report \
         Insufficient; if one now identifies, remove it from EXPECTED_INSUFFICIENT"
    );
}
