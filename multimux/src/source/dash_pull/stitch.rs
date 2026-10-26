//! One continuous sample timeline across Period boundaries and gaps.
//!
//! `dash_runtime` hands samples on the container's own media timeline: a
//! Period that restarts its clock at zero (GPAC's multi-Period output does),
//! a lost segment, or an unaligned Representation switch all make that
//! timeline jump, and the core says so with `Output::Discontinuity`. A `Trunk`
//! feeds segmenters that expect time to move forward, so the stitcher removes
//! the jump: the first sample after a discontinuity is placed where the
//! previous content ended, and everything that follows keeps its distance from
//! it. One offset serves every track, so audio and video keep their relative
//! alignment inside the new Period.
//!
//! A boundary the core marks as continuous, and a presentation that never
//! signals a discontinuity, are passed through untouched.

use std::collections::HashMap;

use transmux::{Sample, TrackId, TrackSpec};

const MICROS_PER_SECOND: i128 = 1_000_000;
const HALF_SECOND_US: i128 = MICROS_PER_SECOND / 2;

struct TrackClock {
    timescale: u32,
    end_us: Option<i128>,
    generation: u64,
    offset_ticks: i64,
}

#[derive(Default)]
pub(super) struct Stitcher {
    tracks: HashMap<TrackId, TrackClock>,
    offset_us: i128,
    generation: u64,
    pending: bool,
}

fn ticks_to_us(ticks: i64, timescale: u32) -> i128 {
    i128::from(ticks) * MICROS_PER_SECOND / i128::from(timescale)
}

fn us_to_ticks(us: i128, timescale: u32) -> i64 {
    let ticks = (us * i128::from(timescale) + HALF_SECOND_US).div_euclid(MICROS_PER_SECOND);
    i64::try_from(ticks.clamp(i128::from(i64::MIN), i128::from(i64::MAX))).unwrap_or(0)
}

impl Stitcher {
    pub(super) fn set_tracks(&mut self, specs: &[TrackSpec]) {
        for spec in specs {
            let clock = self.tracks.entry(spec.track_id).or_insert(TrackClock {
                timescale: spec.timescale,
                end_us: None,
                generation: self.generation,
                offset_ticks: 0,
            });
            clock.timescale = spec.timescale;
        }
    }

    pub(super) fn discontinuity(&mut self) {
        self.pending = true;
    }

    pub(super) fn apply(&mut self, track: TrackId, samples: &mut [Sample]) {
        for sample in samples {
            let Some(clock) = self.tracks.get(&track) else {
                continue;
            };
            let timescale = clock.timescale;
            if timescale == 0 {
                continue;
            }
            if self.pending
                && let Some(first) = sample.dts.or(sample.pts)
            {
                self.pending = false;
                let stitch = self.tracks.values().filter_map(|t| t.end_us).max();
                if let Some(stitch) = stitch {
                    self.offset_us = stitch - ticks_to_us(first, timescale);
                    self.generation += 1;
                }
            }
            let Some(clock) = self.tracks.get_mut(&track) else {
                continue;
            };
            if clock.generation != self.generation {
                clock.generation = self.generation;
                clock.offset_ticks = us_to_ticks(self.offset_us, timescale);
            }
            let shift = clock.offset_ticks;
            sample.dts = sample.dts.map(|t| t.saturating_add(shift));
            sample.pts = sample.pts.map(|t| t.saturating_add(shift));
            if let Some(dts) = sample.dts {
                let end = dts.saturating_add(i64::from(sample.duration.unwrap_or(0)));
                clock.end_us = Some(ticks_to_us(end, timescale));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use broadcast_common::Unpackage;
    use transmux::Sample;
    use transmux::media::Fmp4Demux;

    fn spec(id: u32, timescale: u32) -> TrackSpec {
        let init = std::fs::read(test_fixtures::path("dash/init-stream1.m4s")).expect("init");
        let mut spec = Fmp4Demux::new()
            .unpackage(init.as_slice())
            .expect("demux init")
            .tracks
            .remove(0)
            .spec;
        spec.track_id = TrackId::new(id);
        spec.timescale = timescale;
        spec
    }

    fn sample(dts: i64, duration: u32) -> Sample {
        Sample::new(vec![0u8], Some(dts), Some(dts), Some(duration), true)
    }

    fn dts_of(samples: &[Sample]) -> Vec<i64> {
        samples.iter().map(|s| s.dts.unwrap()).collect()
    }

    #[test]
    fn a_timeline_without_discontinuity_is_untouched() {
        let mut st = Stitcher::default();
        st.set_tracks(&[spec(1, 1000)]);
        let mut a = [sample(5_000, 100), sample(5_100, 100)];
        st.apply(TrackId::new(1), &mut a);
        assert_eq!(dts_of(&a), [5_000, 5_100]);
        let mut b = [sample(5_200, 100)];
        st.apply(TrackId::new(1), &mut b);
        assert_eq!(dts_of(&b), [5_200]);
    }

    #[test]
    fn a_restarted_clock_continues_where_the_previous_period_ended() {
        let mut st = Stitcher::default();
        st.set_tracks(&[spec(1, 1000)]);
        let mut first = [sample(0, 2_000), sample(2_000, 2_000)];
        st.apply(TrackId::new(1), &mut first);
        st.discontinuity();
        let mut second = [sample(0, 2_000), sample(2_000, 2_000)];
        st.apply(TrackId::new(1), &mut second);
        assert_eq!(dts_of(&second), [4_000, 6_000]);
        assert_eq!(second[1].pts, Some(6_000), "pts moves with dts");
    }

    #[test]
    fn a_skipped_segment_is_closed_not_carried_as_a_gap() {
        let mut st = Stitcher::default();
        st.set_tracks(&[spec(1, 1000)]);
        let mut first = [sample(10_000, 2_000)];
        st.apply(TrackId::new(1), &mut first);
        st.discontinuity();
        let mut after = [sample(16_000, 2_000)];
        st.apply(TrackId::new(1), &mut after);
        assert_eq!(dts_of(&after), [12_000]);
    }

    #[test]
    fn every_track_shares_one_offset_so_alignment_inside_the_period_survives() {
        let mut st = Stitcher::default();
        st.set_tracks(&[spec(1, 1000), spec(2, 48_000)]);
        let mut v = [sample(0, 1_000)];
        let mut a = [sample(0, 48_000)];
        st.apply(TrackId::new(1), &mut v);
        st.apply(TrackId::new(2), &mut a);
        st.discontinuity();
        let mut v2 = [sample(0, 1_000)];
        let mut a2 = [sample(2_400, 48_000)];
        st.apply(TrackId::new(1), &mut v2);
        st.apply(TrackId::new(2), &mut a2);
        assert_eq!(dts_of(&v2), [1_000]);
        assert_eq!(
            dts_of(&a2),
            [48_000 + 2_400],
            "audio starts 50 ms after video in the new Period, as it did in the source"
        );
    }

    #[test]
    fn the_stitch_point_is_the_later_of_the_track_ends() {
        let mut st = Stitcher::default();
        st.set_tracks(&[spec(1, 1000), spec(2, 1000)]);
        st.apply(TrackId::new(1), &mut [sample(0, 6_000)]);
        st.apply(TrackId::new(2), &mut [sample(0, 6_037)]);
        st.discontinuity();
        let mut next = [sample(0, 100)];
        st.apply(TrackId::new(1), &mut next);
        assert_eq!(dts_of(&next), [6_037]);
    }

    #[test]
    fn a_discontinuity_before_any_sample_changes_nothing() {
        let mut st = Stitcher::default();
        st.set_tracks(&[spec(1, 1000)]);
        st.discontinuity();
        let mut a = [sample(7_000, 100)];
        st.apply(TrackId::new(1), &mut a);
        assert_eq!(dts_of(&a), [7_000]);
        let mut b = [sample(7_100, 100)];
        st.apply(TrackId::new(1), &mut b);
        assert_eq!(
            dts_of(&b),
            [7_100],
            "the pending flag is spent, not re-armed"
        );
    }

    #[test]
    fn a_second_restart_stacks_on_the_first() {
        let mut st = Stitcher::default();
        st.set_tracks(&[spec(1, 1000)]);
        st.apply(TrackId::new(1), &mut [sample(0, 1_000)]);
        st.discontinuity();
        let mut p2 = [sample(0, 1_000)];
        st.apply(TrackId::new(1), &mut p2);
        st.discontinuity();
        let mut p3 = [sample(0, 1_000)];
        st.apply(TrackId::new(1), &mut p3);
        assert_eq!(dts_of(&p2), [1_000]);
        assert_eq!(dts_of(&p3), [2_000]);
    }

    #[test]
    fn a_timescale_change_between_periods_keeps_the_stitch_in_time() {
        let mut st = Stitcher::default();
        st.set_tracks(&[spec(1, 1000)]);
        st.apply(TrackId::new(1), &mut [sample(0, 2_000)]);
        st.discontinuity();
        st.set_tracks(&[spec(1, 90_000)]);
        let mut next = [sample(0, 90_000)];
        st.apply(TrackId::new(1), &mut next);
        assert_eq!(dts_of(&next), [180_000], "2 s at the new 90 kHz scale");
    }
}
