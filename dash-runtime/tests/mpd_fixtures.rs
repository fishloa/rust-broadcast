//! Every committed DASH MPD parses, and the parse is pinned against values read
//! out of the real files.

use std::path::{Path, PathBuf};

use dash_runtime::mpd::{Mpd, PresentationType};

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("read fixtures dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            collect(&path, out);
        } else if path.extension().is_some_and(|e| e == "mpd") {
            out.push(path);
        }
    }
}

fn all_mpds() -> Vec<PathBuf> {
    let mut out = Vec::new();
    collect(&test_fixtures::path("dash"), &mut out);
    out.sort();
    out
}

#[test]
fn every_committed_mpd_parses() {
    let files = all_mpds();
    assert!(
        files.len() >= 24,
        "expected the full P0 set, found {}",
        files.len()
    );
    for f in files {
        let bytes = std::fs::read(&f).unwrap();
        Mpd::parse(&bytes).unwrap_or_else(|e| panic!("{}: {e}", f.display()));
    }
}

#[test]
fn livesim2_dynamic_mpd_values() {
    let bytes = test_fixtures::read("dash/livesim2/segtimeline_time_a.mpd");
    let mpd = Mpd::parse(&bytes).unwrap();
    assert_eq!(mpd.kind, PresentationType::Dynamic);
    assert_eq!(mpd.availability_start_time_us, Some(0));
    assert_eq!(mpd.publish_time_us, Some(1_791_934_510_000_000));
    assert_eq!(mpd.periods.len(), 1);
    assert_eq!(mpd.periods[0].adaptation_sets.len(), 2);
    assert_eq!(mpd.utc_timings.len(), 1);
}
