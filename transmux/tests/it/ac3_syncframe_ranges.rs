//! `split_ac3_syncframe_ranges` over a real ffmpeg-made AC-3 elementary stream
//! (`fixtures/ps/ffmpeg-ac3-dts.a0.ac3`): contiguous ranges when clean, and, in
//! resync mode, ranges that are NOT contiguous from offset 0 after junk or damage.

use broadcast_bitstream::ac3_syncframe::{
    split_ac3_syncframe_ranges, split_ac3_syncframes_resyncing,
};

const SYNCWORD: [u8; 2] = [0x0B, 0x77];

fn stream() -> Vec<u8> {
    test_fixtures::read("ps/ffmpeg-ac3-dts.a0.ac3")
}

#[test]
fn a_clean_stream_is_split_into_contiguous_frames_covering_every_byte() {
    let es = stream();
    let ranges = split_ac3_syncframe_ranges(&es, false);
    assert!(ranges.len() > 10, "{} frames", ranges.len());
    assert_eq!(ranges[0].0, 0);
    for pair in ranges.windows(2) {
        assert_eq!(pair[0].1, pair[1].0, "frames are back to back");
    }
    assert_eq!(ranges.last().unwrap().1, es.len());
    for &(start, end) in &ranges {
        assert_eq!(es[start..start + 2], SYNCWORD);
        assert!(end > start);
    }
    assert_eq!(split_ac3_syncframe_ranges(&es, true), ranges);
    let slices = split_ac3_syncframes_resyncing(&es, false);
    assert_eq!(slices.len(), ranges.len());
    assert_eq!(slices[3], &es[ranges[3].0..ranges[3].1]);
}

#[test]
fn junk_before_the_first_frame_stops_a_strict_scan_and_is_skipped_by_a_resyncing_one() {
    let es = stream();
    let clean = split_ac3_syncframe_ranges(&es, false);
    let junk = [0x00, 0x11, 0x22, 0x33, 0x44];
    let prefixed = [junk.as_slice(), &es].concat();

    assert!(split_ac3_syncframe_ranges(&prefixed, false).is_empty());
    let resynced = split_ac3_syncframe_ranges(&prefixed, true);
    let shifted: Vec<(usize, usize)> = clean
        .iter()
        .map(|&(s, e)| (s + junk.len(), e + junk.len()))
        .collect();
    assert_eq!(resynced, shifted);
    assert_ne!(
        resynced[0].0, 0,
        "after a resync ranges do not start at offset 0"
    );
}

#[test]
fn a_damaged_frame_loses_only_that_frame_when_resyncing() {
    let es = stream();
    let clean = split_ac3_syncframe_ranges(&es, false);
    let victim = 4;
    let mut damaged = es.clone();
    damaged[clean[victim].0] = 0x00;

    let strict = split_ac3_syncframe_ranges(&damaged, false);
    assert_eq!(
        strict,
        clean[..victim].to_vec(),
        "a strict scan stops at the damage"
    );

    let resynced = split_ac3_syncframe_ranges(&damaged, true);
    let mut expected = clean.clone();
    expected.remove(victim);
    assert_eq!(resynced, expected);
}

#[test]
fn a_truncated_last_frame_is_dropped() {
    let es = stream();
    let clean = split_ac3_syncframe_ranges(&es, false);
    let cut = &es[..es.len() - 1];
    for resync in [false, true] {
        assert_eq!(
            split_ac3_syncframe_ranges(cut, resync),
            clean[..clean.len() - 1].to_vec(),
            "resync={resync}"
        );
    }
}

/// A `0x0B77` whose header does not parse is a false sync *inside* a frame; the
/// two scan modes must part ways there. Strict stops (the payload is not what
/// it claims), resync steps one byte on and recovers every later frame.
#[test]
fn a_false_sync_is_skipped_only_when_resyncing() {
    let es = stream();
    let clean = split_ac3_syncframe_ranges(&es, false);
    let first_two = &clean[..2];
    // fscod 0b11 is reserved (Table 4.1), so this header declares no length.
    let false_sync = [0x0B, 0x77, 0xFF, 0xFF, 0xFF, 0xFF];
    let payload = [&es[..first_two[1].1], &false_sync, &es[first_two[1].1..]].concat();

    assert_eq!(
        split_ac3_syncframe_ranges(&payload, false),
        first_two.to_vec(),
        "a strict scan stops at the false sync"
    );

    let mut expected = first_two.to_vec();
    for &(s, e) in &clean[2..] {
        expected.push((s + false_sync.len(), e + false_sync.len()));
    }
    assert_eq!(
        split_ac3_syncframe_ranges(&payload, true),
        expected,
        "a resyncing scan recovers the frames after the false sync"
    );
}
