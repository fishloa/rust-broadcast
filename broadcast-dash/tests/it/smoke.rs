//! The crate's public entry points reachable from outside it: one MPD parsed
//! by path, one template rendered, one duration read back.

use broadcast_dash::duration::parse_iso8601_duration;
use broadcast_dash::template::{scan_media, substitute};
use broadcast_dash::{Mpd, PresentationType};
use std::time::Duration;
use test_fixtures as fixtures;

#[test]
fn a_committed_mpd_parses_and_describes_its_segments() {
    let text = fixtures::read_to_string("dash/annex-g/example-F1.mpd");
    let mpd = Mpd::parse(text.as_bytes()).expect("example F1 parses");
    assert_eq!(mpd.kind, PresentationType::Static);
    let set = &mpd.periods[0].adaptation_sets[0];
    assert_eq!(set.common.mime_type.as_deref(), Some("video/mp4"));
    assert_eq!(set.representations.len(), 2);
}

#[test]
fn a_media_template_renders_and_an_xs_duration_round_trips() {
    let use_ = scan_media("chunk-$Number%05d$.m4s").expect("valid template");
    assert!(use_.number);
    let url = substitute("chunk-$Number%05d$.m4s", "v", Some(7), None, 0).expect("renders");
    assert_eq!(url, "chunk-00007.m4s");
    assert_eq!(
        parse_iso8601_duration("PT1M30S").expect("valid duration"),
        Duration::from_secs(90)
    );
}
