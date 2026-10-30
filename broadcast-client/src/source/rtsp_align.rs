//! Places every RTSP track's sample times on one common timeline.
//!
//! Each track's RTP timestamps start at an unrelated random base (RFC 3550 5.1), so the raw
//! sample times of two tracks cannot be compared. The common timeline is the video track's own
//! (its sample times pass through unchanged); every other track gets a per-track tick offset
//! added to its pts and dts.
//!
//! Before Sender Reports for the tracks are known the offset lines the track's first sample up
//! with the video track's first sample ([`AvSync::Provisional`]). Once the depacketiser holds a
//! Sender Report (RFC 3550 6.4.1, NTP time to RTP time) for both tracks the offset comes from
//! `RtpStreamDepacketiser::sync_start_decode_times`, which owns the NTP arithmetic
//! ([`AvSync::SenderReport`]).
//!
//! Switching to the new offset never moves a track's times backwards: a forward step is applied at
//! once, a backward step is slewed by at most half a sample duration per sample. A step larger
//! than [`SR_SWITCH_TOLERANCE_US`] is an honest timeline change and is reported as one
//! [`Event::Discontinuity`].

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use transmux::{RtpStreamDepacketiser, Sample, TrackId};

use crate::events::{AvSync, Event};

/// Largest offset change applied without a [`Event::Discontinuity`]. The provisional offset is
/// wrong by the arrival skew between the first audio and first video sample, which on a healthy
/// stream is a few packet times; 100 ms is about five AAC frames at 44.1 kHz, inside the range where
/// audio/video offset is still read as in sync, so a correction up to that size is absorbed
/// quietly and anything larger is reported to the host.
pub(crate) const SR_SWITCH_TOLERANCE_US: i128 = 100_000;
const MAX_HELD_SAMPLES: usize = 256;
const US_PER_SEC: i128 = 1_000_000;
const SLEW_DIVISOR: i64 = 2;

pub(crate) enum Out {
    Sample(TrackId, Sample),
    Event(Event),
}

struct Lane {
    timescale: u32,
    first_pts: Option<i64>,
    delta: i64,
    target: Option<i64>,
    placed: bool,
}

#[derive(Default)]
pub(crate) struct Aligner {
    active: bool,
    video: Option<TrackId>,
    lanes: BTreeMap<TrackId, Lane>,
    anchor: Option<(TrackId, i64)>,
    state: Option<AvSync>,
    held: Vec<(TrackId, Sample)>,
    sr_dirty: bool,
}

fn rescale(ticks: i64, from: u32, to: u32) -> i64 {
    let num = i128::from(ticks) * i128::from(to);
    let den = i128::from(from.max(1));
    let half = den / 2;
    let rounded = if num >= 0 {
        (num + half) / den
    } else {
        (num - half) / den
    };
    i64::try_from(rounded).unwrap_or(if rounded < 0 { i64::MIN } else { i64::MAX })
}

fn ticks_to_us(ticks: i64, timescale: u32) -> i128 {
    i128::from(ticks) * US_PER_SEC / i128::from(timescale.max(1))
}

impl Aligner {
    pub(crate) fn new(tracks: &[(TrackId, bool, u32)]) -> Self {
        let lanes = tracks
            .iter()
            .map(|&(id, _, timescale)| {
                (
                    id,
                    Lane {
                        timescale,
                        first_pts: None,
                        delta: 0,
                        target: None,
                        placed: false,
                    },
                )
            })
            .collect();
        Self {
            active: tracks.len() > 1,
            video: tracks.iter().find(|t| t.1).map(|t| t.0),
            lanes,
            ..Self::default()
        }
    }

    pub(crate) fn note_rtcp(&mut self) {
        self.sr_dirty = true;
    }

    pub(crate) fn push(
        &mut self,
        depa: &RtpStreamDepacketiser,
        track: TrackId,
        sample: Sample,
    ) -> Vec<Out> {
        let (true, Some(pts)) = (self.active, sample.pts) else {
            return alloc::vec![Out::Sample(track, sample)];
        };
        let mut out = Vec::new();
        if let Some(lane) = self.lanes.get_mut(&track)
            && lane.first_pts.is_none()
        {
            lane.first_pts = Some(pts);
        }
        if self.anchor.is_none() {
            if self.video.is_none_or(|v| v == track) {
                self.anchor = Some((track, pts));
            } else {
                self.held.push((track, sample));
                if self.held.len() > MAX_HELD_SAMPLES {
                    self.anchor_on_earliest_held();
                    self.release_held(depa, &mut out);
                }
                return out;
            }
            self.refresh(depa);
            self.place(track, sample, &mut out);
            self.release_held(depa, &mut out);
            return out;
        }
        self.refresh(depa);
        self.place(track, sample, &mut out);
        out
    }

    fn anchor_on_earliest_held(&mut self) {
        if let Some(&(track, _)) = self.held.first()
            && let Some(first) = self.lanes.get(&track).and_then(|l| l.first_pts)
        {
            self.anchor = Some((track, first));
        }
    }

    fn release_held(&mut self, depa: &RtpStreamDepacketiser, out: &mut Vec<Out>) {
        if self.anchor.is_none() {
            return;
        }
        self.refresh(depa);
        for (track, sample) in core::mem::take(&mut self.held) {
            self.place(track, sample, out);
        }
    }

    fn refresh(&mut self, depa: &RtpStreamDepacketiser) {
        let Some((anchor, anchor_first)) = self.anchor else {
            return;
        };
        if !self.sr_dirty {
            return;
        }
        let times: BTreeMap<TrackId, u64> = depa.sync_start_decode_times().into_iter().collect();
        let Some((&anchor_sdt, anchor_scale)) = times
            .get(&anchor)
            .zip(self.lanes.get(&anchor).map(|l| l.timescale))
        else {
            return;
        };
        let mut complete = true;
        for (id, lane) in &mut self.lanes {
            if *id == anchor {
                continue;
            }
            let (Some(&sdt), Some(first)) = (times.get(id), lane.first_pts) else {
                complete = false;
                continue;
            };
            let anchor_in_lane = rescale(anchor_first, anchor_scale, lane.timescale);
            let anchor_sdt_in_lane = rescale(
                i64::try_from(anchor_sdt).unwrap_or(i64::MAX),
                anchor_scale,
                lane.timescale,
            );
            let sdt = i64::try_from(sdt).unwrap_or(i64::MAX);
            lane.target = Some(
                anchor_in_lane
                    .saturating_add(sdt)
                    .saturating_sub(anchor_sdt_in_lane)
                    .saturating_sub(first),
            );
        }
        self.sr_dirty = !complete;
    }

    fn place(&mut self, track: TrackId, mut sample: Sample, out: &mut Vec<Out>) {
        let Some((anchor, anchor_first)) = self.anchor else {
            return;
        };
        let mut discontinuity = false;
        if track != anchor {
            let anchor_scale = self.lanes.get(&anchor).map_or(1, |l| l.timescale);
            let Some(lane) = self.lanes.get_mut(&track) else {
                out.push(Out::Sample(track, sample));
                return;
            };
            if !lane.placed {
                lane.delta = lane.target.unwrap_or_else(|| {
                    rescale(anchor_first, anchor_scale, lane.timescale)
                        .saturating_sub(lane.first_pts.unwrap_or(0))
                });
                lane.placed = true;
            } else if let Some(target) = lane.target
                && target != lane.delta
            {
                let diff = target.saturating_sub(lane.delta);
                if ticks_to_us(diff, lane.timescale).abs() > SR_SWITCH_TOLERANCE_US {
                    lane.delta = target;
                    discontinuity = true;
                } else if diff > 0 {
                    lane.delta = target;
                } else {
                    let step = i64::from(sample.duration.unwrap_or(0)) / SLEW_DIVISOR;
                    lane.delta = lane.delta.saturating_add(diff.max(-step));
                }
            }
            sample.pts = sample.pts.map(|p| p.saturating_add(lane.delta));
            sample.dts = sample.dts.map(|d| d.saturating_add(lane.delta));
        }
        let state = self.current_state(anchor);
        if discontinuity {
            out.push(Out::Event(Event::Discontinuity));
        }
        if self.state != Some(state) {
            self.state = Some(state);
            out.push(Out::Event(Event::AvSync(state)));
        }
        out.push(Out::Sample(track, sample));
    }

    fn current_state(&self, anchor: TrackId) -> AvSync {
        let all = self
            .lanes
            .iter()
            .filter(|(id, _)| **id != anchor)
            .all(|(_, l)| l.target.is_some());
        if all || self.state == Some(AvSync::SenderReport) {
            AvSync::SenderReport
        } else {
            AvSync::Provisional
        }
    }
}
