//! Per-stream state: the chunk plan, the fetch cursor and the synthesized init.

use std::collections::VecDeque;

use broadcast_core::timescale::Timescale;
use broadcast_smooth::{QualityLevel, SmoothManifest, StreamIndex};
use transmux::smooth_fragment::fragment_tfxd;
use transmux::{Sample, TrackId, TrackSpec};

use super::action::RequestId;
use crate::demux::SegmentDemux;

pub(crate) type Chunk = (u64, u64);

#[derive(Debug, Clone, Copy)]
pub(crate) enum Cursor {
    Idle,
    InFlight { id: RequestId, chunk: Chunk },
    Backoff { until: u64, chunk: Chunk },
}

pub(crate) struct HeldFragment {
    pub(crate) start: u64,
    pub(crate) body: Vec<u8>,
    pub(crate) quality: QualityLevel,
}

pub(crate) struct Stream {
    pub(crate) index: StreamIndex,
    pub(crate) timescale: Timescale,
    pub(crate) quality: QualityLevel,
    pub(crate) fetched: QualityLevel,
    pub(crate) applied: QualityLevel,
    pub(crate) spec: Option<TrackSpec>,
    pub(crate) plan: VecDeque<Chunk>,
    pub(crate) known_end: u64,
    pub(crate) cursor: Cursor,
    pub(crate) local_track_id: Option<TrackId>,
    pub(crate) track: Option<TrackId>,
    pub(crate) demux: Option<SegmentDemux>,
    pub(crate) held: Vec<HeldFragment>,
    pub(crate) failures: u32,
    pub(crate) refresh_misses: u32,
    pub(crate) abandoned: bool,
    /// Damage reports the last fragment's demux published, waiting for the
    /// engine to hand them to the caller's policy.
    pub(crate) damage: Vec<broadcast_core::damage::Damage>,
}

impl Stream {
    pub(crate) fn new(index: StreamIndex, timescale: Timescale, quality: QualityLevel) -> Self {
        Self {
            index,
            timescale,
            fetched: quality.clone(),
            applied: quality.clone(),
            spec: None,
            quality,
            plan: VecDeque::new(),
            known_end: 0,
            cursor: Cursor::Idle,
            local_track_id: None,
            track: None,
            demux: None,
            held: Vec::new(),
            failures: 0,
            refresh_misses: 0,
            abandoned: false,
            damage: Vec::new(),
        }
    }

    pub(crate) fn is_drained(&self) -> bool {
        self.abandoned || (matches!(self.cursor, Cursor::Idle) && self.plan.is_empty())
    }

    pub(crate) fn plan_from(&mut self, chunks: Vec<Chunk>) {
        self.known_end = chunks.last().map_or(0, |&(t, d)| t.saturating_add(d));
        self.plan = chunks.into();
    }

    /// Merge a refreshed manifest's chunks into the plan. Returns true when the
    /// timeline has a gap at the join, or when `max_plan_len` forced the oldest
    /// chunks out of the plan: either way playback skips.
    pub(crate) fn merge_refresh(&mut self, chunks: Vec<Chunk>, max_plan_len: usize) -> bool {
        let gap = chunks.first().is_some_and(|&(t, _)| t > self.known_end) && self.known_end != 0;
        let from = if gap { 0 } else { self.known_end };
        for (t, d) in chunks {
            if t >= from {
                self.known_end = t.saturating_add(d).max(self.known_end);
                self.plan.push_back((t, d));
            }
        }
        if self.plan.len() <= max_plan_len {
            return gap;
        }
        while self.plan.len() > max_plan_len {
            self.plan.pop_front();
        }
        true
    }

    /// The reports the last [`Self::fragment_samples`] call's demux
    /// published, in order.
    pub(crate) fn take_damage(&mut self) -> Vec<broadcast_core::damage::Damage> {
        core::mem::take(&mut self.damage)
    }

    /// Feed one fragment body to this stream's demux stage and return its
    /// samples rebased so the fragment starts at its `tfxd` time, or at `start`
    /// when it carries none.
    pub(crate) fn fragment_samples(
        &mut self,
        body: &[u8],
        start: u64,
    ) -> Result<Vec<Sample>, FragmentFault> {
        let local = self.local_track_id.ok_or(FragmentFault::Demux)?;
        let segment = self
            .demux
            .as_mut()
            .ok_or(FragmentFault::Demux)?
            .segment(body)
            .map_err(|_| FragmentFault::Demux)?;
        self.damage.extend(segment.damage);
        if segment.damaged {
            return Err(FragmentFault::Demux);
        }
        let desired = fragment_tfxd(body).map_or(start, |t| t.fragment_absolute_time);
        let desired = i64::try_from(desired).map_err(|_| FragmentFault::TimeOutOfRange)?;
        let mut out = Vec::new();
        for track in segment
            .tracks
            .into_iter()
            .filter(|t| t.spec.track_id == local)
        {
            let Some(first_dts) = track
                .samples
                .first()
                .and_then(|s| s.timing.and_then(|t| t.dts()))
            else {
                continue;
            };
            let offset = desired
                .checked_sub(first_dts)
                .ok_or(FragmentFault::TimeOutOfRange)?;
            for mut sample in track.samples {
                if let Some(t) = sample.timing.as_mut() {
                    let dts = t.dts();
                    let current = dts.unwrap_or(0);
                    let shifted = shift(dts, offset)?.unwrap_or(current);
                    t.shift(shifted - current);
                }
                out.push(sample);
            }
        }
        Ok(out)
    }
}

pub(crate) fn live_join(
    chunks: Vec<Chunk>,
    manifest: &SmoothManifest,
    look_ahead: usize,
) -> Vec<Chunk> {
    let Some(&(edge_start, _)) = chunks.last() else {
        return chunks;
    };
    let mut join = chunks.len().saturating_sub(1).saturating_sub(look_ahead);
    if let Some(dvr) = manifest.dvr_window_length.filter(|&d| d > 0) {
        let window_start = edge_start.saturating_sub(dvr);
        if let Some(first_in_window) = chunks.iter().position(|c| c.0 >= window_start) {
            join = join.max(first_in_window);
        }
    }
    let join_start = chunks[join].0;
    let first = chunks
        .iter()
        .position(|c| c.0 >= join_start)
        .unwrap_or_default();
    chunks[first..].to_vec()
}

#[derive(Debug)]
pub(crate) enum FragmentFault {
    Demux,
    TimeOutOfRange,
}

fn shift(time: Option<i64>, offset: i64) -> Result<Option<i64>, FragmentFault> {
    time.map(|t| t.checked_add(offset)).map_or(Ok(None), |t| {
        t.map(Some).ok_or(FragmentFault::TimeOutOfRange)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest() -> SmoothManifest {
        SmoothManifest::parse(&String::from_utf8_lossy(&test_fixtures::read(
            "smooth/av/Manifest",
        )))
        .expect("the committed manifest parses")
    }

    fn stream() -> Stream {
        let manifest = manifest();
        let index = manifest.streams[0].clone();
        let quality = index.qualities[0].clone();
        Stream::new(index, manifest.timescale, quality)
    }

    #[test]
    fn a_refresh_continuing_at_the_known_end_is_not_a_gap() {
        let mut stream = stream();
        stream.plan_from(vec![(0, 100), (100, 100)]);
        assert!(!stream.merge_refresh(vec![(200, 100)], 10));
        assert_eq!(stream.plan.len(), 3);
        assert_eq!(stream.known_end, 300);
    }

    #[test]
    fn a_refresh_starting_past_the_known_end_is_a_gap() {
        let mut stream = stream();
        stream.plan_from(vec![(0, 100), (100, 100)]);
        assert!(stream.merge_refresh(vec![(250, 100)], 10));
        assert_eq!(stream.plan.len(), 3);
    }

    #[test]
    fn the_first_refresh_is_never_a_gap() {
        let mut stream = stream();
        assert_eq!(stream.known_end, 0);
        assert!(!stream.merge_refresh(vec![(5, 100)], 10));
        assert_eq!(stream.plan.len(), 1);
    }

    #[test]
    fn an_overlapping_refresh_adds_no_duplicates() {
        let mut stream = stream();
        stream.plan_from(vec![(0, 100), (100, 100)]);
        assert!(!stream.merge_refresh(vec![(150, 100)], 10));
        assert_eq!(stream.plan.len(), 2);
    }

    #[test]
    fn a_plan_cap_forced_skip_is_a_gap() {
        let mut stream = stream();
        stream.plan_from(vec![(0, 100)]);
        assert!(stream.merge_refresh(vec![(100, 100), (200, 100)], 2));
        assert_eq!(stream.plan.len(), 2);
    }

    #[test]
    fn a_zero_dvr_window_keeps_the_whole_timeline() {
        let mut manifest = manifest();
        manifest.dvr_window_length = Some(0);
        let chunks = vec![(0, 100), (100, 100), (200, 100)];
        assert_eq!(live_join(chunks.clone(), &manifest, 10), chunks);
    }
}
