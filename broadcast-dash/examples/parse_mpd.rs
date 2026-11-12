//! Parse a real DASH MPD — the committed `fixtures/dash/manifest.mpd` — into the
//! typed model and print its Periods, AdaptationSets and Representations.
//!
//! Run with `cargo run -p broadcast-dash --example parse_mpd`.

use std::path::PathBuf;

use broadcast_dash::Mpd;

fn segment_kind(seg: &broadcast_dash::SegmentLayer) -> &'static str {
    if seg.template.is_some() {
        "SegmentTemplate"
    } else if seg.list.is_some() {
        "SegmentList"
    } else if seg.base.is_some() {
        "SegmentBase"
    } else {
        "none"
    }
}

fn main() {
    let path = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| test_fixtures::path("dash/manifest.mpd"));
    let xml = std::fs::read_to_string(&path).expect("read the committed MPD fixture");

    let mpd = Mpd::parse(xml.as_bytes()).expect("valid MPD");
    println!(
        "MPD id={:?} type={:?} periods={}",
        mpd.id,
        mpd.kind,
        mpd.periods.len()
    );
    for (pi, period) in mpd.periods.iter().enumerate() {
        println!("Period {} (id={:?})", pi, period.id);
        for (ai, set) in period.adaptation_sets.iter().enumerate() {
            println!(
                "  AdaptationSet {} (id={:?}, content_type={:?}, lang={:?})",
                ai, set.id, set.content_type, set.lang
            );
            for rep in &set.representations {
                println!(
                    "    Representation id={} bandwidth={} codecs={:?} segments={}",
                    rep.id,
                    rep.bandwidth,
                    rep.common.codecs,
                    segment_kind(&rep.segment)
                );
            }
        }
    }

    let rendered = mpd.to_xml().expect("render");
    assert_eq!(Mpd::parse(rendered.as_bytes()).expect("re-parse"), mpd);
}
