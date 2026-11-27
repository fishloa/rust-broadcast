//! The one input and output vocabulary every muxer [`Stage`]
//! speaks: a muxer is `Stage<In<'a> = MuxInput, Out = MuxOutput>`.
//!
//! A muxer holds at most one output unit (a fragment, a TS PES, an FLV tag,
//! a Matroska cluster, an RTP packet group). Manifest packagers never see
//! samples: they render from the [`SegmentSummary`] each [`MuxOutput::Segment`]
//! carries.

use alloc::boxed::Box;
use alloc::collections::VecDeque;
use alloc::string::String;
use alloc::vec::Vec;

use broadcast_common::Demand;
use broadcast_common::stage::{Stage, Timestamp};
use bytes::Bytes;

use crate::error::Result;
use crate::ir::{Sample, TrackId, TrackSpec};

/// What a muxer stage is fed.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum MuxInput {
    /// A track is announced, or an announced track's setup changed.
    Track(Box<TrackSpec>),
    /// One coded sample of an announced track, in decode order per track.
    Sample {
        /// The announced track this sample belongs to.
        track: TrackId,
        /// The sample.
        sample: Sample,
    },
    /// The media timeline is discontinuous before the next sample of
    /// `track`, or of every track when `None`.
    ///
    /// A stage that can express the break records it: an HLS
    /// `#EXT-X-DISCONTINUITY` on the next cut, the next fragment's
    /// `SegmentSummary`, or the MPEG-2 TS adaptation-field
    /// `discontinuity_indicator` on the next packet of each affected
    /// elementary PID. One that cannot refuses it with
    /// [`Error::UnsupportedFeature`](crate::Error::UnsupportedFeature) rather
    /// than returning `Ok(())` and dropping the mark.
    Discontinuity {
        /// The track the discontinuity applies to; `None` for all tracks.
        track: Option<TrackId>,
    },
    /// `track` is no longer part of the stream before the next sample of any
    /// other track: the source stopped carrying it (a PMT version bump that
    /// drops an elementary stream, a track that ended early).
    ///
    /// The demux side already names this ([`DemuxEvent::TrackRemoved`]), so a
    /// transform that removes a track says it here. A muxer whose output has a
    /// fixed track set (an fMP4/CMAF init segment, a progressive file) refuses
    /// it with [`Error::UnsupportedFeature`](crate::Error::UnsupportedFeature)
    /// rather than silently continuing to describe a track it will never see
    /// again; an MPEG-2 TS muxer re-emits its PMT without the stream.
    ///
    /// [`DemuxEvent::TrackRemoved`]: crate::ir::DemuxEvent::TrackRemoved
    TrackRemoved {
        /// The track the source stopped carrying.
        track: TrackId,
    },
    /// Close the unit being built now, without waiting for its natural cut.
    Flush,
}

impl MuxInput {
    /// The mux-vocabulary input a demux event becomes, or `None` for an event
    /// with no place in it (a degradation note, a display-geometry change, a
    /// clock reference).
    ///
    /// The demux side's event vocabulary is wider than a muxer's, so this is
    /// the one translation: the CLI and [`crate::Repackage`] both route their
    /// demux events through it before feeding a transform.
    #[must_use]
    pub fn from_demux(event: crate::ir::DemuxEvent) -> Option<Self> {
        use crate::ir::DemuxEvent;
        match event {
            DemuxEvent::TrackAdded(spec) | DemuxEvent::TrackUpdated(spec) => {
                Some(MuxInput::Track(Box::new(spec)))
            }
            DemuxEvent::TrackRemoved { track_id, .. } => {
                Some(MuxInput::TrackRemoved { track: track_id })
            }
            DemuxEvent::Sample { track_id, sample } => Some(MuxInput::Sample {
                track: track_id,
                sample,
            }),
            DemuxEvent::Discontinuity { track, .. } => Some(MuxInput::Discontinuity { track }),
            _ => None,
        }
    }
}

/// What a muxer stage produces.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum MuxOutput {
    /// Bytes appended to the output stream.
    Bytes(Bytes),
    /// Bytes written over an earlier part of the output at `offset`; only a
    /// muxer writing to a seekable output emits this.
    BytesAt {
        /// Absolute byte offset in the output.
        offset: u64,
        /// The bytes to write there.
        bytes: Bytes,
    },
    /// An initialisation segment.
    Init(Bytes),
    /// One finished media segment and its summary.
    Segment(Box<SegmentOut>),
    /// A rendered playlist or manifest.
    Text(String),
}

/// One finished media segment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SegmentOut {
    /// The segment's bytes.
    pub bytes: Bytes,
    /// What a manifest needs to describe the segment as a whole.
    pub summary: SegmentSummary,
    /// One entry per track the segment carries, in the segment's track order:
    /// what a per-representation manifest needs (its own start, duration, byte
    /// count and sample count), which the segment-level summary cannot express.
    pub tracks: Vec<TrackPart>,
}

/// One track's share of a segment: the per-representation view a DASH or Smooth
/// manifest describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrackPart {
    /// The track this part belongs to.
    pub track: TrackId,
    /// Decode time of this track's first sample in the segment, in
    /// [`Self::timescale`] ticks.
    pub start: u64,
    /// This track's duration in the segment, in [`Self::timescale`] ticks.
    pub duration: u64,
    /// Ticks per second of [`Self::start`] and [`Self::duration`].
    pub timescale: u32,
    /// This track's bytes in the segment: its own fragment bytes where the
    /// container separates tracks, its coded sample bytes where it multiplexes
    /// them (an MPEG-2 TS segment).
    pub bytes: u64,
    /// The samples this track contributed to the segment.
    pub samples: u32,
}

/// What a manifest needs to know about one segment, without its samples.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SegmentSummary {
    /// The track whose timeline cut the segment (the anchor track of a
    /// multiplexed segment).
    pub track: TrackId,
    /// 0-based, ever-increasing segment number for this output.
    pub sequence: u64,
    /// Decode time of the segment's first sample, in `timescale` ticks.
    pub start: u64,
    /// Segment duration, in `timescale` ticks.
    pub duration: u64,
    /// Ticks per second of `start` and `duration`.
    pub timescale: u32,
    /// Size of the segment's bytes.
    pub bytes: u64,
    /// The segment starts with a sync sample.
    pub independent: bool,
    /// The segment follows a timeline discontinuity.
    pub discontinuous: bool,
}
/// The identity stage over the mux vocabulary: every input it is fed comes
/// straight back out.
///
/// A driver generic over its transform needs a concrete one where the caller
/// supplied none, which is the only use this has.
#[derive(Debug, Default)]
pub(crate) struct Passthrough {
    ready: VecDeque<MuxInput>,
}

impl Stage for Passthrough {
    type In<'a> = MuxInput;
    type Out = MuxInput;
    type Error = crate::Error;

    fn feed(&mut self, input: MuxInput, _now: Timestamp) -> Result<()> {
        self.ready.push_back(input);
        Ok(())
    }

    fn poll(&mut self) -> Option<MuxInput> {
        self.ready.pop_front()
    }

    fn finish(&mut self) -> Result<()> {
        Ok(())
    }
}

/// A transform plus the one rule every muxer downstream of it already assumes:
/// the track set has settled before the first sample arrives.
///
/// A transform may announce a track later than the demuxer did — a transcode
/// holds its output until its encoder has reported the parameters that name
/// the new codec, which is after it has seen some samples — while a muxer
/// builds its initialisation segment, PMT or file header from the announced
/// track set at its first sample. [`MuxChain`] is the seam between those two
/// facts: it feeds every [`MuxInput`] through the transform and releases the
/// transform's output downstream only once every *expected* track (the tracks
/// the upstream announced) has been announced or removed by the transform.
///
/// It knows nothing about what the transform does. A pass-through transform
/// re-announces every track immediately, so the chain releases everything at
/// once; nothing is special-cased for a codec change.
///
/// The held window holds whole samples, so it is bounded by the caller's
/// [`MuxChain::hold_limit`] and unbounded by default (no library-chosen
/// number). It ends as soon as the last expected track settles; a transform
/// that never announces an expected track leaves the chain holding until
/// [`MuxChain::finish`].
#[non_exhaustive]
pub struct MuxChain<T> {
    transform: T,
    expected: Vec<TrackId>,
    announced: Vec<Option<TrackSpec>>,
    extra: Vec<TrackSpec>,
    settled: usize,
    open: bool,
    /// Track announcements held while the set settles; released first, so a
    /// muxer builds its header from the whole set.
    held_tracks: Vec<MuxInput>,
    held: VecDeque<MuxInput>,
    held_bytes: u64,
    limit: Option<u64>,
    ready: VecDeque<MuxInput>,
    finished: bool,
}

impl<T> MuxChain<T>
where
    T: for<'a> Stage<In<'a> = MuxInput, Out = MuxInput, Error = crate::Error>,
{
    /// Chain `transform`, expecting the upstream's `expected` tracks.
    ///
    /// `expected` is the demuxer's announcement order; [`MuxChain::specs`]
    /// reports the transform's announcements in that order, so an output
    /// derived from it is stable across a pass-through and a transform.
    ///
    /// `transform` is any stage over the mux vocabulary
    /// (`Stage<In = MuxInput, Out = MuxInput, Error = Error>`), taken by value.
    #[must_use]
    pub fn new(transform: T, expected: impl IntoIterator<Item = TrackId>) -> Self {
        let mut expected: Vec<TrackId> = expected.into_iter().collect();
        expected.dedup();
        let announced = expected.iter().map(|_| None).collect();
        let open = expected.is_empty();
        Self {
            transform,
            expected,
            announced,
            extra: Vec::new(),
            settled: 0,
            open,
            held_tracks: Vec::new(),
            held: VecDeque::new(),
            held_bytes: 0,
            limit: None,
            ready: VecDeque::new(),
            finished: false,
        }
    }

    /// Bound the sample bytes held before the track set settles, in whole
    /// samples. Past `max_bytes` [`MuxChain::feed`] fails with
    /// [`crate::Error::TransformHoldExceeded`]. Unset means unbounded.
    #[must_use]
    pub fn hold_limit(mut self, max_bytes: u64) -> Self {
        self.limit = Some(max_bytes);
        self
    }

    /// The announced track set, in the order the upstream announced it, with
    /// any track the transform added appended in announcement order.
    #[must_use]
    pub fn specs(&self) -> Vec<TrackSpec> {
        self.announced
            .iter()
            .flatten()
            .cloned()
            .chain(self.extra.iter().cloned())
            .collect()
    }

    /// Whether the track set has settled and output is released.
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// The transform's current demand.
    #[must_use]
    pub fn demand(&self) -> Demand {
        self.transform.demand()
    }

    /// Feed `input` through the transform and release whatever the track-set
    /// rule allows.
    ///
    /// An input naming a track this chain was not told to expect is dropped
    /// before the transform sees it: the caller named the tracks it keeps, and
    /// a transform has no business transcoding (or a muxer emitting) one the
    /// caller filtered out. A [`MuxInput::Discontinuity`] for no named track
    /// and a [`MuxInput::Flush`] always pass.
    ///
    /// # Errors
    /// The transform's error, or [`crate::Error::TransformHoldExceeded`] past the
    /// caller's cap.
    pub fn feed(&mut self, input: MuxInput) -> Result<()> {
        if !self.keeps(&input) {
            return Ok(());
        }
        self.transform.feed(input, Timestamp::ZERO)?;
        self.drain()
    }

    /// Whether `input` names a track this chain expects, or names none at all.
    fn keeps(&self, input: &MuxInput) -> bool {
        match input {
            MuxInput::Track(spec) => self.expected.contains(&spec.track_id),
            MuxInput::Sample { track, .. } | MuxInput::TrackRemoved { track } => {
                self.expected.contains(track)
            }
            MuxInput::Discontinuity { track } => {
                track.is_none_or(|track| self.expected.contains(&track))
            }
            MuxInput::Flush => true,
        }
    }

    /// Take one released output.
    pub fn poll(&mut self) -> Option<MuxInput> {
        self.ready.pop_front()
    }

    /// Signal end of input: flush the transform, release what it produced, and
    /// release the held window (a transform that never announced an expected
    /// track must not leave the output stalled past end of stream).
    ///
    /// # Errors
    /// The transform's error from [`Stage::finish`].
    pub fn finish(&mut self) -> Result<()> {
        if self.finished {
            return Ok(());
        }
        self.finished = true;
        self.transform.finish()?;
        self.drain()?;
        self.release();
        Ok(())
    }

    /// The track-set rule's settlement bookkeeping for `input`.
    fn record(&mut self, input: &MuxInput) {
        match input {
            MuxInput::Track(spec) => match self.expected.iter().position(|id| *id == spec.track_id)
            {
                Some(index) => {
                    if self.announced[index].is_none() {
                        self.settled += 1;
                    }
                    self.announced[index] = Some(*spec.clone());
                }
                None => match self.extra.iter_mut().find(|s| s.track_id == spec.track_id) {
                    Some(existing) => *existing = *spec.clone(),
                    None => self.extra.push(*spec.clone()),
                },
            },
            MuxInput::TrackRemoved { track } => {
                if let Some(index) = self.expected.iter().position(|id| id == track)
                    && self.announced[index].is_some()
                {
                    self.announced[index] = None;
                    self.settled = self.settled.saturating_sub(1);
                }
                self.extra.retain(|spec| spec.track_id != *track);
            }
            _ => {}
        }
    }

    /// Whether every expected track has been announced.
    fn settled(&self) -> bool {
        self.settled >= self.expected.len()
    }

    /// Collect the transform's queued output and release what the track-set
    /// rule allows.
    fn drain(&mut self) -> Result<()> {
        while let Some(output) = self.transform.poll() {
            self.record(&output);
            if self.open {
                self.ready.push_back(output);
                continue;
            }
            match &output {
                MuxInput::Track(_) => self.held_tracks.push(output),
                MuxInput::Sample { sample, .. } => {
                    self.held_bytes = self.held_bytes.saturating_add(sample.data.len() as u64);
                    if let Some(limit) = self.limit
                        && self.held_bytes > limit
                    {
                        return Err(crate::Error::TransformHoldExceeded { limit });
                    }
                    self.held.push_back(output);
                }
                _ => self.held.push_back(output),
            }
        }
        if !self.open && self.settled() {
            self.release();
        }
        Ok(())
    }

    /// Open the chain: release the held track announcements first (a muxer
    /// builds its init segment, PMT or file header from the whole set, and a
    /// transform may announce one track after another track's samples were
    /// ready), then the held inputs in arrival order.
    fn release(&mut self) {
        self.open = true;
        self.held_bytes = 0;
        self.ready.extend(self.held_tracks.drain(..));
        self.ready.extend(self.held.drain(..));
    }
}
