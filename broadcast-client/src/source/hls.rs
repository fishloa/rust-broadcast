//! Adapter over [`hls_runtime::client::HlsClient`]: per-request connection table, bounded body
//! accumulation, timeouts, retry backoff, and Output-to-Event mapping.

use alloc::collections::{BTreeSet, VecDeque};
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use std::collections::HashMap;

use hls_runtime::client::{
    Action as HlsAction, Error as CoreError, HlsClient, KeyId, Output, ResourceId,
};
use transmux::TrackId;
use url::Url;

use crate::auth::{AuthDecision, AuthState};
use crate::error::Error;
use crate::events::{AUDIO_TRACK_ID_OFFSET, Action, Config, ErrorKind, Event, Status};
use crate::redirect;
use crate::source::fetch::body::BodyAcc;
use crate::source::fetch::conn::{ConnAlloc, ConnLease};
use crate::source::fetch::deadlines::Deadlines;
use crate::source::fetch::failures::{Attempts, Failure, FailureCounter, Verdict, verdict};
use crate::source::fetch::retry::{Backoff, RetryGate, retry_after_us};
use crate::source::fetch::wall::WallClock;
use crate::source::fetch::window::{Lane, Lanes, Window};

const MICROS_PER_MILLI: u64 = 1_000;
const REDIRECT_STATUSES: [u16; 5] = [301, 302, 303, 307, 308];
const STATUS_UNAUTHORIZED: u16 = 401;
const STATUS_PROXY_AUTH_REQUIRED: u16 = 407;
const STATUS_FORBIDDEN: u16 = 403;
const STATUS_NOT_FOUND: u16 = 404;
const STATUS_GONE: u16 = 410;

const METHOD_GET: &str = "GET";

const ENCRYPTED_SAMPLE_ENTRIES: [&str; 2] = ["encv", "enca"];
const UNNAMED_TRACK_ID: u32 = 0;
const FIELD_MAX_MANIFEST_BYTES: &str = "max_manifest_bytes";
const FIELD_MAX_SEGMENT_BYTES: &str = "max_segment_bytes";
const REASON_NON_FRAGMENTED_MP4: &str = "non_fragmented_mp4";
const REASON_DAMAGED_FRAGMENT: &str = "damaged_fragment";

/// Which playlist an adapter plays.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Role {
    Main,
    Audio,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Purpose {
    Playlist,
    Resource(ResourceId),
    Key(KeyId),
}

#[derive(Debug, Clone, Copy)]
struct Timeouts {
    playlist_us: u64,
    segment_us: u64,
    key_us: u64,
}

impl Timeouts {
    fn of(&self, purpose: Purpose) -> u64 {
        match purpose {
            Purpose::Playlist | Purpose::Resource(ResourceId::Init) => self.playlist_us,
            Purpose::Resource(_) => self.segment_us,
            Purpose::Key(_) => self.key_us,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct BodyLimits {
    manifest: usize,
    segment: usize,
    key: usize,
}

struct Req {
    purpose: Purpose,
    original_url: String,
    url: Url,
    byte_range: Option<(u64, u64)>,
    redirects: u32,
    authed: bool,
    stale_replayed: bool,
    lease: Option<ConnLease>,
    started_us: u64,
    responded_ok: bool,
    body: BodyAcc,
}

impl Req {
    fn deadline(&self, timeouts: &Timeouts) -> Option<u64> {
        self.started_us.checked_add(timeouts.of(self.purpose))
    }

    fn new(purpose: Purpose, url: Url, byte_range: Option<(u64, u64)>, body_limit: usize) -> Self {
        Self {
            purpose,
            original_url: url.to_string(),
            url,
            byte_range,
            redirects: 0,
            authed: false,
            stale_replayed: false,
            lease: None,
            started_us: 0,
            responded_ok: false,
            body: BodyAcc::new(body_limit),
        }
    }
}

pub(crate) struct HlsSource {
    client: HlsClient,
    body_limits: BodyLimits,
    max_key_failures: u32,
    key_failures: Attempts,
    max_segment_failures: u32,
    seg_failures: FailureCounter<ResourceId>,
    seg_retry: BTreeSet<ResourceId>,
    skip_error: Option<ErrorKind>,
    retry_after_us: u64,
    reported: BTreeSet<&'static str>,
    fatal: bool,
    reserved_id_reported: bool,
    dropped_reported: BTreeSet<TrackId>,
    skipped_reported: BTreeSet<(Option<u32>, String)>,
    skipped_kind: Option<ErrorKind>,
    damaged_reported: bool,
    oversize_reported: BTreeSet<&'static str>,
    timeouts: Timeouts,
    backoff: Backoff,
    max_redirects: u32,
    reqs: HashMap<u32, Req>,
    deadlines: Deadlines<u32>,
    parked: Vec<Req>,
    lanes: Lanes<HlsAction>,
    window: Window<ResourceId>,
    gate: RetryGate,
    retry_refetch: Option<(u64, String)>,
    init_retry: bool,
    attempt: Attempts,
    established: bool,
    live: bool,
    conn_alloc: ConnAlloc,
    role: Role,
    actions: VecDeque<Action>,
    events: VecDeque<Event>,
}

impl HlsSource {
    pub(crate) fn new(url: &Url, config: &Config, conn_alloc: ConnAlloc, role: Role) -> Self {
        let mut client = HlsClient::new(url.as_str());
        client.set_variant_policy(config.variant_policy.clone());
        let mut source = Self {
            client,
            body_limits: BodyLimits {
                manifest: config.max_manifest_bytes,
                segment: config.max_segment_bytes,
                key: config.max_key_bytes,
            },
            max_key_failures: config.max_key_failures,
            key_failures: Attempts::default(),
            max_segment_failures: config.max_segment_failures,
            seg_failures: FailureCounter::new(),
            seg_retry: BTreeSet::new(),
            skip_error: None,
            retry_after_us: 0,
            reported: BTreeSet::new(),
            fatal: false,
            reserved_id_reported: false,
            dropped_reported: BTreeSet::new(),
            skipped_reported: BTreeSet::new(),
            skipped_kind: None,
            damaged_reported: false,
            oversize_reported: BTreeSet::new(),
            timeouts: Timeouts {
                playlist_us: config.request_timeout_us,
                segment_us: config.segment_timeout_us,
                key_us: config.hls_key_timeout_us,
            },
            backoff: Backoff::new(&config.backoff_ms, config.backoff_cap_ms),
            max_redirects: config.max_redirects,
            reqs: HashMap::new(),
            deadlines: Deadlines::new(),
            parked: Vec::new(),
            lanes: Lanes::new(),
            window: Window::new(config.max_inflight_segments),
            gate: RetryGate::default(),
            retry_refetch: None,
            init_retry: false,
            attempt: Attempts::default(),
            established: false,
            live: false,
            conn_alloc,
            role,
            actions: VecDeque::new(),
            events: VecDeque::new(),
        };
        source.pump_core();
        source
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

    pub(crate) fn owns(&self, conn: u32) -> bool {
        self.reqs.contains_key(&conn)
    }

    pub(crate) fn endlist_seen(&self) -> bool {
        self.client.endlist_seen()
    }

    pub(crate) fn target_duration_secs(&self) -> Option<u32> {
        self.client.target_duration_secs()
    }

    pub(crate) fn is_fatal(&self) -> bool {
        self.fatal
    }

    pub(crate) fn halt(&mut self) {
        self.fatal = true;
        self.reset_in_flight();
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
        self.client
            .select_variant(index)
            .map_err(|_| Error::NoSuchVariant(index))?;
        if self.fatal {
            return Ok(());
        }
        self.reset_in_flight();
        self.seg_failures.clear_all();
        self.attempt.reset();
        self.pump_core();
        self.drain_outputs();
        if !auth.is_waiting() {
            self.advance(now, auth);
        }
        Ok(())
    }

    pub(crate) fn reconnect(&mut self, now: u64, auth: &mut AuthState) {
        if self.fatal {
            return;
        }
        let in_flight: Vec<Purpose> = self.reqs.values().map(|r| r.purpose).collect();
        for purpose in in_flight {
            self.init_retry |= purpose == Purpose::Resource(ResourceId::Init);
            self.client_failed(purpose);
        }
        self.reset_in_flight();
        self.attempt.reset();
        self.retry_refetch = Some((now, self.client.playlist_url().to_string()));
        self.pump_core();
        self.drain_outputs();
        if !auth.is_waiting() {
            self.advance(now, auth);
        }
    }

    pub(crate) fn resume(&mut self, now: u64, auth: &mut AuthState) {
        if self.fatal {
            return;
        }
        self.attempt.reset();
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
                None => self.fail(req, now),
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
            if !req.body.accept(status, headers, req.byte_range) {
                self.actions.push_back(Action::Close { conn });
                self.fail_status(req, now, Some(status));
                if !auth.is_waiting() {
                    self.advance(now, auth);
                }
                return;
            }
            req.responded_ok = true;
            if !self.established {
                self.established = true;
                self.events.push_back(Event::Status(Status::Establishing));
            }
            self.reqs.insert(conn, req);
        } else {
            self.actions.push_back(Action::Close { conn });
            if status == STATUS_FORBIDDEN {
                self.events.push_back(Event::Forbidden { status });
            }
            if self.role == Role::Main
                && req.purpose == Purpose::Playlist
                && (status == STATUS_NOT_FOUND || status == STATUS_GONE)
            {
                self.events.push_back(Event::Status(Status::NotFound));
            }
            self.retry_after_us = retry_after_us(status, headers, now, wall);
            self.fail_status(req, now, Some(status));
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
        let Some(req) = self.reqs.get_mut(&conn) else {
            return;
        };
        if !req.responded_ok || bytes.is_empty() {
            return;
        }
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
                self.fail(req, now);
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
        let Some(req) = self.reqs.remove(&conn) else {
            return;
        };
        if error.is_some() || !req.responded_ok || req.body.is_truncated() {
            self.fail(req, now);
        } else {
            let mut req = req;
            if req.body.finish() {
                self.deliver(req, now);
            } else {
                self.fail(req, now);
            }
        }
        if !auth.is_waiting() {
            self.advance(now, auth);
        }
    }

    fn body_limit(&self, purpose: Purpose) -> usize {
        match purpose {
            Purpose::Playlist => self.body_limits.manifest,
            Purpose::Resource(_) => self.body_limits.segment,
            Purpose::Key(_) => self.body_limits.key,
        }
    }

    fn deliver(&mut self, req: Req, now: u64) {
        match req.purpose {
            Purpose::Resource(id) if id != ResourceId::Init => {
                self.window.hold(id, req.body.into_bytes());
                self.release_ready(now);
            }
            Purpose::Key(id) => self.deliver_key(id, req.body.as_slice(), now),
            Purpose::Playlist => {
                self.client.set_playlist_base(req.url.as_str());
                self.deliver_now(
                    Purpose::Playlist,
                    req.body.as_slice(),
                    Some(req.original_url),
                    now,
                );
            }
            purpose => self.deliver_now(purpose, req.body.as_slice(), Some(req.original_url), now),
        }
    }

    fn deliver_key(&mut self, id: KeyId, body: &[u8], now: u64) {
        match self.client.on_key(id, body) {
            Ok(()) => {
                self.attempt.reset();
                self.pump_core();
                self.drain_outputs();
            }
            Err(CoreError::UnrequestedKey { .. }) => {}
            Err(e) => {
                self.pump_core();
                self.drain_outputs();
                if let CoreError::DecryptFailed { id: rid, .. } | CoreError::NotMedia { id: rid } =
                    &e
                    && !matches!(rid, ResourceId::Init)
                {
                    self.window.requeue(*rid);
                    if self.core_error(&e) {
                        return;
                    }
                    self.fail_purpose(Purpose::Resource(*rid), None, now, None);
                    return;
                }
                if self.core_error(&e) {
                    return;
                }
                self.fail_purpose(Purpose::Key(id), None, now, None);
            }
        }
    }

    fn core_error(&mut self, error: &CoreError) -> bool {
        match classify(error) {
            Some((kind, true)) => {
                self.go_fatal(kind);
                true
            }
            Some((kind, false)) => self.report(kind),
            None => false,
        }
    }

    fn report(&mut self, kind: ErrorKind) -> bool {
        if verdict(self.key_failures.bump(), self.max_key_failures) == Verdict::GiveUp {
            self.go_fatal(kind);
            return true;
        }
        if self.reported.insert(kind.name()) {
            self.events.push_back(Event::Error { kind, fatal: false });
        }
        false
    }

    fn go_fatal(&mut self, kind: ErrorKind) {
        self.events.push_back(Event::Error { kind, fatal: true });
        self.fatal = true;
        self.reset_in_flight();
    }

    fn release_ready(&mut self, now: u64) {
        while let Some((id, body)) = self.window.pop_ready() {
            self.deliver_now(Purpose::Resource(id), &body, None, now);
        }
    }

    fn deliver_now(&mut self, purpose: Purpose, body: &[u8], retry_url: Option<String>, now: u64) {
        let result = match purpose {
            Purpose::Playlist => self.client.on_playlist(body),
            Purpose::Resource(id) => self.client.on_resource(id, body),
            Purpose::Key(_) => Ok(()),
        };
        let delivered = result.is_ok();
        if delivered {
            if self.key_failures.is_zero() {
                self.attempt.reset();
            }
            if let Purpose::Resource(id) = purpose {
                self.seg_failures.clear(&id);
            }
        } else if let Purpose::Resource(id) = purpose
            && id != ResourceId::Init
        {
            self.window.requeue(id);
        }
        self.pump_core();
        self.drain_outputs();
        if delivered {
            if purpose == Purpose::Playlist {
                self.purge_unrequested();
                self.release_ready(now);
            }
        } else {
            if purpose == Purpose::Resource(ResourceId::Init)
                && matches!(&result, Err(CoreError::InitUndecodable))
            {
                if !self.report(ErrorKind::InitDecodeFailed) {
                    self.retry_init_later(now);
                }
                return;
            }
            if let Err(e) = &result
                && self.core_error(e)
            {
                return;
            }
            self.fail_purpose(purpose, retry_url, now, None);
        }
    }

    fn purge_unrequested(&mut self) {
        let failed = self.window.failed();
        if failed.is_empty() {
            return;
        }
        let requested: BTreeSet<ResourceId> = self
            .lanes
            .resource_actions()
            .filter_map(|action| match action {
                HlsAction::FetchResource { id, .. } => Some(*id),
                _ => None,
            })
            .collect();
        for id in failed {
            if !requested.contains(&id) {
                self.window.retire(&id);
                self.seg_failures.clear(&id);
                self.seg_retry.remove(&id);
            }
        }
    }

    fn client_failed(&mut self, purpose: Purpose) {
        match purpose {
            Purpose::Playlist => self.client.on_error(None),
            Purpose::Resource(id) => self.client.on_error(Some(id)),
            Purpose::Key(id) => self.client.on_key_error(id),
        }
    }

    fn fail(&mut self, req: Req, now: u64) {
        self.fail_status(req, now, None);
    }

    fn fail_status(&mut self, req: Req, now: u64, status: Option<u16>) {
        if let Purpose::Key(_) = req.purpose
            && status != Some(STATUS_FORBIDDEN)
            && self.report(ErrorKind::KeyFetchFailed { status })
        {
            return;
        }
        if req.purpose == Purpose::Resource(ResourceId::Init) {
            if !self.report(ErrorKind::InitFetchFailed { status }) {
                self.fail_init_fetch(now);
            }
            return;
        }
        if let Purpose::Resource(id) = req.purpose {
            self.window.fail(id);
        }
        self.fail_purpose(req.purpose, Some(req.original_url), now, status);
    }

    fn fail_init_fetch(&mut self, now: u64) {
        self.client_failed(Purpose::Resource(ResourceId::Init));
        self.pump_core();
        self.drain_outputs();
        self.retry_init_later(now);
    }

    fn retry_init_later(&mut self, now: u64) {
        self.init_retry = true;
        let refetch = (!self.chain_alive() && !self.client.endlist_seen())
            .then(|| self.client.playlist_url().to_string());
        self.schedule_retry(now, refetch, None);
    }

    fn fail_purpose(
        &mut self,
        purpose: Purpose,
        retry_url: Option<String>,
        now: u64,
        status: Option<u16>,
    ) {
        self.client_failed(purpose);
        self.pump_core();
        self.drain_outputs();
        if let Purpose::Resource(id) = purpose
            && id != ResourceId::Init
        {
            self.fail_segment(id, status, now);
            return;
        }
        let refetch = match purpose {
            _ if self.chain_alive() => None,
            Purpose::Playlist => retry_url,
            Purpose::Resource(_) | Purpose::Key(_) => Some(self.client.playlist_url().to_string()),
        };
        self.schedule_retry(now, refetch, None);
    }

    fn fail_segment(&mut self, id: ResourceId, status: Option<u16>, now: u64) {
        let Failure { count, status } = self.seg_failures.record(id, status);
        if verdict(u64::from(count), self.max_segment_failures) == Verdict::GiveUp {
            self.give_up_segment(id, status, now);
            return;
        }
        let vod = self.client.endlist_seen();
        let refetch = if vod {
            self.seg_retry.insert(id);
            None
        } else if self.chain_alive() {
            None
        } else {
            Some(self.client.playlist_url().to_string())
        };
        self.schedule_retry(now, refetch, Some(count));
    }

    fn give_up_segment(&mut self, id: ResourceId, status: Option<u16>, now: u64) {
        self.seg_failures.clear(&id);
        let kind = ErrorKind::SegmentFetchFailed { status };
        if self.client.endlist_seen() {
            self.go_fatal(kind);
            return;
        }
        self.skip_error = Some(kind);
        self.window.retire(&id);
        self.seg_retry.remove(&id);
        self.client.skip_resource(id);
        self.pump_core();
        self.drain_outputs();
        self.release_ready(now);
    }

    fn chain_alive(&self) -> bool {
        self.retry_refetch.is_some()
            || self
                .lanes
                .any_other(|a| matches!(a, HlsAction::FetchPlaylist { .. }))
            || self.reqs.values().any(|r| r.purpose == Purpose::Playlist)
    }

    fn schedule_retry(&mut self, now: u64, refetch: Option<String>, segment_attempt: Option<u32>) {
        let attempt = match segment_attempt {
            Some(count) => u64::from(count),
            None => self.attempt.bump(),
        };
        let wait = self.backoff.delay_us(attempt).max(self.retry_after_us);
        let at = now.saturating_add(wait);
        self.events
            .push_back(Event::Status(Status::Reconnecting { attempt }));
        self.gate.hold_until(at);
        if let Some(url) = refetch
            && self.retry_refetch.is_none()
        {
            self.retry_refetch = Some((at, url));
        }
        self.actions.push_back(Action::WakeAt(at));
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
                self.fail(req, now);
            }
        }
    }

    fn next_action(&mut self) -> Option<HlsAction> {
        self.lanes.next(self.window.is_open())
    }

    fn advance(&mut self, now: u64, auth: &mut AuthState) {
        if self.fatal {
            return;
        }
        if let Some((at, _)) = &self.retry_refetch
            && now >= *at
            && let Some((_, url)) = self.retry_refetch.take()
        {
            self.issue(Purpose::Playlist, &url, None, now, auth);
        }
        loop {
            if !self.gate.lifted(now) {
                return;
            }
            if core::mem::take(&mut self.init_retry) && self.client.retry_init() {
                self.pump_core();
            }
            if !self.seg_retry.is_empty() {
                for id in core::mem::take(&mut self.seg_retry) {
                    self.client.retry_resource(id);
                }
                self.pump_core();
            }
            let Some(action) = self.next_action() else {
                return;
            };
            match action {
                HlsAction::FetchPlaylist { .. } => {
                    if let Some(url) = action.playlist_request_url() {
                        self.issue(Purpose::Playlist, &url, None, now, auth);
                    }
                }
                HlsAction::FetchResource {
                    id,
                    url,
                    byte_range,
                } => self.issue(Purpose::Resource(id), &url, byte_range, now, auth),
                HlsAction::FetchKey { id, url } => {
                    self.issue(Purpose::Key(id), &url, None, now, auth);
                }
                HlsAction::WaitMs(ms) => {
                    let at = now.saturating_add(ms.saturating_mul(MICROS_PER_MILLI));
                    self.gate.hold_until(at);
                    self.actions.push_back(Action::WakeAt(at));
                }
                _ => {}
            }
        }
    }

    fn issue(
        &mut self,
        purpose: Purpose,
        url: &str,
        byte_range: Option<(u64, u64)>,
        now: u64,
        auth: &mut AuthState,
    ) {
        if let Purpose::Resource(id) = purpose
            && id != ResourceId::Init
        {
            self.window.issue(id);
        }
        match Url::parse(url) {
            Ok(parsed) => self.send(
                Req::new(purpose, parsed, byte_range, self.body_limit(purpose)),
                auth,
                now,
            ),
            Err(_) => {
                if let Purpose::Resource(id) = purpose {
                    self.window.fail(id);
                }
                self.fail_purpose(purpose, None, now, None);
            }
        }
    }

    fn send(&mut self, mut req: Req, auth: &mut AuthState, now: u64) {
        let headers = auth.request_headers(&req.url, now);
        req.authed = crate::auth::carries_credentials(&headers);
        req.started_us = now;
        req.responded_ok = false;
        req.body.reset();
        let lease = self.conn_alloc.lease();
        let conn = lease.id();
        req.lease = Some(lease);
        let timeout = self.timeouts.of(req.purpose);
        self.actions.push_back(Action::HttpRequest {
            conn,
            method: METHOD_GET.to_string(),
            url: req.url.to_string(),
            headers,
            byte_range: req.byte_range,
        });
        self.actions
            .push_back(Action::WakeAt(now.saturating_add(timeout)));
        let deadline = now.checked_add(timeout);
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
    }

    fn reset_in_flight(&mut self) {
        let mut conns: Vec<u32> = self.reqs.keys().copied().collect();
        conns.sort_unstable();
        for conn in conns {
            self.actions.push_back(Action::Close { conn });
        }
        self.reqs.clear();
        self.deadlines.clear();
        self.parked.clear();
        self.lanes.clear();
        self.window.clear();
        self.retry_refetch = None;
        self.gate.clear();
    }

    fn remap(&self, id: TrackId) -> TrackId {
        match self.role {
            Role::Main => id,
            Role::Audio => TrackId::new(id.value().saturating_add(AUDIO_TRACK_ID_OFFSET)),
        }
    }

    fn is_reserved(&self, id: TrackId) -> bool {
        self.role == Role::Main && id.value() >= AUDIO_TRACK_ID_OFFSET
    }

    fn pump_core(&mut self) {
        while let Some(action) = self.client.poll() {
            let lane = match &action {
                HlsAction::FetchResource { id, .. } if *id != ResourceId::Init => {
                    if self.window.contains(id) {
                        Lane::Retry
                    } else {
                        Lane::Fresh
                    }
                }
                _ => Lane::Other,
            };
            self.lanes.push(lane, action);
        }
    }

    fn drain_outputs(&mut self) {
        while let Some(output) = self.client.next_output() {
            match output {
                Output::Master(selection) => self.events.push_back(Event::Variants(selection)),
                Output::Tracks(specs) => {
                    let (kept, rejected): (Vec<_>, Vec<_>) = specs
                        .into_iter()
                        .partition(|spec| !self.is_reserved(spec.track_id));
                    if let Some(first) = rejected.first()
                        && !self.reserved_id_reported
                    {
                        self.reserved_id_reported = true;
                        self.events.push_back(Event::Error {
                            kind: ErrorKind::TrackIdReserved {
                                track_id: first.track_id.value(),
                            },
                            fatal: false,
                        });
                    }
                    let mut specs = kept;
                    for spec in &mut specs {
                        spec.track_id = self.remap(spec.track_id);
                    }
                    if specs.is_empty()
                        && let Some(kind) = self.skipped_kind.clone()
                    {
                        self.go_fatal(kind);
                        return;
                    }
                    self.events.push_back(Event::Tracks(specs));
                }
                Output::Samples { track_id, .. } | Output::DisplayChanged { track_id, .. }
                    if self.is_reserved(track_id) => {}
                Output::Samples { track_id, samples } => {
                    if !samples.is_empty() {
                        self.key_failures.reset();
                        self.reported.clear();
                    }
                    if !self.live && !samples.is_empty() {
                        self.live = true;
                        self.events.push_back(Event::Status(Status::Live));
                    }
                    for sample in samples {
                        self.events.push_back(Event::Sample {
                            track: self.remap(track_id),
                            sample,
                            join: None,
                        });
                    }
                }
                Output::DisplayChanged {
                    track_id,
                    pts,
                    display,
                } => self.events.push_back(Event::DisplayChanged {
                    track: self.remap(track_id),
                    pts,
                    display,
                }),
                Output::TrackDropped {
                    track_id,
                    stream_type,
                } => {
                    let track_id = self.remap(track_id);
                    if self.dropped_reported.insert(track_id) {
                        self.events.push_back(Event::Error {
                            kind: ErrorKind::TrackDropped {
                                track_id: track_id.value(),
                                stream_type,
                                fourcc: None,
                            },
                            fatal: false,
                        });
                    }
                }
                Output::TrackSkipped {
                    track_id, fourcc, ..
                } => {
                    let track_id = track_id.map(|id| self.remap(id).value());
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
                Output::Discontinuity => {
                    if let Some(kind) = self.skip_error.take() {
                        self.events.push_back(Event::Error { kind, fatal: false });
                    }
                    self.events.push_back(Event::Discontinuity);
                }
                Output::EndOfStream => self.events.push_back(Event::Status(Status::Ended)),
                _ => {}
            }
        }
    }
}

fn oversize_field(purpose: Purpose) -> Option<&'static str> {
    match purpose {
        Purpose::Playlist => Some(FIELD_MAX_MANIFEST_BYTES),
        Purpose::Resource(_) => Some(FIELD_MAX_SEGMENT_BYTES),
        Purpose::Key(_) => None,
    }
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

fn classify(error: &CoreError) -> Option<(ErrorKind, bool)> {
    match error {
        CoreError::NotFragmented { .. } => Some((
            ErrorKind::UnsupportedContainer {
                reason: REASON_NON_FRAGMENTED_MP4,
            },
            true,
        )),
        CoreError::DecryptFailed { .. } => Some((ErrorKind::DecryptFailed, false)),
        CoreError::BadKeyLength { .. } => Some((ErrorKind::BadKey, false)),
        CoreError::UnsupportedEncryption { method, .. } => Some((
            ErrorKind::UnsupportedEncryption {
                method: method.to_string(),
            },
            true,
        )),
        CoreError::PlaylistParse(broadcast_hls::Error::UnsupportedKeyFormat { keyformat }) => {
            Some((
                ErrorKind::UnsupportedEncryption {
                    method: keyformat.clone(),
                },
                true,
            ))
        }
        CoreError::PlaylistParse(broadcast_hls::Error::KeyWithoutUri { method }) => Some((
            ErrorKind::UnsupportedEncryption {
                method: method.to_string(),
            },
            true,
        )),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAX_TEST_BODY: usize = 1024;
    use crate::source::fetch::deadlines::{SLACK_FACTOR, SLACK_FLOOR};

    fn source_with_auth() -> (HlsSource, AuthState, Url) {
        let url = Url::parse("http://o.test/p.m3u8").unwrap();
        let auth = AuthState::new(url.clone(), Vec::new());
        let source = HlsSource::new(&url, &Config::default(), ConnAlloc::new(), Role::Main);
        (source, auth, url)
    }

    #[test]
    fn the_deadline_heap_stays_proportional_to_live_requests() {
        let (mut source, mut auth, url) = source_with_auth();
        for now in 0..10_000u64 {
            source.send(
                Req::new(Purpose::Playlist, url.clone(), None, MAX_TEST_BODY),
                &mut auth,
                now,
            );
            let conn = source.reqs.keys().next().copied().unwrap();
            source.reqs.remove(&conn);
        }
        assert!(
            source.deadlines.len() <= SLACK_FLOOR + SLACK_FACTOR,
            "{} stale deadlines kept",
            source.deadlines.len()
        );
    }

    #[test]
    fn expire_fails_only_the_overdue_live_request() {
        let (mut source, mut auth, url) = source_with_auth();
        let timeout = source.timeouts.playlist_us;
        source.send(
            Req::new(Purpose::Playlist, url.clone(), None, MAX_TEST_BODY),
            &mut auth,
            0,
        );
        let overdue = source.reqs.keys().next().copied().unwrap();
        source.send(
            Req::new(Purpose::Playlist, url.clone(), None, MAX_TEST_BODY),
            &mut auth,
            10,
        );
        let completed = source
            .reqs
            .keys()
            .find(|c| **c != overdue)
            .copied()
            .unwrap();
        source.reqs.remove(&completed);
        source.take_actions();

        source.expire(timeout - 1);
        assert!(source.take_actions().is_empty());
        assert!(source.reqs.contains_key(&overdue));

        source.expire(timeout);
        let closes: Vec<u32> = source
            .take_actions()
            .into_iter()
            .filter_map(|a| match a {
                Action::Close { conn } => Some(conn),
                _ => None,
            })
            .collect();
        assert_eq!(closes, vec![overdue]);

        source.expire(timeout * 2);
        assert!(
            !source
                .take_actions()
                .iter()
                .any(|a| matches!(a, Action::Close { .. })),
            "a request that completed before its deadline is never expired"
        );
    }
    #[test]
    fn a_stale_deadline_never_expires_a_newer_request_on_a_reused_id() {
        let (mut source, mut auth, url) = source_with_auth();
        let timeout = source.timeouts.playlist_us;
        source.send(
            Req::new(Purpose::Playlist, url.clone(), None, MAX_TEST_BODY),
            &mut auth,
            0,
        );
        let conn = source.reqs.keys().next().copied().unwrap();
        source.reqs.remove(&conn);
        let mut newer = Req::new(Purpose::Playlist, url, None, MAX_TEST_BODY);
        newer.started_us = timeout / 2;
        source.reqs.insert(conn, newer);
        source.take_actions();

        source.expire(timeout);
        assert!(source.take_actions().is_empty());
        assert!(source.reqs.contains_key(&conn));
    }
}
