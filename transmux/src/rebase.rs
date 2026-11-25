//! Timeline-conditioning transforms over the sample stream — PTS/DTS rebase,
//! offset, and discontinuity-gap insertion (issue #476).
//!
//! Media plane step 2c: [`Sample`] `dts`/`pts` are
//! **absolute** ticks in the track's media timescale, not a running sum
//! anchored on a track-level start. These stages shift that absolute pair
//! directly:
//!
//! - [`RebaseStage`] — re-origin each track so its first sample starts at
//!   decode time 0. Built with an exact per-track [`RebaseOrigin`] (from a
//!   sample plan) or a [`RebaseStage::start_window`] that discovers each
//!   track's own first sample, holding output no longer than the caller's
//!   window or byte cap.
//! - [`OffsetStage`] — shift a track by a signed delta (saturating at 0 on
//!   underflow, per track, on its first sample's anchor).
//! - [`GapStage`] — push every sample from a given per-track index onward (or
//!   the whole track, at index 0) later by a gap (splice / concat
//!   conditioning).
//!
//! Each is a [`Stage`] fed `(TrackId, Sample)` and emitting the same pair, so a
//! transform chain is a chain of stages with no whole [`Media`](crate::ir::Media)
//! in memory. The former whole-input free functions are gone with them.
//!
//! **33-bit MPEG-2 Systems wrap-unrolling happens at the demux edge**
//! ([`crate::ts_demux`] unwraps the wire's 33-bit PTS/DTS once,
//! incrementally), so every `Sample` this crate produces is already absolute.

use alloc::collections::{BTreeMap, VecDeque};

use broadcast_common::stage::{Stage, Timestamp};

use crate::TrackId;
use crate::error::{Error, Result};
use crate::ir::Sample;

/// One exact decode-time origin per track, in that track's own media
/// timescale ticks — the form a sample plan supplies to [`RebaseStage`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RebaseOrigin {
    origins: BTreeMap<TrackId, i64>,
}

impl RebaseOrigin {
    /// No origins.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The origin of `track`, in its own media timescale ticks.
    #[must_use]
    pub fn with(mut self, track: TrackId, ticks: i64) -> Self {
        self.origins.insert(track, ticks);
        self
    }

    /// Record `track`'s origin.
    pub fn set(&mut self, track: TrackId, ticks: i64) {
        self.origins.insert(track, ticks);
    }

    /// The origin recorded for `track`, if any.
    #[must_use]
    pub fn get(&self, track: TrackId) -> Option<i64> {
        self.origins.get(&track).copied()
    }
}

/// How a [`RebaseStage`] learns each track's origin.
#[derive(Debug, Clone)]
enum Mode {
    /// Every track's origin is supplied up front.
    Exact,
    /// Each track is rebased on its own first sample, buffering output no
    /// longer than `window` of driver time or `max_bytes` of sample bytes.
    Window {
        /// The caller's wall-clock hold.
        window: core::time::Duration,
        /// The caller's byte cap on buffered samples.
        max_bytes: u64,
    },
}

/// Re-origin every track's decode timeline so its first sample starts at 0.
///
/// Drive it with [`feed`](Stage::feed), then [`poll`](Stage::poll) for the
/// rebased samples, and [`finish`](Stage::finish) at end of input. A sample
/// carrying no timestamp is passed through untouched — no timestamp is ever
/// fabricated.
#[derive(Debug, Clone)]
pub struct RebaseStage {
    mode: Mode,
    origins: BTreeMap<TrackId, i64>,
    pending: VecDeque<(TrackId, Sample)>,
    pending_bytes: u64,
    started: Option<Timestamp>,
    deadline: Option<Timestamp>,
    released: bool,
    ready: VecDeque<(TrackId, Sample)>,
}

impl RebaseStage {
    /// Rebase with exact per-track origins (from a sample plan).
    #[must_use]
    pub fn with_origin(origin: RebaseOrigin) -> Self {
        Self {
            mode: Mode::Exact,
            origins: origin.origins,
            pending: VecDeque::new(),
            pending_bytes: 0,
            started: None,
            deadline: None,
            released: true,
            ready: VecDeque::new(),
        }
    }

    /// Rebase each track on its own first sample, holding output for at most
    /// `window` of driver time or `max_bytes` of buffered sample bytes.
    ///
    /// Buffering ends when the window closes (`on_deadline`), the byte cap is
    /// reached, or the stage is finished. A track whose first sample arrives
    /// after the window closed still rebases on that first sample, so it never
    /// emits a negative timestamp.
    #[must_use]
    pub fn start_window(window: core::time::Duration, max_bytes: u64) -> Self {
        Self {
            mode: Mode::Window { window, max_bytes },
            origins: BTreeMap::new(),
            pending: VecDeque::new(),
            pending_bytes: 0,
            started: None,
            deadline: None,
            released: false,
            ready: VecDeque::new(),
        }
    }

    /// Release every held sample, in arrival order.
    fn release(&mut self) {
        if self.released {
            return;
        }
        self.released = true;
        for (track, sample) in &self.pending {
            if !self.origins.contains_key(track) {
                let ticks = sample.timing.and_then(|t| t.dts()).unwrap_or(0);
                self.origins.insert(*track, ticks);
            }
        }
        while let Some((track, sample)) = self.pending.pop_front() {
            self.emit(track, sample);
        }
        self.pending_bytes = 0;
    }

    /// The origin for `track`, discovering it from `sample` in window mode.
    fn origin_for(&mut self, track: TrackId, sample: &Sample) -> Result<i64> {
        if let Some(ticks) = self.origins.get(&track) {
            return Ok(*ticks);
        }
        match &self.mode {
            Mode::Exact => Err(Error::RebaseMissingOrigin { track }),
            Mode::Window { .. } => {
                let ticks = sample.timing.and_then(|t| t.dts()).unwrap_or(0);
                self.origins.insert(track, ticks);
                Ok(ticks)
            }
        }
    }

    fn emit(&mut self, track: TrackId, mut sample: Sample) {
        let origin = self.origins.get(&track).copied().unwrap_or(0);
        if origin != 0
            && let Some(timing) = sample.timing.as_mut()
        {
            timing.shift(-origin);
        }
        self.ready.push_back((track, sample));
    }
}

impl Stage for RebaseStage {
    type In<'a> = (TrackId, Sample);
    type Out = (TrackId, Sample);
    type Error = Error;

    fn feed(&mut self, (track, sample): Self::In<'_>, now: Timestamp) -> Result<()> {
        if self.released {
            self.origin_for(track, &sample)?;
            self.emit(track, sample);
            return Ok(());
        }
        if self.started.is_none() {
            self.started = Some(now);
            if let Mode::Window { window, .. } = &self.mode {
                self.deadline = now.checked_add(*window);
            }
        }
        let bytes = u64::try_from(sample.data.len()).unwrap_or(u64::MAX);
        let max_bytes = match &self.mode {
            Mode::Window { max_bytes, .. } => *max_bytes,
            Mode::Exact => u64::MAX,
        };
        if self.pending_bytes.saturating_add(bytes) > max_bytes {
            return Err(Error::RebaseWindowOverflow { cap: max_bytes });
        }
        self.pending_bytes = self.pending_bytes.saturating_add(bytes);
        self.pending.push_back((track, sample));
        if self.deadline.is_some_and(|deadline| now >= deadline) {
            self.release();
        }
        Ok(())
    }

    fn poll(&mut self) -> Option<Self::Out> {
        self.ready.pop_front()
    }

    fn finish(&mut self) -> Result<()> {
        self.release();
        Ok(())
    }

    fn next_deadline(&self) -> Option<Timestamp> {
        if self.released { None } else { self.deadline }
    }

    fn on_deadline(&mut self, now: Timestamp) {
        if self.deadline.is_some_and(|deadline| now >= deadline) {
            self.release();
        }
    }
}

/// Shift a track's decode timeline by a signed delta, saturating at 0.
///
/// The shift is fixed per track from its first sample's anchor, so a negative
/// delta that would push the anchor below 0 is clamped and the *same*
/// effective delta is applied to every sample of that track — the anchor and
/// the sample timeline never disagree. A sample carrying no timestamp is left
/// untouched.
#[derive(Debug, Clone)]
pub struct OffsetStage {
    delta_ticks: i64,
    effective: BTreeMap<TrackId, i64>,
    ready: VecDeque<(TrackId, Sample)>,
}

impl OffsetStage {
    /// Shift every track by `delta_ticks` in its own media timescale.
    #[must_use]
    pub fn new(delta_ticks: i64) -> Self {
        Self {
            delta_ticks,
            effective: BTreeMap::new(),
            ready: VecDeque::new(),
        }
    }

    fn effective_for(&mut self, track: TrackId, sample: &Sample) -> i64 {
        if let Some(delta) = self.effective.get(&track) {
            return *delta;
        }
        let anchor = sample.timing.and_then(|t| t.dts()).unwrap_or(0);
        let effective = self.delta_ticks.max(-anchor);
        self.effective.insert(track, effective);
        effective
    }
}

impl Stage for OffsetStage {
    type In<'a> = (TrackId, Sample);
    type Out = (TrackId, Sample);
    type Error = Error;

    fn feed(&mut self, (track, mut sample): Self::In<'_>, _now: Timestamp) -> Result<()> {
        let delta = self.effective_for(track, &sample);
        if delta != 0
            && let Some(timing) = sample.timing.as_mut()
        {
            timing.shift(delta);
        }
        self.ready.push_back((track, sample));
        Ok(())
    }

    fn poll(&mut self) -> Option<Self::Out> {
        self.ready.pop_front()
    }

    fn finish(&mut self) -> Result<()> {
        Ok(())
    }
}

/// Insert a discontinuity gap before a given per-track sample index.
///
/// Every sample at or after `at_sample_index` (per track) has its `dts`/`pts`
/// pushed later by `gap_ticks`; earlier samples are unchanged. At index 0 the
/// whole track shifts. A sample carrying no timestamp is left untouched.
#[derive(Debug, Clone)]
pub struct GapStage {
    gap_ticks: i64,
    at_sample_index: usize,
    counts: BTreeMap<TrackId, usize>,
    ready: VecDeque<(TrackId, Sample)>,
}

impl GapStage {
    /// Insert `gap_ticks` before each track's sample `at_sample_index`.
    #[must_use]
    pub fn new(gap_ticks: u32, at_sample_index: usize) -> Self {
        Self {
            gap_ticks: i64::from(gap_ticks),
            at_sample_index,
            counts: BTreeMap::new(),
            ready: VecDeque::new(),
        }
    }
}

impl Stage for GapStage {
    type In<'a> = (TrackId, Sample);
    type Out = (TrackId, Sample);
    type Error = Error;

    fn feed(&mut self, (track, mut sample): Self::In<'_>, _now: Timestamp) -> Result<()> {
        let count = self.counts.entry(track).or_insert(0);
        if *count >= self.at_sample_index
            && let Some(timing) = sample.timing.as_mut()
        {
            timing.shift(self.gap_ticks);
        }
        *count = count.saturating_add(1);
        self.ready.push_back((track, sample));
        Ok(())
    }

    fn poll(&mut self) -> Option<Self::Out> {
        self.ready.pop_front()
    }

    fn finish(&mut self) -> Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{SampleFlags, SampleTiming};
    use alloc::vec;
    use alloc::vec::Vec;

    fn sample(dts: i64, duration: u32) -> Sample {
        Sample {
            data: vec![0u8; 4].into(),
            timing: Some(SampleTiming::timed_with_duration(dts, dts, duration)),
            flags: SampleFlags::SYNC,
            provenance: None,
            display: None,
            protection: None,
        }
    }

    fn samples_at(start: i64, durs: &[u32]) -> Vec<Sample> {
        let mut dts = start;
        let mut out = Vec::with_capacity(durs.len());
        for &d in durs {
            out.push(sample(dts, d));
            dts += i64::from(d);
        }
        out
    }

    fn run<S>(stage: &mut S, track: TrackId, input: Vec<Sample>) -> Vec<Sample>
    where
        S: for<'a> Stage<In<'a> = (TrackId, Sample), Out = (TrackId, Sample), Error = Error>,
    {
        let mut out = Vec::new();
        for sample in input {
            stage
                .feed((track, sample), Timestamp::ZERO)
                .expect("feed must not fail for valid input");
            while let Some((_, sample)) = stage.poll() {
                out.push(sample);
            }
        }
        stage.finish().expect("finish must not fail");
        while let Some((_, sample)) = stage.poll() {
            out.push(sample);
        }
        out
    }

    fn dts(samples: &[Sample]) -> Vec<Option<i64>> {
        samples
            .iter()
            .map(|s| s.timing.and_then(|t| t.dts()))
            .collect()
    }

    #[test]
    fn rebase_with_origin_clears_the_first_sample_and_keeps_the_grid() {
        let track = TrackId::new(1);
        let mut stage = RebaseStage::with_origin(RebaseOrigin::new().with(track, 90_000));
        let out = run(&mut stage, track, samples_at(90_000, &[3000, 3000]));
        assert_eq!(dts(&out), vec![Some(0), Some(3000)]);
        assert_eq!(
            out[1].timing.and_then(|t| t.pts()),
            Some(3000),
            "pts moves with dts"
        );
    }

    #[test]
    fn rebase_leaves_an_untimed_sample_untouched() {
        let track = TrackId::new(1);
        let mut samples = samples_at(1000, &[10]);
        samples[0].timing = None;
        let mut stage = RebaseStage::with_origin(RebaseOrigin::new().with(track, 1000));
        let out = run(&mut stage, track, samples);
        assert_eq!(dts(&out), vec![None], "never fabricate a dts");
        assert_eq!(
            out[0].timing.and_then(|t| t.pts()),
            None,
            "never fabricate a pts"
        );
    }

    #[test]
    fn rebase_missing_origin_is_reported() {
        let track = TrackId::new(7);
        let mut stage = RebaseStage::with_origin(RebaseOrigin::new());
        let err = stage
            .feed((track, sample(0, 10)), Timestamp::ZERO)
            .expect_err("a track with no origin must fail");
        assert_eq!(err, Error::RebaseMissingOrigin { track });
    }

    #[test]
    fn offset_shifts_anchor_and_samples_together() {
        let track = TrackId::new(1);
        let mut plus = OffsetStage::new(50);
        let shifted = run(&mut plus, track, samples_at(100, &[10]));
        assert_eq!(dts(&shifted), vec![Some(150)]);
        let mut minus = OffsetStage::new(-1000);
        let clamped = run(&mut minus, track, shifted);
        assert_eq!(
            dts(&clamped),
            vec![Some(0)],
            "underflow saturates at 0 and the clamped delta applies to every sample"
        );
    }

    #[test]
    fn gap_extends_timeline_and_shifts_later_samples() {
        let track = TrackId::new(1);
        let mut stage = GapStage::new(500, 2);
        let out = run(&mut stage, track, samples_at(0, &[100, 100, 100]));
        assert_eq!(
            dts(&out),
            vec![Some(0), Some(100), Some(700)],
            "the gap pushes the insertion point and every later sample out"
        );
    }

    #[test]
    fn gap_at_index_zero_shifts_every_sample() {
        let track = TrackId::new(1);
        let mut stage = GapStage::new(250, 0);
        let out = run(&mut stage, track, samples_at(1000, &[100, 100]));
        assert_eq!(dts(&out), vec![Some(1250), Some(1350)]);
    }

    #[test]
    fn a_late_track_in_a_start_window_never_goes_negative() {
        let early = TrackId::new(1);
        let late = TrackId::new(2);
        let mut stage = RebaseStage::start_window(core::time::Duration::from_millis(100), 1 << 20);
        stage
            .feed((early, sample(5000, 100)), Timestamp::ZERO)
            .expect("feed");
        let deadline = stage.next_deadline().expect("a window has a deadline");
        stage.on_deadline(deadline);
        stage
            .feed((late, sample(10, 100)), deadline)
            .expect("a late track rebases on its own first sample");
        stage.finish().expect("finish");
        let mut early_out = Vec::new();
        let mut late_out = Vec::new();
        while let Some((track, sample)) = stage.poll() {
            if track == early {
                early_out.push(sample);
            } else {
                late_out.push(sample);
            }
        }
        assert_eq!(dts(&early_out), vec![Some(0)]);
        assert_eq!(
            dts(&late_out),
            vec![Some(0)],
            "a track's first sample is non-negative even when it arrives late"
        );
    }

    #[test]
    fn a_start_window_byte_cap_is_a_typed_error() {
        let track = TrackId::new(1);
        let mut stage = RebaseStage::start_window(core::time::Duration::from_secs(10), 2);
        let err = stage
            .feed((track, sample(0, 100)), Timestamp::ZERO)
            .expect_err("four bytes exceed a two-byte cap");
        assert_eq!(err, Error::RebaseWindowOverflow { cap: 2 });
    }
}
