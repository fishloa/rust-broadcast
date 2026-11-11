//! Per-stream state: the chunk plan, the fetch cursor and the synthesized init.

use std::collections::VecDeque;

use broadcast_common::Unpackage;
use broadcast_smooth::{QualityLevel, SmoothManifest, StreamIndex};
use transmux::smooth_fragment::fragment_tfxd;
use transmux::{Fmp4Demux, Sample, TrackId};

use super::action::RequestId;

pub(crate) type Chunk = (u64, u64);

#[derive(Debug, Clone, Copy)]
pub(crate) enum Cursor {
    Idle,
    InFlight { id: RequestId, chunk: Chunk },
    Backoff { until: u64, chunk: Chunk },
}

pub(crate) struct Stream {
    pub(crate) index: StreamIndex,
    pub(crate) timescale: u64,
    pub(crate) quality: QualityLevel,
    pub(crate) plan: VecDeque<Chunk>,
    pub(crate) known_end: u64,
    pub(crate) cursor: Cursor,
    pub(crate) local_track_id: Option<u32>,
    pub(crate) track: Option<TrackId>,
    pub(crate) init: Option<Vec<u8>>,
    pub(crate) held: Vec<(u64, Vec<u8>)>,
    pub(crate) failures: u32,
    pub(crate) refresh_misses: u32,
    pub(crate) abandoned: bool,
}

impl Stream {
    pub(crate) fn new(index: StreamIndex, timescale: u64, quality: QualityLevel) -> Self {
        Self {
            index,
            timescale,
            quality,
            plan: VecDeque::new(),
            known_end: 0,
            cursor: Cursor::Idle,
            local_track_id: None,
            track: None,
            init: None,
            held: Vec::new(),
            failures: 0,
            refresh_misses: 0,
            abandoned: false,
        }
    }

    pub(crate) fn is_drained(&self) -> bool {
        self.abandoned || (matches!(self.cursor, Cursor::Idle) && self.plan.is_empty())
    }

    pub(crate) fn plan_from(&mut self, chunks: Vec<Chunk>) {
        self.known_end = chunks.last().map_or(0, |&(t, d)| t.saturating_add(d));
        self.plan = chunks.into();
    }

    pub(crate) fn merge_refresh(&mut self, chunks: Vec<Chunk>) -> bool {
        let gap = chunks.first().is_some_and(|&(t, _)| t > self.known_end) && self.known_end != 0;
        let from = if gap { 0 } else { self.known_end };
        for (t, d) in chunks {
            if t >= from {
                self.known_end = t.saturating_add(d).max(self.known_end);
                self.plan.push_back((t, d));
            }
        }
        gap
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

pub(crate) fn demux_fragment(
    init: &[u8],
    local_track_id: u32,
    body: &[u8],
    start: u64,
) -> Result<Vec<Sample>, transmux::Error> {
    let mut data = Vec::with_capacity(init.len() + body.len());
    data.extend_from_slice(init);
    data.extend_from_slice(body);
    let media = Fmp4Demux::new().unpackage(data.as_slice())?;
    let desired = fragment_tfxd(body).map_or(start, |t| t.fragment_absolute_time);
    let mut out = Vec::new();
    for track in media
        .tracks
        .into_iter()
        .filter(|t| t.spec.track_id == TrackId::new(local_track_id))
    {
        let Some(first_dts) = track.samples.first().map(|s| s.dts.unwrap_or_default()) else {
            continue;
        };
        let offset = i64::try_from(desired).unwrap_or(i64::MAX) - first_dts;
        for mut sample in track.samples {
            sample.dts = sample.dts.map(|d| d + offset);
            sample.pts = sample.pts.map(|p| p + offset);
            out.push(sample);
        }
    }
    Ok(out)
}
