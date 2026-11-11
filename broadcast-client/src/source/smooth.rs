#![allow(clippy::collapsible_if, clippy::redundant_clone)]
//! Smooth Streaming (MS-SSTR) pull source.

use alloc::collections::VecDeque;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use std::collections::HashMap;

use broadcast_common::Unpackage;
use broadcast_smooth::{QualityLevel, SmoothManifest, StreamIndex, StreamType};
use hls_runtime::client::{MasterSelection, VariantInfo};
use transmux::dash::rfc6381_track_codec_string;
use transmux::pipeline::build_init_segment;
use transmux::smooth_fragment::{
    FragmentState, fragment_state, fragment_tfxd, fragment_track_id, manifest_declares_protection,
};
use transmux::{Fmp4Demux, TrackId, TrackSpec, track_spec_from_quality_level};
use url::Url;

use crate::auth::{AuthDecision, AuthState};
use crate::error::Error;
use crate::events::{Action, Config, ErrorKind, Event, ResumeMode, Status};
use crate::playout::Source;
use crate::redirect;
use crate::source::fetch::conn::ConnAlloc;

const MICROS_PER_MILLI: u64 = 1_000;
const REDIRECT_STATUSES: [u16; 5] = [301, 302, 303, 307, 308];
const SYNTHETIC_MOVIE_TIMESCALE: u32 = 90_000;
const LIVE_JOIN_FRAGMENTS: usize = 2;
const MAX_REFRESH_MISSES: u32 = 5;

#[derive(Debug, Clone, Copy)]
enum ReqKind {
    Manifest,
    Fragment { stream_idx: usize },
}

struct Req {
    kind: ReqKind,
    url: Url,
    body: Vec<u8>,
    status: Option<u16>,
    headers: Vec<(String, String)>,
    started_us: u64,
    last_activity_us: u64,
    authed: bool,
    redirects: u32,
    stale_replayed: bool,
}

struct StreamRuntime {
    stream: StreamIndex,
    timescale: u64,
    quality: QualityLevel,
    chunks: Vec<(u64, u64)>,
    next: usize,
    in_flight: bool,
    current_idx: Option<usize>,
    current_start: u64,
    current_is_first: bool,
    track_id: Option<u32>,
    init_bytes: Option<Vec<u8>>,
    first_body: Option<Vec<u8>>,
    failures: u32,
    retry_at: Option<u64>,
    frontier: u64,
    refresh_misses: u32,
    track_spec: Option<TrackSpec>,
}

pub(crate) struct SmoothSource {
    manifest_url: Url,
    config: Config,
    conn_alloc: ConnAlloc,
    reqs: HashMap<u32, Req>,
    manifest: Option<SmoothManifest>,
    streams: Vec<StreamRuntime>,
    first_pending: bool,
    tracks_announced: bool,
    first_bodies: Vec<Option<Vec<u8>>>,
    chosen_video: usize,
    pending_variant: Option<usize>,
    is_live: bool,
    ended: bool,
    fatal: bool,
    now: u64,
    actions: VecDeque<Action>,
    events: VecDeque<Event>,
    refresh_at: Option<u64>,
    manifest_failures: u32,
    variants_emitted: bool,
    parked: Vec<Req>,
}

impl SmoothSource {
    pub(crate) fn new(url: &Url, config: &Config, conn_alloc: ConnAlloc) -> Self {
        let mut src = Self {
            manifest_url: url.clone(),
            config: config.clone(),
            conn_alloc,
            reqs: HashMap::new(),
            manifest: None,
            streams: Vec::new(),
            first_pending: false,
            tracks_announced: false,
            first_bodies: Vec::new(),
            chosen_video: 0,
            pending_variant: None,
            is_live: false,
            ended: false,
            fatal: false,
            now: 0,
            actions: VecDeque::new(),
            events: VecDeque::new(),
            refresh_at: None,
            manifest_failures: 0,
            variants_emitted: false,
            parked: Vec::new(),
        };
        src.events.push_back(Event::Status(Status::Establishing));
        src.fetch_manifest(url.clone());
        src
    }

    fn fetch_manifest(&mut self, url: Url) {
        let conn = self.conn_alloc.lease().id();
        self.reqs.insert(
            conn,
            Req {
                kind: ReqKind::Manifest,
                url: url.clone(),
                body: Vec::new(),
                status: None,
                headers: Vec::new(),
                started_us: self.now,
                last_activity_us: self.now,
                authed: false,
                redirects: 0,
                stale_replayed: false,
            },
        );
        self.actions.push_back(Action::HttpRequest {
            conn,
            method: "GET".to_string(),
            url: url.to_string(),
            headers: Vec::new(),
            byte_range: None,
            identity: None,
        });
        self.actions.push_back(Action::WakeAt(
            self.now
                .saturating_add(self.config.segment_inactivity_timeout_us.max(1)),
        ));
    }

    pub(crate) fn take_actions(&mut self) -> VecDeque<Action> {
        core::mem::take(&mut self.actions)
    }

    pub(crate) fn take_events(&mut self) -> VecDeque<Event> {
        core::mem::take(&mut self.events)
    }

    pub(crate) fn conns(&self) -> Vec<u32> {
        self.reqs.keys().copied().collect()
    }

    pub(crate) fn is_ended(&self) -> bool {
        self.ended
    }

    pub(crate) fn playout_source(&self) -> Source {
        if self.is_live && !self.ended {
            Source::Live {
                target_duration_secs: None,
            }
        } else {
            Source::Vod
        }
    }

    pub(crate) fn poll(&mut self, now: u64, _auth: &mut AuthState) {
        self.now = now;
        if self.fatal || self.ended {
            return;
        }
        // inactivity timeout handling
        let timeout = self.config.segment_inactivity_timeout_us;
        let mut stalled: Vec<u32> = Vec::new();
        for (conn, req) in &self.reqs {
            let last = if req.last_activity_us == 0 {
                req.started_us
            } else {
                req.last_activity_us
            };
            if now.saturating_sub(last) >= timeout {
                stalled.push(*conn);
            }
        }
        for conn in stalled {
            if self.fatal || self.ended {
                break;
            }
            if let Some(req) = self.reqs.remove(&conn) {
                self.actions.push_back(Action::Close { conn });
                self.events.push_back(Event::Status(Status::RequestStalled {
                    resume: ResumeMode::Full,
                }));
                // retry like transport failure
                match req.kind {
                    ReqKind::Manifest => self.report_bad_manifest("inactivity", now),
                    ReqKind::Fragment { stream_idx: si } => {
                        self.fragment_failed(si, None, now);
                    }
                }
            }
        }
        if self.fatal || self.ended {
            return;
        }
        // retry timers
        for i in 0..self.streams.len() {
            if let Some(t) = self.streams[i].retry_at {
                if now >= t {
                    self.streams[i].retry_at = None;
                    self.schedule_stream(i);
                }
            }
        }
        if let Some(t) = self.refresh_at {
            if now >= t {
                self.refresh_at = None;
                self.fetch_manifest(self.manifest_url.clone());
            }
        }
        self.schedule();
        self.check_completion();
    }

    fn schedule(&mut self) {
        if self.fatal || self.ended || self.manifest.is_none() {
            return;
        }
        for i in 0..self.streams.len() {
            if !self.streams[i].in_flight && self.streams[i].next < self.streams[i].chunks.len() {
                self.schedule_stream(i);
            }
        }
    }

    fn schedule_stream(&mut self, i: usize) {
        if self.streams[i].in_flight {
            return;
        }
        if self.streams[i].next >= self.streams[i].chunks.len() {
            return;
        }
        let idx = self.streams[i].next;
        let start = self.streams[i].chunks[idx].0;
        self.streams[i].in_flight = true;
        self.streams[i].current_idx = Some(idx);
        self.streams[i].current_start = start;
        self.streams[i].current_is_first = self.streams[i].track_id.is_none();
        let quality = self.streams[i].quality.clone();
        let bitrate = quality.bitrate;
        let stream = self.streams[i].stream.clone();
        let url_str = stream.resolve_fragment_url(bitrate, start);
        let url = self
            .manifest_url
            .join(&url_str)
            .unwrap_or_else(|_| self.manifest_url.clone());
        let conn = self.conn_alloc.lease().id();
        self.reqs.insert(
            conn,
            Req {
                kind: ReqKind::Fragment { stream_idx: i },
                url: url.clone(),
                body: Vec::new(),
                status: None,
                headers: Vec::new(),
                started_us: self.now,
                last_activity_us: self.now,
                authed: false,
                redirects: 0,
                stale_replayed: false,
            },
        );
        self.actions.push_back(Action::HttpRequest {
            conn,
            method: "GET".to_string(),
            url: url.to_string(),
            headers: Vec::new(),
            byte_range: None,
            identity: None,
        });
        self.actions.push_back(Action::WakeAt(
            self.now
                .saturating_add(self.config.segment_inactivity_timeout_us.max(1)),
        ));
    }

    pub(crate) fn on_response(
        &mut self,
        conn: u32,
        status: u16,
        headers: &[(String, String)],
        now: u64,
        _wall: Option<&crate::source::fetch::wall::WallClock>,
        auth: &mut AuthState,
    ) {
        let Some(mut req) = self.reqs.remove(&conn) else {
            return;
        };
        auth.receive_cookies(&req.url, headers, now);
        if REDIRECT_STATUSES.contains(&status) {
            self.actions.push_back(Action::Close { conn });
            match redirect::resolve(&req.url, headers, req.redirects, self.config.max_redirects) {
                Some(target) => {
                    req.redirects = req.redirects.saturating_add(1);
                    req.url = target;
                    if matches!(req.kind, ReqKind::Manifest) {
                        self.manifest_url = req.url.clone();
                    }
                    let url_str = req.url.to_string();
                    let h = auth.request_headers(&req.url, now);
                    let new_conn = self.conn_alloc.lease().id();
                    req.started_us = now;
                    req.last_activity_us = now;
                    self.reqs.insert(new_conn, req);
                    self.actions.push_back(Action::HttpRequest {
                        conn: new_conn,
                        method: "GET".to_string(),
                        url: url_str,
                        headers: h,
                        byte_range: None,
                        identity: None,
                    });
                    self.actions.push_back(Action::WakeAt(
                        now.saturating_add(self.config.segment_inactivity_timeout_us.max(1)),
                    ));
                }
                None => {
                    if matches!(req.kind, ReqKind::Manifest) {
                        self.report_bad_manifest("redirect", now);
                    }
                }
            }
            return;
        }
        if (status == 401 || status == 407) && req.url.origin() == auth.origin().origin() {
            self.actions.push_back(Action::Close { conn });
            let url_str = req.url.to_string();
            let url_clone = req.url.clone();
            match auth.handle_unauthorized(status, headers, req.authed, req.stale_replayed) {
                AuthDecision::Replay => {
                    req.authed = true;
                    let h = auth.request_headers(&url_clone, now);
                    let new_conn = self.conn_alloc.lease().id();
                    req.started_us = now;
                    req.last_activity_us = now;
                    self.reqs.insert(new_conn, req);
                    self.actions.push_back(Action::HttpRequest {
                        conn: new_conn,
                        method: "GET".to_string(),
                        url: url_str,
                        headers: h,
                        byte_range: None,
                        identity: None,
                    });
                    self.actions.push_back(Action::WakeAt(
                        now.saturating_add(self.config.segment_inactivity_timeout_us.max(1)),
                    ));
                }
                AuthDecision::ReplayStale => {
                    req.authed = true;
                    req.stale_replayed = true;
                    let h = auth.request_headers(&url_clone, now);
                    let new_conn = self.conn_alloc.lease().id();
                    req.started_us = now;
                    req.last_activity_us = now;
                    self.reqs.insert(new_conn, req);
                    self.actions.push_back(Action::HttpRequest {
                        conn: new_conn,
                        method: "GET".to_string(),
                        url: url_str,
                        headers: h,
                        byte_range: None,
                        identity: None,
                    });
                    self.actions.push_back(Action::WakeAt(
                        now.saturating_add(self.config.segment_inactivity_timeout_us.max(1)),
                    ));
                }
                AuthDecision::Wait(events) => {
                    self.events.extend(events);
                    self.parked.push(req);
                }
            }
            return;
        }
        if !(200..300).contains(&status) {
            self.actions.push_back(Action::Close { conn });
            match req.kind {
                ReqKind::Manifest => {
                    if status == 404 || status == 410 {
                        self.events.push_back(Event::Status(Status::NotFound));
                    }
                    self.report_bad_manifest("fetch_failed", now);
                }
                ReqKind::Fragment { stream_idx: si } => {
                    self.fragment_failed(si, Some(status), now);
                }
            }
            return;
        }
        req.status = Some(status);
        req.headers = headers.to_vec();
        if req.started_us == 0 {
            req.started_us = now;
        }
        req.last_activity_us = now;
        self.reqs.insert(conn, req);
    }

    pub(crate) fn on_data(&mut self, conn: u32, bytes: &[u8], now: u64, _auth: &mut AuthState) {
        if let Some(req) = self.reqs.get_mut(&conn) {
            req.body.extend_from_slice(bytes);
            req.last_activity_us = now;
            if req.started_us == 0 {
                req.started_us = now;
            }
        }
    }

    pub(crate) fn on_closed(
        &mut self,
        conn: u32,
        error: Option<&str>,
        now: u64,
        _auth: &mut AuthState,
    ) {
        let req = match self.reqs.remove(&conn) {
            Some(r) => r,
            None => return,
        };
        if error.is_some() {
            match req.kind {
                ReqKind::Manifest => {
                    self.report_bad_manifest("transport", now);
                }
                ReqKind::Fragment { stream_idx: si } => {
                    self.fragment_failed(si, None, now);
                }
            }
            return;
        }
        let status = req.status.unwrap_or(0);
        match req.kind {
            ReqKind::Manifest => {
                if !(200..300).contains(&status) {
                    if status == 404 || status == 410 {
                        self.events.push_back(Event::Status(Status::NotFound));
                    }
                    self.report_bad_manifest("fetch_failed", now);
                    return;
                }
                let text = match String::from_utf8(req.body.clone()) {
                    Ok(t) => t,
                    Err(_) => {
                        self.report_bad_manifest("invalid_utf8", now);
                        return;
                    }
                };
                if manifest_declares_protection(&text) {
                    self.events.push_back(Event::Error {
                        kind: ErrorKind::UnsupportedEncryption {
                            method: "protection".to_string(),
                        },
                        fatal: true,
                    });
                    self.fatal = true;
                    return;
                }
                match SmoothManifest::parse(&text) {
                    Err(_) => {
                        self.report_bad_manifest("xml", now);
                    }
                    Ok(m) => {
                        let was_refresh = self.manifest.is_some();
                        self.manifest_failures = 0;
                        if was_refresh {
                            let prev_live = self.is_live;
                            self.is_live = m.is_live;
                            // refresh streams
                            for si in 0..self.streams.len() {
                                // find matching stream
                                let old_stream = self.streams[si].stream.clone();
                                if let Some(found) =
                                    m.streams.iter().find(|s| s.same_stream(&old_stream))
                                {
                                    self.streams[si].refresh_misses = 0;
                                    // new chunks at/after frontier
                                    let frontier = self.streams[si].frontier;
                                    let new_chunks = found.enumerate_chunks().unwrap_or_default();
                                    // filter
                                    let mut filtered: Vec<(u64, u64)> = new_chunks
                                        .iter()
                                        .copied()
                                        .filter(|c| c.0 >= frontier)
                                        .collect();
                                    if filtered.is_empty() {
                                        // window moved? if first new start > frontier
                                        if let Some(first) = new_chunks.first() {
                                            if first.0 > frontier {
                                                self.events.push_back(Event::Discontinuity);
                                            }
                                        }
                                        filtered = new_chunks;
                                    } else if let Some(first) = new_chunks.first() {
                                        if first.0 > frontier {
                                            self.events.push_back(Event::Discontinuity);
                                            filtered = new_chunks;
                                        }
                                    }
                                    if !self.is_live && prev_live {
                                        // ended after drain handled by completion
                                    }
                                    self.streams[si].chunks = filtered;
                                    self.streams[si].next = 0;
                                    self.streams[si].in_flight = false;
                                } else {
                                    self.streams[si].refresh_misses += 1;
                                    if self.streams[si].refresh_misses >= MAX_REFRESH_MISSES {
                                        // abandon: mark chunks empty
                                        self.streams[si].chunks.clear();
                                    }
                                }
                            }
                            // remove abandoned streams? leave empty
                            if !m.is_live {
                                self.is_live = false;
                            }
                        } else {
                            self.is_live = m.is_live;
                            self.build_streams(&m);
                        }
                        self.manifest = Some(m.clone());
                        self.emit_variants_if_needed();
                        self.schedule();
                    }
                }
            }
            ReqKind::Fragment { stream_idx: si } => {
                if si >= self.streams.len() {
                    return;
                }
                if !(200..300).contains(&status) {
                    self.fragment_failed(si, Some(status), now);
                    return;
                }
                let body = req.body.clone();
                if fragment_state(&body) == FragmentState::Encrypted {
                    self.events.push_back(Event::Error {
                        kind: ErrorKind::UnsupportedEncryption {
                            method: "fragment".to_string(),
                        },
                        fatal: true,
                    });
                    self.fatal = true;
                    self.streams[si].in_flight = false;
                    return;
                }
                let is_first = self.streams[si].current_is_first;
                let start_time = self.streams[si].current_start;
                if self.streams[si].track_id.is_none() {
                    let tid = fragment_track_id(&body).unwrap_or(if si == 0 { 1 } else { 2 });
                    self.streams[si].track_id = Some(tid);
                    self.streams[si].first_body = Some(body.clone());
                }
                if let Some(idx) = self.streams[si].current_idx {
                    self.streams[si].next = idx + 1;
                }
                self.streams[si].in_flight = false;
                self.streams[si].failures = 0;
                self.streams[si].frontier = start_time;
                if !self.tracks_announced {
                    let all_first = self.streams.iter().all(|s| s.first_body.is_some());
                    if all_first {
                        let mut specs = Vec::new();
                        for (i, s) in self.streams.iter_mut().enumerate() {
                            let tid = s.track_id.unwrap_or(if i == 0 { 1 } else { 2 });
                            let ts = s.timescale as u32;
                            let spec = track_spec_from_quality_level(
                                TrackId::new(tid),
                                ts,
                                s.stream.stream_type,
                                &s.quality,
                            )
                            .ok();
                            s.track_spec = spec.clone();
                            if let Some(sp) = spec {
                                let init = build_init_segment(
                                    std::slice::from_ref(&sp),
                                    SYNTHETIC_MOVIE_TIMESCALE,
                                )
                                .ok();
                                s.init_bytes = init;
                                specs.push(sp);
                            }
                        }
                        self.events.push_back(Event::Tracks(specs));
                        self.tracks_announced = true;
                        self.emit_variants_if_needed();
                        for i in 0..self.streams.len() {
                            if let Some(b) = self.streams[i].first_body.take() {
                                let start =
                                    self.streams[i].chunks.first().map(|c| c.0).unwrap_or(0);
                                self.process_fragment(i, b, start);
                            }
                        }
                    }
                }
                if self.tracks_announced && !is_first {
                    self.process_fragment(si, body, start_time);
                }
                self.schedule();
                self.check_completion();
            }
        }
    }

    fn process_fragment(&mut self, si: usize, body: Vec<u8>, start_time: u64) {
        if si >= self.streams.len() {
            return;
        }
        let init = match self.streams[si].init_bytes.clone() {
            Some(i) => i,
            None => return,
        };
        let mut data = init;
        data.extend_from_slice(&body);
        let media = match Fmp4Demux::new().unpackage(&data) {
            Ok(m) => m,
            Err(_) => return,
        };
        for track in media.tracks {
            let tid = track.spec.track_id;
            let desired = fragment_tfxd(&body)
                .map(|t| t.fragment_absolute_time)
                .unwrap_or(start_time);
            if track.samples.is_empty() {
                continue;
            }
            let first_dts = track.samples[0].dts.unwrap_or(0);
            let offset = desired as i64 - first_dts;
            for mut s in track.samples {
                if let Some(d) = s.dts {
                    s.dts = Some(d + offset);
                }
                if let Some(p) = s.pts {
                    s.pts = Some(p + offset);
                }
                self.events.push_back(Event::Sample {
                    track: tid,
                    sample: s,
                    join: None,
                });
            }
        }
    }

    fn build_streams(&mut self, m: &SmoothManifest) {
        // select video stream (first video)
        let video_si = m
            .streams
            .iter()
            .find(|s| s.stream_type == StreamType::Video);
        let audio_si = m
            .streams
            .iter()
            .find(|s| s.stream_type == StreamType::Audio);
        // chosen video variant
        if let Some(vs) = video_si {
            let variants: Vec<VariantInfo> = vs
                .qualities
                .iter()
                .map(|q| {
                    let ts = vs.timescale.unwrap_or(m.timescale);
                    let spec = track_spec_from_quality_level(
                        TrackId::new(1),
                        ts as u32,
                        StreamType::Video,
                        q,
                    )
                    .ok();
                    let codecs = spec
                        .as_ref()
                        .and_then(|s| rfc6381_track_codec_string(s).ok());
                    VariantInfo {
                        url: String::new(),
                        bandwidth: q.bitrate as u32,
                        average_bandwidth: None,
                        resolution: q.width.zip(q.height),
                        codecs,
                        audio_group: None,
                        subtitles_group: None,
                    }
                })
                .collect();
            self.chosen_video =
                hls_runtime::client::choose_variant(&variants, &self.config.variant_policy);
        }
        // build stream runtime for video
        if let Some(vs) = video_si.cloned() {
            let ts = vs.timescale.unwrap_or(m.timescale);
            let qi = self.chosen_video.min(vs.qualities.len().saturating_sub(1));
            let quality = vs.qualities[qi].clone();
            let chunks = vs.enumerate_chunks().unwrap_or_default();
            let chunks = if self.is_live {
                self.live_slice(&chunks, m)
            } else {
                chunks
            };
            self.streams.push(StreamRuntime {
                stream: vs,
                timescale: ts,
                quality,
                chunks,
                next: 0,
                in_flight: false,
                current_idx: None,
                current_start: 0,
                current_is_first: true,
                track_id: None,
                init_bytes: None,
                first_body: None,
                failures: 0,
                retry_at: None,
                frontier: 0,
                refresh_misses: 0,
                track_spec: None,
            });
        }
        // audio
        if let Some(as_) = audio_si.cloned() {
            let ts = as_.timescale.unwrap_or(m.timescale);
            let qi = 0;
            let quality = as_.qualities[qi].clone();
            let chunks = as_.enumerate_chunks().unwrap_or_default();
            let chunks = if self.is_live {
                self.live_slice(&chunks, m)
            } else {
                chunks
            };
            self.streams.push(StreamRuntime {
                stream: as_,
                timescale: ts,
                quality,
                chunks,
                next: 0,
                in_flight: false,
                current_idx: None,
                current_start: 0,
                current_is_first: true,
                track_id: None,
                init_bytes: None,
                first_body: None,
                failures: 0,
                retry_at: None,
                frontier: 0,
                refresh_misses: 0,
                track_spec: None,
            });
        }
        self.first_pending = true;
        self.first_bodies = vec![None; self.streams.len()];
        // emit variants
        self.emit_variants_if_needed();
    }

    fn live_slice(&self, chunks: &[(u64, u64)], m: &SmoothManifest) -> Vec<(u64, u64)> {
        if chunks.is_empty() {
            return chunks.to_vec();
        }
        // determine join index
        let edge_idx = chunks.len() - 1;
        let mut join_idx = edge_idx.saturating_sub(LIVE_JOIN_FRAGMENTS);
        // DVR window
        if let Some(dvr) = m.dvr_window_length {
            if dvr > 0 {
                let last_start = chunks[edge_idx].0;
                let window_start = last_start.saturating_sub(dvr);
                // first chunk with start >= window_start
                let mut widx = 0;
                for (i, c) in chunks.iter().enumerate() {
                    if c.0 >= window_start {
                        widx = i;
                        break;
                    }
                }
                if widx > join_idx {
                    join_idx = widx;
                }
            }
        }
        let join_start = chunks[join_idx].0;
        // slice each stream's chunks starting at first >= join_start
        let mut idx = 0;
        for (i, c) in chunks.iter().enumerate() {
            if c.0 >= join_start {
                idx = i;
                break;
            }
        }
        chunks[idx..].to_vec()
    }

    fn emit_variants_if_needed(&mut self) {
        if self.variants_emitted {
            return;
        }
        if self.streams.is_empty() {
            return;
        }
        // find first video stream index in self.streams
        // build variants from first stream's stream? We stored qualities? Need access. We'll just emit with empty if not available. But tests check variants. Let's build from first stream's stream_index qualities.
        // We need manifest for qualities. Use self.manifest.
        if let Some(m) = &self.manifest {
            if let Some(vs) = m
                .streams
                .iter()
                .find(|s| s.stream_type == StreamType::Video)
            {
                let variants: Vec<VariantInfo> = vs
                    .qualities
                    .iter()
                    .map(|q| {
                        let ts = vs.timescale.unwrap_or(m.timescale);
                        let spec = track_spec_from_quality_level(
                            TrackId::new(1),
                            ts as u32,
                            StreamType::Video,
                            q,
                        )
                        .ok();
                        let codecs = spec
                            .as_ref()
                            .and_then(|s| rfc6381_track_codec_string(s).ok());
                        VariantInfo {
                            url: String::new(),
                            bandwidth: q.bitrate as u32,
                            average_bandwidth: None,
                            resolution: q.width.zip(q.height),
                            codecs,
                            audio_group: None,
                            subtitles_group: None,
                        }
                    })
                    .collect();
                let chosen = self.chosen_video.min(variants.len().saturating_sub(1));
                self.events.push_back(Event::Variants(MasterSelection {
                    variants,
                    chosen,
                    renditions: Vec::new(),
                }));
                self.variants_emitted = true;
            }
        }
    }

    fn fragment_failed(&mut self, si: usize, status: Option<u16>, now: u64) {
        if si >= self.streams.len() {
            return;
        }
        let max = self.config.max_segment_failures;
        self.streams[si].failures += 1;
        self.streams[si].in_flight = false;
        if self.streams[si].failures >= max {
            if self.is_live {
                self.events.push_back(Event::Error {
                    kind: ErrorKind::SegmentFetchFailed { status },
                    fatal: false,
                });
                self.events.push_back(Event::Discontinuity);
                if let Some(idx) = self.streams[si].current_idx {
                    self.streams[si].next = idx + 1;
                }
            } else {
                self.events.push_back(Event::Error {
                    kind: ErrorKind::SegmentFetchFailed { status },
                    fatal: true,
                });
                self.fatal = true;
            }
        } else {
            if let Some(st) = status {
                if (st == 404 || st == 410) && self.is_live {
                    self.fetch_manifest(self.manifest_url.clone());
                }
            }
            let delay = self.backoff_delay(self.streams[si].failures);
            self.streams[si].retry_at =
                Some(now.saturating_add(delay.saturating_mul(MICROS_PER_MILLI)));
            self.actions.push_back(Action::WakeAt(
                now.saturating_add(delay.saturating_mul(MICROS_PER_MILLI)),
            ));
        }
    }

    fn report_bad_manifest(&mut self, _reason: &str, now: u64) {
        self.manifest_failures += 1;
        let fatal = self.manifest_failures >= self.config.max_key_failures;
        self.events.push_back(Event::Error {
            kind: ErrorKind::SmoothManifest {
                reason: "smooth_manifest",
            },
            fatal,
        });
        if fatal {
            self.fatal = true;
        }
        let delay = self.backoff_delay(self.manifest_failures);
        self.refresh_at = Some(now.saturating_add(delay.saturating_mul(MICROS_PER_MILLI)));
        self.actions.push_back(Action::WakeAt(
            now.saturating_add(delay.saturating_mul(MICROS_PER_MILLI)),
        ));
    }

    fn backoff_delay(&self, n: u32) -> u64 {
        let idx = n.saturating_sub(1) as usize;
        self.config
            .backoff_ms
            .get(idx)
            .copied()
            .unwrap_or(self.config.backoff_cap_ms)
            .max(1)
    }

    pub(crate) fn select_variant(
        &mut self,
        index: usize,
        _now: u64,
        _auth: &mut AuthState,
    ) -> Result<(), Error> {
        if self.ended {
            return Ok(());
        }
        // validate against variants count
        let count = self
            .manifest
            .as_ref()
            .and_then(|m| {
                m.streams
                    .iter()
                    .find(|s| s.stream_type == StreamType::Video)
                    .map(|s| s.qualities.len())
            })
            .unwrap_or(0);
        if index >= count {
            return Err(Error::NoSuchVariant(index));
        }
        self.chosen_video = index;
        self.pending_variant = Some(index);
        Ok(())
    }

    pub(crate) fn select_audio(
        &mut self,
        _choice: Option<usize>,
        _now: u64,
        _auth: &mut AuthState,
    ) -> Result<(), Error> {
        Ok(())
    }

    pub(crate) fn reconnect(&mut self, _now: u64, _auth: &mut AuthState) {}

    pub(crate) fn resume(&mut self, now: u64, auth: &mut AuthState) {
        let parked = core::mem::take(&mut self.parked);
        for req in parked {
            let url_str = req.url.to_string();
            let h = auth.request_headers(&req.url, now);
            let new_conn = self.conn_alloc.lease().id();
            let mut req = req;
            req.started_us = now;
            req.last_activity_us = now;
            self.reqs.insert(new_conn, req);
            self.actions.push_back(Action::HttpRequest {
                conn: new_conn,
                method: "GET".to_string(),
                url: url_str,
                headers: h,
                byte_range: None,
                identity: None,
            });
            self.actions.push_back(Action::WakeAt(
                now.saturating_add(self.config.segment_inactivity_timeout_us.max(1)),
            ));
        }
        if !auth.is_waiting() {
            self.schedule();
        }
    }

    pub(crate) fn set_wall_clock(&mut self, _now_us: u64, _unix_us: u64) {}

    fn check_completion(&mut self) {
        if self.ended || self.fatal {
            return;
        }
        if !self.tracks_announced {
            return;
        }
        let all_done = self
            .streams
            .iter()
            .all(|s| !s.in_flight && s.next >= s.chunks.len());
        if all_done {
            if self.is_live {
                if self.refresh_at.is_none() {
                    let last_dur = self
                        .streams
                        .first()
                        .and_then(|s| s.chunks.last().map(|c| c.1))
                        .unwrap_or(1_000_000);
                    let half = (last_dur / 2).clamp(500_000, 5_000_000);
                    let t = self.now.saturating_add(half);
                    self.refresh_at = Some(t);
                    self.actions.push_back(Action::WakeAt(t));
                }
            } else {
                self.events.push_back(Event::Status(Status::Ended));
                self.ended = true;
            }
        }
    }
}
