//! Timeline-jump detection: a new timeline announced by the media alone, with no TS
//! discontinuity flag (origin restart, a recorder looping a file, a channel swap behind one URL).

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::vec::Vec;

use transmux::{JoinDecision, LiveJoin, Sample, TrackId, TrackSpec};

/// Default backward step that marks a new timeline, microseconds.
pub const DEFAULT_JUMP_BACKWARD_US: u64 = 1_000_000;
/// Default slack added to the elapsed delivery time before a forward step marks a new timeline,
/// microseconds.
pub const DEFAULT_JUMP_FORWARD_SLACK_US: u64 = 5_000_000;

/// Thresholds of the timeline-jump detector, see [`crate::Config::timeline_jump`].
///
/// Between two consecutive kept samples of a track, with `step = pts - previous pts` converted
/// to microseconds, a new timeline starts when `step < -backward_us` or
/// `step > elapsed_us + forward_slack_us`, where `elapsed_us` is the session clock time between
/// the deliveries of the two samples (zero inside one burst).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimelineJumpConfig {
    /// A backward step larger than this is a new timeline, microseconds. B-frame reordering
    /// stays far below the default.
    pub backward_us: u64,
    /// A forward step larger than the elapsed delivery time plus this is a new timeline,
    /// microseconds.
    pub forward_slack_us: u64,
}

impl Default for TimelineJumpConfig {
    fn default() -> Self {
        Self {
            backward_us: DEFAULT_JUMP_BACKWARD_US,
            forward_slack_us: DEFAULT_JUMP_FORWARD_SLACK_US,
        }
    }
}

const MICROS_PER_SEC: i128 = 1_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    Video,
    Audio,
    Other,
}

#[derive(Debug, Clone, Copy)]
struct TrackInfo {
    role: Role,
    timescale: u32,
}

#[derive(Debug, Clone, Copy)]
struct Last {
    pts_us: i128,
    now_us: u64,
}

struct Epoch {
    reference_us: i128,
    window_us: i128,
    stale: BTreeSet<TrackId>,
}

pub(crate) struct JumpDetector {
    config: Option<TimelineJumpConfig>,
    tracks: BTreeMap<TrackId, TrackInfo>,
    audio_order: Vec<TrackId>,
    rendered: Option<BTreeSet<TrackId>>,
    last: BTreeMap<TrackId, Last>,
    delivered: BTreeSet<TrackId>,
    post_reset: BTreeSet<TrackId>,
    epoch: Option<Epoch>,
    handled: bool,
}

impl JumpDetector {
    pub(crate) fn new(config: Option<TimelineJumpConfig>) -> Self {
        Self {
            config,
            tracks: BTreeMap::new(),
            audio_order: Vec::new(),
            rendered: None,
            last: BTreeMap::new(),
            delivered: BTreeSet::new(),
            post_reset: BTreeSet::new(),
            epoch: None,
            handled: false,
        }
    }

    pub(crate) fn on_tracks(&mut self, specs: &[TrackSpec]) {
        self.tracks.clear();
        self.audio_order.clear();
        for spec in specs {
            let role = if LiveJoin::for_config(&spec.config).is_some() {
                Role::Video
            } else if spec.config.audio_sample_rate().is_some() {
                self.audio_order.push(spec.track_id);
                Role::Audio
            } else {
                Role::Other
            };
            self.tracks.insert(
                spec.track_id,
                TrackInfo {
                    role,
                    timescale: spec.timescale,
                },
            );
        }
        let present = &self.tracks;
        self.last.retain(|t, _| present.contains_key(t));
    }

    pub(crate) fn set_rendered(&mut self, tracks: &[TrackId]) {
        self.rendered = Some(tracks.iter().copied().collect());
    }

    pub(crate) fn clear(&mut self) {
        self.last.clear();
        self.post_reset.clear();
        self.epoch = None;
        self.handled = false;
    }

    pub(crate) fn begin_epoch(&mut self, jumped: TrackId, sample: &Sample) {
        let (Some(cfg), Some(pts), Some(info)) =
            (self.config, sample.pts, self.tracks.get(&jumped).copied())
        else {
            return;
        };
        if info.timescale == 0 {
            return;
        }
        let stale: BTreeSet<TrackId> = self
            .delivered
            .iter()
            .copied()
            .filter(|t| *t != jumped && self.tracks.contains_key(t))
            .collect();
        self.delivered.clear();
        self.epoch = Some(Epoch {
            reference_us: Self::to_us(pts, info.timescale),
            window_us: i128::from(cfg.forward_slack_us),
            stale,
        });
    }

    pub(crate) fn screen(&mut self, track: TrackId, sample: &Sample) -> bool {
        let Some(epoch) = self.epoch.as_mut() else {
            return true;
        };
        if !epoch.stale.contains(&track) {
            return true;
        }
        let (Some(pts), Some(info)) = (sample.pts, self.tracks.get(&track)) else {
            return true;
        };
        if info.timescale == 0 {
            return true;
        }
        let offset = Self::to_us(pts, info.timescale) - epoch.reference_us;
        if offset.abs() <= epoch.window_us {
            epoch.stale.remove(&track);
            return true;
        }
        false
    }

    pub(crate) fn note_reset(&mut self) {
        self.post_reset.clear();
        self.handled = true;
    }

    pub(crate) fn already_handled(&self) -> bool {
        self.handled
    }

    fn has_video(&self) -> bool {
        self.tracks.values().any(|i| i.role == Role::Video)
    }

    fn first_rendered_audio(&self) -> Option<TrackId> {
        match &self.rendered {
            Some(set) => self.audio_order.iter().copied().find(|t| set.contains(t)),
            None => self.audio_order.first().copied(),
        }
    }

    fn watched(&self, track: TrackId, join: Option<JoinDecision>) -> Option<TrackInfo> {
        if matches!(join, Some(JoinDecision::Drop(_))) {
            return None;
        }
        let info = *self.tracks.get(&track)?;
        let eligible = match info.role {
            Role::Video => true,
            Role::Audio => !self.has_video() && self.first_rendered_audio() == Some(track),
            Role::Other => false,
        };
        (eligible && info.timescale != 0).then_some(info)
    }

    fn to_us(pts: i64, timescale: u32) -> i128 {
        i128::from(pts) * MICROS_PER_SEC / i128::from(timescale)
    }

    pub(crate) fn is_jump(
        &self,
        track: TrackId,
        sample: &Sample,
        join: Option<JoinDecision>,
        now_us: u64,
    ) -> bool {
        let (Some(cfg), Some(pts)) = (self.config, sample.pts) else {
            return false;
        };
        let Some(info) = self.watched(track, join) else {
            return false;
        };
        let Some(prev) = self.last.get(&track) else {
            return false;
        };
        let step = Self::to_us(pts, info.timescale) - prev.pts_us;
        let elapsed = i128::from(now_us.saturating_sub(prev.now_us));
        step < -i128::from(cfg.backward_us) || step > elapsed + i128::from(cfg.forward_slack_us)
    }

    pub(crate) fn record(
        &mut self,
        track: TrackId,
        sample: &Sample,
        join: Option<JoinDecision>,
        now_us: u64,
    ) {
        self.delivered.insert(track);
        let Some(pts) = sample.pts else { return };
        let Some(info) = self.watched(track, join) else {
            return;
        };
        let continued = self.last.contains_key(&track) && self.post_reset.contains(&track);
        self.last.insert(
            track,
            Last {
                pts_us: Self::to_us(pts, info.timescale),
                now_us,
            },
        );
        self.post_reset.insert(track);
        if continued {
            self.handled = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use broadcast_common::Unpackage;
    use transmux::{DropReason, TsDemux};

    const VIDEO_TS: i64 = 90_000;
    const T0: u64 = 7_000_000;

    struct Fixture {
        video: TrackSpec,
        audio: TrackSpec,
        sample: Sample,
    }

    fn fixture() -> Fixture {
        let data = test_fixtures::read("ts/h264_aac.ts");
        let media = TsDemux::new().unpackage(&data[..]).unwrap();
        let mut video = None;
        let mut audio = None;
        for t in media.tracks {
            if LiveJoin::for_config(&t.spec.config).is_some() {
                video = Some((t.spec, t.samples));
            } else {
                audio = Some(t.spec);
            }
        }
        let (video, samples) = video.unwrap();
        Fixture {
            video,
            audio: audio.unwrap(),
            sample: samples[0].clone(),
        }
    }

    fn at(f: &Fixture, pts: i64) -> Sample {
        let mut s = f.sample.clone();
        s.pts = Some(pts);
        s
    }

    fn detector(f: &Fixture, config: TimelineJumpConfig) -> JumpDetector {
        let mut d = JumpDetector::new(Some(config));
        d.on_tracks(&[f.video.clone(), f.audio.clone()]);
        d
    }

    fn jump_after(
        d: &mut JumpDetector,
        f: &Fixture,
        first_us: u64,
        pts: i64,
        second_us: u64,
    ) -> bool {
        let id = f.video.track_id;
        d.clear();
        d.record(id, &at(f, 0), Some(JoinDecision::Keep), first_us);
        d.is_jump(id, &at(f, pts), Some(JoinDecision::Keep), second_us)
    }

    #[test]
    fn the_backward_threshold_is_exclusive_and_exact() {
        let f = fixture();
        let mut d = detector(&f, TimelineJumpConfig::default());
        assert!(!jump_after(&mut d, &f, T0, -VIDEO_TS, T0), "exactly -1 s");
        assert!(
            jump_after(&mut d, &f, T0, -VIDEO_TS - 1, T0),
            "one tick past -1 s"
        );
        assert!(!jump_after(&mut d, &f, T0, -VIDEO_TS + 1, T0));
    }

    #[test]
    fn the_forward_threshold_is_elapsed_plus_slack_exclusive() {
        let f = fixture();
        let mut d = detector(&f, TimelineJumpConfig::default());
        let slack_ticks = 5 * VIDEO_TS;
        assert!(
            !jump_after(&mut d, &f, T0, slack_ticks, T0),
            "exactly +5 s, no elapsed"
        );
        assert!(jump_after(&mut d, &f, T0, slack_ticks + 1, T0));
        let elapsed_ticks = 2 * VIDEO_TS;
        assert!(
            !jump_after(&mut d, &f, T0, slack_ticks + elapsed_ticks, T0 + 2_000_000),
            "exactly elapsed + 5 s"
        );
        assert!(jump_after(
            &mut d,
            &f,
            T0,
            slack_ticks + elapsed_ticks + 1,
            T0 + 2_000_000
        ));
    }

    #[test]
    fn a_dropped_sample_neither_triggers_nor_is_remembered() {
        let f = fixture();
        let mut d = detector(&f, TimelineJumpConfig::default());
        let id = f.video.track_id;
        d.record(id, &at(&f, 0), Some(JoinDecision::Keep), T0);
        let dropped = Some(JoinDecision::Drop(DropReason::BeforeRandomAccessPoint));
        let far = 100 * VIDEO_TS;
        assert!(!d.is_jump(id, &at(&f, far), dropped, T0));
        d.record(id, &at(&f, far), dropped, T0);
        assert!(!d.is_jump(id, &at(&f, 1), Some(JoinDecision::Keep), T0));
        assert!(d.is_jump(id, &at(&f, far), Some(JoinDecision::Keep), T0));
    }

    #[test]
    fn a_disabled_detector_never_fires() {
        let f = fixture();
        let mut d = JumpDetector::new(None);
        d.on_tracks(std::slice::from_ref(&f.video));
        let id = f.video.track_id;
        d.record(id, &at(&f, 0), Some(JoinDecision::Keep), T0);
        assert!(!d.is_jump(id, &at(&f, 1_000 * VIDEO_TS), Some(JoinDecision::Keep), T0));
    }

    #[test]
    fn audio_is_watched_only_without_video() {
        let f = fixture();
        let audio = f.audio.track_id;
        let rate = i64::from(f.audio.timescale);
        let mut with_video = detector(&f, TimelineJumpConfig::default());
        with_video.record(audio, &at(&f, 0), None, T0);
        assert!(!with_video.is_jump(audio, &at(&f, 100 * rate), None, T0));
        let mut only_audio = JumpDetector::new(Some(TimelineJumpConfig::default()));
        only_audio.on_tracks(std::slice::from_ref(&f.audio));
        only_audio.record(audio, &at(&f, 0), None, T0);
        assert!(only_audio.is_jump(audio, &at(&f, 100 * rate), None, T0));
    }
}
