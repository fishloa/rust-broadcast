//! Availability and live-edge arithmetic: the closed forms against brute force
//! over the per-segment formulas, monotonicity, window bounds and extreme
//! inputs (no panic, no wrap).

use crate::common;

use common::Rng;
use dash_runtime::address::{PlanOptions, plan_representation};
use dash_runtime::mpd::Mpd;
use dash_runtime::selection::RepRef;
use dash_runtime::timing::availability::{
    ato_seconds_to_us, complete_count_constant, init_available_from, segment_available_from,
    segment_available_until,
};
use dash_runtime::timing::live_edge::{join_index, newest_complete, oldest_in_window, window};

const NS: &str = "urn:mpeg:dash:schema:mpd:2011";

fn dynamic_mpd(
    ast_s: i64,
    ps_s: u64,
    ts: u64,
    d: u64,
    tsb_s: Option<u64>,
    ato: Option<f64>,
) -> Mpd {
    let ast = jiff::Timestamp::from_second(ast_s).unwrap();
    let tsb = tsb_s.map_or(String::new(), |t| {
        format!(r#" timeShiftBufferDepth="PT{t}S""#)
    });
    let ato = ato.map_or(String::new(), |a| {
        format!(r#" availabilityTimeOffset="{a}""#)
    });
    let xml = format!(
        r#"<MPD xmlns="{NS}" type="dynamic" availabilityStartTime="{ast}"{tsb}>
           <Period id="p" start="PT{ps_s}S"><AdaptationSet mimeType="video/mp4">
           <Representation id="v" bandwidth="1"><SegmentTemplate timescale="{ts}" duration="{d}"{ato} media="$Number$.m4s"/></Representation>
           </AdaptationSet></Period></MPD>"#
    );
    Mpd::parse(xml.as_bytes()).unwrap_or_else(|e| panic!("{e}\n{xml}"))
}

#[test]
fn closed_form_equals_brute_force_over_segment_formulas() {
    let mut rng = Rng(0xA11CE);
    let scales = [1u64, 25, 1000, 12_800, 44_100, 48_000, 90_000, 1_000_000];
    let mut checked = 0;
    for _ in 0..2_000 {
        let ts = scales[rng.below(scales.len() as u64) as usize];
        let d = rng.range(1, ts * 6 + 7);
        let ast_s = i64::try_from(rng.range(0, 4_000_000_000)).unwrap();
        let ps_s = rng.below(10_000);
        let tsb_s = (rng.below(3) != 0).then(|| rng.range(1, 600));
        let ato =
            (rng.below(3) == 0).then(|| (rng.below(300) as f64 / 100.0) * (d as f64 / ts as f64));
        let m = dynamic_mpd(ast_s, ps_s, ts, d, tsb_s, ato);
        let p = plan_representation(
            &m,
            RepRef::at(0, 0, 0),
            &PlanOptions::default().with_availability_time_offset(true),
        )
        .unwrap();
        let seg_us = (i128::from(d) * 1_000_000 / i128::from(ts)).max(1);
        let origin = (i128::from(ast_s) + i128::from(ps_s)) * 1_000_000;
        let horizon = seg_us * 300;
        let now = origin + i128::from(rng.below(u64::try_from(horizon).unwrap())) - seg_us / 2;
        let brute_newest = (0..1_000u64)
            .take_while(|i| p.plan.get(*i).unwrap().available_from_us.unwrap() <= now)
            .last();
        assert_eq!(
            newest_complete(&p.plan, now),
            brute_newest,
            "ts={ts} d={d} ato={ato:?}"
        );
        if let Some(newest) = brute_newest {
            let brute_oldest = (0..=newest).find(|i| {
                p.plan
                    .get(*i)
                    .unwrap()
                    .available_until_us
                    .is_none_or(|u| u >= now)
            });
            assert_eq!(oldest_in_window(&p.plan, now), brute_oldest);
            if let Some(w) = window(&p.plan, now) {
                assert!(w.oldest <= w.newest);
                let delay = i128::from(rng.below(120)) * 1_000_000;
                let j = join_index(&p.plan, now, delay).unwrap();
                assert!(j >= w.oldest && j <= w.newest);
            }
        }
        checked += 1;
    }
    assert_eq!(checked, 2_000);
}

#[test]
fn newest_complete_is_monotone_in_the_clock() {
    let mut rng = Rng(5);
    let m = dynamic_mpd(1_700_000_000, 0, 90_000, 180_000, Some(60), None);
    let p = plan_representation(&m, RepRef::at(0, 0, 0), &PlanOptions::default()).unwrap();
    let base = 1_700_000_000i128 * 1_000_000;
    let mut last: Option<u64> = None;
    let mut now = base;
    for _ in 0..5_000 {
        now += i128::from(rng.below(3_000_000));
        let n = newest_complete(&p.plan, now);
        assert!(n >= last, "{n:?} < {last:?}");
        last = n;
        if let Some(w) = window(&p.plan, now) {
            assert!(w.oldest <= w.newest);
        }
    }
}

#[test]
fn extreme_inputs_never_panic_or_wrap() {
    for (ast_s, d, ts) in [
        (0i64, 1u64, 1u64),
        (i64::from(i32::MAX), u64::MAX, 1),
        (253_402_207_200, 1, 1_000_000),
        (-62_135_596_800, u64::from(u32::MAX), 1),
    ] {
        let m = dynamic_mpd(ast_s, 0, ts, d.max(1), Some(1), Some(1.0e300));
        let p = plan_representation(
            &m,
            RepRef::at(0, 0, 0),
            &PlanOptions::default().with_availability_time_offset(true),
        )
        .unwrap();
        for now in [i128::MIN / 4, -1, 0, i128::from(i64::MAX), i128::MAX / 4] {
            let _ = newest_complete(&p.plan, now);
            let _ = oldest_in_window(&p.plan, now);
            let _ = join_index(&p.plan, now, i128::MAX / 8);
        }
        let _ = p.plan.get(u64::MAX);
        let _ = p.plan.get(0);
    }
    assert_eq!(complete_count_constant(i128::MAX, 0, 0, 0, 1, 1), u64::MAX);
    assert_eq!(complete_count_constant(i128::MIN, 0, 0, 0, 1, 1), 0);
    assert_eq!(complete_count_constant(0, 0, 0, 0, 0, 1), 0);
    assert_eq!(complete_count_constant(0, 0, 0, 0, 1, 0), 0);
    assert_eq!(ato_seconds_to_us(f64::INFINITY), ato_seconds_to_us(1.0e30));
    assert_eq!(ato_seconds_to_us(f64::NAN), 0);
    assert_eq!(ato_seconds_to_us(1.5), 1_500_000);
    assert_eq!(ato_seconds_to_us(-0.25), -250_000);
}

#[test]
fn availability_time_offset_makes_segments_available_earlier_only_when_applied() {
    let m = dynamic_mpd(1_700_000_000, 0, 1, 2, Some(30), Some(1.5));
    let with = plan_representation(
        &m,
        RepRef::at(0, 0, 0),
        &PlanOptions::default().with_availability_time_offset(true),
    )
    .unwrap();
    let without = plan_representation(&m, RepRef::at(0, 0, 0), &PlanOptions::default()).unwrap();
    let a = with.plan.get(0).unwrap().available_from_us.unwrap();
    let b = without.plan.get(0).unwrap().available_from_us.unwrap();
    assert_eq!(b - a, 1_500_000);
    assert_eq!(
        with.plan.get(0).unwrap().available_until_us,
        without.plan.get(0).unwrap().available_until_us,
        "the end of the window is not shifted"
    );
}

#[test]
fn public_formulas_on_hand_computed_numbers() {
    // AST 1000 + PeriodStart 100 + MPDstart 10 + MPDduration 5 - ATO 3.
    assert_eq!(segment_available_from(1000, 100, 10, 5, 3), 1112);
    // AST 1000 + PeriodStart 100 + MPDstart 10 + 2 x MPDduration 5 + TSB 7.
    assert_eq!(
        segment_available_until(1000, 100, 10, 5, Some(7)),
        Some(1127)
    );
    assert_eq!(segment_available_until(1000, 100, 10, 5, None), None);
    assert_eq!(init_available_from(1000, 100), 1100);
}
