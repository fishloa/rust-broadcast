//! Issue #1186: E-AC-3 config recovery now waits for a complete access unit,
//! and (when the programme has dependent substreams) holds the final access
//! unit of a PES back so a dependent substream starting the next PES can join
//! it. Real broadcast captures carry NO dependent substreams, so these runs
//! prove the new machinery changes nothing for them: the config is found, no
//! dependent substream is invented, and every sample is exactly one whole
//! independent syncframe at the frame-exact cadence.
//!
//! Inputs: the committed `fixtures/ts/dolby/eac3.ts` and
//! `fixtures/ts/gulli-opengop.ts`, plus (skipped cleanly when absent) the
//! first 24 MB of the local archive's `france-tnt-uhf32.ts` (never committed).

use std::path::PathBuf;

use broadcast_common::Unpackage;
use transmux::pipeline::CodecConfig;
use transmux::{Ec3SyncframeInfo, TsDemux};

const AC3_SYNC: [u8; 2] = [0x0B, 0x77];

/// Per E-AC-3 track: `(samples, whole-frame timeline gaps)`.
fn check(label: &str, ts: &[u8]) -> Vec<(usize, usize)> {
    let media = TsDemux::new().unpackage(ts).expect("demux");
    let mut out = Vec::new();
    for track in &media.tracks {
        let CodecConfig::Eac3 { config, .. } = &track.spec.config else {
            continue;
        };
        assert!(
            config
                .substreams
                .iter()
                .all(|s| s.num_dep_sub == 0 && s.chan_loc.is_none()),
            "{label}: a real 5.1 capture has no dependent substream, got {config:?}"
        );
        assert!(!track.samples.is_empty(), "{label}: samples emitted");
        let mut prev_dts: Option<i64> = None;
        let mut gaps = 0usize;
        for (i, s) in track.samples.iter().enumerate() {
            assert_eq!(&s.data[..2], &AC3_SYNC, "{label}: sample {i} starts at a syncword");
            let frames = Ec3SyncframeInfo::from_es_all(&s.data);
            assert_eq!(frames.len(), 1, "{label}: sample {i} holds one syncframe");
            let info = frames[0];
            assert_eq!(
                (info.frmsiz as usize + 1) * 2,
                s.data.len(),
                "{label}: sample {i} is exactly one whole syncframe"
            );
            assert_eq!(s.duration, Some(info.samples_per_frame()));
            if let Some(p) = prev_dts {
                // Frame-exact cadence, except where the real capture itself
                // re-anchors (a timeline gap is a whole number of frames).
                let delta = s.dts.unwrap() - p;
                let frame = info.samples_per_frame() as i64;
                if delta != frame {
                    assert!(
                        delta > frame && delta % frame == 0,
                        "{label}: sample {i} dts delta {delta} is neither one frame nor a whole-frame gap"
                    );
                    gaps += 1;
                }
            }
            prev_dts = s.dts;
        }
        out.push((track.samples.len(), gaps));
    }
    out
}

fn committed(name: &str) -> Vec<u8> {
    let path: PathBuf = [env!("CARGO_MANIFEST_DIR"), "..", "fixtures", "ts", name]
        .iter()
        .collect();
    std::fs::read(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

#[test]
fn committed_real_eac3_captures_unchanged() {
    assert_eq!(check("dolby/eac3.ts", &committed("dolby/eac3.ts")), [(58, 0)]);
    assert_eq!(
        check("gulli-opengop.ts", &committed("gulli-opengop.ts")),
        [(230, 0)]
    );
}

#[test]
fn local_archive_france_tnt_eac3_unchanged() {
    let path: PathBuf = [
        env!("CARGO_MANIFEST_DIR"),
        "..",
        ".test-streams",
        "france-tnt-uhf32.ts",
    ]
    .iter()
    .collect();
    let Ok(f) = std::fs::File::open(&path) else {
        eprintln!("SKIP: {} not present (local archive only)", path.display());
        return;
    };
    use std::io::Read;
    let mut buf = Vec::new();
    f.take(24 * 1024 * 1024).read_to_end(&mut buf).unwrap();
    // Cut to a whole number of TS packets.
    buf.truncate(buf.len() / 188 * 188);
    // Pinned from the pre-#1186 demuxer's output over the same bytes (the
    // capture itself carries timeline gaps); identical after the change.
    assert_eq!(
        check("france-tnt-uhf32.ts[..24MB]", &buf),
        [
            (222, 3),
            (222, 3),
            (217, 5),
            (216, 5),
            (216, 5),
            (204, 7),
            (210, 6),
            (189, 5),
            (210, 8),
            (222, 4),
            (222, 4),
            (217, 4)
        ]
    );
}
