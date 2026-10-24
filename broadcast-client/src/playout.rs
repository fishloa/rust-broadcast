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
    /// start lead of the source kind. A value not above [`PlayoutConfig::pause_lead_us`] would
    /// pause and resume on every update, so the effective resume lead is never less than
    /// `pause_lead_us` plus one microsecond.
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

    pub(crate) fn disengage(&mut self) {
        self.engaged = false;
        self.rejoin();
        self.ended = false;
        self.out.clear();
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
        let floor_us = self.config.pause_lead_us.saturating_add(1);
        match self.config.resume_lead_us {
            Some(us) => us.max(floor_us).saturating_mul(TICKS_PER_US),
            None => self.start_lead().max(floor_us.saturating_mul(TICKS_PER_US)),
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use broadcast_common::Unpackage;
    use transmux::TsDemux;

    const FIXTURE: &str = "ts/h264_aac.ts";

    #[test]
    fn sample_at_the_end_of_the_i64_range_clamps_instead_of_wrapping() {
        let data = test_fixtures::read(FIXTURE);
        let media = TsDemux::new().unpackage(&data[..]).unwrap();
        let track = media
            .tracks
            .into_iter()
            .find(|t| t.spec.config.audio_sample_rate().is_some())
            .unwrap();
        let mut p = Playout::new(PlayoutConfig::default());
        p.engage(Source::Vod);
        p.on_tracks(std::slice::from_ref(&track.spec));
        let mut s = track.samples[0].clone();
        s.pts = Some(i64::MAX);
        s.duration = Some(3_000);
        p.on_sample(track.spec.track_id, &s, None);
        let obs = p.obs.get(&track.spec.track_id).unwrap();
        assert_eq!(obs.end, u64::MAX);
        assert_eq!(obs.first_pts, u64::MAX);
    }

    const ONE_SECOND_TICKS: u64 = 27_000_000;

    fn audio_track() -> (TrackSpec, Vec<Sample>) {
        let data = test_fixtures::read(FIXTURE);
        let media = TsDemux::new().unpackage(&data[..]).unwrap();
        let track = media
            .tracks
            .into_iter()
            .find(|t| t.spec.config.audio_sample_rate().is_some())
            .unwrap();
        (track.spec, track.samples)
    }

    fn video_spec() -> TrackSpec {
        let data = test_fixtures::read(FIXTURE);
        let media = TsDemux::new().unpackage(&data[..]).unwrap();
        media
            .tracks
            .into_iter()
            .find(|t| LiveJoin::for_config(&t.spec.config).is_some())
            .unwrap()
            .spec
    }

    fn one_second_sample(template: &Sample, spec: &TrackSpec, index: i64) -> Sample {
        let timescale = i64::from(spec.timescale);
        let mut s = template.clone();
        s.pts = Some(index * timescale);
        s.dts = s.pts;
        s.duration = Some(spec.timescale);
        s
    }

    fn buffer_events(p: &mut Playout) -> Vec<BufferEvent> {
        p.drain()
            .into_iter()
            .filter_map(|e| match e {
                Event::Buffer(b) => Some(b),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_resume_lead_not_above_the_pause_lead_does_not_toggle_pause_and_resume() {
        let (spec, samples) = audio_track();
        let mut p = Playout::new(PlayoutConfig {
            vod_start_lead_us: 1,
            pause_lead_us: 0,
            resume_lead_us: Some(0),
            ..PlayoutConfig::default()
        });
        p.engage(Source::Vod);
        p.on_tracks(std::slice::from_ref(&spec));
        p.on_sample(
            spec.track_id,
            &one_second_sample(&samples[0], &spec, 0),
            None,
        );
        let started = buffer_events(&mut p);
        assert!(
            matches!(started.as_slice(), [BufferEvent::Start { .. }]),
            "{started:?}"
        );
        for _ in 0..6 {
            p.on_position(Some(ONE_SECOND_TICKS));
        }
        let events = buffer_events(&mut p);
        assert_eq!(
            events,
            vec![BufferEvent::Pause {
                hold_at_27mhz: ONE_SECOND_TICKS,
                starving_track: Some(spec.track_id.value()),
            }],
            "pause once, never resume without new media"
        );
    }

    #[test]
    fn a_selection_made_before_engage_is_applied_when_the_gate_engages() {
        let (audio, _) = audio_track();
        let video = video_spec();
        let mut p = Playout::new(PlayoutConfig::default());
        p.set_rendered(&[audio.track_id]);
        p.engage(Source::Vod);
        p.on_tracks(&[video, audio.clone()]);
        assert_eq!(p.rendered, BTreeSet::from([audio.track_id]));
    }

    #[test]
    fn a_track_that_leaves_the_set_loses_its_observations_and_cannot_pin_the_lead() {
        let (audio, samples) = audio_track();
        let video = video_spec();
        let mut p = Playout::new(PlayoutConfig::default());
        p.engage(Source::Vod);
        p.on_tracks(&[video.clone(), audio.clone()]);
        p.on_sample(
            audio.track_id,
            &one_second_sample(&samples[0], &audio, 0),
            None,
        );
        assert!(p.obs.contains_key(&audio.track_id));
        p.on_tracks(std::slice::from_ref(&video));
        assert!(!p.obs.contains_key(&audio.track_id));
        assert!(!p.infos.contains_key(&audio.track_id));
        assert_eq!(p.limiting(), None);
    }

    #[test]
    fn disengage_stops_gating_and_forgets_progress() {
        let mut p = Playout::new(PlayoutConfig::default());
        p.engage(Source::Vod);
        p.on_ended();
        assert!(p.ended);
        p.disengage();
        assert!(!p.engaged);
        assert!(!p.ended);
        p.on_ended();
        assert!(!p.ended);
    }
}
