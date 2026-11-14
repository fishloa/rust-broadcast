//! Smooth Streaming (MS-SSTR) pull source: the HTTP, auth and event plumbing around
//! the shared [`smooth_runtime::SmoothClient`] engine.

use alloc::collections::VecDeque;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use std::collections::HashMap;

use hls_runtime::client::{MasterSelection, VariantInfo, choose_variant};
use smooth_runtime::{
    Action, Dropped, FetchFailure, Output, QualityInfo, QualitySelector, RequestId, SmoothClient,
    SmoothClientConfig, SmoothError,
};
use url::Url;

use crate::error::{Error, TransportError};
use crate::events::{Action as HostAction, Config, ErrorKind, Event, ResumeMode, Status};
use crate::playout::Source;
use crate::redirect;
use crate::source::fetch::conn::ConnAlloc;
use crate::source::fetch::deadline;
use crate::source::fetch::retry::duration_us;
use broadcast_auth::{AuthDecision, AuthState};
use broadcast_common::stage::Timestamp;

const REDIRECT_STATUSES: [u16; 5] = [301, 302, 303, 307, 308];
const HTTP_UNAUTHORIZED: u16 = 401;
const HTTP_PROXY_AUTH_REQUIRED: u16 = 407;
const SMOOTH_MANIFEST_REASON: &str = "smooth_manifest";

#[derive(Debug, Clone, Copy)]
enum ReqKind {
    Manifest,
    Fragment,
}

struct Req {
    kind: ReqKind,
    id: RequestId,
    url: Url,
    body: Vec<u8>,
    status: Option<u16>,
    started_us: u64,
    last_activity_us: u64,
    authed: bool,
    redirects: u32,
    stale_replayed: bool,
}

pub(crate) struct SmoothSource {
    config: Config,
    conn_alloc: ConnAlloc,
    client: SmoothClient,
    reqs: HashMap<u32, Req>,
    parked: Vec<Req>,
    actions: VecDeque<HostAction>,
    events: VecDeque<Event>,
    armed_wake: Option<u64>,
}

fn variant_info(q: &QualityInfo) -> VariantInfo {
    let mut info = VariantInfo::new(String::new(), u32::try_from(q.bitrate).unwrap_or(u32::MAX));
    info.resolution = q.resolution;
    info.codecs = q.codecs.clone();
    info
}

fn engine_config(url: &Url, config: &Config, jitter_seed: u64) -> SmoothClientConfig {
    let policy = config.variant_policy.clone();
    SmoothClientConfig::new(url.as_str())
        .with_max_fragment_failures(config.max_segment_failures)
        .with_max_manifest_failures(config.max_key_failures)
        .with_backoff(config.backoff.clone())
        .with_jitter_seed(jitter_seed)
        .with_video_quality(QualitySelector::new(move |qualities| {
            let variants: Vec<VariantInfo> = qualities.iter().map(variant_info).collect();
            choose_variant(&variants, &policy)
        }))
}

fn fatal_error_kind(error: &SmoothError) -> (ErrorKind, bool) {
    let kind = match error {
        SmoothError::ManifestProtected => ErrorKind::UnsupportedEncryption {
            method: "protection".to_string(),
        },
        SmoothError::FragmentEncrypted => ErrorKind::UnsupportedEncryption {
            method: "fragment".to_string(),
        },
        SmoothError::Fragment { status } => ErrorKind::SegmentFetchFailed { status: *status },
        _ => ErrorKind::SmoothManifest {
            reason: SMOOTH_MANIFEST_REASON,
        },
    };
    (kind, true)
}

impl SmoothSource {
    pub(crate) fn new(url: &Url, config: &Config, conn_alloc: ConnAlloc, jitter_seed: u64) -> Self {
        let mut source = Self {
            config: config.clone(),
            conn_alloc,
            client: SmoothClient::new(engine_config(url, config, jitter_seed)),
            reqs: HashMap::new(),
            parked: Vec::new(),
            actions: VecDeque::new(),
            events: VecDeque::new(),
            armed_wake: None,
        };
        source.events.push_back(Event::Status(Status::Establishing));
        source.pump(0);
        source
    }

    pub(crate) fn take_actions(&mut self) -> VecDeque<HostAction> {
        core::mem::take(&mut self.actions)
    }

    pub(crate) fn take_events(&mut self) -> VecDeque<Event> {
        core::mem::take(&mut self.events)
    }

    pub(crate) fn conns(&self) -> Vec<u32> {
        self.reqs.keys().copied().collect()
    }

    pub(crate) fn is_ended(&self) -> bool {
        self.client.is_ended()
    }

    pub(crate) fn playout_source(&self) -> Source {
        if self.client.is_live() {
            Source::Live {
                target_duration_secs: None,
            }
        } else {
            Source::Vod
        }
    }

    fn inactivity_us(&self) -> u64 {
        duration_us(self.config.segment_inactivity_timeout)
    }

    fn arm_inactivity(&mut self, now: u64) {
        // A deadline past the end of the clock can never be reached: no wake for it.
        if let Some(at) = deadline(
            Timestamp::from_micros(now),
            self.config.segment_inactivity_timeout,
        ) {
            self.actions.push_back(HostAction::WakeAt(at));
        }
    }

    fn start(&mut self, mut req: Req, headers: Vec<(String, String)>, now: u64) {
        let conn = self.conn_alloc.lease().id();
        req.started_us = now;
        req.last_activity_us = now;
        self.actions.push_back(HostAction::HttpRequest {
            conn,
            method: "GET".to_string(),
            url: req.url.to_string(),
            headers,
            byte_range: None,
            identity: None,
        });
        self.reqs.insert(conn, req);
        self.arm_inactivity(now);
    }

    fn pump(&mut self, now: u64) {
        while let Some(action) = self.client.poll(Timestamp::from_micros(now)) {
            let (kind, id, url) = match action {
                Action::FetchManifest { id, url } => (ReqKind::Manifest, id, url),
                Action::FetchFragment { id, url, .. } => (ReqKind::Fragment, id, url),
                _ => continue,
            };
            let req = Req {
                kind,
                id,
                url,
                body: Vec::new(),
                status: None,
                started_us: now,
                last_activity_us: now,
                authed: false,
                redirects: 0,
                stale_replayed: false,
            };
            self.start(req, Vec::new(), now);
        }
        while let Some(output) = self.client.next_output() {
            self.map_output(output);
        }
        let deadline = self.client.next_deadline().filter(|&t| t > now);
        if deadline != self.armed_wake {
            self.armed_wake = deadline;
            if let Some(t) = deadline {
                self.actions
                    .push_back(HostAction::WakeAt(Timestamp::from_micros(t)));
            }
        }
    }

    fn map_output(&mut self, output: Output) {
        match output {
            Output::Qualities { qualities, chosen } => {
                self.events.push_back(Event::Variants(MasterSelection {
                    variants: qualities.iter().map(variant_info).collect(),
                    chosen,
                    renditions: Vec::new(),
                }));
            }
            Output::ManifestNotFound => self.events.push_back(Event::Status(Status::NotFound)),
            Output::Tracks(specs) | Output::TracksChanged(specs) => {
                self.events.push_back(Event::Tracks(specs));
            }
            Output::Sample { track, sample } => self.events.push_back(Event::Sample {
                track,
                sample,
                join: None,
            }),
            Output::Discontinuity => self.events.push_back(Event::Discontinuity),
            Output::Dropped(dropped) => self.map_dropped(&dropped),
            Output::Ended => self.events.push_back(Event::Status(Status::Ended)),
            Output::Fatal(error) => {
                let (kind, fatal) = fatal_error_kind(&error);
                self.events.push_back(Event::Error { kind, fatal });
            }
            _ => {}
        }
    }

    fn map_dropped(&mut self, dropped: &Dropped) {
        let kind = match dropped {
            Dropped::Manifest(_) => ErrorKind::SmoothManifest {
                reason: SMOOTH_MANIFEST_REASON,
            },
            Dropped::Fragment { status, .. } => ErrorKind::SegmentFetchFailed { status: *status },
            Dropped::FragmentUrl { .. } => ErrorKind::SegmentFetchFailed { status: None },
            _ => return,
        };
        self.events.push_back(Event::Error { kind, fatal: false });
    }

    fn fail_request(&mut self, req: &Req, status: Option<u16>, now: u64) {
        self.client.on_failure(
            Timestamp::from_micros(now),
            req.id,
            FetchFailure {
                status,
                ..FetchFailure::default()
            },
        );
        self.pump(now);
    }

    pub(crate) fn poll(&mut self, now: u64, _auth: &mut AuthState) {
        if self.client.is_fatal() || self.client.is_ended() {
            return;
        }
        let timeout = self.inactivity_us();
        let stalled: Vec<u32> = self
            .reqs
            .iter()
            .filter(|(_, r)| {
                let last = if r.last_activity_us == 0 {
                    r.started_us
                } else {
                    r.last_activity_us
                };
                now.saturating_sub(last) >= timeout
            })
            .map(|(c, _)| *c)
            .collect();
        for conn in stalled {
            if let Some(req) = self.reqs.remove(&conn) {
                self.actions.push_back(HostAction::Close { conn });
                self.events.push_back(Event::Status(Status::RequestStalled {
                    resume: ResumeMode::Full,
                }));
                self.client.on_failure(
                    Timestamp::from_micros(now),
                    req.id,
                    FetchFailure::default(),
                );
            }
        }
        self.pump(now);
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
        auth.receive_cookies(&req.url, headers, Timestamp::from_micros(now));
        if REDIRECT_STATUSES.contains(&status) {
            self.actions.push_back(HostAction::Close { conn });
            let target =
                redirect::resolve(&req.url, headers, req.redirects, self.config.max_redirects);
            match target {
                Some(target) => {
                    req.redirects = req.redirects.saturating_add(1);
                    req.url = target;
                    if matches!(req.kind, ReqKind::Manifest) {
                        self.client.set_manifest_url(req.url.clone());
                    }
                    let h = auth.request_headers(&req.url, Timestamp::from_micros(now));
                    self.start(req, h, now);
                }
                None => self.fail_request(&req, None, now),
            }
            return;
        }
        let challenged = status == HTTP_UNAUTHORIZED || status == HTTP_PROXY_AUTH_REQUIRED;
        if challenged && req.url.origin() == auth.origin().origin() {
            self.actions.push_back(HostAction::Close { conn });
            match auth.handle_unauthorized(status, headers, req.authed, req.stale_replayed) {
                AuthDecision::Replay => {
                    req.authed = true;
                    let h = auth.request_headers(&req.url, Timestamp::from_micros(now));
                    self.start(req, h, now);
                }
                AuthDecision::ReplayStale => {
                    req.authed = true;
                    req.stale_replayed = true;
                    let h = auth.request_headers(&req.url, Timestamp::from_micros(now));
                    self.start(req, h, now);
                }
                AuthDecision::Wait(events) => {
                    self.events.extend(events.into_iter().map(Event::from));
                    self.parked.push(req);
                }
                _ => {}
            }
            return;
        }
        if !(200..300).contains(&status) {
            self.actions.push_back(HostAction::Close { conn });
            self.fail_request(&req, Some(status), now);
            return;
        }
        req.status = Some(status);
        req.last_activity_us = now;
        self.reqs.insert(conn, req);
    }

    pub(crate) fn on_data(&mut self, conn: u32, bytes: &[u8], now: u64, _auth: &mut AuthState) {
        if let Some(req) = self.reqs.get_mut(&conn) {
            req.body.extend_from_slice(bytes);
            req.last_activity_us = now;
        }
    }

    pub(crate) fn on_closed(
        &mut self,
        conn: u32,
        error: Option<&TransportError>,
        now: u64,
        _auth: &mut AuthState,
    ) {
        let Some(req) = self.reqs.remove(&conn) else {
            return;
        };
        let ok = req.status.is_some_and(|s| (200..300).contains(&s));
        if error.is_some() || !ok {
            let http_status = if error.is_some() { None } else { req.status };
            return self.fail_request(&req, http_status, now);
        }
        match req.kind {
            ReqKind::Manifest => {
                self.client
                    .on_manifest(Timestamp::from_micros(now), req.id, &req.body)
            }
            ReqKind::Fragment => {
                self.client
                    .on_fragment(Timestamp::from_micros(now), req.id, &req.body)
            }
        }
        self.pump(now);
    }

    pub(crate) fn select_variant(
        &mut self,
        index: usize,
        _now: u64,
        _auth: &mut AuthState,
    ) -> Result<(), Error> {
        if self.client.is_ended() {
            return Ok(());
        }
        if self.client.select_video_quality(index) {
            Ok(())
        } else {
            Err(Error::NoSuchVariant(index))
        }
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
        for req in core::mem::take(&mut self.parked) {
            let h = auth.request_headers(&req.url, Timestamp::from_micros(now));
            self.start(req, h, now);
        }
        self.pump(now);
    }

    pub(crate) fn set_wall_clock(&mut self, _now_us: u64, _unix_us: u64) {}
}
