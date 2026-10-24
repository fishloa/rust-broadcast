//! Adapter over [`hls_runtime::client::HlsClient`]: per-request connection table, bounded body
//! accumulation, timeouts, retry backoff, and Output-to-Event mapping.

use alloc::collections::{BTreeSet, VecDeque};
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicU32, Ordering};
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

const MICROS_PER_MILLI: u64 = 1_000;
const REDIRECT_STATUSES: [u16; 5] = [301, 302, 303, 307, 308];
const STATUS_UNAUTHORIZED: u16 = 401;
const STATUS_PROXY_AUTH_REQUIRED: u16 = 407;
const STATUS_FORBIDDEN: u16 = 403;
const STATUS_NOT_FOUND: u16 = 404;
const STATUS_GONE: u16 = 410;
const AUTH_STATUSES: [u16; 3] = [
    STATUS_UNAUTHORIZED,
    STATUS_PROXY_AUTH_REQUIRED,
    STATUS_FORBIDDEN,
];
const METHOD_GET: &str = "GET";

const FIRST_CONN_ID: u32 = 1;

/// Connection-id allocator shared by every adapter of one session.
#[derive(Debug, Clone)]
pub(crate) struct ConnAlloc(Arc<AtomicU32>);

impl ConnAlloc {
    pub(crate) fn new() -> Self {
        Self(Arc::new(AtomicU32::new(FIRST_CONN_ID)))
    }

    fn next(&self) -> u32 {
        self.0
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |c| {
                Some(c.checked_add(1).unwrap_or(FIRST_CONN_ID))
            })
            .unwrap_or(FIRST_CONN_ID)
    }
}

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

struct Req {
    purpose: Purpose,
    original_url: String,
    url: Url,
    byte_range: Option<(u64, u64)>,
    redirects: u32,
    authed: bool,
    started_us: u64,
    responded_ok: bool,
    expected_len: Option<u64>,
    body: Vec<u8>,
}

impl Req {
    fn new(purpose: Purpose, url: Url, byte_range: Option<(u64, u64)>) -> Self {
        Self {
            purpose,
            original_url: url.to_string(),
            url,
            byte_range,
            redirects: 0,
            authed: false,
            started_us: 0,
            responded_ok: false,
            expected_len: None,
            body: Vec::new(),
        }
    }
}

pub(crate) struct HlsSource {
    client: HlsClient,
    max_playlist_bytes: usize,
    max_segment_bytes: usize,
    max_key_bytes: usize,
    key_timeout_us: u64,
    max_key_failures: u32,
    key_failures: u32,
    reported: BTreeSet<&'static str>,
    fatal: bool,
    playlist_timeout_us: u64,
    segment_timeout_us: u64,
    backoff_ms: Vec<u64>,
    backoff_cap_ms: u64,
    max_redirects: u32,
    reqs: HashMap<u32, Req>,
    parked: Vec<Req>,
    pending: VecDeque<HlsAction>,
    order: VecDeque<ResourceId>,
    held: HashMap<ResourceId, Vec<u8>>,
    failed: BTreeSet<ResourceId>,
    gate_until: Option<u64>,
    retry_refetch: Option<(u64, String)>,
    attempt: u64,
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
            max_playlist_bytes: config.max_playlist_bytes,
            max_segment_bytes: config.max_segment_bytes,
            max_key_bytes: config.max_key_bytes,
            key_timeout_us: config.hls_key_timeout_us,
            max_key_failures: config.max_key_failures,
            key_failures: 0,
            reported: BTreeSet::new(),
            fatal: false,
            playlist_timeout_us: config.hls_request_timeout_us,
            segment_timeout_us: config.hls_segment_timeout_us,
            backoff_ms: config.hls_backoff_ms.clone(),
            backoff_cap_ms: config.hls_backoff_cap_ms,
            max_redirects: config.max_redirects,
            reqs: HashMap::new(),
            parked: Vec::new(),
            pending: VecDeque::new(),
            order: VecDeque::new(),
            held: HashMap::new(),
            failed: BTreeSet::new(),
            gate_until: None,
            retry_refetch: None,
            attempt: 0,
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
        self.attempt = 0;
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
            self.client_failed(purpose);
        }
        self.reset_in_flight();
        self.attempt = 0;
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
        self.attempt = 0;
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
            match auth.handle_unauthorized(status, headers, req.authed) {
                AuthDecision::Replay => self.send(req, auth, now),
                AuthDecision::Wait(events) => {
                    self.events.extend(events);
                    self.parked.push(req);
                }
            }
        } else if (200..300).contains(&status) {
            req.responded_ok = true;
            req.expected_len = headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
                .and_then(|(_, v)| v.trim().parse::<u64>().ok());
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
            self.fail_status(req, now, Some(status));
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
        let limit = match req.purpose {
            Purpose::Playlist => self.max_playlist_bytes,
            Purpose::Resource(_) => self.max_segment_bytes,
            Purpose::Key(_) => self.max_key_bytes,
        };
        if req.body.len().saturating_add(bytes.len()) > limit {
            if let Some(req) = self.reqs.remove(&conn) {
                self.actions.push_back(Action::Close { conn });
                self.fail(req, now);
                if !auth.is_waiting() {
                    self.advance(now, auth);
                }
            }
            return;
        }
        req.body.extend_from_slice(bytes);
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
        let truncated = req
            .expected_len
            .is_some_and(|n| n != u64::try_from(req.body.len()).unwrap_or(u64::MAX));
        if error.is_some() || !req.responded_ok || truncated {
            self.fail(req, now);
        } else {
            self.deliver(req, now);
        }
        if !auth.is_waiting() {
            self.advance(now, auth);
        }
    }

    fn deliver(&mut self, req: Req, now: u64) {
        match req.purpose {
            Purpose::Resource(id) if id != ResourceId::Init => {
                self.held.insert(id, req.body);
                self.release_ready(now);
            }
            Purpose::Key(id) => self.deliver_key(id, &req.body, now),
            purpose => self.deliver_now(purpose, &req.body, Some(req.original_url), now),
        }
    }

    fn deliver_key(&mut self, id: KeyId, body: &[u8], now: u64) {
        match self.client.on_key(id, body) {
            Ok(()) => {
                self.attempt = 0;
                self.pump_core();
                self.drain_outputs();
            }
            Err(CoreError::UnrequestedKey { .. }) => {}
            Err(e) => {
                self.pump_core();
                self.drain_outputs();
                if let CoreError::DecryptFailed { id: rid, .. } = &e
                    && !matches!(rid, ResourceId::Init)
                {
                    self.order.push_front(*rid);
                    self.failed.insert(*rid);
                    if self.core_error(&e) {
                        return;
                    }
                    self.fail_purpose(Purpose::Resource(*rid), None, now);
                    return;
                }
                if self.core_error(&e) {
                    return;
                }
                self.fail_purpose(Purpose::Key(id), None, now);
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
        self.key_failures = self.key_failures.saturating_add(1);
        if self.key_failures >= self.max_key_failures {
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
        while let Some(front) = self.order.front().copied() {
            if self.failed.contains(&front) {
                return;
            }
            let Some(body) = self.held.remove(&front) else {
                return;
            };
            self.order.pop_front();
            self.deliver_now(Purpose::Resource(front), &body, None, now);
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
            if self.key_failures == 0 {
                self.attempt = 0;
            }
        } else if let Purpose::Resource(id) = purpose
            && id != ResourceId::Init
        {
            self.order.push_front(id);
            self.failed.insert(id);
        }
        self.pump_core();
        self.drain_outputs();
        if delivered {
            if purpose == Purpose::Playlist {
                self.purge_unrequested();
                self.release_ready(now);
            }
        } else {
            if let Err(e) = &result
                && self.core_error(e)
            {
                return;
            }
            self.fail_purpose(purpose, retry_url, now);
        }
    }

    fn purge_unrequested(&mut self) {
        let failed: Vec<ResourceId> = self.failed.iter().copied().collect();
        for id in failed {
            let requested = self
                .pending
                .iter()
                .any(|a| matches!(a, HlsAction::FetchResource { id: pid, .. } if *pid == id));
            if !requested {
                self.failed.remove(&id);
                self.order.retain(|o| *o != id);
                self.held.remove(&id);
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
            && !status.is_some_and(|s| AUTH_STATUSES.contains(&s))
            && self.report(ErrorKind::KeyFetchFailed { status })
        {
            return;
        }
        if let Purpose::Resource(id) = req.purpose
            && id != ResourceId::Init
        {
            self.failed.insert(id);
        }
        self.fail_purpose(req.purpose, Some(req.original_url), now);
    }

    fn fail_purpose(&mut self, purpose: Purpose, retry_url: Option<String>, now: u64) {
        self.client_failed(purpose);
        self.pump_core();
        self.drain_outputs();
        let refetch = match purpose {
            Purpose::Playlist => retry_url,
            Purpose::Resource(_) | Purpose::Key(_) if self.chain_alive() => None,
            Purpose::Resource(_) | Purpose::Key(_) => Some(self.client.playlist_url().to_string()),
        };
        self.schedule_retry(now, refetch);
    }

    fn chain_alive(&self) -> bool {
        self.retry_refetch.is_some()
            || self
                .pending
                .iter()
                .any(|a| matches!(a, HlsAction::FetchPlaylist { .. }))
            || self.reqs.values().any(|r| r.purpose == Purpose::Playlist)
    }

    fn backoff_us(&self) -> u64 {
        let idx = usize::try_from(self.attempt.saturating_sub(1)).unwrap_or(usize::MAX);
        self.backoff_ms
            .get(idx)
            .copied()
            .unwrap_or(self.backoff_cap_ms)
            .saturating_mul(MICROS_PER_MILLI)
    }

    fn schedule_retry(&mut self, now: u64, refetch: Option<String>) {
        self.attempt = self.attempt.saturating_add(1);
        let at = now.saturating_add(self.backoff_us());
        self.events.push_back(Event::Status(Status::Reconnecting {
            attempt: self.attempt,
        }));
        self.gate_until = Some(self.gate_until.map_or(at, |g| g.max(at)));
        if let Some(url) = refetch
            && self.retry_refetch.is_none()
        {
            self.retry_refetch = Some((at, url));
        }
        self.actions.push_back(Action::WakeAt(at));
    }

    fn expire(&mut self, now: u64) {
        let playlist_timeout = self.playlist_timeout_us;
        let segment_timeout = self.segment_timeout_us;
        let key_timeout = self.key_timeout_us;
        let mut expired: Vec<u32> = self
            .reqs
            .iter()
            .filter(|(_, r)| {
                let limit = match r.purpose {
                    Purpose::Playlist => playlist_timeout,
                    Purpose::Resource(_) => segment_timeout,
                    Purpose::Key(_) => key_timeout,
                };
                now.saturating_sub(r.started_us) >= limit
            })
            .map(|(c, _)| *c)
            .collect();
        expired.sort_unstable();
        for conn in expired {
            if let Some(req) = self.reqs.remove(&conn) {
                self.actions.push_back(Action::Close { conn });
                self.fail(req, now);
            }
        }
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
            if let Some(g) = self.gate_until {
                if now < g {
                    return;
                }
                self.gate_until = None;
            }
            let Some(action) = self.pending.pop_front() else {
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
                    self.gate_until = Some(at);
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
            self.failed.remove(&id);
            if !self.order.contains(&id) {
                self.order.push_back(id);
            }
        }
        match Url::parse(url) {
            Ok(parsed) => self.send(Req::new(purpose, parsed, byte_range), auth, now),
            Err(_) => {
                if let Purpose::Resource(id) = purpose {
                    self.failed.insert(id);
                }
                self.fail_purpose(purpose, None, now);
            }
        }
    }

    fn send(&mut self, mut req: Req, auth: &mut AuthState, now: u64) {
        let headers = auth.request_headers(&req.url, now);
        req.authed = headers
            .iter()
            .any(|(k, _)| k.eq_ignore_ascii_case("authorization"));
        req.started_us = now;
        req.responded_ok = false;
        req.expected_len = None;
        req.body.clear();
        let conn = self.conn_alloc.next();
        let timeout = match req.purpose {
            Purpose::Playlist => self.playlist_timeout_us,
            Purpose::Resource(_) => self.segment_timeout_us,
            Purpose::Key(_) => self.key_timeout_us,
        };
        self.actions.push_back(Action::HttpRequest {
            conn,
            method: METHOD_GET.to_string(),
            url: req.url.to_string(),
            headers,
            byte_range: req.byte_range,
        });
        self.actions
            .push_back(Action::WakeAt(now.saturating_add(timeout)));
        self.reqs.insert(conn, req);
    }

    fn reset_in_flight(&mut self) {
        let mut conns: Vec<u32> = self.reqs.keys().copied().collect();
        conns.sort_unstable();
        for conn in conns {
            self.actions.push_back(Action::Close { conn });
        }
        self.reqs.clear();
        self.parked.clear();
        self.pending.clear();
        self.order.clear();
        self.held.clear();
        self.failed.clear();
        self.retry_refetch = None;
        self.gate_until = None;
    }

    fn remap(&self, id: TrackId) -> TrackId {
        match self.role {
            Role::Main => id,
            Role::Audio => TrackId::new(id.value().saturating_add(AUDIO_TRACK_ID_OFFSET)),
        }
    }

    fn pump_core(&mut self) {
        while let Some(action) = self.client.poll() {
            self.pending.push_back(action);
        }
    }

    fn drain_outputs(&mut self) {
        while let Some(output) = self.client.next_output() {
            match output {
                Output::Master(selection) => self.events.push_back(Event::Variants(selection)),
                Output::Tracks(mut specs) => {
                    for spec in &mut specs {
                        spec.track_id = self.remap(spec.track_id);
                    }
                    self.events.push_back(Event::Tracks(specs));
                }
                Output::Samples { track_id, samples } => {
                    if !samples.is_empty() {
                        self.key_failures = 0;
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
                Output::Discontinuity => self.events.push_back(Event::Discontinuity),
                Output::EndOfStream => self.events.push_back(Event::Status(Status::Ended)),
                _ => {}
            }
        }
    }
}

fn classify(error: &CoreError) -> Option<(ErrorKind, bool)> {
    match error {
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
