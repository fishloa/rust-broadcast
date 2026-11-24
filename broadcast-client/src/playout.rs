//! Deterministic segment/lead playout gate for sources the buffering controller does not serve:
//! HLS live, HLS VOD and finite TS. It turns per-track media ends and the player's playback
//! position into the same [`BufferEvent`]s the controller produces for live TS-over-HTTP.

use alloc::collections::{BTreeMap, BTreeSet, VecDeque};
use alloc::vec::Vec;
use core::time::Duration;

use transmux::{JoinDecision, Sample, TrackId, TrackSpec};

use crate::buffer::{lowest_id_per_kind, negative_origin, pts_to_27mhz_from, raw_27mhz};
use crate::events::{BufferEvent, Event, PauseReason};
use broadcast_common::clock33::{Clock27, PCR_HZ, PCR_WRAP_27MHZ, pcr_forward_distance};
use broadcast_common::timescale::Timescale;
use broadcast_common::units::MICROS_PER_SEC;

/// Largest forward 27 MHz distance that is unambiguous: beyond half the wrap
/// the nearer direction is backwards.
const CLOCK_27MHZ_MAX_FORWARD: u64 = PCR_WRAP_27MHZ / 2;
const CLOCK_COVER_27MHZ: u64 = PCR_WRAP_27MHZ - 1;
/// 27 MHz ticks in one microsecond.
const TICKS_PER_MICRO: u64 = PCR_HZ / MICROS_PER_SEC;

/// Default start lead for VOD sources (HLS with `EXT-X-ENDLIST`, finite TS).
pub const DEFAULT_VOD_START_LEAD: Duration = Duration::from_secs(1);
/// Default start lead for live HLS.
pub const DEFAULT_LIVE_START_LEAD: Duration = Duration::from_secs(6);
/// Default lead at or below which a started gate pauses.
pub const DEFAULT_PAUSE_LEAD: Duration = Duration::ZERO;
const TARGET_DURATION_LEAD_FACTOR: u32 = 3;
/// A lead ([`PlayoutConfig::vod_start_lead`], [`PlayoutConfig::live_start_lead`],
/// [`PlayoutConfig::resume_lead`], [`PlayoutConfig::pause_lead`]) on the 27 MHz playback clock.
///
/// A lead the stream cannot reach keeps the gate closed: a `vod_start_lead` of [`Duration::MAX`]
/// never emits `Start` before the source ends, and a `pause_lead` at or above the lead the
/// source sustains pauses on every update. `3 x EXT-X-TARGETDURATION` is a `u32` of seconds, so
/// the live cap is floored to whole 27 MHz ticks, a sub-tick lead pausing or resuming at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LeadTicks(Clock27);

impl LeadTicks {
    /// The lead as a playback position on the 27 MHz clock.
    #[must_use]
    pub const fn position(self) -> Clock27 {
        self.0
    }

    /// The later of two leads.
    #[must_use]
    pub fn max(self, other: Self) -> Self {
        if pcr_forward_distance(self.0.ticks(), other.0.ticks()) < CLOCK_27MHZ_MAX_FORWARD {
            other
        } else {
            self
        }
    }

    /// The earlier of two leads.
    #[must_use]
    pub fn min(self, other: Self) -> Self {
        if pcr_forward_distance(self.0.ticks(), other.0.ticks()) < CLOCK_27MHZ_MAX_FORWARD {
            self
        } else {
            other
        }
    }

    /// The next lead after this one; one tick past the far side of the wrap (only a lead taken
    /// from [`Duration::MAX`] is there) stays where it is.
    #[must_use]
    pub fn next(self) -> Self {
        let ticks = self.0.ticks().wrapping_add(1) % PCR_WRAP_27MHZ;
        if ticks == 0 {
            return self;
        }
        Self(Clock27::from_27mhz_wrapping(ticks))
    }
}

impl From<Duration> for LeadTicks {
    fn from(lead: Duration) -> Self {
        let micros = lead.as_micros().min(u128::from(u64::MAX));
        let ticks =
            u64::try_from(micros).map_or(u64::MAX, |micros| micros.saturating_mul(TICKS_PER_MICRO));
        Self(Clock27::from_27mhz_wrapping(ticks))
    }
}

/// Leads of the playout gate (HLS live, HLS VOD, finite TS). Live TS-over-HTTP keeps using
/// [`crate::Config::buffer`]. A lead longer than the 27 MHz clock the events carry is reduced
/// onto it, as [`LeadTicks`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct PlayoutConfig {
    /// Media that must be buffered past the origin before `Start`, VOD sources.
    pub vod_start_lead: Duration,
    /// Media that must be buffered past the origin before `Start`, live HLS.
    pub live_start_lead: Duration,
    /// Media that must be buffered past the held position before `Resume`; `None` uses the
    /// start lead of the source kind. A value not above [`PlayoutConfig::pause_lead`] would
    /// pause and resume on every update, so the effective resume lead is never less than
    /// `pause_lead` plus one microsecond.
    pub resume_lead: Option<Duration>,
    /// A started gate pauses when the lead (minimum rendered media end minus the playback
    /// position) is at or below this.
    pub pause_lead: Duration,
}

impl Default for PlayoutConfig {
    fn default() -> Self {
        Self {
            vod_start_lead: DEFAULT_VOD_START_LEAD,
            live_start_lead: DEFAULT_LIVE_START_LEAD,
            resume_lead: None,
            pause_lead: DEFAULT_PAUSE_LEAD,
        }
    }
}

impl PlayoutConfig {
    /// Set the VOD start lead (media buffered past the origin before `Start`).
    #[must_use]
    pub fn with_vod_start_lead(mut self, vod_start_lead: Duration) -> Self {
        self.vod_start_lead = vod_start_lead;
        self
    }

    /// Set the live start lead (media buffered past the origin before `Start`).
    #[must_use]
    pub fn with_live_start_lead(mut self, live_start_lead: Duration) -> Self {
        self.live_start_lead = live_start_lead;
        self
    }

    /// Set the resume lead; `None` falls back to the source kind's start lead.
    #[must_use]
    pub fn with_resume_lead(mut self, resume_lead: Duration) -> Self {
        self.resume_lead = Some(resume_lead);
        self
    }

    /// Set the pause lead (a started gate pauses at or below this).
    #[must_use]
    pub fn with_pause_lead(mut self, pause_lead: Duration) -> Self {
        self.pause_lead = pause_lead;
        self
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
    timescale: Timescale,
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
    origin_shift: Option<i128>,
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
            origin_shift: None,
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
        for spec in specs {
            let kind = if spec.config.is_video() {
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
        }
        self.retarget_default();
        self.step();
    }

    fn retarget_default(&mut self) {
        if self.rendered_explicit {
            return;
        }
        let (video, audio) = lowest_id_per_kind(
            self.infos
                .iter()
                .map(|(id, i)| (*id, i.kind == Kind::Video, i.kind == Kind::Audio)),
        );
        self.rendered = video.into_iter().chain(audio).collect();
    }

    pub(crate) fn set_rendered(&mut self, tracks: Option<&[TrackId]>) {
        self.rendered_explicit = tracks.is_some();
        match tracks {
            Some(t) => self.rendered = t.iter().copied().collect(),
            None => self.retarget_default(),
        }
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
        let (Some(info), Some(pts)) = (
            self.infos.get(&track).copied(),
            sample.timing.and_then(|t| t.pts()),
        ) else {
            return;
        };
        let dur = i64::from(sample.timing.and_then(|t| t.duration()).unwrap_or(0));
        let timescale = info.timescale;
        let shift = *self
            .origin_shift
            .get_or_insert_with(|| negative_origin(raw_27mhz(pts, timescale)));
        let entry = self.obs.entry(track).or_insert(Obs {
            first_pts: pts_to_27mhz_from(pts, timescale, shift),
            max_pts: pts,
            end: 0,
        });
        entry.max_pts = entry.max_pts.max(pts);
        let end = match info.kind {
            Kind::Video => pts_to_27mhz_from(entry.max_pts.saturating_add(dur), timescale, shift),
            Kind::Audio | Kind::Other => {
                pts_to_27mhz_from(pts.saturating_add(dur), timescale, shift)
            }
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
        self.origin_shift = None;
        self.started = false;
        self.paused = false;
        self.position = None;
        self.hold = 0;
    }

    pub(crate) fn drain(&mut self) -> Vec<Event> {
        self.out.drain(..).map(Event::Buffer).collect()
    }

    fn start_lead(&self) -> LeadTicks {
        match self.source {
            Source::Vod => LeadTicks::from(self.config.vod_start_lead),
            Source::Live {
                target_duration_secs,
            } => target_duration_secs.map_or(LeadTicks::from(self.config.live_start_lead), |td| {
                let cap = u64::from(td)
                    .saturating_mul(u64::from(TARGET_DURATION_LEAD_FACTOR))
                    .saturating_mul(MICROS_PER_SEC);
                LeadTicks::from(Duration::from_micros(cap))
                    .min(LeadTicks::from(self.config.live_start_lead))
            }),
        }
    }

    fn resume_lead(&self) -> LeadTicks {
        let floor = LeadTicks::from(self.config.pause_lead);
        let floor = LeadTicks::from(Duration::from_micros(1)).max(floor).next();
        match self.config.resume_lead {
            Some(lead) => LeadTicks::from(lead).max(floor),
            None => self.start_lead().max(floor),
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
        let cover = end.saturating_sub(origin).min(CLOCK_COVER_27MHZ);
        if self.ended || cover >= self.start_lead().position().ticks() {
            self.started = true;
            self.position = None;
            self.out.push_back(BufferEvent::Start { at_27mhz: origin });
        }
    }

    fn try_resume(&mut self) {
        let lead = self
            .limiting()
            .map(|(_, end)| end.saturating_sub(self.hold));
        if self.ended
            || lead
                .is_some_and(|l| l.min(CLOCK_COVER_27MHZ) >= self.resume_lead().position().ticks())
        {
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
        let cover = end.saturating_sub(position).min(CLOCK_COVER_27MHZ);
        if cover <= LeadTicks::from(self.config.pause_lead).position().ticks() {
            self.paused = true;
            self.hold = position;
            self.out.push_back(BufferEvent::Pause {
                hold_at_27mhz: position,
                reason: PauseReason::DataArriving,
                starving_track: Some(track.value()),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use broadcast_common::Unpackage;
    use broadcast_common::clock33::PCR_HZ;
    use transmux::{LiveJoin, TsDemux};

    const FIXTURE: &str = "ts/h264-aac.ts";

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
        if let Some(t) = s.timing.as_mut() {
            *t = t
                .with_composition(i64::MAX, i64::MAX)
                .with_duration(std::num::NonZeroU32::new(3_000).expect("non-zero"))
        } else {
            s.timing = Some(transmux::ir::SampleTiming::timed_with_duration(
                i64::MAX,
                i64::MAX,
                3_000,
            ));
        }
        p.on_sample(track.spec.track_id, &s, None);
        let obs = p.obs.get(&track.spec.track_id).unwrap();
        assert_eq!(obs.end, u64::MAX);
        assert_eq!(obs.first_pts, u64::MAX);
    }

    const ONE_SECOND_TICKS: u64 = PCR_HZ;

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
        let timescale = i64::from(spec.timescale.get());
        let mut s = template.clone();
        let pts = index * timescale;
        if let Some(t) = s.timing.as_mut() {
            *t = t.with_composition(pts, pts).with_duration(
                std::num::NonZeroU32::new(spec.timescale.get())
                    .unwrap_or(std::num::NonZeroU32::MIN),
            )
        } else {
            s.timing = Some(transmux::ir::SampleTiming::timed_with_duration(
                pts,
                pts,
                spec.timescale.get(),
            ));
        }
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
            vod_start_lead: Duration::from_micros(1),
            pause_lead: Duration::ZERO,
            resume_lead: Some(Duration::ZERO),
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
                reason: PauseReason::DataArriving,
                starving_track: Some(spec.track_id.value()),
            }],
            "pause once, never resume without new media"
        );
    }

    #[test]
    fn a_start_lead_beyond_the_clock_never_starts_and_zero_starts_at_once() {
        let (spec, samples) = audio_track();
        let mut never = Playout::new(PlayoutConfig::default().with_vod_start_lead(Duration::MAX));
        never.engage(Source::Vod);
        never.on_tracks(std::slice::from_ref(&spec));
        never.on_sample(
            spec.track_id,
            &one_second_sample(&samples[0], &spec, 0),
            None,
        );
        assert!(
            never.start_lead().position().ticks() > ONE_SECOND_TICKS,
            "a lead longer than the clock still holds the gate closed"
        );
        assert_eq!(buffer_events(&mut never), vec![]);
        let mut at_once =
            Playout::new(PlayoutConfig::default().with_vod_start_lead(Duration::ZERO));
        at_once.engage(Source::Vod);
        at_once.on_tracks(std::slice::from_ref(&spec));
        at_once.on_sample(
            spec.track_id,
            &one_second_sample(&samples[0], &spec, 0),
            None,
        );
        assert_eq!(
            buffer_events(&mut at_once),
            vec![BufferEvent::Start { at_27mhz: 0 }]
        );
    }

    #[test]
    fn a_selection_made_before_engage_is_applied_when_the_gate_engages() {
        let (audio, _) = audio_track();
        let video = video_spec();
        let mut p = Playout::new(PlayoutConfig::default());
        p.set_rendered(Some(&[audio.track_id]));
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

    #[test]
    fn a_negative_unrolled_timeline_starts_relative_to_its_first_sample() {
        let (spec, samples) = audio_track();
        let mut p = Playout::new(PlayoutConfig::default());
        p.engage(Source::Vod);
        p.on_tracks(std::slice::from_ref(&spec));
        let negative = |index: i64| one_second_sample(&samples[0], &spec, index - NEGATIVE_SECONDS);
        p.on_sample(spec.track_id, &negative(0), None);
        assert_eq!(
            buffer_events(&mut p),
            vec![BufferEvent::Start { at_27mhz: 0 }],
            "times are measured from the first kept sample, never clamped to a flat zero"
        );
        p.on_sample(spec.track_id, &negative(1), None);
        let obs = p.obs.get(&spec.track_id).unwrap();
        assert_eq!(obs.end, 2 * ONE_SECOND_TICKS);
        p.on_position(Some(2 * ONE_SECOND_TICKS));
        assert_eq!(
            buffer_events(&mut p),
            vec![BufferEvent::Pause {
                hold_at_27mhz: 2 * ONE_SECOND_TICKS,
                reason: PauseReason::DataArriving,
                starving_track: Some(spec.track_id.value()),
            }]
        );
    }

    #[test]
    fn a_gate_whose_media_starts_inside_the_last_second_of_the_clock_still_opens() {
        let mut spec = audio_track().0;
        spec.timescale = Timescale::new(27_000).expect("a non-zero timescale");
        let mut p = Playout::new(PlayoutConfig::default().with_vod_start_lead(Duration::ZERO));
        p.engage(Source::Vod);
        p.on_tracks(std::slice::from_ref(&spec));
        let timescale = i64::from(spec.timescale.get());
        let start = i64::try_from(PCR_WRAP_27MHZ / 27).expect("a 27 kHz PTS fits") - 4 * timescale;
        let mut s = audio_track().1[0].clone();
        s.timing = Some(transmux::ir::SampleTiming::timed_with_duration(
            start,
            start,
            spec.timescale.get(),
        ));
        p.on_sample(spec.track_id, &s, None);
        let events = buffer_events(&mut p);
        assert!(
            events
                .iter()
                .any(|e| matches!(e, BufferEvent::Start { .. })),
            "the gate opens for media inside the last ticks of the clock: {events:?}"
        );
    }

    const LOW_AUDIO_ID: u32 = 20;
    const HIGH_AUDIO_ID: u32 = 30;
    const EMITTED_SECONDS: i64 = 3;

    fn with_id(spec: &TrackSpec, id: u32) -> TrackSpec {
        let mut s = spec.clone();
        s.track_id = TrackId::new(id);
        s
    }

    fn both_sources() -> [Source; 2] {
        [
            Source::Vod,
            Source::Live {
                target_duration_secs: None,
            },
        ]
    }

    #[test]
    fn the_default_rendered_set_is_the_lowest_id_of_each_kind_in_any_announce_order() {
        let (audio, _) = audio_track();
        let video = video_spec();
        let low = with_id(&audio, LOW_AUDIO_ID);
        let high = with_id(&audio, HIGH_AUDIO_ID);
        for source in both_sources() {
            let mut p = Playout::new(PlayoutConfig::default());
            p.engage(source);
            p.on_tracks(&[high.clone(), video.clone()]);
            assert_eq!(p.rendered, BTreeSet::from([video.track_id, high.track_id]));
            p.on_tracks(&[high.clone(), video.clone(), low.clone()]);
            assert_eq!(p.rendered, BTreeSet::from([video.track_id, low.track_id]));
        }
    }

    #[test]
    fn declaring_the_other_audio_late_retargets_and_is_seeded_from_its_observations() {
        let (audio, samples) = audio_track();
        let video = video_spec();
        let low = with_id(&audio, LOW_AUDIO_ID);
        let high = with_id(&audio, HIGH_AUDIO_ID);
        for source in both_sources() {
            let mut p = Playout::new(PlayoutConfig::default());
            p.engage(source);
            p.on_tracks(&[high.clone(), video.clone(), low.clone()]);
            for i in 0..EMITTED_SECONDS {
                for spec in [&video, &low, &high] {
                    p.on_sample(
                        spec.track_id,
                        &one_second_sample(&samples[0], spec, i),
                        None,
                    );
                }
            }
            p.set_rendered(Some(&[video.track_id, high.track_id]));
            assert_eq!(p.rendered, BTreeSet::from([video.track_id, high.track_id]));
            assert_eq!(
                p.limiting().map(|(_, end)| end),
                Some(u64::try_from(EMITTED_SECONDS).unwrap() * ONE_SECOND_TICKS)
            );
            assert!(p.obs.contains_key(&high.track_id));
        }
    }

    #[test]
    fn unwrapped_pts_beyond_33bit_behaves_normally() {
        let (spec, samples) = audio_track();
        let mut p = Playout::new(PlayoutConfig::default());
        p.engage(Source::Vod);
        p.on_tracks(std::slice::from_ref(&spec));
        let ts = i64::from(spec.timescale.get());
        let base: i64 = 1 << 33;
        let mut s1 = samples[0].clone();
        let pts1 = base + 5 * ts;
        if let Some(t) = s1.timing.as_mut() {
            *t = t.with_composition(pts1, pts1).with_duration(
                std::num::NonZeroU32::new(spec.timescale.get())
                    .unwrap_or(std::num::NonZeroU32::MIN),
            )
        } else {
            s1.timing = Some(transmux::ir::SampleTiming::timed_with_duration(
                pts1,
                pts1,
                spec.timescale.get(),
            ));
        }
        p.on_sample(spec.track_id, &s1, None);
        let evs = buffer_events(&mut p);
        assert!(
            evs.iter().any(|e| matches!(e, BufferEvent::Start { .. })),
            "Start not emitted for already-unwrapped PTS beyond 2^33: {evs:?}"
        );
        let first_end = p.obs.get(&spec.track_id).unwrap().end;
        let mut s2 = samples[0].clone();
        let pts2 = base + 6 * ts;
        if let Some(t) = s2.timing.as_mut() {
            *t = t.with_composition(pts2, pts2).with_duration(
                std::num::NonZeroU32::new(spec.timescale.get())
                    .unwrap_or(std::num::NonZeroU32::MIN),
            )
        } else {
            s2.timing = Some(transmux::ir::SampleTiming::timed_with_duration(
                pts2,
                pts2,
                spec.timescale.get(),
            ));
        }
        p.on_sample(spec.track_id, &s2, None);
        let _ = buffer_events(&mut p);
        let second_end = p.obs.get(&spec.track_id).unwrap().end;
        assert!(
            second_end > first_end,
            "end did not advance for already-unwrapped PTS beyond 2^33: {first_end} -> {second_end}"
        );
    }

    const NEGATIVE_SECONDS: i64 = 50_000;

    fn sample_at_ticks(template: &Sample, pts: i64, duration: u32) -> Sample {
        let mut s = template.clone();
        let duration = std::num::NonZeroU32::new(duration).expect("a non-zero duration");
        if let Some(t) = s.timing.as_mut() {
            *t = t.with_composition(pts, pts).with_duration(duration);
        } else {
            s.timing = Some(transmux::ir::SampleTiming::timed_with_duration(
                pts,
                pts,
                duration.get(),
            ));
        }
        s
    }

    #[test]
    fn the_resume_lead_is_one_tick_past_the_pause_lead_and_wraps_back_to_itself() {
        assert_eq!(
            LeadTicks::from(Duration::from_micros(1))
                .next()
                .position()
                .ticks(),
            28
        );
        let at_boundary = LeadTicks(Clock27::from_27mhz_wrapping(PCR_WRAP_27MHZ - 1));
        assert_eq!(at_boundary.position().ticks(), PCR_WRAP_27MHZ - 1);
        assert_eq!(at_boundary.next(), at_boundary);
    }

    #[test]
    fn a_cover_exactly_at_the_pause_lead_resumes_only_past_it() {
        let (mut spec, samples) = audio_track();
        spec.timescale = Timescale::new(27_000_000).expect("a non-zero timescale");
        let mut p = Playout::new(PlayoutConfig {
            vod_start_lead: Duration::ZERO,
            pause_lead: Duration::ZERO,
            ..PlayoutConfig::default()
        });
        p.engage(Source::Vod);
        p.on_tracks(std::slice::from_ref(&spec));
        p.on_sample(spec.track_id, &sample_at_ticks(&samples[0], 0, 27), None);
        assert!(matches!(
            buffer_events(&mut p).as_slice(),
            [BufferEvent::Start { at_27mhz: 0 }]
        ));
        p.on_position(Some(27));
        assert_eq!(
            buffer_events(&mut p),
            vec![BufferEvent::Pause {
                hold_at_27mhz: 27,
                reason: PauseReason::DataArriving,
                starving_track: Some(spec.track_id.value()),
            }]
        );
        p.on_sample(spec.track_id, &sample_at_ticks(&samples[0], 27, 27), None);
        assert_eq!(
            buffer_events(&mut p),
            vec![],
            "a cover of 27 ticks is short of the resume lead of 28"
        );
        p.on_sample(spec.track_id, &sample_at_ticks(&samples[0], 54, 1), None);
        assert_eq!(
            buffer_events(&mut p),
            vec![BufferEvent::Resume { at_27mhz: 27 }],
            "a cover of 28 ticks resumes"
        );
    }

    #[test]
    fn samples_before_engage_and_after_the_end_are_not_observed() {
        let (spec, samples) = audio_track();
        let first = one_second_sample(&samples[0], &spec, 0);
        let mut p = Playout::new(PlayoutConfig::default());
        p.on_sample(spec.track_id, &first, None);
        assert!(p.obs.is_empty(), "a disengaged gate records nothing");
        p.engage(Source::Vod);
        p.on_tracks(std::slice::from_ref(&spec));
        p.on_ended();
        p.on_sample(spec.track_id, &first, None);
        assert!(p.obs.is_empty(), "an ended gate records nothing");
    }

    #[test]
    fn the_origin_waits_for_video_until_the_source_ends_then_falls_back_to_audio() {
        let (audio, samples) = audio_track();
        let video = video_spec();
        let mut p = Playout::new(PlayoutConfig::default());
        p.engage(Source::Vod);
        p.on_tracks(&[video, audio.clone()]);
        p.on_sample(
            audio.track_id,
            &one_second_sample(&samples[0], &audio, 0),
            None,
        );
        assert_eq!(
            p.origin(false),
            None,
            "audio alone is not an origin while a video track is rendered"
        );
        p.on_ended();
        let audio_first = p
            .obs
            .get(&audio.track_id)
            .expect("audio observed")
            .first_pts;
        assert_eq!(p.origin(false), Some(audio_first));
    }
}
