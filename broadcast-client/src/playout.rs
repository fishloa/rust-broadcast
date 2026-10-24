//! Deterministic segment/lead playout gate for sources the buffering controller does not serve:
//! HLS live, HLS VOD and finite TS. It turns per-track media ends and the player's playback
//! position into the same [`BufferEvent`]s the controller produces for live TS-over-HTTP.

use alloc::collections::{BTreeMap, BTreeSet, VecDeque};
use alloc::vec::Vec;

use transmux::{JoinDecision, LiveJoin, Sample, TrackId, TrackSpec};

use crate::buffer::pts_to_27mhz;
use crate::events::{BufferEvent, Event};

/// Default start lead for VOD sources (HLS with `EXT-X-ENDLIST`, finite TS), microseconds.
pub const DEFAULT_VOD_START_LEAD_US: u64 = 1_000_000;
/// Default start lead for live HLS, microseconds.
pub const DEFAULT_LIVE_START_LEAD_US: u64 = 6_000_000;
/// Default lead at or below which a started gate pauses, microseconds.
pub const DEFAULT_PAUSE_LEAD_US: u64 = 0;
const TICKS_PER_US: u64 = 27;
const TARGET_DURATION_LEAD_FACTOR: u64 = 3;
const MICROS_PER_SEC: u64 = 1_000_000;

/// Leads of the playout gate (HLS live, HLS VOD, finite TS). Live TS-over-HTTP keeps using
/// [`crate::Config::buffer`].
///
/// For live HLS the start lead is `min(3 x EXT-X-TARGETDURATION, live_start_lead_us)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlayoutConfig {
    /// Media that must be buffered past the origin before `Start`, VOD sources.
    pub vod_start_lead_us: u64,
    /// Media that must be buffered past the origin before `Start`, live HLS.
    pub live_start_lead_us: u64,
    /// Media that must be buffered past the held position before `Resume`; `None` uses the
    /// start lead of the source kind.
    pub resume_lead_us: Option<u64>,
    /// A started gate pauses when the lead (minimum rendered media end minus the playback
    /// position) is at or below this.
    pub pause_lead_us: u64,
}

impl Default for PlayoutConfig {
    fn default() -> Self {
        Self {
            vod_start_lead_us: DEFAULT_VOD_START_LEAD_US,
            live_start_lead_us: DEFAULT_LIVE_START_LEAD_US,
            resume_lead_us: None,
            pause_lead_us: DEFAULT_PAUSE_LEAD_US,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Source {
    Vod,
    Live { target_duration_secs: Option<u32> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Video,
    Audio,
    Other,
}

#[derive(Debug, Clone, Copy)]
struct TrackInfo {
    kind: Kind,
    timescale: u32,
}

#[derive(Debug, Clone, Copy)]
struct Obs {
    first_pts: u64,
    max_pts: i64,
    end: u64,
}

pub(crate) struct Playout {
    config: PlayoutConfig,
    engaged: bool,
    source: Source,
    infos: BTreeMap<TrackId, TrackInfo>,
    rendered: BTreeSet<TrackId>,
    rendered_explicit: bool,
    obs: BTreeMap<TrackId, Obs>,
    started: bool,
    paused: bool,
    ended: bool,
    position: Option<u64>,
    hold: u64,
    out: VecDeque<BufferEvent>,
}

impl Playout {
    pub(crate) fn new(config: PlayoutConfig) -> Self {
        Self {
            config,
            engaged: false,
            source: Source::Vod,
            infos: BTreeMap::new(),
            rendered: BTreeSet::new(),
            rendered_explicit: false,
            obs: BTreeMap::new(),
            started: false,
            paused: false,
            ended: false,
            position: None,
            hold: 0,
            out: VecDeque::new(),
        }
    }

    pub(crate) fn engage(&mut self, source: Source) {
        self.engaged = true;
        self.source = source;
    }

    pub(crate) fn set_source(&mut self, source: Source) {
        self.source = source;
    }

    pub(crate) fn on_tracks(&mut self, specs: &[TrackSpec]) {
        if !self.engaged {
            return;
        }
        let ids: BTreeSet<TrackId> = specs.iter().map(|s| s.track_id).collect();
        self.infos.retain(|t, _| ids.contains(t));
        self.obs.retain(|t, _| ids.contains(t));
        if !self.rendered_explicit {
            self.rendered.retain(|t| ids.contains(t));
        }
        for spec in specs {
            let kind = if LiveJoin::for_config(&spec.config).is_some() {
                Kind::Video
            } else if spec.config.audio_sample_rate().is_some() {
                Kind::Audio
            } else {
                Kind::Other
            };
            self.infos.insert(
                spec.track_id,
                TrackInfo {
                    kind,
                    timescale: spec.timescale,
                },
            );
            if self.rendered_explicit || kind == Kind::Other {
                continue;
            }
            let covered = self
                .rendered
                .iter()
                .any(|t| self.infos.get(t).is_some_and(|i| i.kind == kind));
            if !covered {
                self.rendered.insert(spec.track_id);
            }
        }
        self.step();
    }

    pub(crate) fn set_rendered(&mut self, tracks: &[TrackId]) {
        if !self.engaged {
            return;
        }
        self.rendered_explicit = true;
        self.rendered = tracks.iter().copied().collect();
        self.step();
    }

    pub(crate) fn on_sample(
        &mut self,
        track: TrackId,
        sample: &Sample,
        join: Option<JoinDecision>,
    ) {
        if !self.engaged || self.ended {
            return;
        }
        if matches!(join, Some(JoinDecision::Drop(_))) {
            return;
        }
        let (Some(info), Some(pts)) = (self.infos.get(&track).copied(), sample.pts) else {
            return;
        };
        let dur = i64::from(sample.duration.unwrap_or(0));
        let timescale = info.timescale;
        let entry = self.obs.entry(track).or_insert(Obs {
            first_pts: pts_to_27mhz(pts, timescale),
            max_pts: pts,
            end: 0,
        });
        entry.max_pts = entry.max_pts.max(pts);
        let end = match info.kind {
            Kind::Video => pts_to_27mhz(entry.max_pts.saturating_add(dur), timescale),
            Kind::Audio | Kind::Other => pts_to_27mhz(pts.saturating_add(dur), timescale),
        };
        entry.end = match info.kind {
            Kind::Video => end,
            Kind::Audio | Kind::Other => entry.end.max(end),
        };
        self.step();
    }

    pub(crate) fn on_position(&mut self, pts: Option<u64>) {
        if !self.engaged {
            return;
        }
        self.position = pts;
        self.step();
    }

    pub(crate) fn on_ended(&mut self) {
        if !self.engaged {
            return;
        }
        self.ended = true;
        self.step();
    }

    pub(crate) fn reset(&mut self) {
        self.rejoin();
        self.ended = false;
    }

    pub(crate) fn rejoin(&mut self) {
        self.obs.clear();
        self.started = false;
        self.paused = false;
        self.position = None;
        self.hold = 0;
    }

    pub(crate) fn drain(&mut self) -> Vec<Event> {
        self.out.drain(..).map(Event::Buffer).collect()
    }

    fn start_lead(&self) -> u64 {
        let us = match self.source {
            Source::Vod => self.config.vod_start_lead_us,
            Source::Live {
                target_duration_secs,
            } => target_duration_secs.map_or(self.config.live_start_lead_us, |td| {
                self.config.live_start_lead_us.min(
                    u64::from(td)
                        .saturating_mul(TARGET_DURATION_LEAD_FACTOR)
                        .saturating_mul(MICROS_PER_SEC),
                )
            }),
        };
        us.saturating_mul(TICKS_PER_US)
    }

    fn resume_lead(&self) -> u64 {
        self.config
            .resume_lead_us
            .map_or_else(|| self.start_lead(), |us| us.saturating_mul(TICKS_PER_US))
    }

    fn origin(&self, only_complete: bool) -> Option<u64> {
        let complete =
            !self.rendered.is_empty() && self.rendered.iter().all(|t| self.obs.contains_key(t));
        if only_complete && !complete {
            return None;
        }
        let first = |kind: Option<Kind>| {
            self.rendered
                .iter()
                .filter(|t| kind.is_none_or(|k| self.infos.get(t).is_some_and(|i| i.kind == k)))
                .filter_map(|t| self.obs.get(t))
                .map(|o| o.first_pts)
                .min()
        };
        let has_video = self
            .rendered
            .iter()
            .any(|t| self.infos.get(t).is_some_and(|i| i.kind == Kind::Video));
        if has_video && !self.ended {
            first(Some(Kind::Video))
        } else {
            first(Some(Kind::Video)).or_else(|| first(None))
        }
    }

    fn limiting(&self) -> Option<(TrackId, u64)> {
        self.rendered
            .iter()
            .filter_map(|t| self.obs.get(t).map(|o| (*t, o.end)))
            .min_by_key(|(_, end)| *end)
    }

    fn step(&mut self) {
        if !self.engaged {
            return;
        }
        if !self.started {
            self.try_start();
        } else if self.paused {
            self.try_resume();
        } else {
            self.try_pause();
        }
    }

    fn try_start(&mut self) {
        let origin = if self.ended {
            self.origin(false)
        } else {
            self.origin(true)
        };
        let (Some(origin), Some((_, end))) = (origin, self.limiting()) else {
            return;
        };
        if self.ended || end.saturating_sub(origin) >= self.start_lead() {
            self.started = true;
            self.out.push_back(BufferEvent::Start { at_27mhz: origin });
        }
    }

    fn try_resume(&mut self) {
        let lead = self
            .limiting()
            .map(|(_, end)| end.saturating_sub(self.hold));
        if self.ended || lead.is_some_and(|l| l >= self.resume_lead()) {
            self.paused = false;
            self.out.push_back(BufferEvent::Resume {
                at_27mhz: self.hold,
            });
        }
    }

    fn try_pause(&mut self) {
        if self.ended {
            return;
        }
        let (Some(position), Some((track, end))) = (self.position, self.limiting()) else {
            return;
        };
        if end.saturating_sub(position) <= self.config.pause_lead_us.saturating_mul(TICKS_PER_US) {
            self.paused = true;
            self.hold = position;
            self.out.push_back(BufferEvent::Pause {
                hold_at_27mhz: position,
                starving_track: Some(track.value()),
            });
        }
    }
}
