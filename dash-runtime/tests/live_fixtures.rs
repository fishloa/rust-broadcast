//! Dynamic fixtures: the availability window at `publishTime` against the
//! segments the real ffmpeg / livesim2 run had on disk, and consistency of the
//! plan across two consecutive real MPD refreshes.

mod common;

use common::load;
use dash_runtime::mpd::PresentationType;
use dash_runtime::selection::RepRef;
use dash_runtime::timing::datetime::parse_xs_datetime;
use dash_runtime::timing::live_edge::{join_index, newest_complete, oldest_in_window, window};

const SECOND_US: i128 = 1_000_000;

fn ts(text: &str) -> i128 {
    i128::from(parse_xs_datetime(text).unwrap().unix_us)
}

#[test]
fn ffmpeg_live_number_window_matches_the_capture() {
    let l = load("dash/live-number/live.mpd");
    assert_eq!(l.mpd.kind, PresentationType::Dynamic);
    let publish = i128::from(l.mpd.publish_time_us.unwrap());
    assert_eq!(publish, ts("2026-10-26T09:18:20.392Z"));
    let video = l.plan(RepRef::at(0, 0, 0));
    assert!(video.plan.len().is_none(), "no Period end: open-ended plan");
    let newest = newest_complete(&video.plan, publish).unwrap();
    assert_eq!(video.plan.get(newest).unwrap().number, 7);
    let oldest = oldest_in_window(&video.plan, publish).unwrap();
    assert_eq!(video.plan.get(oldest).unwrap().number, 3);
    for number in 5..=7 {
        let seg = video.plan.get(number - 1).unwrap();
        assert_eq!(seg.number, number);
        assert!(
            seg.index >= oldest && seg.index <= newest,
            "segment {number} is in the window"
        );
        let file = l.file_of(&video, &seg.reference);
        assert!(file.is_file(), "{}", file.display());
    }
    let eight = video.plan.get(7).unwrap();
    assert!(
        eight.available_from_us.unwrap() > publish,
        "segment 8 is on disk but not yet announced available"
    );
    assert_eq!(eight.reference, "seg-0-008.m4s");
    // SPD 2 s: T_join = publish - 2 s - AST = 13.667 s, inside segment 7.
    let delay = 2 * SECOND_US;
    let join = join_index(&video.plan, publish, delay).unwrap();
    assert_eq!(video.plan.get(join).unwrap().number, 7);
    // Delay 0 puts T_join inside the not-yet-complete segment 8: clamped back to 7.
    let join0 = join_index(&video.plan, publish, 0).unwrap();
    assert_eq!(video.plan.get(join0).unwrap().number, 7);
    // A much older clock clamps to the oldest segment still in the window.
    let w = window(&video.plan, publish).unwrap();
    assert_eq!(
        join_index(&video.plan, publish, 100 * SECOND_US).unwrap(),
        w.oldest
    );
    // Before the first segment completes there is nothing to join.
    let ast = i128::from(l.mpd.availability_start_time_us.unwrap());
    assert!(newest_complete(&video.plan, ast + SECOND_US).is_none());
    // Init is available from AST + PeriodStart.
    assert_eq!(video.plan.init().unwrap().available_from_us, Some(ast));
}

#[test]
fn ffmpeg_live_timeline_window_matches_the_capture() {
    let l = load("dash/live-timeline/live.mpd");
    let publish = i128::from(l.mpd.publish_time_us.unwrap());
    let video = l.plan(RepRef::at(0, 0, 0));
    let audio = l.plan(RepRef::at(0, 1, 0));
    assert_eq!(video.plan.len(), Some(4));
    assert_eq!(audio.plan.len(), Some(4));
    let numbers = |p: &dash_runtime::address::PlannedRepresentation| -> Vec<u64> {
        p.plan.iter().map(|s| s.number).collect()
    };
    assert_eq!(numbers(&video), vec![3, 4, 5, 6]);
    assert_eq!(numbers(&audio), vec![5, 6, 7, 8]);
    for p in [&video, &audio] {
        for seg in p.plan.iter() {
            assert!(l.file_of(p, &seg.reference).is_file());
        }
    }
    assert_eq!(video.plan.get(0).unwrap().reference, "seg-0-003.m4s");
    // The newest listed video segment ends 0.56 s after publishTime: listed
    // segments may run one ahead (IOP 4.3.6), the formula still gates it.
    let newest = newest_complete(&video.plan, publish).unwrap();
    assert_eq!(video.plan.get(newest).unwrap().number, 5);
    assert_eq!(video.plan.get(3).unwrap().number, 6);
    assert!(video.plan.get(3).unwrap().available_from_us.unwrap() > publish);
}

#[test]
fn livesim2_time_timeline_window_and_numbers() {
    let l = load("dash/livesim2/segtimeline_time_a.mpd");
    assert_eq!(
        l.mpd.availability_start_time_us,
        Some(0),
        "epoch zero is a value"
    );
    let publish = i128::from(l.mpd.publish_time_us.unwrap());
    let audio = l.plan(RepRef::at(0, 0, 0));
    let video = l.plan(RepRef::at(0, 1, 0));
    assert_eq!(video.plan.len(), Some(31));
    let first = video.plan.get(0).unwrap();
    assert_eq!(first.reference, "V300/161274100320000.m4s");
    assert_eq!(first.time_ticks, 161_274_100_320_000);
    let last = video.plan.get(30).unwrap();
    assert_eq!(last.reference, "V300/161274105720000.m4s");
    assert_eq!(
        last.available_from_us.unwrap(),
        publish,
        "newest segment completes at publishTime"
    );
    assert_eq!(newest_complete(&video.plan, publish), Some(30));
    assert_eq!(oldest_in_window(&video.plan, publish), Some(0));
    assert_eq!(video.plan.init().unwrap().reference, "V300/init.mp4");
    let total: u64 = audio.plan.iter().map(|s| s.duration_ticks).sum();
    assert_eq!(audio.plan.get(0).unwrap().time_ticks, 86_012_853_504_000);
    let audio_end_us = (i128::from(86_012_853_504_000u64) + i128::from(total)) * SECOND_US / 48_000;
    assert!(
        (audio_end_us - publish).abs() < 2 * SECOND_US,
        "audio list ends near publishTime"
    );
}

#[test]
fn livesim2_consecutive_refreshes_keep_numbers_stable() {
    let a = load("dash/livesim2/segtimelinenr_a.mpd");
    let b = load("dash/livesim2/segtimelinenr_b.mpd");
    assert!(
        b.mpd.publish_time_us > a.mpd.publish_time_us,
        "b is the later refresh"
    );
    for set in 0..2 {
        let pa = a.plan(RepRef::at(0, set, 0));
        let pb = b.plan(RepRef::at(0, set, 0));
        let by_number = |p: &dash_runtime::address::PlannedRepresentation| {
            p.plan
                .iter()
                .map(|s| (s.number, (s.time_ticks, s.duration_ticks, s.reference)))
                .collect::<std::collections::BTreeMap<_, _>>()
        };
        let (ma, mb) = (by_number(&pa), by_number(&pb));
        let overlap: Vec<_> = ma.keys().filter(|n| mb.contains_key(n)).collect();
        assert!(overlap.len() >= 2, "set {set}: the two windows overlap");
        for n in overlap {
            assert_eq!(
                ma[n], mb[n],
                "set {set}: segment {n} identical across the refresh"
            );
        }
        let first_b = *mb.keys().next().unwrap();
        let first_a = *ma.keys().next().unwrap();
        assert!(first_b >= first_a, "the window only moves forward");
    }
}

#[test]
fn livesim2_number_variant_uses_a_large_start_number() {
    let l = load("dash/livesim2/segtimelinenr_a.mpd");
    let video = l.plan(RepRef::at(0, 1, 0));
    let first = video.plan.get(0).unwrap();
    assert_eq!(first.number, 895_967_208);
    assert_eq!(first.reference, format!("V300/{}.m4s", first.number));
}
