//! [`SmoothClient`]: the sans-IO Smooth Streaming session.

use std::collections::VecDeque;
use std::time::Duration;

use broadcast_smooth::{SmoothManifest, StreamIndex, StreamType};
use transmux::dash::rfc6381_track_codec_string;
use transmux::pipeline::build_init_segment;
use transmux::smooth_fragment::{
    FragmentState, fragment_state, fragment_track_id, manifest_declares_protection,
};
use transmux::{TrackId, TrackSpec, track_spec_from_quality_level};
use url::Url;

use broadcast_common::backoff::{RetrySchedule, mix_seed};
use broadcast_common::url_ref;

use super::action::{Action, FetchFailure, RequestId, StreamId};
use super::config::{SYNTHETIC_MOVIE_TIMESCALE, SmoothConfig};
use super::output::{Dropped, ManifestFault, Output, QualityInfo};
use super::stream::{Chunk, Cursor, HeldFragment, Stream, demux_fragment, live_join};
use crate::error::SmoothError;
use broadcast_common::units::MICROS_PER_SEC;

/// The shortest retry delay: a zero step must never become a burst of retries at one instant.
const MIN_BACKOFF_MS: u64 = 1;
const REFRESH_DIVISOR: u64 = 2;
const FIRST_TRACK_ID: u32 = 1;
const FOURCC_AVC: [&str; 2] = ["H264", "AVC1"];
const FOURCC_HEVC: [&str; 3] = ["H265", "HEVC", "HVC1"];
const FOURCC_HEVC_ALT: &str = "HEV1";

fn codec_family(four_cc: &str) -> String {
    let upper = four_cc.to_ascii_uppercase();
    if FOURCC_AVC.contains(&upper.as_str()) {
        FOURCC_AVC[0].to_string()
    } else if FOURCC_HEVC.contains(&upper.as_str()) || upper == FOURCC_HEVC_ALT {
        FOURCC_HEVC[0].to_string()
    } else {
        upper
    }
}

fn timescale_of(manifest: &SmoothManifest, index: &StreamIndex) -> Option<u32> {
    u32::try_from(index.timescale.unwrap_or(manifest.timescale))
        .ok()
        .filter(|&t| t != 0)
}

fn primary_video(manifest: &SmoothManifest) -> Option<&StreamIndex> {
    manifest.streams.iter().find(|s| {
        s.stream_type == StreamType::Video
            && !s.qualities.is_empty()
            && timescale_of(manifest, s).is_some()
    })
}

#[derive(Debug, Clone, Copy)]
enum ManifestCursor {
    Idle,
    InFlight(RequestId),
}

/// The Smooth Streaming client engine. See the [crate docs](crate).
pub struct SmoothClient {
    config: SmoothConfig,
    manifest_url: Url,
    streams: Vec<Stream>,
    started: bool,
    announced: bool,
    is_live: bool,
    ended: bool,
    fatal: bool,
    video_quality_count: usize,
    manifest: ManifestCursor,
    manifest_at: Option<u64>,
    manifest_failures: u32,
    retry: RetrySchedule,
    next_request: u64,
    actions: VecDeque<Action>,
    outputs: VecDeque<Output>,
}

impl SmoothClient {
    /// A client for the manifest at `manifest_url`; the first [`Action`] is its fetch.
    pub fn new(manifest_url: Url, config: SmoothConfig) -> Self {
        let retry = RetrySchedule::from_steps(
            &config
                .backoff_ms
                .iter()
                .map(|ms| Duration::from_millis((*ms).max(MIN_BACKOFF_MS)))
                .collect::<Vec<_>>(),
            Duration::from_millis(config.backoff_cap_ms.max(MIN_BACKOFF_MS)),
            mix_seed(config.jitter_seed),
        );
        Self {
            config,
            manifest_url,
            streams: Vec::new(),
            started: false,
            announced: false,
            is_live: false,
            ended: false,
            fatal: false,
            video_quality_count: 0,
            manifest: ManifestCursor::Idle,
            manifest_at: Some(0),
            manifest_failures: 0,
            retry,
            next_request: 0,
            actions: VecDeque::new(),
            outputs: VecDeque::new(),
        }
    }

    /// Re-base relative fragment URLs on `url`, after a redirect of the manifest.
    pub fn set_manifest_url(&mut self, url: Url) {
        self.manifest_url = url;
    }

    /// True once a manifest has been accepted and the streams are resolved.
    pub fn is_started(&self) -> bool {
        self.started
    }

    /// True while the presentation is live and the session has not ended.
    pub fn is_live(&self) -> bool {
        self.is_live && !self.ended
    }

    /// True once a static presentation has played to its end.
    pub fn is_ended(&self) -> bool {
        self.ended
    }

    /// True once the session has ended fatally.
    pub fn is_fatal(&self) -> bool {
        self.fatal
    }

    /// How many `QualityLevel`s the first video stream offers (0 before the first manifest).
    pub fn video_quality_count(&self) -> usize {
        self.video_quality_count
    }

    /// Fetch the first video stream's later fragments at `QualityLevel` `index`
    /// (an index into [`Output::Qualities`]); false when there is no such level.
    ///
    /// Fragments already requested keep their level. Once the tracks are
    /// announced, a level of the same codec family is accepted even when its
    /// codec configuration (parameter sets, resolution) differs: the new
    /// configuration is announced as [`Output::TracksChanged`] before the first
    /// sample of the new level. A level of another codec family is refused (false).
    pub fn select_video_quality(&mut self, index: usize) -> bool {
        let Some(si) = self
            .streams
            .iter()
            .position(|s| s.index.stream_type == StreamType::Video)
        else {
            return false;
        };
        let Some(quality) = self.streams[si].index.qualities.get(index).cloned() else {
            return false;
        };
        if self.announced && !self.switchable(si, &quality) {
            return false;
        }
        self.streams[si].quality = quality;
        true
    }

    fn switchable(&self, si: usize, candidate: &broadcast_smooth::QualityLevel) -> bool {
        let stream = &self.streams[si];
        codec_family(&stream.applied.four_cc) == codec_family(&candidate.four_cc)
            && track_spec_from_quality_level(
                TrackId::new(FIRST_TRACK_ID),
                stream.timescale,
                stream.index.stream_type,
                candidate,
            )
            .is_ok()
    }

    /// The earliest instant, in microseconds, at which [`Self::poll`] has more to do
    /// without any new input.
    pub fn next_deadline(&self) -> Option<u64> {
        if self.fatal || self.ended {
            return None;
        }
        let backoffs = self.streams.iter().filter_map(|s| match s.cursor {
            Cursor::Backoff { until, .. } => Some(until),
            _ => None,
        });
        backoffs.chain(self.manifest_at).min()
    }

    /// The next fetch the host should perform, if any.
    pub fn poll(&mut self, now: u64) -> Option<Action> {
        if !self.fatal && !self.ended {
            self.fire_timers(now);
            self.schedule();
            self.advance(now);
        }
        self.actions.pop_front()
    }

    /// The next thing to act on, if any.
    pub fn next_output(&mut self) -> Option<Output> {
        self.outputs.pop_front()
    }

    /// A manifest body arrived for request `id`.
    pub fn on_manifest(&mut self, now: u64, id: RequestId, bytes: &[u8]) {
        if !self.take_manifest(id) {
            return;
        }
        let Ok(text) = std::str::from_utf8(bytes) else {
            return self.manifest_failed(now, ManifestFault::Utf8, 0);
        };
        if manifest_declares_protection(text) {
            return self.fail(SmoothError::ManifestProtected);
        }
        match SmoothManifest::parse(text) {
            Ok(manifest) => self.accept_manifest(&manifest),
            Err(_) => self.manifest_failed(now, ManifestFault::Parse, 0),
        }
        self.advance(now);
    }

    /// A fragment body arrived for request `id`.
    pub fn on_fragment(&mut self, now: u64, id: RequestId, bytes: &[u8]) {
        let Some((si, chunk)) = self.take_fragment(id) else {
            return;
        };
        match fragment_state(bytes) {
            FragmentState::Clear => {}
            FragmentState::Encrypted => return self.fail(SmoothError::FragmentEncrypted),
            _ => return self.fragment_failed(now, si, chunk, None, 0),
        }
        if self.streams[si].local_track_id.is_none() {
            match fragment_track_id(bytes) {
                Ok(tid) => self.streams[si].local_track_id = Some(tid),
                Err(_) => return self.fragment_failed(now, si, chunk, None, 0),
            }
        }
        self.streams[si].failures = 0;
        self.streams[si].cursor = Cursor::Idle;
        let quality = self.streams[si].fetched.clone();
        if self.announced {
            self.emit_fragment(si, chunk.0, bytes, &quality);
        } else {
            self.streams[si].held.push(HeldFragment {
                start: chunk.0,
                body: bytes.to_vec(),
                quality,
            });
            self.try_announce();
        }
        self.advance(now);
    }

    /// The fetch for request `id` produced no usable body.
    pub fn on_failure(&mut self, now: u64, id: RequestId, failure: FetchFailure) {
        if self.take_manifest(id) {
            if failure.is_not_found() {
                self.outputs.push_back(Output::ManifestNotFound);
            }
            return self.manifest_failed(now, ManifestFault::Fetch, failure.retry_after_us);
        }
        if let Some((si, chunk)) = self.take_fragment(id) {
            self.fragment_failed(now, si, chunk, failure.status, failure.retry_after_us);
            self.advance(now);
        }
    }

    fn mint(&mut self) -> RequestId {
        self.next_request += 1;
        RequestId(self.next_request)
    }

    fn fail(&mut self, error: SmoothError) {
        self.fatal = true;
        self.outputs.push_back(Output::Fatal(error));
    }

    fn take_manifest(&mut self, id: RequestId) -> bool {
        if matches!(self.manifest, ManifestCursor::InFlight(m) if m == id) {
            self.manifest = ManifestCursor::Idle;
            return !self.fatal;
        }
        false
    }

    fn take_fragment(&mut self, id: RequestId) -> Option<(usize, Chunk)> {
        if self.fatal {
            return None;
        }
        self.streams
            .iter()
            .position(|s| matches!(s.cursor, Cursor::InFlight { id: i, .. } if i == id))
            .and_then(|si| match self.streams[si].cursor {
                Cursor::InFlight { chunk, .. } => Some((si, chunk)),
                _ => None,
            })
    }

    /// The wait before retry number `attempt` (1-based: the first failure is attempt 1). The
    /// shared [`RetrySchedule`] counts retries from 0, so the first failure takes its first step
    /// exactly and every later one a jittered step.
    fn backoff_us(&self, attempt: u32) -> u64 {
        let delay = self.retry.delay_for_attempt(attempt.saturating_sub(1));
        u64::try_from(delay.as_micros()).unwrap_or(u64::MAX)
    }

    fn fire_timers(&mut self, now: u64) {
        let due: Vec<usize> = self
            .streams
            .iter()
            .enumerate()
            .filter(|(_, s)| matches!(s.cursor, Cursor::Backoff { until, .. } if until <= now))
            .map(|(i, _)| i)
            .collect();
        for si in due {
            if let Cursor::Backoff { chunk, .. } = self.streams[si].cursor {
                self.issue_fragment(si, chunk);
            }
        }
        if self.manifest_at.is_some_and(|t| t <= now)
            && matches!(self.manifest, ManifestCursor::Idle)
        {
            self.manifest_at = None;
            let id = self.mint();
            self.manifest = ManifestCursor::InFlight(id);
            self.actions.push_back(Action::FetchManifest {
                id,
                url: self.manifest_url.clone(),
            });
        }
    }

    fn schedule(&mut self) {
        if !self.started {
            return;
        }
        for si in 0..self.streams.len() {
            let stream = &mut self.streams[si];
            if stream.abandoned || !matches!(stream.cursor, Cursor::Idle) {
                continue;
            }
            if let Some(chunk) = stream.plan.pop_front() {
                self.issue_fragment(si, chunk);
            }
        }
    }

    fn issue_fragment(&mut self, si: usize, chunk: Chunk) {
        let stream = &self.streams[si];
        let relative = stream
            .index
            .resolve_fragment_url(stream.quality.bitrate, chunk.0);
        let Ok(url) = url_ref::resolve(&self.manifest_url, &relative) else {
            self.streams[si].cursor = Cursor::Idle;
            self.outputs
                .push_back(Output::Dropped(Dropped::FragmentUrl {
                    stream: StreamId(si),
                }));
            return;
        };
        let id = self.mint();
        self.streams[si].fetched = self.streams[si].quality.clone();
        self.streams[si].cursor = Cursor::InFlight { id, chunk };
        self.actions.push_back(Action::FetchFragment {
            id,
            stream: StreamId(si),
            start: chunk.0,
            duration: chunk.1,
            url,
        });
    }

    fn manifest_failed(&mut self, now: u64, fault: ManifestFault, wait_us: u64) {
        self.manifest_failures += 1;
        if self.manifest_failures >= self.config.max_manifest_failures {
            return self.fail(SmoothError::Manifest(fault));
        }
        self.outputs
            .push_back(Output::Dropped(Dropped::Manifest(fault)));
        self.manifest_at =
            Some(now.saturating_add(self.backoff_us(self.manifest_failures).max(wait_us)));
    }

    fn fragment_failed(
        &mut self,
        now: u64,
        si: usize,
        chunk: Chunk,
        status: Option<u16>,
        wait_us: u64,
    ) {
        self.streams[si].failures += 1;
        if self.streams[si].failures >= self.config.max_fragment_failures {
            if !self.is_live {
                return self.fail(SmoothError::Fragment { status });
            }
            self.streams[si].failures = 0;
            self.streams[si].cursor = Cursor::Idle;
            self.outputs.push_back(Output::Dropped(Dropped::Fragment {
                stream: StreamId(si),
                status,
            }));
            self.outputs.push_back(Output::Discontinuity);
            return;
        }
        let missing = FetchFailure {
            status,
            retry_after_us: 0,
        }
        .is_not_found();
        if self.is_live && missing && matches!(self.manifest, ManifestCursor::Idle) {
            self.manifest_at = Some(now);
        }
        let until = now.saturating_add(self.backoff_us(self.streams[si].failures).max(wait_us));
        self.streams[si].cursor = Cursor::Backoff { until, chunk };
    }

    fn accept_manifest(&mut self, manifest: &SmoothManifest) {
        self.manifest_failures = 0;
        if self.started {
            return self.refresh_streams(manifest);
        }
        match self.build_streams(manifest) {
            Ok(true) => {
                self.started = true;
                self.is_live = manifest.is_live;
                self.offer_qualities(manifest);
            }
            Ok(false) => self.fail(SmoothError::NoUsableStream),
            Err(e) => self.fail(e),
        }
    }

    fn build_streams(&mut self, manifest: &SmoothManifest) -> Result<bool, SmoothError> {
        for index in &manifest.streams {
            if !matches!(index.stream_type, StreamType::Video | StreamType::Audio) {
                continue;
            }
            let pick = if primary_video(manifest).is_some_and(|v| std::ptr::eq(v, index)) {
                self.choose_video_quality(manifest, index)
            } else {
                0
            };
            let Some(quality) = index.qualities.get(pick).cloned() else {
                continue;
            };
            let Some(timescale) = timescale_of(manifest, index) else {
                continue;
            };
            let chunks = match index.enumerate_chunks() {
                Ok(chunks) => chunks,
                Err(e) => {
                    return Err(SmoothError::Timeline {
                        stream: self.streams.len(),
                        reason: e.to_string(),
                    });
                }
            };
            let chunks = if manifest.is_live {
                live_join(chunks, manifest, self.config.live_join_fragments)
            } else {
                chunks
            };
            let mut stream = Stream::new(index.clone(), timescale, quality);
            stream.plan_from(chunks);
            self.streams.push(stream);
        }
        Ok(!self.streams.is_empty())
    }

    fn quality_infos(manifest: &SmoothManifest, index: &StreamIndex) -> Vec<QualityInfo> {
        let timescale = timescale_of(manifest, index);
        index
            .qualities
            .iter()
            .map(|q| {
                let codecs = timescale
                    .and_then(|ts| {
                        track_spec_from_quality_level(
                            TrackId::new(FIRST_TRACK_ID),
                            ts,
                            index.stream_type,
                            q,
                        )
                        .ok()
                    })
                    .and_then(|spec| rfc6381_track_codec_string(&spec).ok());
                QualityInfo {
                    bitrate: q.bitrate,
                    resolution: q.width.zip(q.height),
                    codecs,
                }
            })
            .collect()
    }

    fn choose_video_quality(&mut self, manifest: &SmoothManifest, index: &StreamIndex) -> usize {
        let infos = Self::quality_infos(manifest, index);
        self.video_quality_count = infos.len();
        self.config.video_quality.choose(&infos)
    }

    fn offer_qualities(&mut self, manifest: &SmoothManifest) {
        let Some(video) = primary_video(manifest) else {
            return;
        };
        let qualities = Self::quality_infos(manifest, video);
        let chosen = self.config.video_quality.choose(&qualities);
        self.outputs
            .push_back(Output::Qualities { qualities, chosen });
    }

    fn refresh_streams(&mut self, manifest: &SmoothManifest) {
        self.is_live = manifest.is_live;
        let mut discontinuity = false;
        for si in 0..self.streams.len() {
            if self.streams[si].abandoned {
                continue;
            }
            let found = manifest
                .streams
                .iter()
                .find(|s| s.same_stream(&self.streams[si].index));
            let Some(found) = found else {
                self.streams[si].refresh_misses += 1;
                if self.streams[si].refresh_misses >= self.config.max_refresh_misses {
                    self.streams[si].abandoned = true;
                    self.streams[si].plan.clear();
                    self.outputs.push_back(Output::Dropped(Dropped::StreamLost {
                        stream: StreamId(si),
                    }));
                }
                continue;
            };
            self.streams[si].refresh_misses = 0;
            match found.enumerate_chunks() {
                Ok(chunks) => discontinuity |= self.streams[si].merge_refresh(chunks),
                Err(_) => {
                    self.outputs
                        .push_back(Output::Dropped(Dropped::Manifest(ManifestFault::Timeline)));
                }
            }
            self.streams[si].index = found.clone();
        }
        if discontinuity {
            self.outputs.push_back(Output::Discontinuity);
        }
    }

    fn try_announce(&mut self) {
        let live: Vec<usize> = (0..self.streams.len())
            .filter(|&i| !self.streams[i].abandoned)
            .collect();
        if self.announced || live.is_empty() {
            return;
        }
        if live
            .iter()
            .any(|&i| self.streams[i].local_track_id.is_none())
        {
            return;
        }
        let mut specs = Vec::new();
        for (position, &si) in live.iter().enumerate() {
            let global = TrackId::new(
                FIRST_TRACK_ID.saturating_add(u32::try_from(position).unwrap_or(u32::MAX)),
            );
            let local = self.streams[si].local_track_id.unwrap_or(FIRST_TRACK_ID);
            let quality = self.streams[si]
                .held
                .first()
                .map_or_else(|| self.streams[si].quality.clone(), |h| h.quality.clone());
            match self.setup_track(si, local, global, &quality) {
                Ok(spec) => {
                    self.streams[si].track = Some(global);
                    self.streams[si].spec = Some(spec.clone());
                    specs.push(spec);
                }
                Err(reason) => return self.fail(SmoothError::TrackSetup { reason }),
            }
        }
        self.announced = true;
        self.outputs.push_back(Output::Tracks(specs));
        for &si in &live {
            for held in std::mem::take(&mut self.streams[si].held) {
                self.emit_fragment(si, held.start, &held.body, &held.quality);
            }
        }
    }

    fn setup_track(
        &mut self,
        si: usize,
        local: u32,
        global: TrackId,
        quality: &broadcast_smooth::QualityLevel,
    ) -> Result<TrackSpec, String> {
        let stream = &self.streams[si];
        let mut spec = track_spec_from_quality_level(
            TrackId::new(local),
            stream.timescale,
            stream.index.stream_type,
            quality,
        )
        .map_err(|e| e.to_string())?;
        let init = build_init_segment(std::slice::from_ref(&spec), SYNTHETIC_MOVIE_TIMESCALE)
            .map_err(|e| e.to_string())?;
        self.streams[si].init = Some(init);
        self.streams[si].applied = quality.clone();
        spec.track_id = global;
        Ok(spec)
    }

    fn emit_fragment(
        &mut self,
        si: usize,
        start: u64,
        body: &[u8],
        quality: &broadcast_smooth::QualityLevel,
    ) {
        if self.streams[si].applied != *quality && !self.reconfigure(si, quality) {
            return self.outputs.push_back(Output::Dropped(Dropped::Fragment {
                stream: StreamId(si),
                status: None,
            }));
        }
        self.emit_samples(si, start, body);
    }

    fn reconfigure(&mut self, si: usize, quality: &broadcast_smooth::QualityLevel) -> bool {
        let (Some(local), Some(global)) = (self.streams[si].local_track_id, self.streams[si].track)
        else {
            return false;
        };
        let Ok(spec) = self.setup_track(si, local, global, quality) else {
            return false;
        };
        if self.streams[si].spec.as_ref() != Some(&spec) {
            self.streams[si].spec = Some(spec);
            let specs = self.streams.iter().filter_map(|s| s.spec.clone()).collect();
            self.outputs.push_back(Output::TracksChanged(specs));
        }
        true
    }

    fn emit_samples(&mut self, si: usize, start: u64, body: &[u8]) {
        let stream = &self.streams[si];
        let (Some(init), Some(local), Some(track)) =
            (stream.init.as_deref(), stream.local_track_id, stream.track)
        else {
            return;
        };
        match demux_fragment(init, local, body, start) {
            Ok(samples) => {
                for sample in samples {
                    self.outputs.push_back(Output::Sample { track, sample });
                }
            }
            Err(_) => self.outputs.push_back(Output::Dropped(Dropped::Fragment {
                stream: StreamId(si),
                status: None,
            })),
        }
    }

    fn advance(&mut self, now: u64) {
        if self.fatal || self.ended || !self.announced {
            return;
        }
        if !self.streams.iter().all(Stream::is_drained) {
            return;
        }
        if !self.is_live {
            self.ended = true;
            self.outputs.push_back(Output::Ended);
        } else if self.manifest_at.is_none() && matches!(self.manifest, ManifestCursor::Idle) {
            self.manifest_at = Some(now.saturating_add(self.refresh_wait_us()));
        }
    }

    fn refresh_wait_us(&self) -> u64 {
        let last = self.streams.iter().find(|s| !s.abandoned).and_then(|s| {
            s.index
                .enumerate_chunks()
                .ok()?
                .last()
                .map(|c| (c.1, s.timescale))
        });
        let half = last.map_or(self.config.refresh_max_us, |(d, ts)| {
            d.saturating_mul(MICROS_PER_SEC) / u64::from(ts).max(1) / REFRESH_DIVISOR
        });
        half.clamp(self.config.refresh_min_us, self.config.refresh_max_us)
    }
}
