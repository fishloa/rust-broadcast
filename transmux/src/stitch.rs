//! One continuous sample timeline across discontinuities (DASH Period boundaries, skipped segments).
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
//!
//! Every track's timescale is a `broadcast_common::timescale::Timescale`
//! (non-zero by construction: a wire `mdhd.timescale` of `0` is refused when
//! the box parses), so nothing here can treat a zero as `1`.

use alloc::collections::BTreeMap;

use broadcast_common::timescale::{Rounding, Timescale, from_micros, to_micros};

use crate::{Sample, TrackId, TrackSpec};

struct TrackClock {
    timescale: Timescale,
    end_us: Option<i128>,
    generation: u64,
    offset_ticks: i64,
}

/// Removes timeline jumps from the samples a host hands on, one offset for
/// every track. Feed it the announced tracks ([`Self::set_tracks`]), each
/// discontinuity the source reports ([`Self::discontinuity`]) and every batch
/// of samples ([`Self::apply`]).
#[derive(Default)]
pub struct Stitcher {
    tracks: BTreeMap<TrackId, TrackClock>,
    offset_us: i128,
    generation: u64,
    pending: bool,
}

fn ticks_to_us(ticks: i64, timescale: Timescale) -> i128 {
    to_micros(i128::from(ticks), timescale, Rounding::TowardZero)
}

/// `us` microseconds as ticks at `timescale`, saturating at the `i64` range.
/// Uses `broadcast_common::timescale` so the conversion lives in one place;
/// `us_to_ticks_agrees_with_the_old_arithmetic` pins the rounding.
fn us_to_ticks(us: i128, timescale: Timescale) -> i64 {
    let ticks = from_micros(us, timescale, Rounding::Nearest);
    i64::try_from(ticks.clamp(i128::from(i64::MIN), i128::from(i64::MAX))).unwrap_or(0)
}

impl Stitcher {
    /// Register the tracks (or a changed timescale) the samples belong to.
    pub fn set_tracks(&mut self, specs: &[TrackSpec]) {
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

    /// The next sample starts a new timeline: place it where the content so far ended.
    pub fn discontinuity(&mut self) {
        self.pending = true;
    }

    /// Shift `samples` of `track` onto the continuous timeline, in place.
    pub fn apply(&mut self, track: TrackId, samples: &mut [Sample]) {
        for sample in samples {
            let Some(clock) = self.tracks.get(&track) else {
                continue;
            };
            let timescale = clock.timescale;
            if self.pending
                && let Some(first) = sample
                    .timing
                    .map(|t| t.dts)
                    .or(sample.timing.map(|t| t.pts))
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
            if let Some(t) = sample.timing.as_mut() {
                t.dts = t.dts.saturating_add(shift);
                t.pts = t.pts.saturating_add(shift);
            }
            if let Some(dts) = sample.timing.map(|t| t.dts) {
                let dur = sample.timing.map(|t| t.duration).unwrap_or(0);
                let end = dts.saturating_add(i64::from(dur));
                clock.end_us = Some(ticks_to_us(end, timescale));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::Error;
    use crate::media::Fmp4Demux;
    use broadcast_common::units::MICROS_PER_SEC;
    use broadcast_common::{Parse, Unpackage};

    fn spec(id: u32, timescale: u32) -> TrackSpec {
        let init = std::fs::read(test_fixtures::path("dash/init-stream1.m4s")).expect("init");
        let mut spec = Fmp4Demux::new()
            .unpackage(init.as_slice())
            .expect("demux init")
            .tracks
            .remove(0)
            .spec;
        spec.track_id = TrackId::new(id);
        spec.timescale = Timescale::try_new(timescale).expect("test timescale is non-zero");
        spec
    }

    fn sample(dts: i64, duration: u32) -> Sample {
        Sample::new(vec![0u8], Some(dts), Some(dts), Some(duration), true)
    }

    fn dts_of(samples: &[Sample]) -> Vec<i64> {
        samples
            .iter()
            .map(|s| s.timing.map(|t| t.dts).unwrap())
            .collect()
    }

    fn old_us_to_ticks(us: i128, timescale: u32) -> i64 {
        let ticks = (us * i128::from(timescale) + (MICROS_PER_SEC as i128) / 2)
            .div_euclid(MICROS_PER_SEC as i128);
        i64::try_from(ticks.clamp(i128::from(i64::MIN), i128::from(i64::MAX))).unwrap_or(0)
    }

    #[test]
    fn us_to_ticks_agrees_with_the_old_arithmetic() {
        let cases = [
            (0i128, 1u32),
            (500, 1),
            (1_500, 1),
            (-500, 1),
            (-1_500, 1),
            (1_000, 1),
            (1_500, 90_000),
            (33_333, 90_000),
            (1_000_000, 90_000),
            (4_200_000, 90_000),
            (-1_500, 90_000),
            (1_000_000, 48_000),
            (2_500, 48_000),
        ];
        for (us, timescale) in cases {
            assert_eq!(
                us_to_ticks(us, Timescale::try_new(timescale).expect("non-zero")),
                old_us_to_ticks(us, timescale),
                "us={us} timescale={timescale}"
            );
        }
    }

    /// A minimal `mdhd` **v0** box carrying `timescale` (ISO/IEC 14496-12:2026
    /// §8.4.2): full-box header, creation/modification time, then the 32-bit
    /// `timescale`, the 32-bit duration and the packed `language`.
    fn mdhd_v0(timescale: u32) -> alloc::vec::Vec<u8> {
        let mut body = alloc::vec![0u8; 32];
        body[4..8].copy_from_slice(b"mdhd");
        body[20..24].copy_from_slice(&timescale.to_be_bytes());
        body[28..30].copy_from_slice(&0x55C4u16.to_be_bytes()); // "und"
        body
    }

    /// A zero timescale never reaches the stitcher: a track's timescale is a
    /// `Timescale` (non-zero by construction), so the boundary that reads a
    /// wire `mdhd.timescale` is where a real zero is refused — with a typed
    /// error naming the field, not a silent 1 Hz fallback.
    #[test]
    fn a_track_with_a_zero_timescale_is_a_typed_error_not_a_silent_one() {
        let err = crate::init_segment::MediaHeaderBox::parse(&mdhd_v0(0))
            .expect_err("a wire mdhd.timescale of 0 must not parse");
        assert_eq!(
            err,
            Error::InvalidValue(broadcast_common::error::InvalidValue {
                field: "timescale",
                value: 0,
                reason: "must be non-zero",
            })
        );
        let ok = crate::init_segment::MediaHeaderBox::parse(&mdhd_v0(90_000))
            .expect("a non-zero mdhd.timescale parses");
        assert_eq!(ok.timescale, broadcast_common::timescale!(90_000));
        assert!(Timescale::new(0).is_none(), "the type has no zero value");
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
        assert_eq!(
            second[1].timing.map(|t| t.pts),
            Some(6_000),
            "pts moves with dts"
        );
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
    #[test]
    fn the_offset_is_rounded_to_the_nearest_tick_not_truncated() {
        let mut st = Stitcher::default();
        st.set_tracks(&[spec(1, 90_000)]);
        st.apply(TrackId::new(1), &mut [sample(0, 90_001)]);
        st.discontinuity();
        let mut next = [sample(0, 1)];
        st.apply(TrackId::new(1), &mut next);
        assert_eq!(
            dts_of(&next),
            [90_001],
            "the previous end is 1 000 011 us, which is 90 000.99 ticks"
        );
    }

    #[test]
    fn a_track_nobody_announced_is_left_alone() {
        let mut st = Stitcher::default();
        let mut a = [sample(5, 1)];
        st.apply(TrackId::new(9), &mut a);
        assert_eq!(dts_of(&a), [5]);
    }
}
