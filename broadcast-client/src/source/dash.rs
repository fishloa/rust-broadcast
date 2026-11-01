//! Adapter over [`dash_runtime::client::DashClient`]: per-request connection table, bounded body
//! accumulation, timeouts, retry pacing, and the Output-to-Event mapping (`docs/dash-runtime/
//! DESIGN.md` section 6).
//!
//! The core owns the failure counters, the in-flight window, the availability gating and the
//! early/expired/missing classification; this adapter owns what is not DASH at all: the wire
//! requests, the cookie/auth/redirect layer, the backoff arithmetic and `Retry-After`, and the
//! translation of the core's outputs into session events.

use alloc::collections::{BTreeMap, BTreeSet, VecDeque};
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use std::collections::HashMap;

use broadcast_hls::MediaType;
use dash_runtime::client::{
    Action as DashAction, DashClient, DropCause, Error as CoreError, Failure, FailureClass,
    FatalKind, ManifestSelection, Notice, Output, PullLimits, Rejection, ResourceId, ResponseMeta,
    UtcId, UtcMethod, UtcResponse,
};
use dash_runtime::error::MpdError;
use dash_runtime::limits::Limits;
use dash_runtime::selection::{DropReason, Policy};
use hls_runtime::client::{MasterSelection, RenditionInfo, VariantInfo};
use transmux::{TrackId, TrackSpec};
use url::Url;

use crate::auth::{AuthDecision, AuthState};
use crate::error::Error;
use crate::events::{Action, Config, ErrorKind, Event, ResumeMode, Status};
use crate::playout::Source;
use crate::redirect;
use crate::source::fetch::body::BodyAcc;
use crate::source::fetch::conn::{ConnAlloc, ConnLease};
use crate::source::fetch::deadlines::Deadlines;
use crate::source::fetch::failures::{Attempts, Verdict, verdict};
use crate::source::fetch::inactivity::Inactivity;
use crate::source::fetch::resume::{Basis, HeadOutcome, Parked, Stash, accept_head};
use crate::source::fetch::retry::{Backoff, RefusedCapture, retry_after_us};
use crate::source::fetch::wall::WallClock;

const MICROS_PER_MILLI: u64 = 1_000;
const REDIRECT_STATUSES: [u16; 5] = [301, 302, 303, 307, 308];
const STATUS_UNAUTHORIZED: u16 = 401;
const STATUS_PROXY_AUTH_REQUIRED: u16 = 407;
const STATUS_FORBIDDEN: u16 = 403;
const STATUS_NOT_FOUND: u16 = 404;
const STATUS_GONE: u16 = 410;
const STATUS_NOT_MODIFIED: u16 = 304;
const STATUS_RANGE_NOT_SATISFIABLE: u16 = 416;

const METHOD_GET: &str = "GET";
const METHOD_HEAD: &str = "HEAD";
const HEADER_IF_NONE_MATCH: &str = "If-None-Match";
const HEADER_IF_MODIFIED_SINCE: &str = "If-Modified-Since";
const HEADER_DATE: &str = "date";
const HEADER_ETAG: &str = "etag";
const HEADER_LAST_MODIFIED: &str = "last-modified";

const UTC_BODY_LIMIT: usize = 4 * 1024;
const MIN_WAKE_STEP_US: u64 = MICROS_PER_MILLI;
const MAX_ADVANCE_ROUNDS: usize = 64;

const ENCRYPTED_SAMPLE_ENTRIES: [&str; 2] = ["encv", "enca"];
const UNNAMED_TRACK_ID: u32 = 0;
const FIELD_MAX_MANIFEST_BYTES: &str = "max_manifest_bytes";
const FIELD_MAX_SEGMENT_BYTES: &str = "max_segment_bytes";
const REASON_NON_FRAGMENTED_MP4: &str = "non_fragmented_mp4";
const REASON_DAMAGED_FRAGMENT: &str = "damaged_fragment";
const REASON_INVALID_UTF8: &str = "invalid_utf8";
const REASON_XML: &str = "xml";
const REASON_NOT_MPD: &str = "not_mpd";
const REASON_INVALID: &str = "invalid";
const REASON_TOO_LARGE: &str = "too_large";
const REASON_UNKNOWN: &str = "unknown";
const REASON_NO_PLAYABLE: &str = "no_playable_representation";
const REASON_CONTENT_PROTECTION: &str = "content_protection";
const FEATURE_PRESELECTION: &str = "preselection";
const FEATURE_XLINK: &str = "xlink";
const FEATURE_MPD_PATCH: &str = "mpd_patch";
const FEATURE_EVENT_STREAM: &str = "event_stream";
const FEATURE_LOW_LATENCY: &str = "low_latency";
const FEATURE_UTC_SCHEME: &str = "utc_timing_scheme";
const FEATURE_UTC_CROSS_ORIGIN: &str = "utc_cross_origin";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Purpose {
    Mpd,
    Utc(UtcId),
    Resource(ResourceId),
}

#[derive(Debug, Clone, Copy)]
struct Timeouts {
    manifest_us: u64,
    segment_us: u64,
    utc_us: u64,
}

impl Purpose {
    fn is_media(self) -> bool {
        matches!(self, Self::Resource(_))
    }
}

impl Timeouts {
    fn of(&self, purpose: Purpose) -> u64 {
        match purpose {
            Purpose::Mpd
            | Purpose::Resource(ResourceId::Init { .. } | ResourceId::Index { .. }) => {
                self.manifest_us
            }
            Purpose::Resource(_) => self.segment_us,
            Purpose::Utc(_) => self.utc_us,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct BodyLimits {
    manifest: usize,
    segment: usize,
}

#[derive(Debug, Clone, Default)]
struct Head {
    date: Option<String>,
    etag: Option<String>,
    last_modified: Option<String>,
}

impl Head {
    fn of(headers: &[(String, String)]) -> Self {
        let find = |name: &str| {
            headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(name))
                .map(|(_, v)| v.trim().to_string())
        };
        Self {
            date: find(HEADER_DATE),
            etag: find(HEADER_ETAG),
            last_modified: find(HEADER_LAST_MODIFIED),
        }
    }
}

struct Req {
    purpose: Purpose,
    method: &'static str,
    original_url: String,
    url: Url,
    byte_range: Option<(u64, u64)>,
    conditional: Vec<(String, String)>,
    redirects: u32,
    authed: bool,
    stale_replayed: bool,
    lease: Option<ConnLease>,
    started_us: u64,
    received_us: u64,
    last_activity_us: u64,
    responded_ok: bool,
    basis: Option<Basis>,
    resume_from: Option<u64>,
    head: Head,
    body: BodyAcc,
}

impl Req {
    fn deadline(&self, timeouts: &Timeouts) -> Option<u64> {
        self.started_us.checked_add(timeouts.of(self.purpose))
    }

    fn new(
        purpose: Purpose,
        method: &'static str,
        url: Url,
        byte_range: Option<(u64, u64)>,
        body_limit: usize,
    ) -> Self {
        Self {
            purpose,
            method,
            original_url: url.to_string(),
            url,
            byte_range,
            conditional: Vec::new(),
            redirects: 0,
            authed: false,
            stale_replayed: false,
            lease: None,
            started_us: 0,
            received_us: 0,
            last_activity_us: 0,
            responded_ok: false,
            basis: None,
            resume_from: None,
            head: Head::default(),
            body: BodyAcc::new(body_limit),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct VariantKey {
    bandwidth: u64,
    resolution: Option<(u32, u32)>,
    codecs: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AudioKey {
    name: String,
    language: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AudioMode {
    Auto,
    Manual(Option<usize>),
}

#[derive(Default)]
struct Known {
    variants: Vec<VariantKey>,
    chosen: Option<usize>,
    audio: Vec<AudioKey>,
    chosen_audio: Option<usize>,
}

pub(crate) struct DashSource {
    client: DashClient,
    mpd_url: Url,
    dash_config: dash_runtime::client::DashConfig,
    pull: PullLimits,
    body_limits: BodyLimits,
    timeouts: Timeouts,
    backoff: Backoff,
    max_redirects: u32,
    max_key_failures: u32,
    mpd_failures: Attempts,
    mpd_accepted: bool,
    retry_after_us: u64,
    refused_captures: BTreeMap<u32, RefusedCapture>,
    reqs: HashMap<u32, Req>,
    deadlines: Deadlines<u32>,
    inactivity: Inactivity,
    stash: Stash<Purpose>,
    parked: Vec<Req>,
    retries: BTreeMap<ResourceId, u64>,
    mpd_retry_at: Option<u64>,
    wake_pushed: Option<u64>,
    reported: BTreeSet<&'static str>,
    features_reported: BTreeSet<&'static str>,
    drop_errors: Vec<ErrorKind>,
    skipped_reported: BTreeSet<(Option<u32>, String)>,
    skipped_kind: Option<ErrorKind>,
    damaged_reported: bool,
    oversize_reported: BTreeSet<&'static str>,
    init_undecodable: BTreeSet<ResourceId>,
    encryption_seen: bool,
    tracks: Vec<TrackSpec>,
    audio_stopped: bool,
    known: Known,
    manual_variant: Option<VariantKey>,
    pending_variant: Option<usize>,
    audio_mode: AudioMode,
    manual_audio: Option<AudioKey>,
    pending_audio: Option<Option<usize>>,
    announced_audio: Option<usize>,
    anchor: Option<(u64, u64)>,
    established: bool,
    live: bool,
    ended: bool,
    fatal: bool,
    conn_alloc: ConnAlloc,
    actions: VecDeque<Action>,
    events: VecDeque<Event>,
}

impl DashSource {
    pub(crate) fn new(url: &Url, config: &Config, conn_alloc: ConnAlloc) -> Self {
        let mut pull = PullLimits::default();
        pull.policy = Policy::default();
        pull.policy.max_bandwidth = config.variant_policy.max_bandwidth.map(u64::from);
        pull.policy.max_height = config.variant_policy.max_height;
        pull.policy.codec_prefixes = config
            .variant_policy
            .supported_codecs
            .clone()
            .unwrap_or_default();
        pull.policy.preferred_audio_language = config.preferred_audio_language.clone();
        pull.limits = Limits::default();
        pull.limits.max_bytes = config.max_manifest_bytes;
        pull.max_segment_bytes = config.max_segment_bytes;
        pull.max_inflight_segments = config.max_inflight_segments;
        pull.max_segment_failures = config.max_segment_failures;
        pull.max_init_failures = config.max_key_failures;
        let client = DashClient::new(url.as_str(), config.dash, pull.clone());
        let mut source = Self {
            client,
            mpd_url: url.clone(),
            dash_config: config.dash,
            pull,
            body_limits: BodyLimits {
                manifest: config.max_manifest_bytes,
                segment: config.max_segment_bytes,
            },
            timeouts: Timeouts {
                manifest_us: config.request_timeout_us,
                segment_us: config.segment_timeout_us,
                utc_us: config.dash.utc_timeout_us,
            },
            backoff: Backoff::new(&config.backoff_ms, config.backoff_cap_ms),
            max_redirects: config.max_redirects,
            max_key_failures: config.max_key_failures,
            mpd_failures: Attempts::default(),
            mpd_accepted: false,
            retry_after_us: 0,
            refused_captures: BTreeMap::new(),
            reqs: HashMap::new(),
            deadlines: Deadlines::new(),
            inactivity: Inactivity::new(config.segment_inactivity_timeout_us),
            stash: Stash::new(),
            parked: Vec::new(),
            retries: BTreeMap::new(),
            mpd_retry_at: None,
            wake_pushed: None,
            reported: BTreeSet::new(),
            features_reported: BTreeSet::new(),
            drop_errors: Vec::new(),
            skipped_reported: BTreeSet::new(),
            skipped_kind: None,
            damaged_reported: false,
            oversize_reported: BTreeSet::new(),
            init_undecodable: BTreeSet::new(),
            encryption_seen: false,
            tracks: Vec::new(),
            audio_stopped: false,
            known: Known::default(),
            manual_variant: None,
            pending_variant: None,
            audio_mode: if config.auto_select_audio {
                AudioMode::Auto
            } else {
                AudioMode::Manual(None)
            },
            manual_audio: None,
            pending_audio: None,
            announced_audio: None,
            anchor: None,
            established: false,
            live: false,
            ended: false,
            fatal: false,
            conn_alloc,
            actions: VecDeque::new(),
            events: VecDeque::new(),
        };
        source.drain_outputs();
        source
    }

    pub(crate) fn take_actions(&mut self) -> VecDeque<Action> {
        core::mem::take(&mut self.actions)
    }

    pub(crate) fn take_events(&mut self) -> VecDeque<Event> {
        core::mem::take(&mut self.events)
    }

    pub(crate) fn conns(&self) -> Vec<u32> {
        self.reqs
            .keys()
            .chain(self.refused_captures.keys())
            .copied()
            .collect()
    }

    pub(crate) fn is_ended(&self) -> bool {
        self.ended
    }

    pub(crate) fn playout_source(&self) -> Source {
        if self.client.is_static() == Some(true) {
            Source::Vod
        } else {
            Source::Live {
                target_duration_secs: self
                    .client
                    .max_segment_duration_secs()
                    .map(|s| u32::try_from(s).unwrap_or(u32::MAX)),
            }
        }
    }

    pub(crate) fn set_wall_clock(&mut self, now_us: u64, unix_us: u64) {
        self.anchor = Some((now_us, unix_us));
        if self.fatal {
            return;
        }
        self.client.set_wall_clock(now_us, unix_us);
        self.drain_outputs();
    }

    pub(crate) fn poll(&mut self, now: u64, auth: &mut AuthState) {
        if self.fatal || auth.is_waiting() {
            return;
        }
        self.expire(now);
        self.advance(now, auth);
    }

    pub(crate) fn select_variant(
        &mut self,
        index: usize,
        now: u64,
        auth: &mut AuthState,
    ) -> Result<(), Error> {
        let Some(key) = self.known.variants.get(index).cloned() else {
            return Err(Error::NoSuchVariant(index));
        };
        if self.fatal || self.ended {
            return Ok(());
        }
        self.manual_variant = Some(key);
        self.pending_variant = Some(index);
        self.try_pending();
        self.drain_outputs();
        if !auth.is_waiting() {
            self.advance(now, auth);
        }
        Ok(())
    }

    pub(crate) fn select_audio(
        &mut self,
        choice: Option<usize>,
        now: u64,
        auth: &mut AuthState,
    ) -> Result<(), Error> {
        if self.known.audio.is_empty() || choice.is_some_and(|i| i >= self.known.audio.len()) {
            return Err(Error::NoSuchRendition);
        }
        if self.fatal || self.ended {
            return Ok(());
        }
        self.audio_mode = AudioMode::Manual(choice);
        self.manual_audio = choice.and_then(|i| self.known.audio.get(i).cloned());
        self.pending_audio = Some(choice);
        self.try_pending();
        self.drain_outputs();
        if !auth.is_waiting() {
            self.advance(now, auth);
        }
        Ok(())
    }

    pub(crate) fn reconnect(&mut self, now: u64, auth: &mut AuthState) {
        if self.fatal || self.ended {
            return;
        }
        self.reset_in_flight();
        let mut client =
            DashClient::new(self.mpd_url.as_str(), self.dash_config, self.pull.clone());
        if let Some((mono, unix)) = self.anchor {
            client.set_wall_clock(mono, unix);
        }
        self.client = client;
        self.mpd_accepted = false;
        self.mpd_failures.reset();
        self.init_undecodable.clear();
        self.known = Known::default();
        self.announced_audio = None;
        self.pending_variant = None;
        self.pending_audio = None;
        self.events.push_back(Event::Discontinuity);
        self.drain_outputs();
        if !auth.is_waiting() {
            self.advance(now, auth);
        }
    }

    pub(crate) fn resume(&mut self, now: u64, auth: &mut AuthState) {
        if self.fatal {
            return;
        }
        let parked: Vec<Req> = core::mem::take(&mut self.parked);
        for req in parked {
            self.send(req, auth, now);
        }
        self.advance(now, auth);
    }

    pub(crate) fn on_response(
        &mut self,
        conn: u32,
        status: u16,
        headers: &[(String, String)],
        now: u64,
        wall: Option<&WallClock>,
        auth: &mut AuthState,
    ) {
        if self.fatal {
            return;
        }
        let Some(mut req) = self.reqs.remove(&conn) else {
            return;
        };
        auth.receive_cookies(&req.url, headers, now);
        if REDIRECT_STATUSES.contains(&status) {
            self.actions.push_back(Action::Close { conn });
            match redirect::resolve(&req.url, headers, req.redirects, self.max_redirects) {
                Some(target) => {
                    req.redirects = req.redirects.saturating_add(1);
                    req.url = target;
                    self.send(req, auth, now);
                }
                None => self.fail(req, now, Failure::Transport),
            }
        } else if (status == STATUS_UNAUTHORIZED || status == STATUS_PROXY_AUTH_REQUIRED)
            && req.url.origin() == auth.origin().origin()
        {
            self.actions.push_back(Action::Close { conn });
            match auth.handle_unauthorized(status, headers, req.authed, req.stale_replayed) {
                AuthDecision::Replay => self.send(req, auth, now),
                AuthDecision::ReplayStale => {
                    req.stale_replayed = true;
                    self.send(req, auth, now);
                }
                AuthDecision::Wait(events) => {
                    self.events.extend(events);
                    self.parked.push(req);
                }
            }
        } else if (200..300).contains(&status) {
            match accept_head(
                &mut req.body,
                &mut req.basis,
                &mut req.resume_from,
                status,
                headers,
                req.byte_range,
            ) {
                HeadOutcome::Refused => {
                    self.actions.push_back(Action::Close { conn });
                    self.fail(req, now, Failure::Status(status));
                    if !auth.is_waiting() {
                        self.advance(now, auth);
                    }
                    return;
                }
                HeadOutcome::Fallback => {
                    self.actions.push_back(Action::Close { conn });
                    self.send(req, auth, now);
                }
                HeadOutcome::Accepted => {
                    req.responded_ok = true;
                    req.received_us = now;
                    req.last_activity_us = now;
                    req.head = Head::of(headers);
                    if !self.established {
                        self.established = true;
                        self.events.push_back(Event::Status(Status::Establishing));
                    }
                    self.reqs.insert(conn, req);
                }
            }
        } else if status == STATUS_RANGE_NOT_SATISFIABLE && req.resume_from.is_some() {
            self.actions.push_back(Action::Close { conn });
            req.resume_from = None;
            self.send(req, auth, now);
        } else if status == STATUS_NOT_MODIFIED && req.purpose == Purpose::Mpd {
            self.actions.push_back(Action::Close { conn });
            req.received_us = now;
            self.deliver_unchanged(&req, now);
        } else {
            let refused = status == 429 || status == 503;
            if status == STATUS_FORBIDDEN {
                self.events.push_back(Event::Forbidden { status });
            }
            if req.purpose == Purpose::Mpd
                && !self.mpd_accepted
                && (status == STATUS_NOT_FOUND || status == STATUS_GONE)
            {
                self.events.push_back(Event::Status(Status::NotFound));
            }
            let ra = retry_after_us(status, headers, now, wall);
            self.retry_after_us = ra;
            if refused {
                let cap = RefusedCapture::new(status, ra, headers);
                if cap.wants_body() {
                    self.refused_captures.insert(conn, cap);
                } else {
                    self.events.push_back(cap.into_event());
                    self.actions.push_back(Action::Close { conn });
                }
            } else {
                self.actions.push_back(Action::Close { conn });
            }
            self.fail(req, now, Failure::Status(status));
            self.retry_after_us = 0;
        }
        if !auth.is_waiting() {
            self.advance(now, auth);
        }
    }

    pub(crate) fn on_data(&mut self, conn: u32, bytes: &[u8], now: u64, auth: &mut AuthState) {
        if self.fatal {
            return;
        }
        if let Some(cap) = self.refused_captures.get_mut(&conn) {
            if cap.push(bytes)
                && let Some(cap) = self.refused_captures.remove(&conn)
            {
                self.events.push_back(cap.into_event());
                self.actions.push_back(Action::Close { conn });
            }
            return;
        }
        let Some(req) = self.reqs.get_mut(&conn) else {
            return;
        };
        if !req.responded_ok || bytes.is_empty() {
            return;
        }
        req.last_activity_us = now;
        let field = oversize_field(req.purpose);
        if req.body.push(bytes).is_err() {
            if let Some(field) = field
                && self.oversize_reported.insert(field)
            {
                self.events.push_back(Event::Error {
                    kind: ErrorKind::BodyTooLarge { field },
                    fatal: false,
                });
            }
            if let Some(req) = self.reqs.remove(&conn) {
                self.actions.push_back(Action::Close { conn });
                self.fail(req, now, Failure::TooLarge);
                if !auth.is_waiting() {
                    self.advance(now, auth);
                }
            }
        }
    }

    pub(crate) fn on_closed(
        &mut self,
        conn: u32,
        error: Option<&str>,
        now: u64,
        auth: &mut AuthState,
    ) {
        if self.fatal {
            return;
        }
        if let Some(cap) = self.refused_captures.remove(&conn) {
            self.events.push_back(cap.into_event());
        }
        let Some(mut req) = self.reqs.remove(&conn) else {
            return;
        };
        let head_request = req.method == METHOD_HEAD;
        let truncated = !head_request && req.body.is_truncated();
        if error.is_some() || !req.responded_ok || truncated {
            self.fail(req, now, Failure::Transport);
        } else if req.body.finish() {
            self.deliver(req, now);
        } else {
            self.fail(req, now, Failure::Transport);
        }
        if !auth.is_waiting() {
            self.advance(now, auth);
        }
    }

    fn body_limit(&self, purpose: Purpose) -> usize {
        match purpose {
            Purpose::Mpd => self.body_limits.manifest,
            Purpose::Utc(_) => UTC_BODY_LIMIT,
            Purpose::Resource(_) => self.body_limits.segment,
        }
    }

    fn deliver(&mut self, req: Req, now: u64) {
        match req.purpose {
            Purpose::Mpd => self.deliver_mpd(&req, now),
            Purpose::Utc(id) => self.deliver_utc(id, &req, now),
            Purpose::Resource(id) => self.deliver_resource(id, req.body.as_slice(), now),
        }
        self.drain_outputs();
    }

    fn meta<'r>(req: &'r Req, final_url: Option<&'r str>) -> ResponseMeta<'r> {
        let mut meta = ResponseMeta::new(req.started_us, req.received_us);
        if let Some(date) = req.head.date.as_deref() {
            meta = meta.with_date(date);
        }
        if let Some(etag) = req.head.etag.as_deref() {
            meta = meta.with_etag(etag);
        }
        if let Some(last_modified) = req.head.last_modified.as_deref() {
            meta = meta.with_last_modified(last_modified);
        }
        if let Some(url) = final_url {
            meta = meta.with_final_url(url);
        }
        meta
    }

    fn deliver_unchanged(&mut self, req: &Req, now: u64) {
        let meta = Self::meta(req, None);
        let _ = self.client.on_mpd_unchanged(now, &meta);
        self.mpd_failures.reset();
        self.drain_outputs();
    }

    fn deliver_mpd(&mut self, req: &Req, now: u64) {
        let final_url = (req.url.as_str() != req.original_url).then(|| req.url.to_string());
        let meta = Self::meta(req, final_url.as_deref());
        match self.client.on_mpd_with(now, req.body.as_slice(), &meta) {
            Ok(()) => {
                self.mpd_failures.reset();
                self.mpd_accepted = true;
            }
            Err(CoreError::Mpd(error)) => self.mpd_rejected(&error, now),
            Err(_) => {}
        }
    }

    fn mpd_rejected(&mut self, error: &MpdError, now: u64) {
        let (kind, definitive) = match error {
            MpdError::Unsupported(unsupported) => (
                ErrorKind::UnsupportedAddressing {
                    reason: unsupported.name(),
                },
                !self.mpd_accepted,
            ),
            other => (
                ErrorKind::MpdParse {
                    reason: mpd_reason(other),
                },
                false,
            ),
        };
        let attempt = self.mpd_failures.bump();
        if definitive || verdict(attempt, self.max_key_failures) == Verdict::GiveUp {
            self.go_fatal(kind);
            return;
        }
        if self.reported.insert(kind.name()) {
            self.events.push_back(Event::Error { kind, fatal: false });
        }
        self.schedule_mpd_retry(now, attempt);
    }

    fn deliver_utc(&mut self, id: UtcId, req: &Req, now: u64) {
        let mut response = UtcResponse::new(req.started_us, req.received_us, req.body.as_slice());
        if let Some(date) = req.head.date.as_deref() {
            response = response.with_date(date);
        }
        let _ = self.client.on_utc_response(now, id, &response);
    }

    fn deliver_resource(&mut self, id: ResourceId, body: &[u8], now: u64) {
        let result = match id {
            ResourceId::Init { .. } => self.client.on_init(now, id, body),
            ResourceId::Index { .. } => self.client.on_index(now, id, body),
            _ => self.client.on_segment(now, id, body),
        };
        match result {
            Ok(()) => {
                self.init_undecodable.remove(&id);
            }
            Err(CoreError::Rejected { id, reason, class }) => {
                self.rejected(id, reason, class, now);
            }
            Err(_) => {}
        }
    }

    fn rejected(&mut self, id: ResourceId, reason: Rejection, class: FailureClass, now: u64) {
        match reason {
            Rejection::TooLarge => {
                if self.oversize_reported.insert(FIELD_MAX_SEGMENT_BYTES) {
                    self.events.push_back(Event::Error {
                        kind: ErrorKind::BodyTooLarge {
                            field: FIELD_MAX_SEGMENT_BYTES,
                        },
                        fatal: false,
                    });
                }
            }
            Rejection::InitUndecodable => {
                self.init_undecodable.insert(id);
                if self.reported.insert(ErrorKind::InitDecodeFailed.name()) {
                    self.events.push_back(Event::Error {
                        kind: ErrorKind::InitDecodeFailed,
                        fatal: false,
                    });
                }
            }
            _ => {
                self.init_undecodable.remove(&id);
            }
        }
        self.after_class(id, class, now);
    }

    fn fail(&mut self, req: Req, now: u64, failure: Failure) {
        self.fail_purpose(req.purpose, now, failure);
    }

    fn fail_purpose(&mut self, purpose: Purpose, now: u64, failure: Failure) {
        match purpose {
            Purpose::Mpd => {
                let class = self.client.on_mpd_failure(now, failure);
                if let FailureClass::Retry { attempt, .. } = class {
                    self.schedule_mpd_retry(now, u64::from(attempt));
                }
            }
            Purpose::Utc(id) => {
                let _ = self.client.on_utc_failure(now, id, failure);
            }
            Purpose::Resource(id) => {
                self.init_undecodable.remove(&id);
                if let Ok(class) = self.client.on_failure(now, id, failure) {
                    self.after_class(id, class, now);
                }
            }
        }
        self.drain_outputs();
    }

    fn after_class(&mut self, id: ResourceId, class: FailureClass, now: u64) {
        if !keeps_parked_prefix(class) {
            self.stash.discard(&Purpose::Resource(id));
        }
        if let FailureClass::Retry { attempt, .. } = class {
            let wait = self
                .backoff
                .delay_us(u64::from(attempt))
                .max(self.retry_after_us);
            let at = now.saturating_add(wait);
            self.retries.insert(id, at);
            self.events.push_back(Event::Status(Status::Reconnecting {
                attempt: u64::from(attempt),
            }));
            self.actions.push_back(Action::WakeAt(at));
        }
    }

    fn schedule_mpd_retry(&mut self, now: u64, attempt: u64) {
        let wait = self.backoff.delay_us(attempt).max(self.retry_after_us);
        let at = now.saturating_add(wait);
        self.mpd_retry_at = Some(at);
        self.events
            .push_back(Event::Status(Status::Reconnecting { attempt }));
        self.actions.push_back(Action::WakeAt(at));
    }

    fn go_fatal(&mut self, kind: ErrorKind) {
        self.events.push_back(Event::Error { kind, fatal: true });
        self.fatal = true;
        self.reset_in_flight();
    }

    fn run_due_retries(&mut self, now: u64) {
        if let Some(at) = self.mpd_retry_at
            && now >= at
        {
            self.mpd_retry_at = None;
            self.client.retry_mpd(now);
        }
        let due: Vec<ResourceId> = self
            .retries
            .iter()
            .filter(|(_, at)| **at <= now)
            .map(|(id, _)| *id)
            .collect();
        for id in due {
            self.retries.remove(&id);
            if !self.client.retry(now, id) {
                self.stash.discard(&Purpose::Resource(id));
            }
        }
    }

    fn expire(&mut self, now: u64) {
        let reqs = &self.reqs;
        let timeouts = &self.timeouts;
        let overdue = self.deadlines.expire(now, |conn, deadline| {
            reqs.get(conn)
                .is_some_and(|r| r.deadline(timeouts) == Some(deadline))
        });
        for conn in overdue {
            if let Some(req) = self.reqs.remove(&conn) {
                self.actions.push_back(Action::Close { conn });
                self.fail(req, now, Failure::Timeout);
            }
        }
        self.expire_inactive(now);
    }

    fn expire_inactive(&mut self, now: u64) {
        let reqs = &self.reqs;
        let expired = self
            .inactivity
            .expire(now, |conn| reqs.get(&conn).map(|r| r.last_activity_us));
        for at in expired.wakes {
            self.actions.push_back(Action::WakeAt(at));
        }
        for conn in expired.stalled {
            if let Some(req) = self.reqs.remove(&conn) {
                self.stalled(req, conn, now);
            }
        }
    }

    fn stalled(&mut self, mut req: Req, conn: u32, now: u64) {
        let received = u64::try_from(req.body.len()).unwrap_or(u64::MAX);
        let kept = (req.responded_ok || req.resume_from.is_some())
            .then(|| req.basis.take())
            .flatten()
            .filter(|basis| basis.request_headers(received).is_some());
        let resume = if kept.is_some() {
            ResumeMode::Range
        } else {
            ResumeMode::Full
        };
        if let Some(basis) = kept {
            self.stash.put(
                req.purpose,
                Parked {
                    url: req.original_url.clone(),
                    byte_range: req.byte_range,
                    body: core::mem::replace(&mut req.body, BodyAcc::new(0)),
                    basis,
                },
            );
        }
        self.events
            .push_back(Event::Status(Status::RequestStalled { resume }));
        self.actions.push_back(Action::Close { conn });
        self.fail(req, now, Failure::Timeout);
    }

    fn advance(&mut self, now: u64, auth: &mut AuthState) {
        for _ in 0..MAX_ADVANCE_ROUNDS {
            if self.fatal {
                return;
            }
            self.run_due_retries(now);
            let mut issued = false;
            while let Some(action) = self.client.poll(now) {
                issued = true;
                self.issue(action, now, auth);
                if self.fatal {
                    return;
                }
            }
            self.drain_outputs();
            if !issued {
                break;
            }
        }
        self.sync_wake(now);
    }

    fn sync_wake(&mut self, now: u64) {
        if self.fatal {
            return;
        }
        let Some(wake) = self.client.next_wake() else {
            return;
        };
        let at = if wake > now {
            wake
        } else {
            now.saturating_add(MIN_WAKE_STEP_US)
        };
        if self.wake_pushed != Some(at) {
            self.wake_pushed = Some(at);
            self.actions.push_back(Action::WakeAt(at));
        }
    }

    fn issue(&mut self, action: DashAction, now: u64, auth: &mut AuthState) {
        let (purpose, method, url, byte_range, conditional) = match action {
            DashAction::FetchMpd { url, validators } => {
                let mut conditional = Vec::new();
                if let Some(etag) = validators.etag {
                    conditional.push((HEADER_IF_NONE_MATCH.to_string(), etag));
                }
                if let Some(modified) = validators.last_modified {
                    conditional.push((HEADER_IF_MODIFIED_SINCE.to_string(), modified));
                }
                (Purpose::Mpd, METHOD_GET, url, None, conditional)
            }
            DashAction::FetchUtcTiming {
                id, url, method, ..
            } => {
                let method = match method {
                    UtcMethod::Head => METHOD_HEAD,
                    _ => METHOD_GET,
                };
                (Purpose::Utc(id), method, url, None, Vec::new())
            }
            DashAction::FetchInit {
                id,
                url,
                byte_range,
            }
            | DashAction::FetchIndex {
                id,
                url,
                byte_range,
            }
            | DashAction::FetchSegment {
                id,
                url,
                byte_range,
            } => (
                Purpose::Resource(id),
                METHOD_GET,
                url,
                byte_range,
                Vec::new(),
            ),
            _ => return,
        };
        match Url::parse(&url) {
            Ok(parsed) => {
                let mut req = Req::new(
                    purpose,
                    method,
                    parsed,
                    byte_range,
                    self.body_limit(purpose),
                );
                req.conditional = conditional;
                if let Some(parked) = self.stash.take(&purpose, &req.original_url, byte_range) {
                    req.resume_from = Some(u64::try_from(parked.body.len()).unwrap_or(u64::MAX));
                    req.body = parked.body;
                    req.basis = Some(parked.basis);
                }
                self.send(req, auth, now);
            }
            Err(_) => self.fail_purpose(purpose, now, Failure::Transport),
        }
    }

    fn flush_refused(&mut self) {
        for (conn, cap) in core::mem::take(&mut self.refused_captures) {
            self.events.push_back(cap.into_event());
            self.actions.push_back(Action::Close { conn });
        }
    }

    fn send(&mut self, mut req: Req, auth: &mut AuthState, now: u64) {
        self.flush_refused();
        let mut headers = auth.request_headers(&req.url, now);
        req.authed = crate::auth::carries_credentials(&headers);
        headers.extend(req.conditional.iter().cloned());
        req.started_us = now;
        req.last_activity_us = now;
        req.responded_ok = false;
        let resume = req
            .resume_from
            .zip(req.basis.as_ref())
            .and_then(|(received, basis)| basis.request_headers(received));
        match &resume {
            Some(resume) => headers.extend(resume.iter().cloned()),
            None => {
                req.resume_from = None;
                req.basis = None;
                req.body.reset();
            }
        }
        let lease = self.conn_alloc.lease();
        let conn = lease.id();
        req.lease = Some(lease);
        let timeout = self.timeouts.of(req.purpose);
        self.actions.push_back(Action::HttpRequest {
            conn,
            method: req.method.to_string(),
            url: req.url.to_string(),
            headers,
            byte_range: req.byte_range.filter(|_| resume.is_none()),
            identity: None,
        });
        self.actions
            .push_back(Action::WakeAt(now.saturating_add(timeout)));
        let deadline = now.checked_add(timeout);
        if req.purpose.is_media()
            && let Some(at) = self.inactivity.arm(conn, now)
        {
            self.actions.push_back(Action::WakeAt(at));
        }
        self.reqs.insert(conn, req);
        if let Some(deadline) = deadline {
            self.deadlines.arm(conn, deadline);
        }
        let timeouts = &self.timeouts;
        self.deadlines.compact(self.reqs.len(), || {
            self.reqs
                .iter()
                .filter_map(|(conn, r)| r.deadline(timeouts).map(|d| (d, *conn)))
        });
        let reqs = &self.reqs;
        self.inactivity.compact(|| {
            reqs.iter()
                .filter(|(_, r)| r.purpose.is_media())
                .map(|(conn, r)| (*conn, r.last_activity_us))
                .collect()
        });
    }

    fn reset_in_flight(&mut self) {
        let mut conns: Vec<u32> = self.reqs.keys().copied().collect();
        conns.sort_unstable();
        for conn in conns {
            self.actions.push_back(Action::Close { conn });
        }
        self.reqs.clear();
        self.deadlines.clear();
        self.inactivity.clear();
        self.stash.clear();
        self.parked.clear();
        self.retries.clear();
        self.mpd_retry_at = None;
    }

    fn try_pending(&mut self) {
        if let Some(index) = self.pending_variant {
            match self.client.select_representation(index) {
                Ok(()) => {
                    self.known.chosen = Some(index);
                    self.pending_variant = None;
                }
                Err(CoreError::NotSwitchable) => {}
                Err(_) => self.pending_variant = None,
            }
        }
        if let Some(choice) = self.pending_audio {
            match self.client.select_audio(choice) {
                Ok(()) => {
                    self.pending_audio = None;
                    self.audio_changed(choice);
                }
                Err(CoreError::NotSwitchable) => {}
                Err(_) => self.pending_audio = None,
            }
        }
    }

    fn audio_changed(&mut self, choice: Option<usize>) {
        self.known.chosen_audio = choice;
        let stopped = choice.is_none();
        if stopped != self.audio_stopped {
            self.audio_stopped = stopped;
            let specs = self.visible_tracks();
            self.events.push_back(Event::Tracks(specs));
        }
        self.announce_audio();
    }

    fn announce_audio(&mut self) {
        if self.announced_audio == self.known.chosen_audio {
            return;
        }
        self.announced_audio = self.known.chosen_audio;
        let active = self
            .known
            .chosen_audio
            .and_then(|i| self.known.audio.get(i));
        self.events.push_back(Event::AudioRendition {
            index: self.known.chosen_audio,
            name: active.map(|a| a.name.clone()),
            language: active.and_then(|a| a.language.clone()),
        });
    }

    fn visible_tracks(&self) -> Vec<TrackSpec> {
        self.tracks
            .iter()
            .filter(|s| !(self.audio_stopped && s.config.audio_sample_rate().is_some()))
            .cloned()
            .collect()
    }

    fn on_manifest(&mut self, selection: &ManifestSelection) {
        self.known.variants = selection
            .variants
            .iter()
            .map(|v| VariantKey {
                bandwidth: v.bandwidth,
                resolution: v.resolution,
                codecs: v.codecs.clone(),
            })
            .collect();
        self.known.chosen = selection.chosen;
        self.known.audio = selection
            .audio
            .iter()
            .map(|a| AudioKey {
                name: a.name.clone(),
                language: a.language.clone(),
            })
            .collect();
        self.known.chosen_audio = selection.chosen_audio;
        if let Some(key) = &self.manual_variant
            && let Some(index) = self.known.variants.iter().position(|v| v == key)
            && Some(index) != selection.chosen
        {
            self.pending_variant = Some(index);
        }
        let desired = match self.audio_mode {
            AudioMode::Manual(None) if !self.known.audio.is_empty() => Some(None),
            AudioMode::Manual(Some(_)) => self
                .manual_audio
                .as_ref()
                .and_then(|key| self.known.audio.iter().position(|a| a == key))
                .filter(|index| Some(*index) != selection.chosen_audio)
                .map(Some),
            _ => None,
        };
        if let Some(choice) = desired {
            self.pending_audio = Some(choice);
        }
        self.known.chosen_audio = desired.unwrap_or(selection.chosen_audio);
        self.events
            .push_back(Event::Variants(master_selection(selection)));
        self.announce_audio();
    }

    fn drain_outputs(&mut self) {
        loop {
            while let Some(output) = self.client.next_output() {
                self.map_output(output);
                if self.fatal {
                    return;
                }
            }
            if self.pending_variant.is_none() && self.pending_audio.is_none() {
                break;
            }
            let before = (self.pending_variant, self.pending_audio);
            self.try_pending();
            if (self.pending_variant, self.pending_audio) == before {
                break;
            }
        }
    }

    fn map_output(&mut self, output: Output) {
        match output {
            Output::Manifest(selection) => self.on_manifest(&selection),
            Output::Tracks(specs) => {
                self.tracks = specs;
                let visible = self.visible_tracks();
                if visible.is_empty()
                    && let Some(kind) = self.skipped_kind.clone()
                {
                    self.go_fatal(kind);
                    return;
                }
                self.events.push_back(Event::Tracks(visible));
            }
            Output::Samples { track_id, samples } => self.on_samples(track_id, samples),
            Output::Discontinuity => self.events.push_back(Event::Discontinuity),
            Output::TrackDropped { cause, .. } => self.on_dropped(cause),
            Output::TrackSkipped {
                track_id, fourcc, ..
            } => {
                let track_id = track_id.map(TrackId::value);
                if self.skipped_reported.insert((track_id, fourcc.clone())) {
                    let kind = skipped_kind(track_id, fourcc);
                    self.skipped_kind.get_or_insert_with(|| kind.clone());
                    self.events.push_back(Event::Error { kind, fatal: false });
                }
            }
            Output::FragmentDamaged { .. } if !self.damaged_reported => {
                self.damaged_reported = true;
                self.events.push_back(Event::Error {
                    kind: ErrorKind::UnsupportedContainer {
                        reason: REASON_DAMAGED_FRAGMENT,
                    },
                    fatal: false,
                });
            }
            Output::Notice(notice) => self.on_notice(notice),
            Output::ClockSynced {
                source,
                uncertainty_us,
            } => self.events.push_back(Event::ClockSynced {
                source,
                uncertainty_us,
            }),
            Output::SegmentLost { status, .. } => self.events.push_back(Event::Error {
                kind: ErrorKind::SegmentFetchFailed { status },
                fatal: false,
            }),
            Output::Fatal(kind) => {
                let mapped = self.fatal_kind(kind);
                self.go_fatal(mapped);
            }
            Output::EndOfStream => {
                self.ended = true;
                self.events.push_back(Event::Status(Status::Ended));
            }
            _ => {}
        }
    }

    fn on_samples(&mut self, track_id: TrackId, samples: Vec<transmux::Sample>) {
        if samples.is_empty() {
            return;
        }
        if self.audio_stopped
            && self
                .tracks
                .iter()
                .any(|s| s.track_id == track_id && s.config.audio_sample_rate().is_some())
        {
            return;
        }
        self.reported.clear();
        if !self.live {
            self.live = true;
            self.events.push_back(Event::Status(Status::Live));
        }
        for sample in samples {
            self.events.push_back(Event::Sample {
                track: track_id,
                sample,
                join: None,
            });
        }
    }

    fn on_dropped(&mut self, cause: DropCause) {
        if cause == DropCause::Selection(DropReason::UnsupportedEncryption) {
            self.encryption_seen = true;
        }
        let kind = match cause {
            DropCause::Selection(DropReason::UnsupportedEncryption) => {
                ErrorKind::UnsupportedEncryption {
                    method: REASON_CONTENT_PROTECTION.to_string(),
                }
            }
            DropCause::Addressing(reason) => ErrorKind::UnsupportedAddressing { reason },
            DropCause::CrossOrigin | DropCause::UnusableUrl => ErrorKind::UnsupportedAddressing {
                reason: cause.name(),
            },
            _ => ErrorKind::TrackDropped {
                track_id: UNNAMED_TRACK_ID,
                stream_type: None,
                fourcc: None,
            },
        };
        if self.drop_errors.contains(&kind) {
            return;
        }
        self.drop_errors.push(kind.clone());
        self.events.push_back(Event::Error { kind, fatal: false });
    }

    fn on_notice(&mut self, notice: Notice) {
        let feature = match notice {
            Notice::PreselectionIgnored => FEATURE_PRESELECTION,
            Notice::XlinkSkipped => FEATURE_XLINK,
            Notice::PatchIgnored => FEATURE_MPD_PATCH,
            Notice::EventStreamIgnored => FEATURE_EVENT_STREAM,
            Notice::LowLatencyIgnored => FEATURE_LOW_LATENCY,
            Notice::UtcSchemeUnsupported => FEATURE_UTC_SCHEME,
            Notice::UtcCrossOrigin => FEATURE_UTC_CROSS_ORIGIN,
            Notice::FellBehind => {
                if self.reported.insert(ErrorKind::LiveEdgeLost.name()) {
                    self.events.push_back(Event::Error {
                        kind: ErrorKind::LiveEdgeLost,
                        fatal: false,
                    });
                }
                return;
            }
            _ => return,
        };
        if self.features_reported.insert(feature) {
            self.events.push_back(Event::Error {
                kind: ErrorKind::UnsupportedFeature { feature },
                fatal: false,
            });
        }
    }

    fn fatal_kind(&self, kind: FatalKind) -> ErrorKind {
        match kind {
            FatalKind::ClockUnavailable => ErrorKind::ClockUnavailable,
            FatalKind::MpdInvalid { reason }
                if reason == REASON_NO_PLAYABLE && self.encryption_seen =>
            {
                ErrorKind::UnsupportedEncryption {
                    method: REASON_CONTENT_PROTECTION.to_string(),
                }
            }
            FatalKind::MpdInvalid { reason } => ErrorKind::MpdInvalid { reason },
            FatalKind::UnsupportedAddressing { reason } => {
                ErrorKind::UnsupportedAddressing { reason }
            }
            FatalKind::SegmentFetchFailed { status, .. } => {
                ErrorKind::SegmentFetchFailed { status }
            }
            FatalKind::InitFetchFailed { id, status } => {
                if self.init_undecodable.contains(&id) {
                    ErrorKind::InitDecodeFailed
                } else {
                    ErrorKind::InitFetchFailed { status }
                }
            }
            FatalKind::NonFragmentedMp4 { .. } => ErrorKind::UnsupportedContainer {
                reason: REASON_NON_FRAGMENTED_MP4,
            },
            _ => ErrorKind::MpdInvalid {
                reason: REASON_UNKNOWN,
            },
        }
    }
}

fn oversize_field(purpose: Purpose) -> Option<&'static str> {
    match purpose {
        Purpose::Mpd => Some(FIELD_MAX_MANIFEST_BYTES),
        Purpose::Resource(_) => Some(FIELD_MAX_SEGMENT_BYTES),
        Purpose::Utc(_) => None,
    }
}

fn mpd_reason(error: &MpdError) -> &'static str {
    match error {
        MpdError::InvalidUtf8 { .. } => REASON_INVALID_UTF8,
        MpdError::Xml { .. } => REASON_XML,
        MpdError::NotMpd { .. } => REASON_NOT_MPD,
        MpdError::Invalid { .. } => REASON_INVALID,
        MpdError::TooLarge { .. } => REASON_TOO_LARGE,
        _ => REASON_UNKNOWN,
    }
}

fn keeps_parked_prefix(class: FailureClass) -> bool {
    matches!(class, FailureClass::Retry { .. })
}

fn skipped_kind(track_id: Option<u32>, fourcc: String) -> ErrorKind {
    if ENCRYPTED_SAMPLE_ENTRIES.contains(&fourcc.as_str()) {
        ErrorKind::UnsupportedEncryption { method: fourcc }
    } else {
        ErrorKind::TrackDropped {
            track_id: track_id.unwrap_or(UNNAMED_TRACK_ID),
            stream_type: None,
            fourcc: Some(fourcc),
        }
    }
}

fn master_selection(selection: &ManifestSelection) -> MasterSelection {
    let variants = selection
        .variants
        .iter()
        .map(|v| VariantInfo {
            url: v.init_url.clone().unwrap_or_default(),
            bandwidth: u32::try_from(v.bandwidth).unwrap_or(u32::MAX),
            average_bandwidth: None,
            resolution: v.resolution,
            codecs: v.codecs.clone(),
            audio_group: v.audio_group.clone(),
            subtitles_group: None,
        })
        .collect();
    let renditions = selection
        .audio
        .iter()
        .map(|a| RenditionInfo {
            media_type: MediaType::Audio,
            group_id: a.group.clone().unwrap_or_else(|| a.set.to_string()),
            name: a.name.clone(),
            language: a.language.clone(),
            default: a.default,
            autoselect: a.autoselect,
            url: a.init_url.clone(),
        })
        .collect();
    MasterSelection {
        variants,
        chosen: selection.chosen.unwrap_or(0),
        renditions,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_runtime::client::FailureKind;

    #[test]
    fn only_a_resource_awaiting_retry_keeps_its_parked_prefix() {
        assert!(keeps_parked_prefix(FailureClass::Retry {
            attempt: 1,
            kind: FailureKind::Transport
        }));
        for class in [
            FailureClass::GiveUp,
            FailureClass::Expired,
            FailureClass::SegmentSkipped,
            FailureClass::EndTail,
            FailureClass::NotYetAvailable { retry_at_us: 5 },
        ] {
            assert!(!keeps_parked_prefix(class), "{class:?}");
        }
    }
}
