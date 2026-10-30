mod common;
use common::load;
use dash_runtime::address::list;
use dash_runtime::mpd::inherit::resolve_segment_info;
#[test]
fn segmentlist_expand_pins_timing() {
    let l = load("dash/segmentlist/list.mpd");
    let period = &l.mpd.periods[0];
    let set = &period.adaptation_sets[0];
    let rep = &set.representations[0];
    let info = resolve_segment_info(period, set, rep).unwrap();
    let list_info = match info {
        dash_runtime::mpd::inherit::SegmentInfo::List(x) => x,
        _ => panic!("not list"),
    };
    let segs = list::expand(&list_info, None, None).unwrap();
    assert_eq!(segs[0].number, 1);
    assert_eq!(segs[0].time, 0);
    assert_eq!(segs[1].time, 2_000_000);
    assert_eq!(segs.len(), 6);
}
