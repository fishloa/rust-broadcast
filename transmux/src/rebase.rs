//! Timeline-conditioning transforms over the sample stream — PTS/DTS rebase,
//! offset, and discontinuity-gap insertion (issue #476).
//!
//! Media plane step 2c: [`Sample`] `dts`/`pts` are
//! **absolute** ticks in the track's media timescale, not a running sum
//! anchored on a track-level start. These stages shift that absolute pair
//! directly:
//!
//! - [`RebaseStage`] — re-origin every track onto one common decode origin,
//!   so the earliest first sample starts at 0 and inter-track sync is kept.
//!   Built with an exact per-track [`RebaseOrigin`] (from a sample plan) or a
//!   [`RebaseStage::start_window`] that discovers the origin across announced
//!   tracks, holding output no longer than the caller's window or byte cap.
//! - [`OffsetStage`] — shift a track by a signed delta (saturating at 0 on
//!   underflow, per track, on its first sample's anchor).
//! - [`GapStage`] — push every sample from a given per-track index onward (or
//!   the whole track, at index 0) later by a gap (splice / concat
//!   conditioning).
//!
//! [`RebaseStage`] is fed [`MuxInput`] because a track announcement carries the
//! timescale its origin must be converted into; [`OffsetStage`] and
//! [`GapStage`] are fed `(TrackId, Sample)`. No transform holds a whole
//! [`Media`](crate::ir::Media); the former whole-input free functions are gone.
//!
//! **33-bit MPEG-2 Systems wrap-unrolling happens at the demux edge**
//! ([`crate::ts_demux`] unwraps the wire's 33-bit PTS/DTS once,
//! incrementally), so every `Sample` this crate produces is already absolute.

use alloc::collections::{BTreeMap, BTreeSet, VecDeque};

use broadcast_common::stage::{Stage, Timestamp};
use broadcast_common::timescale::{Rounding, Timescale, cmp_ticks, rescale_i64};

use crate::TrackId;
use crate::error::{Error, Result};
use crate::ir::{Sample, TrackSpec};
use crate::mux_stage::MuxInput;

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
    /// One common origin chosen from the tracks that produced a first sample,
    /// buffering output no longer than `window` of driver time or `max_bytes`
    /// of sample bytes.
    Window {
        /// The caller's wall-clock hold.
        window: core::time::Duration,
        /// The caller's byte cap on buffered samples.
        max_bytes: u64,
    },
}

/// Re-origin every track's decode timeline onto one common origin.
///
/// Fed [`MuxInput`]s. A [`MuxInput::Track`] announcement supplies the track's
/// timescale and passes through unchanged. A [`MuxInput::Sample`] is shifted by
/// the track's origin. Built with an exact per-track [`RebaseOrigin`] (from a
/// sample plan) or a [`RebaseStage::start_window`] that discovers one origin
/// for every track.
///
/// Drive it with [`feed`](Stage::feed), then [`poll`](Stage::poll) for the
/// rebased inputs, and [`finish`](Stage::finish) at end of input. A sample
/// carrying no timestamp is passed through untouched — no timestamp is ever
/// fabricated.
#[derive(Debug, Clone)]
pub struct RebaseStage {
    mode: Mode,
    origins: BTreeMap<TrackId, i64>,
    timescales: BTreeMap<TrackId, Timescale>,
    origin: Option<(i64, Timescale)>,
    pending: VecDeque<MuxInput>,
    pending_bytes: u64,
    announced: BTreeSet<TrackId>,
    produced: BTreeSet<TrackId>,
    firsts: BTreeMap<TrackId, (i64, Timescale)>,
    started: Option<Timestamp>,
    deadline: Option<Timestamp>,
    released: bool,
    ready: VecDeque<MuxInput>,
}

impl RebaseStage {
    /// Rebase with exact per-track origins (from a sample plan).
    #[must_use]
    pub fn with_origin(origin: RebaseOrigin) -> Self {
        Self {
            mode: Mode::Exact,
            origins: origin.origins,
            timescales: BTreeMap::new(),
            origin: None,
            pending: VecDeque::new(),
            pending_bytes: 0,
            announced: BTreeSet::new(),
            produced: BTreeSet::new(),
            firsts: BTreeMap::new(),
            started: None,
            deadline: None,
            released: true,
            ready: VecDeque::new(),
        }
    }

    /// Choose one common origin from the first samples seen, holding output for
    /// at most `window` of driver time or `max_bytes` of buffered sample bytes.
    ///
    /// Buffering ends as soon as every announced track has produced a sample,
    /// when the window closes (`on_deadline`), or at `finish`. The origin is
    /// the earliest first decode time across the tracks that produced a timed
    /// sample, compared across timescales exactly; it is converted into each
    /// track's own timescale. A track that first appears after the origin was
    /// chosen uses that same origin, and a sample earlier than it is
    /// [`Error::RebaseBeforeOrigin`] (never clamped).
    #[must_use]
    pub fn start_window(window: core::time::Duration, max_bytes: u64) -> Self {
        Self {
            mode: Mode::Window { window, max_bytes },
            origins: BTreeMap::new(),
            timescales: BTreeMap::new(),
            origin: None,
            pending: VecDeque::new(),
            pending_bytes: 0,
            announced: BTreeSet::new(),
            produced: BTreeSet::new(),
            firsts: BTreeMap::new(),
            started: None,
            deadline: None,
            released: false,
            ready: VecDeque::new(),
        }
    }

    fn record_track(&mut self, spec: &TrackSpec) {
        self.timescales.insert(spec.track_id, spec.timescale);
    }

    /// The origin to subtract for `track`, in its own timescale.
    fn origin_for(&self, track: TrackId) -> Result<Option<i64>> {
        match self.mode {
            Mode::Exact => self
                .origins
                .get(&track)
                .copied()
                .map(Some)
                .ok_or(Error::RebaseMissingOrigin { track }),
            Mode::Window { .. } => match self.origin {
                None => Ok(None),
                Some((ticks, timescale)) => {
                    let track_scale = self
                        .timescales
                        .get(&track)
                        .copied()
                        .ok_or(Error::RebaseUnannouncedTrack { track })?;
                    Ok(Some(rescale_i64(
                        ticks,
                        timescale,
                        track_scale,
                        Rounding::Floor,
                    )))
                }
            },
        }
    }

    fn emit_sample(&mut self, track: TrackId, mut sample: Sample) -> Result<()> {
        if let Some(origin) = self.origin_for(track)?
            && let Some(timing) = sample.timing.as_mut()
        {
            if matches!(self.mode, Mode::Window { .. })
                && let Some(dts) = timing.dts()
                && dts < origin
            {
                return Err(Error::RebaseBeforeOrigin { track, dts, origin });
            }
            if origin != 0 {
                timing.shift(-origin);
            }
        }
        self.ready.push_back(MuxInput::Sample { track, sample });
        Ok(())
    }

    fn push_passthrough(&mut self, input: MuxInput) {
        if self.released {
            self.ready.push_back(input);
        } else {
            self.pending.push_back(input);
        }
    }

    /// Release every held input, choosing the common origin first.
    fn release(&mut self) -> Result<()> {
        if self.released {
            return Ok(());
        }
        self.released = true;
        self.deadline = None;
        self.choose_origin();
        while let Some(input) = self.pending.pop_front() {
            match input {
                MuxInput::Sample { track, sample } => self.emit_sample(track, sample)?,
                other => self.ready.push_back(other),
            }
        }
        self.pending_bytes = 0;
        Ok(())
    }

    /// The earliest first decode time across the tracks that produced a timed
    /// sample, compared as exact rationals (no float).
    fn choose_origin(&mut self) {
        let mut best: Option<(i64, Timescale)> = None;
        for (ticks, timescale) in self.firsts.values().copied() {
            best = Some(match best {
                None => (ticks, timescale),
                Some((best_ticks, best_scale)) => {
                    if cmp_ticks(ticks, timescale, best_ticks, best_scale).is_lt() {
                        (ticks, timescale)
                    } else {
                        (best_ticks, best_scale)
                    }
                }
            });
        }
        self.origin = best;
    }

    fn all_announced_produced(&self) -> bool {
        !self.announced.is_empty() && self.announced.iter().all(|t| self.produced.contains(t))
    }

    fn maybe_release(&mut self, now: Timestamp) -> Result<()> {
        if self.released {
            return Ok(());
        }
        let expired = self.started.is_some() && self.deadline.is_some_and(|d| now >= d);
        if self.all_announced_produced() || expired {
            self.release()?;
        }
        Ok(())
    }
}

impl Stage for RebaseStage {
    type In<'a> = MuxInput;
    type Out = MuxInput;
    type Error = Error;

    fn feed(&mut self, input: Self::In<'_>, now: Timestamp) -> Result<()> {
        match input {
            MuxInput::Track(spec) => {
                self.record_track(&spec);
                if !self.released {
                    self.announced.insert(spec.track_id);
                }
                self.push_passthrough(MuxInput::Track(spec));
                self.maybe_release(now)?;
            }
            MuxInput::Sample { track, sample } => {
                if self.released {
                    self.emit_sample(track, sample)?;
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
                let timescale = self
                    .timescales
                    .get(&track)
                    .copied()
                    .ok_or(Error::RebaseUnannouncedTrack { track })?;
                if self.produced.insert(track)
                    && let Some(dts) = sample.timing.and_then(|t| t.dts())
                {
                    self.firsts.insert(track, (dts, timescale));
                }
                self.pending.push_back(MuxInput::Sample { track, sample });
                self.maybe_release(now)?;
            }
            other => self.push_passthrough(other),
        }
        Ok(())
    }

    fn poll(&mut self) -> Option<Self::Out> {
        self.ready.pop_front()
    }

    fn finish(&mut self) -> Result<()> {
        self.release()
    }

    fn next_deadline(&self) -> Option<Timestamp> {
        if self.released { None } else { self.deadline }
    }

    fn on_deadline(&mut self, now: Timestamp) {
        if self.deadline.is_some_and(|deadline| now >= deadline) {
            // `release` only fails on a sample that precedes the origin, which
            // a driver cannot report from `on_deadline`; such a stream is
            // reported from `feed` before the deadline can matter.
            let _ = self.release();
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
    use crate::ir::SampleTiming;
    use crate::pipeline::CodecConfig;
    use alloc::boxed::Box;
    use alloc::vec;
    use alloc::vec::Vec;
    use isobmff::avc_config::{AVCConfigurationBox, AVCDecoderConfigurationRecord};
    use isobmff::nalu_types::{AvcPps, AvcSps};

    fn sample(dts: i64, duration: u32) -> Sample {
        Sample {
            data: vec![0u8; 4].into(),
            timing: Some(SampleTiming::timed_with_duration(dts, dts, duration)),
            flags: crate::ir::SampleFlags::SYNC,
            provenance: None,
            display: None,
            protection: None,
        }
    }

    fn spec(track: TrackId, timescale: u32) -> TrackSpec {
        let record = AVCDecoderConfigurationRecord {
            configuration_version: 1,
            profile_indication: 66,
            profile_compatibility: 0,
            level_indication: 30,
            length_size_minus_one: 3,
            sps: vec![AvcSps(vec![0x67, 0x42, 0x00, 0x1e])],
            pps: vec![AvcPps(vec![0x68, 0xce, 0x3c, 0x80])],
            chroma_format: None,
            bit_depth_luma_minus8: None,
            bit_depth_chroma_minus8: None,
            sps_ext: vec![],
        };
        TrackSpec::new(
            track,
            Timescale::try_new(timescale).expect("a non-zero timescale"),
            CodecConfig::Avc {
                config: AVCConfigurationBox::new(record),
                width: 16,
                height: 16,
            },
        )
    }

    fn announce(track: TrackId, timescale: u32) -> MuxInput {
        MuxInput::Track(Box::new(spec(track, timescale)))
    }

    fn timed(track: TrackId, dts: i64, duration: u32) -> MuxInput {
        MuxInput::Sample {
            track,
            sample: sample(dts, duration),
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

    fn run_rebase(stage: &mut RebaseStage, inputs: Vec<MuxInput>) -> Vec<MuxInput> {
        let mut out = Vec::new();
        for input in inputs {
            stage.feed(input, Timestamp::ZERO).expect("feed");
            while let Some(item) = stage.poll() {
                out.push(item);
            }
        }
        stage.finish().expect("finish");
        while let Some(item) = stage.poll() {
            out.push(item);
        }
        out
    }

    fn by_track(out: &[MuxInput]) -> BTreeMap<TrackId, Vec<Sample>> {
        let mut tracks: BTreeMap<TrackId, Vec<Sample>> = BTreeMap::new();
        for item in out {
            if let MuxInput::Sample { track, sample } = item {
                tracks.entry(*track).or_default().push(sample.clone());
            }
        }
        tracks
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
        let mut inputs = vec![announce(track, 90_000)];
        for sample in samples_at(90_000, &[3000, 3000]) {
            inputs.push(MuxInput::Sample { track, sample });
        }
        let out = by_track(&run_rebase(&mut stage, inputs));
        let samples = &out[&track];
        assert_eq!(dts(samples), vec![Some(0), Some(3000)]);
        assert_eq!(
            samples[1].timing.and_then(|t| t.pts()),
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
        let out = by_track(&run_rebase(
            &mut stage,
            vec![
                announce(track, 90_000),
                MuxInput::Sample {
                    track,
                    sample: samples.remove(0),
                },
            ],
        ));
        let samples = &out[&track];
        assert_eq!(dts(samples), vec![None], "never fabricate a dts");
        assert_eq!(
            samples[0].timing.and_then(|t| t.pts()),
            None,
            "never fabricate a pts"
        );
    }

    #[test]
    fn rebase_missing_origin_is_reported() {
        let track = TrackId::new(7);
        let mut stage = RebaseStage::with_origin(RebaseOrigin::new());
        stage
            .feed(announce(track, 90_000), Timestamp::ZERO)
            .expect("announce");
        let err = stage
            .feed(timed(track, 0, 10), Timestamp::ZERO)
            .expect_err("a track with no origin must fail");
        assert_eq!(err, Error::RebaseMissingOrigin { track });
    }

    #[test]
    fn a_start_window_keeps_audio_and_video_in_sync() {
        let video = TrackId::new(1);
        let audio = TrackId::new(2);
        let mut stage = RebaseStage::start_window(core::time::Duration::from_secs(10), 1 << 20);
        let out = by_track(&run_rebase(
            &mut stage,
            vec![
                announce(video, 90_000),
                announce(audio, 48_000),
                timed(video, 9000, 3000),
                timed(audio, 1920, 1024),
            ],
        ));
        assert_eq!(
            dts(&out[&audio]),
            vec![Some(0)],
            "audio's 0.04 s first sample is the origin"
        );
        assert_eq!(
            dts(&out[&video]),
            vec![Some(5400)],
            "video's 0.10 s first sample keeps its 0.06 s offset from the origin (5400 @ 90 kHz)"
        );
    }

    #[test]
    fn a_late_track_in_a_start_window_uses_the_chosen_origin() {
        let early = TrackId::new(1);
        let late = TrackId::new(2);
        let mut stage = RebaseStage::start_window(core::time::Duration::from_millis(100), 1 << 20);
        stage
            .feed(announce(early, 90_000), Timestamp::ZERO)
            .expect("announce");
        stage
            .feed(announce(late, 90_000), Timestamp::ZERO)
            .expect("announce");
        stage
            .feed(timed(early, 5000, 100), Timestamp::ZERO)
            .expect("feed");
        let deadline = stage.next_deadline().expect("a window has a deadline");
        stage.on_deadline(deadline);
        stage
            .feed(timed(late, 6000, 100), deadline)
            .expect("a late track uses the chosen origin");
        stage.finish().expect("finish");
        let mut late_out = Vec::new();
        while let Some(item) = stage.poll() {
            if let MuxInput::Sample { track, sample } = item
                && track == late
            {
                late_out.push(sample);
            }
        }
        assert_eq!(
            dts(&late_out),
            vec![Some(1000)],
            "6000 - the origin (5000) is non-negative"
        );
    }

    #[test]
    fn a_late_track_earlier_than_the_chosen_origin_is_an_error() {
        let early = TrackId::new(1);
        let late = TrackId::new(2);
        let mut stage = RebaseStage::start_window(core::time::Duration::from_millis(100), 1 << 20);
        stage
            .feed(announce(early, 90_000), Timestamp::ZERO)
            .expect("announce");
        stage
            .feed(announce(late, 90_000), Timestamp::ZERO)
            .expect("announce");
        stage
            .feed(timed(early, 5000, 100), Timestamp::ZERO)
            .expect("feed");
        let deadline = stage.next_deadline().expect("a window has a deadline");
        stage.on_deadline(deadline);
        let err = stage
            .feed(timed(late, 4000, 100), deadline)
            .expect_err("earlier than the origin must be reported");
        assert_eq!(
            err,
            Error::RebaseBeforeOrigin {
                track: late,
                dts: 4000,
                origin: 5000
            }
        );
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
    fn a_start_window_byte_cap_is_a_typed_error() {
        let track = TrackId::new(1);
        let mut stage = RebaseStage::start_window(core::time::Duration::from_secs(10), 2);
        stage
            .feed(announce(track, 90_000), Timestamp::ZERO)
            .expect("announce");
        let err = stage
            .feed(timed(track, 0, 100), Timestamp::ZERO)
            .expect_err("four bytes exceed a two-byte cap");
        assert_eq!(err, Error::RebaseWindowOverflow { cap: 2 });
    }
}
