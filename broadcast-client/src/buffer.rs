//! Buffer-event feeder: owns the buffering controller and feeding state.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::vec::Vec;

use buffering_runtime::{
    BufferController, ControllerConfig, Event as RtEvent, Hint, StallMemorySnapshot,
};
use transmux::{JoinDecision, LiveJoin, Sample, TrackId, TrackSpec};

use crate::events::{BufferEvent, Event};

/// One input the session fed to the buffering controller.
#[cfg(feature = "test-seams")]
#[doc(hidden)]
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BufferInput {
    /// `on_chunk`.
    Chunk {
        /// Arrival time.
        now_us: u64,
        /// Chunk length.
        bytes: u32,
        /// Last PCR of the chunk.
        pcr: Option<u64>,
    },
    /// `on_track_media`.
    TrackMedia {
        /// Report time.
        now_us: u64,
        /// Controller track key.
        track: u32,
        /// Media end, 27 MHz.
        end: u64,
    },
    /// `on_tick`.
    Tick {
        /// Tick time.
        now_us: u64,
        /// Playback position, 27 MHz.
        pts: Option<u64>,
    },
    /// `on_hint`.
    Hint {
        /// Hint time.
        now_us: u64,
        /// The hint.
        hint: Hint,
    },
    /// `register_track`.
    Register(u32),
    /// `unregister_track`.
    Unregister(u32),
}

#[cfg(feature = "test-seams")]
impl BufferInput {
    /// Spec label.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Chunk { .. } => "chunk",
            Self::TrackMedia { .. } => "track media",
            Self::Tick { .. } => "tick",
            Self::Hint { .. } => "hint",
            Self::Register(_) => "register",
            Self::Unregister(_) => "unregister",
        }
    }
}

#[cfg(feature = "test-seams")]
broadcast_common::impl_spec_display!(BufferInput);

macro_rules! rec {
    ($s:expr, $e:expr) => {
        #[cfg(feature = "test-seams")]
        {
            $s.log.push($e);
        }
    };
}

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
    video_max_pts: BTreeMap<TrackId, i64>,
    audio_max_end: BTreeMap<TrackId, u64>,
    video_join_pts_27: Option<u64>,
    #[cfg(feature = "test-seams")]
    pub(crate) log: Vec<BufferInput>,
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
            video_max_pts: BTreeMap::new(),
            audio_max_end: BTreeMap::new(),
            video_join_pts_27: None,
            #[cfg(feature = "test-seams")]
            log: Vec::new(),
        }
    }

    pub(crate) fn set_finite(&mut self) {
        self.finite = true;
        self.controller = None;
    }

    pub(crate) fn on_track_added(&mut self, spec: &TrackSpec) {
        self.track_specs.insert(spec.track_id, spec.clone());
        if let Some(join) = LiveJoin::for_config(&spec.config) {
            self.live_joins.entry(spec.track_id).or_insert(join);
        }
        if self.rendered_explicit || self.rendered.contains(&spec.track_id) {
            return;
        }
        let is_video = self.live_joins.contains_key(&spec.track_id);
        let is_audio = spec.config.audio_sample_rate().is_some();
        let has_kind = |is_v: bool| {
            self.rendered.iter().any(|t| {
                self.live_joins.contains_key(t) == is_v
                    && (is_v
                        || self
                            .track_specs
                            .get(t)
                            .is_some_and(|s| s.config.audio_sample_rate().is_some()))
            })
        };
        if (is_video && !has_kind(true)) || (is_audio && !has_kind(false)) {
            self.rendered.insert(spec.track_id);
            if let Some(c) = &mut self.controller {
                c.register_track(spec.track_id.value());
                rec!(self, BufferInput::Register(spec.track_id.value()));
            }
        }
    }

    pub(crate) fn set_rendered_tracks(&mut self, tracks: &[TrackId]) {
        self.rendered_explicit = true;
        let new_set: BTreeSet<TrackId> = tracks.iter().copied().collect();
        let removed: Vec<TrackId> = self.rendered.difference(&new_set).copied().collect();
        for t in removed {
            self.rendered.remove(&t);
            self.pending_ends.remove(&t);
            self.pending_zero.remove(&t);
            self.video_max_pts.remove(&t);
            self.audio_max_end.remove(&t);
            self.video_joined.remove(&t);
            if let Some(c) = &mut self.controller {
                c.unregister_track(t.value());
                rec!(self, BufferInput::Unregister(t.value()));
            }
        }
        let added: Vec<TrackId> = new_set.difference(&self.rendered).copied().collect();
        for t in added {
            self.rendered.insert(t);
            if let Some(c) = &mut self.controller {
                c.register_track(t.value());
                rec!(self, BufferInput::Register(t.value()));
            }
        }
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
        let Some(spec) = self.track_specs.get(&track) else {
            return;
        };
        let timescale = spec.timescale;
        let Some(pts) = sample.pts else { return };
        let dur = i64::from(sample.duration.unwrap_or(0));

        if self.live_joins.contains_key(&track) {
            match join {
                Some(JoinDecision::JoinPoint(_)) if !self.video_joined.contains(&track) => {
                    self.video_joined.insert(track);
                    self.pending_zero.insert(track, (pts, timescale));
                    self.video_max_pts.insert(track, pts);
                    if self.video_join_pts_27.is_none() {
                        self.video_join_pts_27 = Some(pts_to_27mhz(pts, timescale));
                    }
                    self.pending_ends
                        .insert(track, pts_to_27mhz(pts + dur, timescale));
                }
                Some(JoinDecision::Keep) if self.video_joined.contains(&track) => {
                    let max_pts = self
                        .video_max_pts
                        .get(&track)
                        .copied()
                        .map_or(pts, |m| m.max(pts));
                    self.video_max_pts.insert(track, max_pts);
                    self.pending_ends
                        .insert(track, pts_to_27mhz(max_pts + dur, timescale));
                }
                _ => {}
            }
            return;
        }

        let has_rendered_video = self
            .rendered
            .iter()
            .any(|t| self.live_joins.contains_key(t));
        let join_pts = match (has_rendered_video, self.video_join_pts_27) {
            (false, _) => None,
            (true, None) => return,
            (true, Some(j)) => Some(j),
        };
        let end = pts_to_27mhz(pts + dur, timescale);
        let max_end = self
            .audio_max_end
            .get(&track)
            .copied()
            .map_or(end, |m| m.max(end));
        self.audio_max_end.insert(track, max_end);
        if join_pts.is_none_or(|j| max_end > j) {
            self.pending_ends.insert(track, max_end);
        }
    }

    pub(crate) fn rejoin(&mut self) {
        self.video_joined.clear();
        self.pending_ends.clear();
        self.pending_zero.clear();
        self.video_max_pts.clear();
        self.audio_max_end.clear();
        self.video_join_pts_27 = None;
        for j in self.live_joins.values_mut() {
            j.rejoin();
        }
    }

    fn ensure_controller(&mut self) {
        if self.finite {
            return;
        }
        if self.controller.is_none()
            && let Ok(mut c) = BufferController::new(self.config, self.seed.take())
        {
            for t in &self.rendered {
                c.register_track(t.value());
                rec!(self, BufferInput::Register(t.value()));
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
        let len = u32::try_from(bytes).unwrap_or(u32::MAX);
        c.on_chunk(now_us, len, pcr);
        rec!(
            self,
            BufferInput::Chunk {
                now_us,
                bytes: len,
                pcr
            }
        );

        let zeros = core::mem::take(&mut self.pending_zero);
        for (t, (pts, ts)) in zeros {
            let end = pts_to_27mhz(pts, ts);
            if let Some(c) = &mut self.controller {
                c.on_track_media(now_us, t.value(), end);
                rec!(
                    self,
                    BufferInput::TrackMedia {
                        now_us,
                        track: t.value(),
                        end
                    }
                );
            }
        }
        let ends = core::mem::take(&mut self.pending_ends);
        for (t, end) in ends {
            if let Some(c) = &mut self.controller {
                c.on_track_media(now_us, t.value(), end);
                rec!(
                    self,
                    BufferInput::TrackMedia {
                        now_us,
                        track: t.value(),
                        end
                    }
                );
            }
        }
    }

    pub(crate) fn on_demux_discontinuity(&mut self, now_us: u64) {
        self.rejoin();
        self.hint(now_us, Hint::PcrDiscontinuity);
    }

    pub(crate) fn hint(&mut self, now_us: u64, h: Hint) {
        if self.finite {
            return;
        }
        self.ensure_controller();
        if let Some(c) = &mut self.controller {
            c.on_hint(now_us, h);
            rec!(self, BufferInput::Hint { now_us, hint: h });
        }
    }

    pub(crate) fn on_playback_position(&mut self, now_us: u64, pts: Option<u64>) {
        if self.finite {
            return;
        }
        if let Some(c) = &mut self.controller {
            c.on_tick(now_us, pts);
            rec!(self, BufferInput::Tick { now_us, pts });
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
