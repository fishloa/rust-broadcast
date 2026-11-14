//! Every committed DASH MPD parses, and the parse is pinned against values read
//! out of the real files.

use broadcast_common::timescale::Timescale;
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

const ISO_LIST_EXAMPLE: &str = "specs/iso_iec_23009-1_dash_2026_ed6_schema/example_G29-2.mpd";

#[test]
fn list_mpd_parses_its_imported_mpds() {
    let Some(root) = test_fixtures::private_root() else {
        eprintln!("SKIPPED list_mpd_parses_its_imported_mpds: private checkout absent");
        return;
    };
    let path = root.join(ISO_LIST_EXAMPLE);
    assert!(
        path.is_file(),
        "private checkout present but {} is missing",
        path.display()
    );
    let mpd = Mpd::parse(&std::fs::read(&path).expect("read")).expect("parse");
    assert_eq!(mpd.kind, PresentationType::List);
    assert_eq!(mpd.periods.len(), 3);
    let urls: Vec<&str> = mpd
        .periods
        .iter()
        .map(|p| p.imported_mpd.as_ref().expect("ImportedMPD").url.as_str())
        .collect();
    assert_eq!(urls, ["ad0.mpd", "ad1.mpd", "ad2.mpd"]);
    assert_eq!(
        mpd.periods[0]
            .imported_mpd
            .as_ref()
            .expect("ImportedMPD")
            .earliest_resolution_time_offset,
        Some(0.0)
    );
}

const INHERITING_TIMELINE: &str = "specs/iso_iec_23009-1_dash_2026_ed6_schema/example_G22.mpd";

#[test]
fn a_representation_inherits_the_adaptation_sets_timeline() {
    let Some(root) = test_fixtures::private_root() else {
        eprintln!(
            "SKIPPED a_representation_inherits_the_adaptation_sets_timeline: private checkout absent"
        );
        return;
    };
    let path = root.join(INHERITING_TIMELINE);
    assert!(
        path.is_file(),
        "private checkout present but {} is missing",
        path.display()
    );
    let mpd = Mpd::parse(&std::fs::read(&path).expect("read")).expect("parse");
    let set = &mpd.periods.first().expect("period").adaptation_sets[0];
    let declared = set
        .segment
        .template
        .as_ref()
        .expect("AdaptationSet SegmentTemplate");
    assert_eq!(
        declared
            .multi
            .timeline
            .as_ref()
            .expect("timeline")
            .entries
            .len(),
        3
    );
    let bare = set
        .representations
        .iter()
        .find(|r| {
            r.segment
                .template
                .as_ref()
                .is_some_and(|t| t.multi.timeline.is_none())
        })
        .expect("a Representation with a SegmentTemplate of its own");
    let info = dash_runtime::mpd::inherit::resolve_segment_info(&mpd.periods[0], set, bare)
        .expect("resolve");
    let dash_runtime::mpd::inherit::SegmentInfo::Template(info) = info else {
        panic!("expected a SegmentTemplate chain");
    };
    assert_eq!(
        info.multi
            .timeline
            .expect("inherited timeline")
            .entries
            .len(),
        3
    );
    assert_eq!(info.multi.timescale, Timescale::from_nonzero(90_000));
}
