//! Buffer-event feeder: owns the buffering controller and feeding state.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::vec::Vec;

use buffering_runtime::{
    BufferController, ControllerConfig, Event as RtEvent, Hint, StallMemorySnapshot,
};
use transmux::{JoinDecision, Sample, TrackId, TrackSpec};

use crate::events::{BufferEvent, ErrorKind, Event, Status, StreamKind};
use crate::join::VideoJoin;

const SILENCE_MARGIN_US: u64 = 500_000;

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

const TICKS_PER_SEC_27MHZ: u64 = 27_000_000;

/// Controller key of the placeholder for a declared stream that has not resolved yet; a real
/// `TrackId` never takes it.
const PENDING_TRACK_KEY: u32 = u32::MAX;

pub(crate) fn raw_27mhz(pts: i64, timescale: u32) -> i128 {
    if timescale == 0 {
        return 0;
    }
    i128::from(pts) * i128::from(TICKS_PER_SEC_27MHZ) / i128::from(timescale)
}

pub(crate) fn negative_origin(raw: i128) -> i128 {
    raw.min(0)
}

pub(crate) fn pts_to_27mhz_from(pts: i64, timescale: u32, origin: i128) -> u64 {
    let v = raw_27mhz(pts, timescale).saturating_sub(origin);
    u64::try_from(v).unwrap_or(if v < 0 { 0 } else { u64::MAX })
}

#[cfg(test)]
pub(crate) fn pts_to_27mhz(pts: i64, timescale: u32) -> u64 {
    pts_to_27mhz_from(pts, timescale, 0)
}

pub(crate) fn lowest_id_per_kind(
    tracks: impl IntoIterator<Item = (TrackId, bool, bool)>,
) -> (Option<TrackId>, Option<TrackId>) {
    let mut video: Option<TrackId> = None;
    let mut audio: Option<TrackId> = None;
    for (id, is_video, is_audio) in tracks {
        let slot = if is_video {
            &mut video
        } else if is_audio {
            &mut audio
        } else {
            continue;
        };
        if slot.is_none_or(|cur| id < cur) {
            *slot = Some(id);
        }
    }
    (video, audio)
}

pub(crate) struct Feeder {
    config: ControllerConfig,
    seed: Option<StallMemorySnapshot>,
    controller: Option<BufferController>,
    finite: bool,
    rendered: BTreeSet<TrackId>,
    rendered_explicit: bool,
    live_joins: BTreeMap<TrackId, VideoJoin>,
    track_specs: BTreeMap<TrackId, TrackSpec>,
    pending_ends: BTreeMap<TrackId, u64>,
    pending_zero: BTreeMap<TrackId, (i64, u32)>,
    video_joined: BTreeSet<TrackId>,
    video_join_at: BTreeMap<TrackId, (i64, u32)>,
    video_max_pts: BTreeMap<TrackId, i64>,
    video_end: BTreeMap<TrackId, i64>,
    audio_max_end: BTreeMap<TrackId, i64>,
    first_seen: BTreeMap<TrackId, i64>,
    independent_clocks: bool,
    video_join_pts_27: Option<u64>,
    origin: Option<i128>,
    last_chunk_us: Option<u64>,
    expecting: bool,
    pending_registered: bool,
    started: bool,
    first_media_us: Option<u64>,
    start_pending_track_wait_us: u64,
    start_pending_track_hard_cap_us: u64,
    pending_events: Vec<Event>,
    cap_notice_emitted: bool,
    holding: bool,
    audio_pending: bool,
    #[cfg(feature = "test-seams")]
    pub(crate) log: Vec<BufferInput>,
}

impl Feeder {
    pub(crate) fn new(
        config: ControllerConfig,
        seed: Option<StallMemorySnapshot>,
        start_pending_track_wait_us: u64,
        start_pending_track_hard_cap_us: u64,
    ) -> Self {
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
            video_join_at: BTreeMap::new(),
            video_max_pts: BTreeMap::new(),
            video_end: BTreeMap::new(),
            audio_max_end: BTreeMap::new(),
            first_seen: BTreeMap::new(),
            independent_clocks: false,
            video_join_pts_27: None,
            origin: None,
            last_chunk_us: None,
            expecting: false,
            pending_registered: false,
            started: false,
            first_media_us: None,
            start_pending_track_wait_us,
            start_pending_track_hard_cap_us,
            pending_events: Vec::new(),
            cap_notice_emitted: false,
            holding: false,
            audio_pending: false,
            #[cfg(feature = "test-seams")]
            log: Vec::new(),
        }
    }

    pub(crate) fn set_independent_clocks(&mut self) {
        self.independent_clocks = true;
    }

    pub(crate) fn set_finite(&mut self) {
        self.finite = true;
        self.controller = None;
        self.pending_registered = false;
    }

    pub(crate) fn has_audio(&self) -> bool {
        self.track_specs
            .values()
            .any(|s| s.config.audio_sample_rate().is_some())
    }

    pub(crate) fn has_video(&self) -> bool {
        self.track_specs
            .keys()
            .any(|t| self.live_joins.contains_key(t))
    }

    pub(crate) fn rendered_explicit(&self) -> bool {
        self.rendered_explicit
    }

    /// A stream the source knows exists (PMT-declared) has no track yet: until one appears, or
    /// [`Config::start_pending_track_wait_us`] after the first media, or the first `Start`, the
    /// controller waits for it like for any registered track that has not delivered. The
    /// placeholder is registered with the first media report, so a stream whose tracks all resolve
    /// before any media never shows it to the controller.
    pub(crate) fn set_expecting(&mut self, now_us: u64, expecting: bool, audio_pending: bool) {
        self.expecting = expecting;
        self.audio_pending = audio_pending;
        self.sync_pending(now_us);
    }

    fn effective_wait_us(&self) -> u64 {
        if self.audio_pending {
            self.start_pending_track_wait_us
                .max(self.start_pending_track_hard_cap_us)
        } else {
            self.start_pending_track_wait_us
        }
    }

    fn sync_pending(&mut self, now_us: u64) {
        let effective = self.effective_wait_us();
        let waited = self.first_media_us.map(|f| now_us.saturating_sub(f));
        let want = self.expecting && !self.started && waited.is_some_and(|w| w < effective);
        if want && !self.holding {
            self.pending_events
                .push(Event::Status(Status::AwaitingTracks));
        }
        if !want
            && self.holding
            && self.expecting
            && self.audio_pending
            && !self.started
            && !self.cap_notice_emitted
        {
            self.cap_notice_emitted = true;
            self.pending_events.push(Event::Error {
                kind: ErrorKind::PendingTrackUnresolved {
                    kind: StreamKind::Audio,
                },
                fatal: false,
            });
        }
        self.holding = want;
        let Some(c) = &mut self.controller else {
            return;
        };
        if want && !self.pending_registered {
            c.register_track(PENDING_TRACK_KEY);
            rec!(self, BufferInput::Register(PENDING_TRACK_KEY));
        } else if !want && self.pending_registered {
            c.unregister_track(PENDING_TRACK_KEY);
            rec!(self, BufferInput::Unregister(PENDING_TRACK_KEY));
        }
        self.pending_registered = want;
    }

    #[cfg(test)]
    pub(crate) fn has_track(&self, t: TrackId) -> bool {
        self.track_specs.contains_key(&t)
    }

    pub(crate) fn default_rendered(&self) -> (Option<TrackId>, Option<TrackId>) {
        lowest_id_per_kind(self.track_specs.iter().map(|(id, spec)| {
            (
                *id,
                self.live_joins.contains_key(id),
                spec.config.audio_sample_rate().is_some(),
            )
        }))
    }

    pub(crate) fn new_connection(&mut self) {
        self.finite = false;
    }

    pub(crate) fn retain_tracks(&mut self, specs: &[TrackSpec]) {
        let present: BTreeSet<TrackId> = specs.iter().map(|s| s.track_id).collect();
        let gone: Vec<TrackId> = self
            .track_specs
            .keys()
            .filter(|t| !present.contains(t))
            .copied()
            .collect();
        for t in gone {
            self.remove_track(t);
        }
    }

    fn remove_track(&mut self, t: TrackId) {
        self.track_specs.remove(&t);
        self.live_joins.remove(&t);
        self.forget_progress(t);
        if self.rendered.remove(&t)
            && let Some(c) = &mut self.controller
        {
            c.unregister_track(t.value());
            rec!(self, BufferInput::Unregister(t.value()));
        }
    }

    fn forget_progress(&mut self, t: TrackId) {
        self.pending_ends.remove(&t);
        self.pending_zero.remove(&t);
        self.video_joined.remove(&t);
        self.video_join_at.remove(&t);
        self.video_max_pts.remove(&t);
        self.video_end.remove(&t);
        self.audio_max_end.remove(&t);
        self.first_seen.remove(&t);
    }

    pub(crate) fn on_track_added(&mut self, spec: &TrackSpec) {
        self.track_specs.insert(spec.track_id, spec.clone());
        if let Some(join) = VideoJoin::for_config(&spec.config) {
            self.live_joins.entry(spec.track_id).or_insert(join);
        }
        self.retarget_default();
    }

    fn retarget_default(&mut self) {
        if self.rendered_explicit {
            return;
        }
        let (video, audio) = self.default_rendered();
        let wanted: BTreeSet<TrackId> = video.into_iter().chain(audio).collect();
        self.apply_rendered(wanted);
    }

    pub(crate) fn set_rendered(&mut self, tracks: Option<&[TrackId]>) {
        self.rendered_explicit = tracks.is_some();
        match tracks {
            Some(t) => self.apply_rendered(t.iter().copied().collect()),
            None => self.retarget_default(),
        }
    }

    fn apply_rendered(&mut self, wanted: BTreeSet<TrackId>) {
        let removed: Vec<TrackId> = self.rendered.difference(&wanted).copied().collect();
        for t in removed {
            self.rendered.remove(&t);
            self.pending_ends.remove(&t);
            self.pending_zero.remove(&t);
            if let Some(c) = &mut self.controller {
                c.unregister_track(t.value());
                rec!(self, BufferInput::Unregister(t.value()));
            }
        }
        if !self
            .rendered
            .iter()
            .any(|t| self.live_joins.contains_key(t))
        {
            self.video_join_pts_27 = None;
        }
        let added: Vec<TrackId> = wanted.difference(&self.rendered).copied().collect();
        for t in &added {
            self.rendered.insert(*t);
            if let Some(c) = &mut self.controller {
                c.register_track(t.value());
                rec!(self, BufferInput::Register(t.value()));
            }
            self.seed_from_history(*t);
        }
        if !added.is_empty()
            && let Some(now_us) = self.last_chunk_us
        {
            self.flush_pending(now_us);
        }
    }

    fn seed_from_history(&mut self, t: TrackId) {
        if self.live_joins.contains_key(&t) {
            if self.video_joined.contains(&t) {
                self.queue_video_join(t);
                self.queue_video_end(t);
            }
        } else if self.audio_max_end.contains_key(&t) {
            self.queue_audio_end(t);
        }
    }

    fn shift(&self, t: TrackId) -> i128 {
        if !self.independent_clocks || self.live_joins.contains_key(&t) {
            return 0;
        }
        let Some(&video) = self
            .rendered
            .iter()
            .find(|v| self.live_joins.contains_key(*v))
        else {
            return 0;
        };
        let (Some(&v_first), Some(&t_first)) =
            (self.first_seen.get(&video), self.first_seen.get(&t))
        else {
            return 0;
        };
        let (Some(v_scale), Some(t_scale)) = (self.timescale_of(video), self.timescale_of(t))
        else {
            return 0;
        };
        raw_27mhz(v_first, v_scale) - raw_27mhz(t_first, t_scale)
    }

    fn conv(&self, t: TrackId, pts: i64, timescale: u32) -> u64 {
        let v =
            (raw_27mhz(pts, timescale) + self.shift(t)).saturating_sub(self.origin.unwrap_or(0));
        u64::try_from(v).unwrap_or(if v < 0 { 0 } else { u64::MAX })
    }

    fn arm_origin(&mut self, t: TrackId, pts: i64, timescale: u32) {
        if self.origin.is_none() {
            self.origin = Some(negative_origin(raw_27mhz(pts, timescale) + self.shift(t)));
        }
    }

    fn timescale_of(&self, t: TrackId) -> Option<u32> {
        self.track_specs.get(&t).map(|s| s.timescale)
    }

    fn queue_video_join(&mut self, t: TrackId) {
        let Some(&(pts, timescale)) = self.video_join_at.get(&t) else {
            return;
        };
        self.arm_origin(t, pts, timescale);
        self.pending_zero.insert(t, (pts, timescale));
        if self.video_join_pts_27.is_none() {
            self.video_join_pts_27 = Some(self.conv(t, pts, timescale));
        }
    }

    fn queue_video_end(&mut self, t: TrackId) {
        let (Some(&end), Some(timescale)) = (self.video_end.get(&t), self.timescale_of(t)) else {
            return;
        };
        self.pending_ends.insert(t, self.conv(t, end, timescale));
    }

    fn queue_audio_end(&mut self, t: TrackId) {
        let (Some(&end), Some(timescale)) = (self.audio_max_end.get(&t), self.timescale_of(t))
        else {
            return;
        };
        let has_rendered_video = self
            .rendered
            .iter()
            .any(|v| self.live_joins.contains_key(v));
        let join_pts = match (has_rendered_video, self.video_join_pts_27) {
            (false, _) => None,
            (true, None) => return,
            (true, Some(j)) => Some(j),
        };
        let first = self.first_seen.get(&t).copied().unwrap_or(end);
        self.arm_origin(t, first, timescale);
        let end_27 = self.conv(t, end, timescale);
        if join_pts.is_none_or(|j| end_27 > j) {
            self.pending_ends.insert(t, end_27);
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
        let Some(timescale) = self.timescale_of(track) else {
            return;
        };
        let Some(pts) = sample.pts else { return };
        let dur = i64::from(sample.duration.unwrap_or(0));
        let rendered = self.rendered.contains(&track);
        self.first_seen.entry(track).or_insert(pts);

        if self.live_joins.contains_key(&track) {
            match join {
                Some(JoinDecision::JoinPoint(_)) if !self.video_joined.contains(&track) => {
                    self.video_joined.insert(track);
                    self.video_join_at.insert(track, (pts, timescale));
                    self.video_max_pts.insert(track, pts);
                    self.video_end.insert(track, pts.saturating_add(dur));
                    if rendered {
                        self.queue_video_join(track);
                        self.queue_video_end(track);
                    }
                }
                Some(JoinDecision::Keep) if self.video_joined.contains(&track) => {
                    let max_pts = self
                        .video_max_pts
                        .get(&track)
                        .copied()
                        .map_or(pts, |m| m.max(pts));
                    self.video_max_pts.insert(track, max_pts);
                    self.video_end.insert(track, max_pts.saturating_add(dur));
                    if rendered {
                        self.queue_video_end(track);
                    }
                }
                _ => {}
            }
            return;
        }

        let end = pts.saturating_add(dur);
        let max_end = self
            .audio_max_end
            .get(&track)
            .copied()
            .map_or(end, |m| m.max(end));
        self.audio_max_end.insert(track, max_end);
        if rendered {
            self.queue_audio_end(track);
        }
    }

    pub(crate) fn rejoin(&mut self) {
        self.video_joined.clear();
        self.pending_ends.clear();
        self.pending_zero.clear();
        self.video_join_at.clear();
        self.video_max_pts.clear();
        self.video_end.clear();
        self.audio_max_end.clear();
        self.first_seen.clear();
        self.video_join_pts_27 = None;
        self.origin = None;
        for j in self.live_joins.values_mut() {
            j.rejoin();
        }
    }

    fn controller_config(&self) -> ControllerConfig {
        let mut cfg = self.config;
        let floor = self
            .start_pending_track_wait_us
            .max(self.start_pending_track_hard_cap_us)
            .saturating_add(SILENCE_MARGIN_US);
        cfg.track_silence_timeout_us = cfg.track_silence_timeout_us.max(floor);
        cfg
    }

    fn ensure_controller(&mut self) {
        if self.finite {
            return;
        }
        if self.controller.is_none()
            && let Ok(mut c) = BufferController::new(self.controller_config(), self.seed.take())
        {
            for t in &self.rendered {
                c.register_track(t.value());
                rec!(self, BufferInput::Register(t.value()));
            }
            self.controller = Some(c);
            self.pending_registered = false;
        }
    }

    fn flush_pending(&mut self, now_us: u64) {
        if self.finite || self.controller.is_none() {
            return;
        }
        let zeros = core::mem::take(&mut self.pending_zero);
        for (t, (pts, ts)) in zeros {
            let end = self.conv(t, pts, ts);
            self.report_media(now_us, t, end);
        }
        let ends = core::mem::take(&mut self.pending_ends);
        for (t, end) in ends {
            self.report_media(now_us, t, end);
        }
    }

    fn report_media(&mut self, now_us: u64, t: TrackId, end: u64) {
        self.first_media_us.get_or_insert(now_us);
        self.sync_pending(now_us);
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

    pub(crate) fn on_chunk(&mut self, now_us: u64, bytes: usize, pcr: Option<u64>) {
        if self.finite {
            self.pending_ends.clear();
            self.pending_zero.clear();
            return;
        }
        self.ensure_controller();
        self.last_chunk_us = Some(now_us);
        self.sync_pending(now_us);
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
        self.flush_pending(now_us);
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
        self.sync_pending(now_us);
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
                        self.started = true;
                        out.push(Event::Buffer(BufferEvent::Start {
                            at_27mhz: media_27mhz,
                        }));
                    }
                    RtEvent::Pause {
                        hold_at_27mhz,
                        reason: rt_reason,
                        track,
                        ..
                    } => {
                        out.push(Event::Buffer(BufferEvent::Pause {
                            hold_at_27mhz,
                            reason: match rt_reason {
                                buffering_runtime::Reason::NoData => {
                                    crate::events::PauseReason::NoData
                                }
                                buffering_runtime::Reason::DataArriving => {
                                    crate::events::PauseReason::DataArriving
                                }
                                buffering_runtime::Reason::Discontinuity => {
                                    crate::events::PauseReason::Discontinuity
                                }
                                buffering_runtime::Reason::Reprime => {
                                    crate::events::PauseReason::Reprime
                                }
                                _ => crate::events::PauseReason::NoData,
                            },
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
        if !self.pending_events.is_empty() {
            out.append(&mut self.pending_events);
        }
        out
    }

    pub(crate) fn stall_memory(&self) -> StallMemorySnapshot {
        self.controller
            .as_ref()
            .map(|c| c.snapshot_memory())
            .unwrap_or_else(|| self.seed.unwrap_or_default())
    }

    pub(crate) fn buffer_stats(&self) -> crate::events::BufferStats {
        self.controller
            .as_ref()
            .map_or_else(crate::events::BufferStats::default, |c| {
                let s = c.stats();
                crate::events::BufferStats {
                    stalls: s.stalls,
                    reprimes: s.reprimes,
                    discontinuities: s.discontinuities,
                    longest_freeze_us: s.longest_freeze_us,
                }
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use broadcast_common::Unpackage;
    use transmux::{LiveJoin, TsDemux};

    const FIXTURE: &str = "ts/h264_aac.ts";
    const FAR_PTS_DURATION: u32 = 3_000;

    struct Streams {
        video: (TrackSpec, Vec<Sample>),
        audio: (TrackSpec, Vec<Sample>),
    }

    fn streams() -> Streams {
        let data = test_fixtures::read(FIXTURE);
        let media = TsDemux::new().unpackage(&data[..]).unwrap();
        let mut video = None;
        let mut audio = None;
        for track in media.tracks {
            if LiveJoin::for_config(&track.spec.config).is_some() {
                video = Some((track.spec, track.samples));
            } else if track.spec.config.audio_sample_rate().is_some() {
                audio = Some((track.spec, track.samples));
            }
        }
        Streams {
            video: video.unwrap(),
            audio: audio.unwrap(),
        }
    }

    fn feeder() -> Feeder {
        Feeder::new(
            ControllerConfig::default(),
            None,
            crate::events::DEFAULT_START_PENDING_TRACK_WAIT_US,
            crate::events::DEFAULT_START_PENDING_TRACK_HARD_CAP_US,
        )
    }

    fn far(sample: &Sample) -> Sample {
        let mut s = sample.clone();
        s.pts = Some(i64::MAX);
        s.dts = Some(i64::MAX);
        s.duration = Some(FAR_PTS_DURATION);
        s
    }

    #[test]
    fn pts_to_27mhz_clamps_instead_of_truncating() {
        assert_eq!(pts_to_27mhz(i64::MAX, 1), u64::MAX);
        assert_eq!(pts_to_27mhz(i64::MIN, 1), 0);
        assert_eq!(pts_to_27mhz(90_000, 90_000), 27_000_000);
        assert_eq!(pts_to_27mhz(1, 0), 0);
    }

    #[test]
    fn audio_sample_at_the_end_of_the_i64_range_does_not_overflow() {
        let st = streams();
        let mut f = feeder();
        f.on_track_added(&st.audio.0);
        let id = st.audio.0.track_id;
        f.on_sample(id, &far(&st.audio.1[0]), None);
        assert_eq!(f.pending_ends.get(&id), Some(&u64::MAX));
    }

    #[test]
    fn video_sample_at_the_end_of_the_i64_range_does_not_overflow() {
        let st = streams();
        let mut f = feeder();
        f.on_track_added(&st.video.0);
        let id = st.video.0.track_id;
        let mut joined = false;
        for s in &st.video.1 {
            let join = f.join_sample(id, s);
            f.on_sample(id, s, join);
            if matches!(join, Some(JoinDecision::JoinPoint(_))) {
                joined = true;
                break;
            }
        }
        assert!(joined, "fixture has no join point");
        let next = far(&st.video.1[st.video.1.len() - 1]);
        let join = f.join_sample(id, &next);
        assert_eq!(join, Some(JoinDecision::Keep));
        f.on_sample(id, &next, join);
        assert_eq!(f.pending_ends.get(&id), Some(&u64::MAX));
    }

    #[test]
    fn retain_tracks_forgets_removed_tracks() {
        let st = streams();
        let mut f = feeder();
        f.on_track_added(&st.video.0);
        f.on_track_added(&st.audio.0);
        let (vid, aid) = (st.video.0.track_id, st.audio.0.track_id);
        for s in &st.video.1 {
            let join = f.join_sample(vid, s);
            f.on_sample(vid, s, join);
            if matches!(join, Some(JoinDecision::JoinPoint(_))) {
                break;
            }
        }
        let mut late_audio = st.audio.1[0].clone();
        late_audio.pts = Some(1 << 40);
        f.on_sample(aid, &late_audio, None);
        assert!(f.rendered.contains(&vid) && f.video_joined.contains(&vid));
        assert!(f.pending_zero.contains_key(&vid) && f.pending_ends.contains_key(&aid));

        f.retain_tracks(std::slice::from_ref(&st.audio.0));
        assert!(!f.track_specs.contains_key(&vid));
        assert!(!f.live_joins.contains_key(&vid));
        assert!(!f.rendered.contains(&vid));
        assert!(!f.video_joined.contains(&vid));
        assert!(!f.pending_zero.contains_key(&vid));
        assert!(!f.pending_ends.contains_key(&vid));
        assert!(!f.video_max_pts.contains_key(&vid));
        assert!(f.track_specs.contains_key(&aid));
        assert!(f.rendered.contains(&aid));
        assert!(f.pending_ends.contains_key(&aid));
    }

    const SLICE_US: u64 = 100_000;
    const SLICE_CHUNK_BYTES: usize = 1316;
    const PREFILL_SLICES: u64 = 50;
    const PREFILL_STEP_US: u64 = 20_000;
    const VIDEO_ID: u32 = 1;
    const AUDIO_A_ID: u32 = 2;
    const AUDIO_B_ID: u32 = 3;
    const SLICES_PER_SEC: i64 = 10;
    const TICK_STEP_US: u64 = 50_000;
    const STARVE_MARGIN_27MHZ: u64 = TICKS_PER_SEC_27MHZ;

    struct Rig {
        f: Feeder,
        key: Sample,
        plain: Sample,
        audio: Sample,
        specs: BTreeMap<u32, TrackSpec>,
        emitting: Vec<u32>,
        now: u64,
        next_slice: i64,
        playing: Option<(u64, u64)>,
        held: Option<u64>,
        events: Vec<(u64, BufferEvent)>,
    }

    fn respec(spec: &TrackSpec, id: u32) -> TrackSpec {
        let mut s = spec.clone();
        s.track_id = TrackId::new(id);
        s
    }

    impl Rig {
        fn new() -> Self {
            let st = streams();
            let mut key = None;
            let mut plain = None;
            let mut probe = LiveJoin::for_config(&st.video.0.config).unwrap();
            for s in &st.video.1 {
                match probe.push(s) {
                    JoinDecision::JoinPoint(_) if key.is_none() => key = Some(s.clone()),
                    JoinDecision::Keep if plain.is_none() => plain = Some(s.clone()),
                    _ => {}
                }
            }
            let specs = BTreeMap::from([
                (VIDEO_ID, respec(&st.video.0, VIDEO_ID)),
                (AUDIO_A_ID, respec(&st.audio.0, AUDIO_A_ID)),
                (AUDIO_B_ID, respec(&st.audio.0, AUDIO_B_ID)),
            ]);
            Self {
                f: feeder(),
                key: key.unwrap(),
                plain: plain.unwrap(),
                audio: st.audio.1[0].clone(),
                specs,
                emitting: Vec::new(),
                now: 0,
                next_slice: 0,
                playing: None,
                held: None,
                events: Vec::new(),
            }
        }

        fn announce(&mut self, ids: &[u32]) {
            for id in ids {
                self.f.on_track_added(&self.specs[id]);
                self.emitting.push(*id);
            }
        }

        fn collect(&mut self) {
            for e in self.f.drain() {
                let Event::Buffer(b) = e else { continue };
                match &b {
                    BufferEvent::Start { at_27mhz } | BufferEvent::Resume { at_27mhz } => {
                        self.playing = Some((*at_27mhz, self.now));
                        self.held = None;
                    }
                    BufferEvent::Pause { hold_at_27mhz, .. } => {
                        self.playing = None;
                        self.held = Some(*hold_at_27mhz);
                    }
                }
                self.events.push((self.now, b));
            }
        }

        fn position(&self) -> Option<u64> {
            match (self.playing, self.held) {
                (Some((base, since)), _) => {
                    Some(base + (self.now - since) * (TICKS_PER_SEC_27MHZ / 1_000_000))
                }
                (None, held) => held,
            }
        }

        fn tick(&mut self) {
            let pos = self.position();
            self.f.on_playback_position(self.now, pos);
            self.collect();
        }

        fn slice(&mut self) {
            let k = self.next_slice;
            self.next_slice += 1;
            for id in self.emitting.clone() {
                let spec = &self.specs[&id];
                let ts = i64::from(spec.timescale);
                let dur = u32::try_from(ts / SLICES_PER_SEC).unwrap();
                let pts = ts + k * i64::from(dur);
                let track = TrackId::new(id);
                let mut s = if id == VIDEO_ID {
                    if k == 0 {
                        self.key.clone()
                    } else {
                        self.plain.clone()
                    }
                } else {
                    self.audio.clone()
                };
                s.pts = Some(pts);
                s.dts = Some(pts);
                s.duration = Some(dur);
                let join = if id == VIDEO_ID && k == 0 {
                    self.f.join_sample(track, &s)
                } else if id == VIDEO_ID {
                    Some(JoinDecision::Keep)
                } else {
                    None
                };
                self.f.on_sample(track, &s, join);
            }
            self.f.on_chunk(self.now, SLICE_CHUNK_BYTES, None);
            self.collect();
        }

        fn prefill(&mut self) {
            for _ in 0..PREFILL_SLICES {
                self.now += PREFILL_STEP_US;
                self.slice();
                self.tick();
            }
        }

        fn run_for(&mut self, us: u64) {
            let end = self.now + us;
            while self.now < end {
                self.now += SLICE_US;
                self.slice();
                self.tick();
            }
        }

        fn tick_for(&mut self, us: u64) {
            let end = self.now + us;
            while self.now < end {
                self.now += TICK_STEP_US;
                self.tick();
            }
        }

        fn emitted_end_27mhz(&self) -> u64 {
            let secs = 1 + self.next_slice / SLICES_PER_SEC;
            u64::try_from(secs).unwrap() * TICKS_PER_SEC_27MHZ
        }

        fn pauses_after(&self, t: u64) -> Vec<(u64, BufferEvent)> {
            self.events
                .iter()
                .filter(|(at, e)| *at > t && matches!(e, BufferEvent::Pause { .. }))
                .cloned()
                .collect()
        }
    }

    #[test]
    fn declaring_an_already_emitting_audio_track_late_is_transparent() {
        let mut r = Rig::new();
        r.announce(&[VIDEO_ID, AUDIO_A_ID, AUDIO_B_ID]);
        r.prefill();
        r.tick_for(2_000_000);
        assert!(
            r.events
                .iter()
                .any(|(_, e)| matches!(e, BufferEvent::Start { .. })),
            "{:?}",
            r.events
        );
        let declared_at = r.now;
        r.f.set_rendered(Some(&[TrackId::new(VIDEO_ID), TrackId::new(AUDIO_B_ID)]));
        r.tick();
        let emitted_end = r.emitted_end_27mhz();
        r.tick_for(8_000_000);
        let pauses = r.pauses_after(declared_at);
        let [(_, BufferEvent::Pause { hold_at_27mhz, .. })] = pauses.as_slice() else {
            panic!("the controller must see the declared track run dry: {pauses:?}");
        };
        assert!(
            *hold_at_27mhz + STARVE_MARGIN_27MHZ >= emitted_end,
            "{hold_at_27mhz} {emitted_end}"
        );
    }

    #[test]
    fn default_rendered_is_the_lowest_id_of_each_kind_whatever_the_announce_order() {
        let mut r = Rig::new();
        r.announce(&[AUDIO_B_ID, VIDEO_ID, AUDIO_A_ID]);
        let want = (Some(TrackId::new(VIDEO_ID)), Some(TrackId::new(AUDIO_A_ID)));
        assert_eq!(r.f.default_rendered(), want);
        assert_eq!(
            r.f.rendered,
            BTreeSet::from([TrackId::new(VIDEO_ID), TrackId::new(AUDIO_A_ID)])
        );
    }

    #[test]
    fn a_lower_id_track_announced_later_takes_over_the_default_without_pausing() {
        let mut r = Rig::new();
        r.announce(&[VIDEO_ID, AUDIO_B_ID]);
        r.prefill();
        r.run_for(2_000_000);
        assert!(
            r.events
                .iter()
                .any(|(_, e)| matches!(e, BufferEvent::Start { .. }))
        );
        let at = r.now;
        r.announce(&[AUDIO_A_ID]);
        r.tick();
        assert_eq!(r.f.default_rendered().1, Some(TrackId::new(AUDIO_A_ID)));
        assert!(r.f.rendered.contains(&TrackId::new(AUDIO_A_ID)));
        assert!(!r.f.rendered.contains(&TrackId::new(AUDIO_B_ID)));
        r.run_for(3_000_000);
        let pauses = r.pauses_after(at);
        assert!(pauses.is_empty(), "{pauses:?}");
    }

    #[test]
    fn declaring_a_track_that_never_emitted_leaves_it_without_an_end() {
        let mut r = Rig::new();
        r.f.on_track_added(&r.specs[&VIDEO_ID]);
        r.f.on_track_added(&r.specs[&AUDIO_A_ID]);
        r.f.on_track_added(&r.specs[&AUDIO_B_ID]);
        r.emitting = vec![VIDEO_ID, AUDIO_A_ID];
        r.prefill();
        let b = TrackId::new(AUDIO_B_ID);
        r.f.set_rendered(Some(&[TrackId::new(VIDEO_ID), b]));
        assert!(r.f.rendered.contains(&b));
        assert!(!r.f.pending_ends.contains_key(&b));
        assert!(!r.f.audio_max_end.contains_key(&b));
    }

    #[test]
    fn av1_and_vp9_tracks_are_default_rendered_video() {
        use transmux::Fmp4Demux;
        for fixture in ["mp4/frag/av1.frag.mp4", "mp4/frag/vp9.frag.mp4"] {
            let data = test_fixtures::read(fixture);
            let media = Fmp4Demux::new().unpackage(&data[..]).unwrap();
            let video = media
                .tracks
                .iter()
                .find(|t| t.spec.config.is_video())
                .unwrap();
            let mut f = feeder();
            f.on_track_added(&video.spec);
            assert_eq!(
                f.default_rendered(),
                (Some(video.spec.track_id), None),
                "{fixture}"
            );
            assert!(f.rendered.contains(&video.spec.track_id));
        }
    }

    #[test]
    fn a_pcr_discontinuity_while_playing_reports_discontinuity_reason() {
        let mut r = Rig::new();
        r.announce(&[VIDEO_ID, AUDIO_A_ID, AUDIO_B_ID]);
        r.prefill();
        assert!(r.playing.is_some(), "must be playing before the hint");
        r.f.hint(r.now, Hint::PcrDiscontinuity);
        r.collect();
        assert!(
            r.events.iter().any(|(_, e)| {
                matches!(e, BufferEvent::Pause { reason, .. } if *reason == crate::events::PauseReason::Discontinuity)
            }),
            "{:?}",
            r.events
        );
        assert_eq!(r.f.buffer_stats().discontinuities, 1);
    }

    #[test]
    fn a_new_connection_clears_the_finite_flag() {
        let mut f = feeder();
        f.set_finite();
        assert!(f.finite);
        f.new_connection();
        assert!(!f.finite);
        f.on_chunk(0, 1316, None);
        assert!(f.controller.is_some());
    }
}
