use broadcast_common::Serialize;
use scte35_splice::SpliceInfoSection;
use scte35_splice::commands::{AnyCommand, SpliceInsert};
use scte35_splice::time::SpliceTime;
use timed_metadata::{TimeAnchor, Timeline};

fn splice_at(pts: u64) -> Vec<u8> {
    let cmd = SpliceInsert {
        splice_event_id: 7,
        out_of_network_indicator: true,
        splice_time: Some(SpliceTime::with_pts(pts)),
        ..Default::default()
    };
    SpliceInfoSection::new_clear(AnyCommand::SpliceInsert(cmd), &[])
        .try_to_bytes()
        .unwrap()
}

#[test]
fn anchor_just_before_the_33_bit_wrap_places_a_cue_just_after_it() {
    const WRAP: u64 = 1 << 33;
    let anchor = TimeAnchor {
        pts_90k: WRAP - 90_000,
        utc_epoch_ms: 1_700_000_000_000,
    };
    let mut tl = Timeline::with_anchor(anchor);
    let ev = tl.push_scte35(&splice_at(90_000)).unwrap();
    let dr = tl.to_daterange(&ev).unwrap();
    assert_eq!(
        dr.start_date, "2023-11-14T22:13:22.000Z",
        "cue 2 s after the anchor (across the PTS wrap) must start 2 s after the anchor UTC"
    );
}
