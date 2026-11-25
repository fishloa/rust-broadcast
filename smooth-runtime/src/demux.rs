//! One streaming MP4 demux stage per Smooth stream: the init segment is fed
//! once and every media segment after it, never concatenated with it.
//!
//! The stage is [`Mp4Demux`] in sequential mode (streaming-first design §4.2,
//! §4.4): it keeps the `moov` it parsed and reads each media segment in turn,
//! closed by [`Mp4Input::SegmentEnd`], so a damaged segment costs only itself.
//! This module folds its [`DemuxEvent`]s into the tracks and samples the
//! client and the origin already shaped their outputs from.

use broadcast_common::stage::{Stage, Timestamp};
use transmux::mp4_demux::{Mp4Demux, Mp4DemuxConfig, Mp4Input};
use transmux::{DemuxEvent, InputDegradation, Sample, Track, TrackId, TrackSpec};

/// A stream's demux stage, with the track set it has resolved so far.
pub(crate) struct SegmentDemux {
    init: Vec<u8>,
    stage: Mp4Demux,
    specs: Vec<(TrackId, TrackSpec)>,
    poisoned: Option<transmux::Error>,
}

/// The tracks and samples one fed segment produced.
pub(crate) struct DemuxedSegment {
    pub(crate) tracks: Vec<Track>,
    pub(crate) damaged: bool,
}

impl SegmentDemux {
    /// A stage fed `init` once, as one unit. Every later [`Self::segment`]
    /// call feeds one media segment under that init.
    pub(crate) fn new(init: &[u8]) -> Result<Self, transmux::Error> {
        Ok(Self {
            init: init.to_vec(),
            stage: Self::open(init)?,
            specs: Vec::new(),
            poisoned: None,
        })
    }

    /// Feed one media segment and drain what it produced.
    ///
    /// A damaged segment, or one the stage refuses, costs only itself: the
    /// stage is rebuilt from the same init, so the next segment demuxes
    /// normally.
    pub(crate) fn segment(&mut self, body: &[u8]) -> Result<DemuxedSegment, transmux::Error> {
        if let Some(error) = &self.poisoned {
            return Err(error.clone());
        }
        let fed = self
            .stage
            .feed(Mp4Input::Next(body), Timestamp::ZERO)
            .and_then(|()| self.stage.feed(Mp4Input::SegmentEnd, Timestamp::ZERO));
        match fed {
            Ok(()) => {
                let segment = self.collect();
                if segment.damaged {
                    self.restart();
                }
                Ok(segment)
            }
            Err(error) => {
                self.restart();
                Err(error)
            }
        }
    }

    fn open(init: &[u8]) -> Result<Mp4Demux, transmux::Error> {
        let mut stage = Mp4Demux::sequential(Mp4DemuxConfig::default());
        stage.feed(Mp4Input::Next(init), Timestamp::ZERO)?;
        stage.feed(Mp4Input::SegmentEnd, Timestamp::ZERO)?;
        while stage.poll().is_some() {}
        Ok(stage)
    }

    fn restart(&mut self) {
        match Self::open(&self.init) {
            Ok(stage) => {
                self.stage = stage;
                self.specs.clear();
            }
            Err(error) => self.poisoned = Some(error),
        }
    }

    fn collect(&mut self) -> DemuxedSegment {
        let mut samples: Vec<(TrackId, Vec<Sample>)> = Vec::new();
        let mut damaged = false;
        while let Some(event) = self.stage.poll() {
            match event {
                DemuxEvent::TrackAdded(spec) => match self.position(spec.track_id) {
                    Some(at) => self.specs[at].1 = spec,
                    None => self.specs.push((spec.track_id, spec)),
                },
                DemuxEvent::TrackUpdated(spec) => {
                    if let Some(at) = self.position(spec.track_id) {
                        self.specs[at].1 = spec;
                    }
                }
                DemuxEvent::Sample {
                    track_id, sample, ..
                } => match samples.iter_mut().find(|(id, _)| *id == track_id) {
                    Some((_, track_samples)) => track_samples.push(sample),
                    None => samples.push((track_id, vec![sample])),
                },
                DemuxEvent::InputDegraded {
                    kind:
                        InputDegradation::MalformedUnit { .. } | InputDegradation::TruncatedUnit { .. },
                    ..
                } => damaged = true,
                _ => {}
            }
        }
        let tracks = self
            .specs
            .iter()
            .map(|(id, spec)| {
                let taken = samples
                    .iter()
                    .position(|(sample_id, _)| sample_id == id)
                    .map(|at| samples.swap_remove(at).1)
                    .unwrap_or_default();
                Track::new(spec.clone(), taken)
            })
            .collect();
        DemuxedSegment { tracks, damaged }
    }

    fn position(&self, id: TrackId) -> Option<usize> {
        self.specs.iter().position(|(spec_id, _)| *spec_id == id)
    }
}
