//! The pre-core planner, kept compiled for tests only.
//!
//! Before `dash_pull` ran on `dash_runtime::client::DashClient` it planned its
//! own segment lists here: `$Number$` with a constant `@duration`, a
//! `SegmentTimeline` with `$Time$`, and a live edge derived from the host wall
//! clock, for the last Period only. That code is no longer reachable from the
//! pull path. It stays as the oracle of the differential test in
//! `differential.rs` (design 8.2(c)): on every presentation this planner
//! could handle, `dash_runtime`'s plan must list the same segments. The tests
//! below are the ones it shipped with, unchanged, so the oracle itself stays
//! pinned.

use std::time::Duration;

use transmux::dash_parse::{Mpd, Period, SegmentTemplate};

use super::parse_iso8601_utc;
use crate::error::{MultimuxError, Result};

/// The Period length (`PEwc - PSwc`, DASH-IF IOP v4.3 §3.10.2.2.3): the
/// Period's own `@duration`, else the Media Presentation Duration measured
/// from the Period start.
pub(super) fn period_length(period: &Period, mpd: &Mpd) -> Option<Duration> {
    period.duration.or_else(|| {
        mpd.media_presentation_duration
            .map(|m| m.saturating_sub(period.start.unwrap_or(Duration::ZERO)))
    })
}

/// Resolve a `SegmentTimeline` template's `(number, time)` plan (ISO/IEC
/// 23009-1 §5.3.9.6 via `SegmentTimeline::resolve`: absolute `@t`, explicit
/// `@t` jumps, negative `@r` bounded by the next `@t` / the Period end).
///
/// `after_end` is the previous plan's high-water mark (end tick of its last
/// segment): only segments starting at or after it are returned (a live
/// refresh). With `None` and a dynamic MPD, only the last `lookahead` segments
/// (the live edge) are kept, not the whole backlog (audit W14b). The second
/// element is the new high-water mark (`None` if nothing was returned).
#[allow(clippy::type_complexity)]
pub(super) fn plan_timeline(
    st: &SegmentTemplate,
    period_len: Option<Duration>,
    after_end: Option<u64>,
    dynamic_lookahead: Option<usize>,
) -> Result<(Vec<(u64, Option<u64>)>, Option<u64>)> {
    let Some(timeline) = &st.timeline else {
        return Ok((Vec::new(), None));
    };
    let period_end_ticks = period_len
        .map(|d| (d.as_secs_f64() * st.timescale as f64).ceil() as u64)
        .filter(|t| *t > 0)
        // `S@t` is on the media timeline, which `@presentationTimeOffset`
        // shifts from Period-relative time (ISO/IEC 23009-1 §5.3.9.6: MPD time
        // is `(S@t - PTO) / timescale`; DVB-DASH / TS 103 285 sets the PTO to
        // the earliest media time), so the Period end on that timeline is the
        // Period length PLUS the offset - else a negative `S@r` (run to the
        // Period end) resolves to zero segments for any large PTO.
        .map(|t| t.saturating_add(st.presentation_time_offset));
    let mut segs = timeline
        .resolve(st.start_number, period_end_ticks)
        .map_err(|e| MultimuxError::Connect {
            reason: format!("dash-pull: segment timeline: {e}"),
        })?;
    // A timeline whose last segment ends BEFORE the high-water mark has gone
    // backwards (a new Period, or an encoder restart resetting the media
    // clock): a sliding window only ever moves forward, and an unchanged
    // refresh ends exactly AT the mark. Filtering such a timeline by
    // `time >= mark` would drop every segment and stall the route, so it is
    // re-baselined like an initial plan.
    let after_end = after_end.filter(|mark| {
        segs.last()
            .is_none_or(|last| last.time.saturating_add(last.duration) >= *mark)
    });
    match after_end {
        Some(end) => segs.retain(|seg| seg.time >= end),
        None => {
            if dynamic_lookahead.is_some() {
                let keep = dynamic_lookahead.unwrap_or(0).max(1);
                if segs.len() > keep {
                    segs.drain(0..segs.len() - keep);
                }
            }
        }
    }
    let end = segs.last().map(|seg| seg.time.saturating_add(seg.duration));
    Ok((
        segs.into_iter()
            .map(|seg| (seg.number, Some(seg.time)))
            .collect(),
        end,
    ))
}

/// Builds a Representation's `(number, time)` segment plan from its
/// effective [`SegmentTemplate`].
///
/// `from_number` is the first `$Number$` to enumerate (the template's own
/// `@startNumber` for the initial plan; `last_number + 1` on a live refresh,
/// so the plan keeps *extending*). `lookahead` bounds a **dynamic** MPD's
/// plan when no total duration is known (audit run 7, W14): a live
/// `$Number$` MPD carries no `mediaPresentationDuration`, so the pre-fix
/// `None => 0` count produced an empty plan forever and the route went Live
/// but emitted nothing. On a static MPD, or once a total duration is known,
/// the count is derived from the duration exactly as before.
pub(super) fn build_plan(
    st: &SegmentTemplate,
    total_duration: Option<Duration>,
    from_number: u64,
    dynamic_lookahead: Option<usize>,
) -> Result<Vec<(u64, Option<u64>)>> {
    if st.timeline.is_some() {
        return plan_timeline(st, total_duration, None, dynamic_lookahead).map(|(plan, _)| plan);
    }
    let Some(duration) = st.duration else {
        return Ok(Vec::new());
    };
    if duration == 0 {
        return Ok(Vec::new());
    }
    let count = match total_duration {
        Some(total) => {
            let total_ticks = (total.as_secs_f64() * st.timescale as f64).ceil() as u64;
            // Cap the derived count: a hostile MPD with a huge
            // `mediaPresentationDuration` and a tiny `@duration` would
            // otherwise collect an unbounded plan into a `Vec` (audit W16,
            // manifest hostile counts). The same bound the `SegmentTimeline`
            // path applies.
            usize::try_from(total_ticks.div_ceil(duration))
                .unwrap_or(usize::MAX)
                .min(MAX_PLAN_ENTRIES)
        }
        // A dynamic MPD with no known total: enumerate a bounded look-ahead
        // window beginning at `from_number`, so the refresh loop can keep
        // extending it as the live edge advances.
        None => dynamic_lookahead.unwrap_or(0),
    };
    if count == 0 {
        return Ok(Vec::new());
    }
    Ok((from_number..from_number.saturating_add(count as u64))
        .map(|n| (n, None))
        .collect())
}

/// The `$Number$` of the **live edge** of a dynamic `$Number$` MPD at
/// `now_unix_secs` — audit W14b.
///
/// A live `$Number$` MPD numbers its segments from `@startNumber` (usually
/// `1`) and carries no total, so a client starting at `@startNumber` requests
/// segments that stopped existing hours ago. ISO/IEC 23009-1 §5.3.9.2 places
/// the availability start time of segment N at
/// `availabilityStartTime + Period@start + (N - startNumber) * segmentDuration
/// minus `presentationTimeOffset`; inverting that, and backing off a
/// `suggestedPresentationDelay` so the edge segment is already complete,
/// gives the current number.
///
/// Everything is computed in the template's **tick** domain (no integer-second
/// truncation, which for a 90090/90000 = 1.001 s segment overshoots the edge
/// by ~0.1% per segment and crawls after a few hours). `spd_ticks` is the
/// suggested-presentation-delay in ticks (the caller passes a documented
/// default of a few segments when the MPD does not carry one).
///
/// Returns `None` when the AST is absent/unparseable, the template is
/// degenerate, or the arithmetic lands before `@startNumber` — the caller
/// then falls back to `@startNumber`.
#[allow(clippy::too_many_arguments)]
pub(super) fn live_edge_number(
    availability_start_time: Option<&str>,
    now_unix_secs: u64,
    duration_ticks: u64,
    timescale: u64,
    start_number: u64,
    period_start_secs: u64,
    presentation_time_offset_ticks: u64,
    spd_ticks: u64,
) -> Option<u64> {
    let ast = parse_iso8601_utc(availability_start_time?)?;
    if duration_ticks == 0 || timescale == 0 {
        return None;
    }
    // Elapsed wall-clock seconds since the presentation began.
    let elapsed_secs = now_unix_secs
        .checked_sub(ast)?
        .checked_sub(period_start_secs)?;
    // Ticks available for media, minus PTO and the presentation-delay back-off.
    let elapsed_ticks = elapsed_secs
        .checked_mul(timescale)?
        .checked_sub(presentation_time_offset_ticks)?
        .checked_sub(spd_ticks)?;
    let elapsed_segments = elapsed_ticks / duration_ticks;
    Some(start_number.saturating_add(elapsed_segments))
}

/// Upper bound on a `$Number$` segment plan's length (audit W16): a hostile
/// or corrupt MPD with an enormous `mediaPresentationDuration` and a tiny
/// `@duration` must not make `build_plan` collect an unbounded `Vec`.
pub(super) const MAX_PLAN_ENTRIES: usize = 100_000;

/// How many `$Number$` segment numbers a **dynamic** MPD with no known total
/// duration is planned ahead by (audit run 7, W14). Small enough that a live
/// edge is only ever probed a few segments ahead (each unavailable number is
/// bounded by the drive loop's bounded 404 retry, then abandoned), large enough
/// that a normal segment cadence is always covered.
pub(super) const LIVE_NUMBER_LOOKAHEAD: usize = 3;

#[cfg(test)]
mod tests {
    use super::*;

    /// Parse a committed real DASH-IF livesim2 MPD (BSD-3-Clause, see
    /// `fixtures/PROVENANCE.md`) and return its video template.
    fn livesim2_video_template(name: &str) -> (Mpd, SegmentTemplate) {
        let path = format!(
            "{}/../fixtures/dash/livesim2/{name}",
            env!("CARGO_MANIFEST_DIR")
        );
        let text = std::fs::read_to_string(&path).expect("fixture");
        let mpd = Mpd::parse(&text).expect("parse");
        let st = mpd.periods[0]
            .adaptation_sets
            .iter()
            .flat_map(|a| a.representations.iter())
            .find(|r| r.id == "V300")
            .and_then(|r| r.segment_template.clone())
            .expect("V300 template");
        (mpd, st)
    }

    /// #1187: a real livesim2 `$Time$` SegmentTimeline MPD has an ABSOLUTE first
    /// `S@t` (161274100320000 ticks, ~21 years of 90 kHz) and a constant
    /// `@startNumber` (default 1). The live refresh must extend the plan by
    /// TIME (ISO/IEC 23009-1 §5.3.9.6): the window slid by two segments, so
    /// exactly two new, gap-free entries are due. Keyed on `$Number$` (the old
    /// behaviour) the refreshed numbers restart at 1, never exceed
    /// `last_number`, and a live `$Time$` route stalls forever.
    #[test]
    fn timeline_refresh_extends_by_time_not_number() {
        let (mpd_a, st_a) = livesim2_video_template("segtimeline_time_a.mpd");
        let (_mpd_b, st_b) = livesim2_video_template("segtimeline_time_b.mpd");
        assert_eq!(st_a.start_number, 1);
        let period_len = period_length(&mpd_a.periods[0], &mpd_a);

        let (plan_a, end_a) =
            plan_timeline(&st_a, period_len, None, Some(LIVE_NUMBER_LOOKAHEAD)).expect("plan a");
        // Initial plan: only the live edge, with absolute times.
        assert_eq!(plan_a.len(), LIVE_NUMBER_LOOKAHEAD);
        let first_t = plan_a[0].1.expect("time");
        assert!(first_t > 161_274_100_320_000, "{first_t}");
        assert_eq!(end_a, Some(plan_a[plan_a.len() - 1].1.unwrap() + 180_000));

        let (plan_b, end_b) =
            plan_timeline(&st_b, period_len, end_a, Some(LIVE_NUMBER_LOOKAHEAD)).expect("plan b");
        assert_eq!(plan_b.len(), 2, "{plan_b:?}");
        assert_eq!(
            plan_b[0].1, end_a,
            "first new segment starts where the old plan ended"
        );
        assert_eq!(plan_b[1].1, end_a.map(|e| e + 180_000));
        assert_eq!(end_b, end_a.map(|e| e + 2 * 180_000));
        // Refreshing again with the same MPD adds nothing.
        let (again, _) =
            plan_timeline(&st_b, period_len, end_b, Some(LIVE_NUMBER_LOOKAHEAD)).expect("plan");
        assert!(again.is_empty());
    }

    /// #1187: `$Number$` + `SegmentTimeline` (livesim2 `segtimelinenr`): the
    /// server advances `@startNumber` as the window slides, so numbers line up
    /// with the absolute times across a refresh.
    #[test]
    fn timeline_with_number_addressing_keeps_numbers_aligned_across_refresh() {
        let (mpd_a, st_a) = livesim2_video_template("segtimelinenr_a.mpd");
        let (_mpd_b, st_b) = livesim2_video_template("segtimelinenr_b.mpd");
        let period_len = period_length(&mpd_a.periods[0], &mpd_a);
        let (plan_a, end_a) = plan_timeline(&st_a, period_len, None, None).expect("a");
        let (plan_b, _) = plan_timeline(&st_b, period_len, end_a, None).expect("b");
        // a lists 31 segments from #895967208; b (4 segments later, startNumber
        // 895967212) adds exactly the 4 whose time is >= a's end, numbered
        // contiguously after a's last.
        assert_eq!(plan_a.len(), 31);
        assert_eq!(plan_b.len(), 4);
        assert_eq!(plan_b[0].0, plan_a[30].0 + 1);
        assert_eq!(plan_b[0].1, end_a);
    }

    /// #1187: a negative `S@r` (real DASH-IF Qualcomm MultiRate vector,
    /// `<S d="24576" r="-1"/>`, static, Period 9:54) repeats to the Period end
    /// (DASH-IF IOP v4.3 §3.10.2.2.4). That vector's licence is unstated, so it
    /// lives in the private repo (`private/fixtures/dash/`) and the test skips
    /// loudly only when the private repo is not checked out.
    #[test]
    fn negative_r_real_vector_runs_to_period_end() {
        let Some(path) = test_fixtures::require(
            "dash/qualcomm-multirate-1c-1.mpd",
            "dash_pull negative-r real vector",
        ) else {
            return;
        };
        let text = std::fs::read_to_string(&path).expect("read vector");
        let mpd = Mpd::parse(&text).expect("parse");
        let period = &mpd.periods[0];
        let period_len = period_length(period, &mpd);
        let counts: Vec<usize> = period
            .adaptation_sets
            .iter()
            .flat_map(|a| a.representations.iter())
            .map(|r| {
                let st = r.segment_template.as_ref().expect("template");
                plan_timeline(st, period_len, None, None)
                    .expect("plan")
                    .0
                    .len()
            })
            .collect();
        // 594 s: video 2 s segments -> 297; audio 94208/48000 s -> ceil(302.65).
        assert_eq!(counts, vec![297, 297, 297, 303]);
    }

    /// REVIEW FINDING (review/runtimes): a negative `S@r` on the last `S` runs
    /// to the Period end, but `S@t` is in the presentation-time domain, which
    /// is offset by `@presentationTimeOffset` (ISO/IEC 23009-1 §5.3.9.6: the
    /// MPD start time is `(S@t - @presentationTimeOffset) / @timescale`; ETSI TS
    /// 103 285 / DVB-DASH sets PTO to the earliest media time with Period
    /// start 0, so a large PTO and a matching large `S@t` are the normal
    /// shape). `plan_timeline` passes the Period length in ticks as if it were
    /// an `S@t`-domain end, so `end - t` saturates to 0 and the whole run
    /// resolves to ZERO segments: a static 10 s Period of 2 s segments
    /// ingests nothing (before #1187 it was at least one segment).
    #[test]
    fn review_negative_r_with_presentation_time_offset_runs_to_period_end() {
        let mpd = r#"<?xml version="1.0"?>
<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="static" mediaPresentationDuration="PT10S" minBufferTime="PT2S" profiles="urn:mpeg:dash:profile:isoff-live:2011">
  <Period id="p0" start="PT0S">
    <AdaptationSet mimeType="video/mp4">
      <SegmentTemplate timescale="90000" presentationTimeOffset="900000000" initialization="init.mp4" media="$Time$.m4s">
        <SegmentTimeline>
          <S t="900000000" d="180000" r="-1"/>
        </SegmentTimeline>
      </SegmentTemplate>
      <Representation id="v" bandwidth="1000000" codecs="avc1.42c01e"/>
    </AdaptationSet>
  </Period>
</MPD>"#;
        let mpd = Mpd::parse(mpd).expect("parse");
        let period = &mpd.periods[0];
        let st = period.adaptation_sets[0].representations[0]
            .segment_template
            .clone()
            .expect("template");
        assert_eq!(st.presentation_time_offset, 900_000_000);
        let (plan, _) = plan_timeline(&st, period_length(period, &mpd), None, None).expect("plan");
        assert_eq!(
            plan.len(),
            5,
            "10 s Period / 2 s segments = 5 segments, got {plan:?}"
        );
        assert_eq!(plan[0].1, Some(900_000_000));
    }

    /// A timeline that restarts LOWER than the high-water mark (a new Period,
    /// or an encoder restart that resets the media clock) is a new baseline:
    /// filtering by `time >= after_end` would drop every segment and stall the
    /// route forever.
    #[test]
    fn a_timeline_that_restarts_lower_is_rebaselined_not_filtered_away() {
        let template = |t: u64| {
            let mpd = format!(
                r#"<?xml version="1.0"?>
<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="dynamic" availabilityStartTime="2024-01-01T00:00:00Z" minimumUpdatePeriod="PT2S" minBufferTime="PT2S" profiles="urn:mpeg:dash:profile:isoff-live:2011">
  <Period id="p" start="PT0S">
    <AdaptationSet mimeType="video/mp4">
      <SegmentTemplate timescale="90000" initialization="init.mp4" media="$Time$.m4s">
        <SegmentTimeline><S t="{t}" d="180000" r="3"/></SegmentTimeline>
      </SegmentTemplate>
      <Representation id="v" bandwidth="1000000" codecs="avc1.42c01e"/>
    </AdaptationSet>
  </Period>
</MPD>"#
            );
            Mpd::parse(&mpd).expect("parse").periods[0].adaptation_sets[0].representations[0]
                .segment_template
                .clone()
                .expect("template")
        };
        let (first, end) = plan_timeline(&template(900_000_000), None, None, Some(8)).expect("a");
        assert_eq!(first.len(), 4);
        // Same MPD again: nothing new (the ordinary refresh is unchanged).
        let (same, _) = plan_timeline(&template(900_000_000), None, end, Some(8)).expect("same");
        assert!(same.is_empty());
        // The timeline restarts at t = 0, far below `end`.
        let (restarted, new_end) = plan_timeline(&template(0), None, end, Some(8)).expect("b");
        assert_eq!(restarted.len(), 4, "restart must be re-baselined");
        assert_eq!(restarted[0].1, Some(0));
        assert_eq!(new_end, Some(4 * 180_000));
    }

    /// W16: a hostile MPD with a huge `mediaPresentationDuration` and a tiny
    /// `@duration` must not collect an unbounded plan.
    #[test]
    fn build_plan_caps_a_hostile_segment_count() {
        let st = SegmentTemplate {
            timescale: 1,
            initialization: None,
            media: None,
            start_number: 1,
            duration: Some(1), // 1 tick
            presentation_time_offset: 0,
            timeline: None,
        };
        // ~10^15 s of "content" at 1-tick segments.
        let total = Duration::from_secs(1_000_000_000_000);
        let plan = build_plan(&st, Some(total), 1, None).expect("plan");
        assert_eq!(
            plan.len(),
            MAX_PLAN_ENTRIES,
            "the plan must be capped, not collect 10^15 entries"
        );
    }

    /// W14b: the current `$Number$` is derived from `availabilityStartTime`
    /// and now, so a stream running for hours does not start at segment 1.
    #[test]
    fn live_edge_number_derives_the_current_number() {
        let ast = "2020-01-01T00:00:00Z"; // 1_577_836_800 Unix seconds.
        assert_eq!(parse_iso8601_utc(ast), Some(1_577_836_800));
        assert_eq!(
            parse_iso8601_utc("2020-01-01T00:00:05.500Z"),
            Some(1_577_836_805)
        );
        assert_eq!(parse_iso8601_utc("not a time"), None);
        assert_eq!(parse_iso8601_utc("2020-13-01T00:00:00Z"), None);

        // 4 s segments, timescale 1000, start 1, no PTO/SPD, no Period@start:
        // one hour later is the 900th segment → number 901.
        let one_hour_later = 1_577_836_800 + 3600;
        assert_eq!(
            live_edge_number(Some(ast), one_hour_later, 4000, 1000, 1, 0, 0, 0),
            Some(901)
        );
        // A 3-segment suggested-presentation-delay backs the edge off by 3.
        assert_eq!(
            live_edge_number(Some(ast), one_hour_later, 4000, 1000, 1, 0, 0, 12_000),
            Some(898)
        );
        // Period@start and PTO each remove that much time.
        assert_eq!(
            // (3600 - 10 s Period@start) * 1000 - 4000 PTO = 3_586_000 ticks
            // / 4000 = 896 segments, + start 1 = 897.
            live_edge_number(Some(ast), one_hour_later, 4000, 1000, 1, 10, 4_000, 0),
            Some(897)
        );
        // A `now` before the AST falls back to `None`.
        assert_eq!(
            live_edge_number(Some(ast), 1_577_000_000, 4000, 1000, 1, 0, 0, 0),
            None
        );
        // No AST → `None`.
        assert_eq!(
            live_edge_number(None, one_hour_later, 4000, 1000, 1, 0, 0, 0),
            None
        );
    }

    /// W14b: fractional segment durations must be computed in the template's
    /// tick domain, not by integer-second division — a 90090/90000 = 1.001 s
    /// segment truncates to 1 s and overshoots the edge by ~0.1% per segment,
    /// which after hours lands many segments past the true edge.
    #[test]
    fn live_edge_number_is_exact_for_fractional_segments_over_six_hours() {
        let ast = "2020-01-01T00:00:00Z";
        let six_hours_later = 1_577_836_800 + 6 * 3600;
        // 90090 ticks @ 90000 = 1.001 s segments.
        let n =
            live_edge_number(Some(ast), six_hours_later, 90_090, 90_000, 1, 0, 0, 0).expect("some");
        // Exact: floor(6*3600 s * 90000 / 90090) = floor(21600*90000/90090).
        let expected = 1 + (21_600u64 * 90_000) / 90_090;
        assert_eq!(n, expected, "tick-domain division must be exact");
        // The naive integer-second computation would give a much smaller
        // number (21600 / 1 = 21600 segments elapsed).
        let naive = 21_601;
        assert!(
            n < naive,
            "the tick-exact number ({n}) must not overshoot the naive one ({naive})"
        );
    }
}
