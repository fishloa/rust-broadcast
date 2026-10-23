//! Buffer-event feeder: owns the buffering controller and feeding state.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::vec::Vec;

use buffering_runtime::{
    BufferController, ControllerConfig, Event as RtEvent, Hint, StallMemorySnapshot,
};
use transmux::{JoinDecision, LiveJoin, Sample, TrackId, TrackSpec};

use crate::events::{BufferEvent, Event};

fn pts_to_27mhz(pts: i64, timescale: u32) -> u64 {
    if timescale == 0 {
        return 0;
    }
    let v = i128::from(pts) * 27_000_000i128 / i128::from(timescale);
    if v < 0 { 0 } else { v as u64 }
}

pub(crate) struct Feeder {
    config: ControllerConfig,
    seed: Option<StallMemorySnapshot>,
    controller: Option<BufferController>,
    finite: bool,
    rendered: BTreeSet<TrackId>,
    rendered_explicit: bool,
    live_joins: BTreeMap<TrackId, LiveJoin>,
    track_specs: BTreeMap<TrackId, TrackSpec>,
    pending_ends: BTreeMap<TrackId, u64>,
    pending_zero: BTreeMap<TrackId, (i64, u32)>,
    video_joined: BTreeSet<TrackId>,
}

impl Feeder {
    pub(crate) fn new(config: ControllerConfig, seed: Option<StallMemorySnapshot>) -> Self {
        Self {
            config,
            seed,
            controller: None,
            finite: false,
            rendered: BTreeSet::new(),
            rendered_explicit: false,
            live_joins: BTreeMap::new(),
            track_specs: BTreeMap::new(),
            pending_ends: BTreeMap::new(),
            pending_zero: BTreeMap::new(),
            video_joined: BTreeSet::new(),
        }
    }

    pub(crate) fn set_finite(&mut self) {
        self.finite = true;
        self.controller = None;
    }

    pub(crate) fn on_track_added(&mut self, spec: &TrackSpec) {
        self.track_specs.insert(spec.track_id, spec.clone());
        if let Some(join) = LiveJoin::for_config(&spec.config) {
            self.live_joins.insert(spec.track_id, join);
        }
        if !self.rendered_explicit {
            let is_video = self.live_joins.contains_key(&spec.track_id);
            let has_video = self
                .rendered
                .iter()
                .any(|t| self.live_joins.contains_key(t));
            let has_audio = self
                .rendered
                .iter()
                .any(|t| !self.live_joins.contains_key(t));
            if (is_video && !has_video) || (!is_video && !has_audio) {
                self.rendered.insert(spec.track_id);
            }
        }
    }

    pub(crate) fn set_rendered_tracks(&mut self, tracks: &[TrackId]) {
        self.rendered_explicit = true;
        let new_set: BTreeSet<TrackId> = tracks.iter().copied().collect();
        let removed: Vec<TrackId> = self.rendered.difference(&new_set).copied().collect();
        for t in removed {
            self.rendered.remove(&t);
            if let Some(c) = &mut self.controller {
                c.unregister_track(t.value());
            }
        }
        let added: Vec<TrackId> = new_set.difference(&self.rendered).copied().collect();
        for t in added {
            self.rendered.insert(t);
            if let Some(c) = &mut self.controller {
                c.register_track(t.value());
            }
        }
        self.rendered = new_set;
    }

    pub(crate) fn join_sample(&mut self, track: TrackId, sample: &Sample) -> Option<JoinDecision> {
        self.live_joins.get_mut(&track).map(|j| j.push(sample))
    }

    pub(crate) fn on_sample(
        &mut self,
        track: TrackId,
        sample: &Sample,
        join: Option<JoinDecision>,
    ) {
        if !self.rendered.contains(&track) {
            return;
        }
        let is_video = self.live_joins.contains_key(&track);
        let has_rendered_video = self
            .rendered
            .iter()
            .any(|t| self.live_joins.contains_key(t));
        let video_joined_exists = self.video_joined.iter().any(|t| self.rendered.contains(t));

        if !is_video && has_rendered_video && !video_joined_exists {
            return;
        }

        let spec = match self.track_specs.get(&track) {
            Some(s) => s,
            None => return,
        };
        let timescale = spec.timescale;
        let Some(pts) = sample.pts else { return };

        if is_video {
            match join {
                Some(JoinDecision::JoinPoint(_)) if !self.video_joined.contains(&track) => {
                    self.video_joined.insert(track);
                    self.pending_zero.insert(track, (pts, timescale));
                }
                Some(JoinDecision::JoinPoint(_)) => {}
                Some(JoinDecision::Drop(_)) => {}
                Some(JoinDecision::Keep) if self.video_joined.contains(&track) => {
                    let dur = i64::from(sample.duration.unwrap_or(0));
                    self.pending_ends
                        .insert(track, pts_to_27mhz(pts + dur, timescale));
                }
                Some(JoinDecision::Keep) => {}
                None => {}
                Some(_) => {}
            }
        } else {
            let dur = i64::from(sample.duration.unwrap_or(0));
            self.pending_ends
                .insert(track, pts_to_27mhz(pts + dur, timescale));
        }
    }

    pub(crate) fn ensure_controller(&mut self) {
        if self.finite {
            return;
        }
        if self.controller.is_none()
            && let Ok(mut c) = BufferController::new(self.config, self.seed.take())
        {
            for t in &self.rendered {
                c.register_track(t.value());
            }
            self.controller = Some(c);
        }
    }

    pub(crate) fn on_chunk(&mut self, now_us: u64, bytes: usize, pcr: Option<u64>) {
        if self.finite {
            self.pending_ends.clear();
            self.pending_zero.clear();
            return;
        }
        self.ensure_controller();
        let Some(c) = &mut self.controller else {
            return;
        };
        c.on_chunk(now_us, bytes as u32, pcr);

        // zero reports first
        let zero_keys: Vec<TrackId> = self.pending_zero.keys().copied().collect();
        for t in zero_keys {
            if let Some((pts, ts)) = self.pending_zero.remove(&t)
                && let Some(c) = &mut self.controller
            {
                c.on_track_media(now_us, t.value(), pts_to_27mhz(pts, ts));
            }
        }
        // pending ends
        let pending: Vec<(TrackId, u64)> =
            self.pending_ends.iter().map(|(k, v)| (*k, *v)).collect();
        self.pending_ends.clear();
        for (t, end) in pending {
            if let Some(c) = &mut self.controller {
                c.on_track_media(now_us, t.value(), end);
            }
        }
    }

    pub(crate) fn on_demux_discontinuity(&mut self, now_us: u64) {
        self.video_joined.clear();
        self.pending_ends.clear();
        self.pending_zero.clear();
        for j in self.live_joins.values_mut() {
            j.rejoin();
        }
        if self.finite {
            return;
        }
        self.ensure_controller();
        if let Some(c) = &mut self.controller {
            c.on_hint(now_us, Hint::PcrDiscontinuity);
        }
    }

    pub(crate) fn hint(&mut self, now_us: u64, h: Hint) {
        if self.finite {
            return;
        }
        self.ensure_controller();
        if let Some(c) = &mut self.controller {
            c.on_hint(now_us, h);
        }
    }

    pub(crate) fn on_playback_position(&mut self, now_us: u64, pts: Option<u64>) {
        if self.finite {
            return;
        }
        if let Some(c) = &mut self.controller {
            c.on_tick(now_us, pts);
        }
    }

    pub(crate) fn drain(&mut self) -> Vec<Event> {
        let mut out = Vec::new();
        if let Some(c) = &mut self.controller {
            while let Some(ev) = c.poll_event() {
                match ev {
                    RtEvent::StartPlaying { media_27mhz, .. } => {
                        out.push(Event::Buffer(BufferEvent::Start {
                            at_27mhz: media_27mhz,
                        }));
                    }
                    RtEvent::Pause {
                        hold_at_27mhz,
                        track,
                        ..
                    } => {
                        out.push(Event::Buffer(BufferEvent::Pause {
                            hold_at_27mhz,
                            starving_track: track,
                        }));
                    }
                    RtEvent::Resume { media_27mhz, .. } => {
                        out.push(Event::Buffer(BufferEvent::Resume {
                            at_27mhz: media_27mhz,
                        }));
                    }
                    _ => {}
                }
            }
        }
        out
    }

    pub(crate) fn stall_memory(&self) -> StallMemorySnapshot {
        self.controller
            .as_ref()
            .map(|c| c.snapshot_memory())
            .unwrap_or_else(|| self.seed.unwrap_or_default())
    }
}
