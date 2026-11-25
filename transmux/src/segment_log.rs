//! What a manifest needs to know about the segments a muxer produced, kept
//! apart from the samples: [`SegmentSummaryLog`].
//!
//! A manifest renderer ([`crate::dash::DashPackager`],
//! [`crate::ll_dash::LlDashPackager`], [`crate::SmoothPackager`],
//! [`crate::media::HlsPackager`]) never sees a sample. It is given the track
//! specs plus this log, which is fed one [`SegmentSummary`](crate::mux_stage::SegmentSummary) per finished segment
//! (each [`MuxOutput::Segment`](crate::mux_stage::MuxOutput::Segment) carries
//! one).
//!
//! # Run-length coding
//!
//! Consecutive segments that share a duration, a contiguity and an
//! independence flag collapse into one [`SegmentRun`]: a constant-rate VOD is
//! then a handful of entries per track however many segments it holds, which is
//! what keeps a manifest's cost independent of the segment count. Per-track
//! totals (bytes, duration, segment count) are running sums, so a bandwidth
//! needs no walk over the runs.
//!
//! # Capacity is the caller's
//!
//! [`LogCapacity::Unbounded`] keeps every segment (a VOD manifest). A live
//! manifest advertises a window, and [`LogCapacity::Window`] drops the oldest
//! segments past it — there is no library default, because how deep a manifest
//! looks back is the caller's policy.

use alloc::collections::VecDeque;
use alloc::vec::Vec;
use core::num::NonZeroUsize;

use crate::error::{Error, Result};
use crate::ir::TrackId;
use crate::mux_stage::{SegmentOut, TrackPart};

/// How many segments a [`SegmentSummaryLog`] keeps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum LogCapacity {
    /// Every segment, from the beginning of the output (a VOD manifest).
    #[default]
    Unbounded,
    /// At most this many of the most recent segments (a live manifest's
    /// advertised window).
    Window(NonZeroUsize),
}

impl LogCapacity {
    /// The manifest-facing label for this capacity.
    pub fn name(&self) -> &'static str {
        match self {
            LogCapacity::Unbounded => "unbounded",
            LogCapacity::Window(_) => "window",
        }
    }
}

broadcast_common::impl_spec_display!(LogCapacity);

/// A run of contiguous segments that share a duration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SegmentRun {
    /// Decode time of the run's first segment, in the track's timescale ticks.
    pub start: u64,
    /// The duration of each segment in the run, in the track's timescale ticks.
    pub duration: u64,
    /// The run's total bytes (the sum over its segments).
    pub bytes: u64,
    /// Every segment in the run starts with a sync sample (they were merged
    /// only when their flags agree).
    pub independent: bool,
    /// How many segments the run covers.
    pub segments: u64,
    /// The samples the run's segments carried, summed.
    pub samples: u64,
}

/// One track's accumulated segment summaries.
#[derive(Debug, Clone)]
struct TrackRunLog {
    track: TrackId,
    timescale: u32,
    /// Oldest first.
    runs: VecDeque<SegmentRun>,
    /// Segments currently retained.
    segments: u64,
    /// Sum of the retained segments' durations, in [`Self::timescale`] ticks.
    duration: u64,
    /// Sum of the retained segments' bytes.
    bytes: u64,
    /// Samples the segments carried, summed.
    samples: u64,
}

impl TrackRunLog {
    fn new(track: TrackId, timescale: u32) -> Self {
        Self {
            track,
            timescale,
            runs: VecDeque::new(),
            segments: 0,
            duration: 0,
            bytes: 0,
            samples: 0,
        }
    }

    /// Append one segment's part for this track, merging it into the last run
    /// when the two are adjacent and alike.
    fn push(&mut self, part: &TrackPart, independent: bool) {
        if let Some(last) = self.runs.back_mut()
            && last.duration == part.duration
            && last.independent == independent
            && last
                .start
                .saturating_add(last.duration.saturating_mul(last.segments))
                == part.start
        {
            last.segments += 1;
            last.bytes = last.bytes.saturating_add(part.bytes);
            last.samples = last.samples.saturating_add(u64::from(part.samples));
        } else {
            self.runs.push_back(SegmentRun {
                start: part.start,
                duration: part.duration,
                bytes: part.bytes,
                independent,
                segments: 1,
                samples: u64::from(part.samples),
            });
        }
        self.segments += 1;
        self.duration = self.duration.saturating_add(part.duration);
        self.bytes = self.bytes.saturating_add(part.bytes);
        self.samples = self.samples.saturating_add(u64::from(part.samples));
    }

    /// Drop the oldest segment, keeping the running totals in step.
    fn drop_oldest(&mut self) -> bool {
        let Some(front) = self.runs.front_mut() else {
            return false;
        };
        let duration = front.duration;
        // A run's `bytes` is a sum over segments that share a duration; the
        // oldest takes an equal share, so the totals keep equalling the sum of
        // the runs' bytes to within the integer division of one share.
        let segment_bytes = front.bytes / front.segments;
        let segment_samples = front.samples / front.segments;
        front.bytes = front.bytes.saturating_sub(segment_bytes);
        front.samples = front.samples.saturating_sub(segment_samples);
        front.segments -= 1;
        front.start = front.start.saturating_add(duration);
        if front.segments == 0 {
            self.runs.pop_front();
        }
        self.segments -= 1;
        self.duration = self.duration.saturating_sub(duration);
        self.bytes = self.bytes.saturating_sub(segment_bytes);
        self.samples = self.samples.saturating_sub(segment_samples);
        true
    }

    /// This track's summaries as a manifest renderer reads them.
    fn summary(&self) -> TrackSummary<'_> {
        TrackSummary {
            track: self.track,
            timescale: self.timescale,
            segments: self.segments,
            duration: self.duration,
            bytes: self.bytes,
            samples: self.samples,
            runs: &self.runs,
        }
    }
}

/// The per-track view a manifest renderer reads.
#[derive(Debug, Clone, Copy)]
pub struct TrackSummary<'a> {
    /// The track the summaries belong to.
    pub track: TrackId,
    /// Ticks per second of every time in this summary.
    pub timescale: u32,
    /// Segments currently retained.
    pub segments: u64,
    /// Sum of the retained segments' durations, in [`Self::timescale`] ticks.
    pub duration: u64,
    /// Sum of the retained segments' bytes.
    pub bytes: u64,
    /// Samples the segments carried.
    pub samples: u64,
    runs: &'a VecDeque<SegmentRun>,
}

impl<'a> TrackSummary<'a> {
    /// The run-length coded segments, oldest first.
    pub fn runs(&self) -> impl Iterator<Item = &'a SegmentRun> + '_ {
        self.runs.iter()
    }

    /// Every retained segment's duration, oldest first.
    pub fn segment_durations(&self) -> impl Iterator<Item = u64> + '_ {
        self.runs.iter().flat_map(|run| {
            core::iter::repeat_n(
                run.duration,
                usize::try_from(run.segments).unwrap_or(usize::MAX),
            )
        })
    }

    /// Decode time just past the newest retained segment, in
    /// [`Self::timescale`] ticks.
    pub fn end(&self) -> u64 {
        self.runs.back().map_or(0, |run| {
            run.start
                .saturating_add(run.duration.saturating_mul(run.segments))
        })
    }
}

/// Every track's segment summaries, as a manifest reads them.
#[derive(Debug, Clone, Default)]
pub struct SegmentSummaryLog {
    capacity: LogCapacity,
    tracks: Vec<TrackRunLog>,
}

impl SegmentSummaryLog {
    /// An empty log keeping [`LogCapacity::Unbounded`] segments.
    pub fn new() -> Self {
        Self::with_capacity(LogCapacity::Unbounded)
    }

    /// An empty log keeping at most `capacity` segments per track.
    pub fn with_capacity(capacity: LogCapacity) -> Self {
        Self {
            capacity,
            tracks: Vec::new(),
        }
    }

    /// The capacity this log keeps to.
    pub fn capacity(&self) -> LogCapacity {
        self.capacity
    }

    /// Record one finished segment: one entry per track the segment carries.
    ///
    /// # Errors
    /// [`Error::InvalidInput`] if a part names a timescale that disagrees with
    /// the one already recorded for its track — a track's timescale is fixed
    /// for the life of its stream (`mdhd`), so a change is a different stream,
    /// not a later segment of the same one.
    pub fn push(&mut self, segment: &SegmentOut) -> Result<()> {
        for part in &segment.tracks {
            self.push_part(part, segment.summary.independent)?;
        }
        Ok(())
    }

    /// Record one track's share of a finished segment, with the segment's
    /// random-access flag.
    ///
    /// # Errors
    /// As [`Self::push`].
    pub fn push_part(&mut self, part: &TrackPart, independent: bool) -> Result<()> {
        let index = match self.tracks.iter().position(|t| t.track == part.track) {
            Some(index) => index,
            None => {
                self.tracks
                    .push(TrackRunLog::new(part.track, part.timescale));
                self.tracks.len() - 1
            }
        };
        let track = &mut self.tracks[index];
        if track.timescale != part.timescale {
            return Err(Error::InvalidInput(
                "a track's segment summaries disagree on its timescale",
            ));
        }
        track.push(part, independent);
        if let LogCapacity::Window(window) = self.capacity {
            while track.segments > window.get() as u64 && track.drop_oldest() {}
        }
        Ok(())
    }

    /// Record every segment of `outputs`, in order.
    ///
    /// # Errors
    /// As [`Self::push`].
    pub fn extend<'s>(&mut self, segments: impl IntoIterator<Item = &'s SegmentOut>) -> Result<()> {
        for segment in segments {
            self.push(segment)?;
        }
        Ok(())
    }

    /// One track's summaries, or `None` when no segment of it was recorded.
    pub fn track(&self, track: TrackId) -> Option<TrackSummary<'_>> {
        self.tracks
            .iter()
            .find(|t| t.track == track)
            .map(TrackRunLog::summary)
    }

    /// Every track's summaries, in first-seen order.
    pub fn tracks(&self) -> impl Iterator<Item = TrackSummary<'_>> + '_ {
        self.tracks.iter().map(TrackRunLog::summary)
    }

    /// `true` when no segment was recorded.
    pub fn is_empty(&self) -> bool {
        self.tracks.iter().all(|t| t.segments == 0)
    }
}
