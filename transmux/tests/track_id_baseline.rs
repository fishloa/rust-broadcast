//! Real-data gate for the typed-id migration (#1159): demuxes every committed
//! fixture (and, when present, every real capture of the private corpus up to 40 MB) and
//! compares the per-track `track_id` values and sample counts with
//! `tests/golden/track_ids.txt`, which was captured when `track_id` was a bare
//! `u32`. A `TrackId` newtype must render and number identically.
//!
//! Intentional changes since the capture (each justified; do not re-bless casually):
//! - `fixtures/ts/m6-{single,discontinuity,duplicate}.ts` track 1 `1x7` -> `1x8` (#1255).
//!   Track 1 is PID 0x82, signalled `stream_type 0x06` + DVB Enhanced AC-3
//!   descriptor (0x7A), i.e. E-AC-3 (the PMT of `m6-single.ts`; the other two are
//!   cuts of the same broadcast). The track did not change codec, id or order:
//!   it gains one real E-AC-3 frame (bsid 16, contiguous with the previous
//!   frame's timestamp). A `0B 77` pair inside the payload whose byte 5 reads as
//!   bsid <= 8 used to be parsed as an E-AC-3 header with a garbage length and
//!   swallow that frame; an AC-3-shaped candidate is now judged by its AC-3 CRC
//!   and rejected. Plain AC-3 PIDs (stream_type 0x81 / AC-3 descriptor) are
//!   unaffected (`audio_pes_straddle::sintel_plain_ac3_stays_ac3_*`).
//! - `tsduck-test/input/test-013.ts` `(none)` -> `1x31494` and
//!   `tsduck-test/reference/test-089/test-089.align-000000.ts` gains a new first track
//!   (`1x102`, every later id +1) since #1326 (#1579). test-013's PMT declares AVC video
//!   0x425, audio 0x426 and teletext 0x428; the capture carries data only on 0x428, and
//!   the absent video head used to hold back every later track until `finish()`, so the
//!   teletext track was never emitted. test-089 `align-000000` opens on an IDR and
//!   carries 1920x1080 H.264 on PID 0x140 (ffprobe agrees), which the 512-AU skip used
//!   to step over and drop; a video head now blocks until it resolves, so it is track 1.
//!   Both are real PMT-declared elementary streams (ISO/IEC 13818-1 Table 2-33,
//!   `elementary_PID`). Both captures are mirrored in `private/fixtures/tsduck/`.
//!
//! Regenerate (only for an intentional behaviour change):
//! `TRACK_ID_BLESS=1 cargo test -p transmux --all-features --test track_id_baseline`
#![cfg(feature = "std")]

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use broadcast_common::Unpackage;
use transmux::{FlvDemux, Fmp4Demux, Media, PsDemux, TsDemux, WebmDemux};

const MAX_BYTES: u64 = 40 * 1024 * 1024;

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, out);
        } else {
            out.push(p);
        }
    }
}

fn demux(path: &Path, bytes: &[u8]) -> Option<Media> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        "ts" | "m2ts" => TsDemux::new().unpackage(bytes).ok(),
        "mp4" | "m4s" | "m4a" | "m4v" | "cmfv" | "cmfa" => Fmp4Demux::new().unpackage(bytes).ok(),
        "mkv" | "webm" => WebmDemux::new().unpackage(bytes).ok(),
        "flv" => FlvDemux::new().unpackage(bytes).ok(),
        "ps" | "mpg" | "mpeg" => PsDemux::new().unpackage(bytes).ok(),
        _ => None,
    }
}

fn summarize(root: &Path, key: impl Fn(&str) -> Option<String>) -> String {
    let mut files = Vec::new();
    walk(root, &mut files);
    files.sort();
    let mut s = String::new();
    for f in files {
        let Ok(md) = std::fs::metadata(&f) else {
            continue;
        };
        if md.len() > MAX_BYTES {
            continue;
        }
        let Ok(bytes) = std::fs::read(&f) else {
            continue;
        };
        let Some(media) = demux(&f, &bytes) else {
            continue;
        };
        let rel = f.strip_prefix(root).unwrap().display().to_string();
        let Some(key) = key(&rel) else {
            continue;
        };
        let mut line = format!("{key}:");
        for t in &media.tracks {
            let _ = write!(line, " {}x{}", t.track_id(), t.samples.len());
        }
        s.push_str(&line);
        s.push('\n');
    }
    s
}

fn golden_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/track_ids.txt")
}

fn lines(s: &str) -> BTreeMap<String, String> {
    s.lines()
        .filter_map(|l| {
            l.split_once(':')
                .map(|(k, v)| (k.to_string(), v.to_string()))
        })
        .collect()
}

#[test]
fn track_ids_match_pre_typed_golden() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut got = summarize(&manifest.join("../fixtures"), |r| {
        Some(format!("fixtures/{r}"))
    });
    // Real-capture corpus. The golden keys predate the private repo and keep
    // their historic `test-streams/...` spelling: `private/fixtures/ts/X` ->
    // `test-streams/X`, `.../tsduck/Y` -> `test-streams/tsduck-test/Y`,
    // `.../mpegh/Z` -> `test-streams/mpegh/Z`; other private entries are new
    // captures and are not in the golden.
    if let Some(private) = test_fixtures::private_root() {
        got.push_str(&summarize(&private.join("fixtures"), |r| {
            if let Some(x) = r.strip_prefix("ts/") {
                Some(format!("test-streams/{x}"))
            } else if let Some(x) = r.strip_prefix("tsduck/") {
                Some(format!("test-streams/tsduck-test/{x}"))
            } else {
                r.strip_prefix("mpegh/")
                    .map(|x| format!("test-streams/mpegh/{x}"))
            }
        }));
    }
    // Fetched (pinned tsduck-test clone) legacy directory.
    got.push_str(&summarize(&manifest.join("../.test-streams"), |r| {
        Some(format!("test-streams/{r}"))
    }));
    if std::env::var_os("TRACK_ID_BLESS").is_some() {
        std::fs::write(golden_path(), &got).unwrap();
        return;
    }
    let want = lines(&std::fs::read_to_string(golden_path()).unwrap());
    let got = lines(&got);
    assert!(!got.is_empty(), "no fixtures found");
    let mut bad = Vec::new();
    let mut checked = 0;
    for (k, v) in &got {
        match want.get(k) {
            Some(w) if w == v => checked += 1,
            Some(w) => bad.push(format!("{k}: got{v} want{w}")),
            // A file absent from the golden is a new capture, not a regression.
            None => {}
        }
    }
    eprintln!("track_id golden: {checked} files matched");
    assert!(bad.is_empty(), "track ids changed: {bad:#?}");
    // Every committed (not real-capture corpus) golden entry must still demux.
    for k in want.keys().filter(|k| k.starts_with("fixtures/")) {
        assert!(
            got.contains_key(k),
            "committed fixture {k} no longer demuxes"
        );
    }
}
