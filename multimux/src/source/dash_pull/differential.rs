//! Design 8.2(c): the core's segment plan against the planner `dash_pull` ran
//! before it (`legacy_plan`), on the presentations that planner could address:
//! `$Number$` with a constant `@duration`, and `SegmentTimeline` with `$Time$`
//! or `$Number$`, over one Period.
//!
//! The two parsers read the same committed MPD; the old planner's
//! `(number, time)` list must be the one `dash_runtime::address` produces. A
//! difference on this subset is either a defect of the core or a documented
//! behaviour change, and each such difference is named in the test that finds
//! it.

use dash_runtime::address::{PlanOptions, plan_representation};
use dash_runtime::mpd::Mpd as NewMpd;
use dash_runtime::selection::RepRef;
use transmux::dash_parse::Mpd as OldMpd;

use super::legacy_plan::{build_plan, period_length, plan_timeline};

struct Both {
    old: OldMpd,
    new: NewMpd,
}

fn load_text(text: &str) -> Both {
    Both {
        old: OldMpd::parse(text).expect("old parser"),
        new: NewMpd::parse(text.as_bytes()).expect("new parser"),
    }
}

fn load(rel: &str) -> Both {
    load_text(&test_fixtures::read_to_string(rel))
}

impl Both {
    fn old_template(&self, p: usize, s: usize, r: usize) -> &transmux::dash_parse::SegmentTemplate {
        self.old.periods[p].adaptation_sets[s].representations[r]
            .segment_template
            .as_ref()
            .expect("the old planner reads templates only")
    }

    fn period_len(&self, p: usize) -> Option<std::time::Duration> {
        period_length(&self.old.periods[p], &self.old)
    }

    /// Every segment the core lists for the Representation, `(number, ticks)`.
    fn new_plan(&self, p: usize, s: usize, r: usize) -> Vec<(u64, u64)> {
        let planned = plan_representation(&self.new, RepRef::at(p, s, r), &PlanOptions::default())
            .expect("the core plans it");
        planned
            .plan
            .iter()
            .take(100_000)
            .map(|seg| (seg.number, seg.time_ticks))
            .collect()
    }

    fn reps(&self, p: usize) -> Vec<(usize, usize)> {
        self.old.periods[p]
            .adaptation_sets
            .iter()
            .enumerate()
            .flat_map(|(s, set)| (0..set.representations.len()).map(move |r| (s, r)))
            .collect()
    }
}

/// The real ffmpeg presentation: a `SegmentTimeline` with an `S@t` that is not
/// zero (2070 ticks of composition offset) and a three-entry audio timeline.
#[test]
fn a_static_timeline_lists_the_same_segments() {
    let both = load("dash/manifest.mpd");
    for (s, r) in both.reps(0) {
        let st = both.old_template(0, s, r);
        let (old, _) = plan_timeline(st, both.period_len(0), None, None).expect("old plan");
        let old: Vec<(u64, u64)> = old
            .into_iter()
            .map(|(n, t)| (n, t.expect("a timeline carries times")))
            .collect();
        assert_eq!(both.new_plan(0, s, r), old, "set {s} representation {r}");
    }
}

/// GPAC's `$Number$` + `@duration` multi-Period presentation: every Period of
/// it is a presentation the old planner could address on its own.
#[test]
fn a_number_template_lists_the_same_numbers_in_every_period() {
    let both = load("dash/multiperiod-restart/multiperiod.mpd");
    assert_eq!(both.old.periods.len(), 2);
    for p in 0..2 {
        for (s, r) in both.reps(p) {
            let st = both.old_template(p, s, r);
            let old = build_plan(st, both.period_len(p), st.start_number, None).expect("old plan");
            let old: Vec<u64> = old.into_iter().map(|(n, _)| n).collect();
            let new: Vec<u64> = both.new_plan(p, s, r).into_iter().map(|(n, _)| n).collect();
            assert_eq!(new, old, "period {p} set {s} representation {r}");
            assert!(new.len() >= 3);
        }
    }
}

/// Real DASH-IF livesim2 `$Time$` timeline: the old planner kept the last
/// three segments of a live window; they are the last three of the core's.
/// After a refresh it extended by time: the segments it added are those the
/// core lists from the old window's end on.
#[test]
fn a_live_time_timeline_agrees_on_the_tail_and_across_a_refresh() {
    let a = load("dash/livesim2/segtimeline_time_a.mpd");
    let b = load("dash/livesim2/segtimeline_time_b.mpd");
    let (s, r) = a
        .reps(0)
        .into_iter()
        .find(|(s, r)| a.old.periods[0].adaptation_sets[*s].representations[*r].id == "V300")
        .expect("the V300 video Representation");
    let (old_a, end_a) =
        plan_timeline(a.old_template(0, s, r), a.period_len(0), None, Some(3)).expect("old a");
    let new_a = a.new_plan(0, s, r);
    let old_a: Vec<(u64, u64)> = old_a.into_iter().map(|(n, t)| (n, t.unwrap())).collect();
    assert_eq!(old_a.len(), 3);
    assert_eq!(new_a[new_a.len() - 3..], old_a[..], "the live edge");

    let (old_b, _) =
        plan_timeline(b.old_template(0, s, r), b.period_len(0), end_a, Some(3)).expect("old b");
    let old_b: Vec<(u64, u64)> = old_b.into_iter().map(|(n, t)| (n, t.unwrap())).collect();
    let end_a = end_a.expect("a window end");
    let new_b: Vec<(u64, u64)> = b
        .new_plan(0, s, r)
        .into_iter()
        .filter(|(_, t)| *t >= end_a)
        .collect();
    assert_eq!(old_b.len(), 2, "the window slid by two segments");
    assert_eq!(new_b, old_b, "what the refresh adds");
}

/// livesim2 `$Number$` + timeline: the server advances `@startNumber` with the
/// window, so numbers line up with times across a refresh.
#[test]
fn a_number_timeline_agrees_in_full_and_across_a_refresh() {
    let a = load("dash/livesim2/segtimelinenr_a.mpd");
    let b = load("dash/livesim2/segtimelinenr_b.mpd");
    let (s, r) = a
        .reps(0)
        .into_iter()
        .find(|(s, r)| a.old.periods[0].adaptation_sets[*s].representations[*r].id == "V300")
        .expect("the V300 video Representation");
    let (old_a, end_a) =
        plan_timeline(a.old_template(0, s, r), a.period_len(0), None, None).expect("old a");
    let old_a: Vec<(u64, u64)> = old_a.into_iter().map(|(n, t)| (n, t.unwrap())).collect();
    assert_eq!(a.new_plan(0, s, r), old_a);
    assert_eq!(old_a.len(), 31);

    let (old_b, _) =
        plan_timeline(b.old_template(0, s, r), b.period_len(0), end_a, None).expect("old b");
    let old_b: Vec<(u64, u64)> = old_b.into_iter().map(|(n, t)| (n, t.unwrap())).collect();
    let end_a = end_a.expect("a window end");
    let new_b: Vec<(u64, u64)> = b
        .new_plan(0, s, r)
        .into_iter()
        .filter(|(_, t)| *t >= end_a)
        .collect();
    assert_eq!(old_b.len(), 4);
    assert_eq!(new_b, old_b);
}

/// A negative `S@r` on a Period with a large `@presentationTimeOffset`: the
/// old planner needed a fix to count five segments here, and the core counts
/// the same five.
#[test]
fn a_negative_repeat_with_a_presentation_time_offset_runs_to_the_period_end() {
    let both = load_text(
        r#"<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="static" mediaPresentationDuration="PT10S" minBufferTime="PT2S" profiles="urn:mpeg:dash:profile:isoff-live:2011">
  <Period id="p0" start="PT0S">
    <AdaptationSet mimeType="video/mp4">
      <SegmentTemplate timescale="90000" presentationTimeOffset="900000000" initialization="init.mp4" media="$Time$.m4s">
        <SegmentTimeline><S t="900000000" d="180000" r="-1"/></SegmentTimeline>
      </SegmentTemplate>
      <Representation id="v" bandwidth="1000000" codecs="avc1.42c01e"/>
    </AdaptationSet>
  </Period>
</MPD>"#,
    );
    let (old, _) = plan_timeline(both.old_template(0, 0, 0), both.period_len(0), None, None)
        .expect("old plan");
    let old: Vec<(u64, u64)> = old.into_iter().map(|(n, t)| (n, t.unwrap())).collect();
    assert_eq!(old.len(), 5);
    assert_eq!(both.new_plan(0, 0, 0), old);
}

/// The real DASH-IF Qualcomm MultiRate vector (private repo, licence
/// unstated): `<S d="24576" r="-1"/>` on a 594 s Period.
#[test]
fn the_multirate_negative_repeat_vector_counts_the_same_segments() {
    let Some(path) = test_fixtures::require(
        "dash/qualcomm-multirate-1c-1.mpd",
        "dash_pull differential negative-r real vector",
    ) else {
        return;
    };
    let both = load_text(&std::fs::read_to_string(path).expect("read vector"));
    let counts: Vec<(usize, usize)> = both
        .reps(0)
        .into_iter()
        .map(|(s, r)| {
            let st = both.old_template(0, s, r);
            let (old, _) = plan_timeline(st, both.period_len(0), None, None).expect("old");
            (old.len(), both.new_plan(0, s, r).len())
        })
        .collect();
    assert!(counts.iter().all(|(old, new)| old == new), "{counts:?}");
    assert_eq!(
        counts.iter().map(|(old, _)| *old).collect::<Vec<_>>(),
        vec![297, 297, 297, 303]
    );
}
