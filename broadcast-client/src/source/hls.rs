//! Adapter over [`hls_runtime::client::HlsClient`]: per-request connection table, bounded body
//! accumulation, timeouts, retry backoff, and Output-to-Event mapping.

use alloc::collections::{BTreeSet, BinaryHeap, VecDeque};
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::cmp::Reverse;
use core::sync::atomic::{AtomicU32, Ordering};
use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};

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
const STATUS_OK: u16 = 200;
const STATUS_PARTIAL_CONTENT: u16 = 206;
const STATUS_UNAUTHORIZED: u16 = 401;
const STATUS_PROXY_AUTH_REQUIRED: u16 = 407;
const STATUS_FORBIDDEN: u16 = 403;
const STATUS_NOT_FOUND: u16 = 404;
const STATUS_GONE: u16 = 410;

const METHOD_GET: &str = "GET";

const FIRST_CONN_ID: u32 = 1;
const ENCRYPTED_SAMPLE_ENTRIES: [&str; 2] = ["encv", "enca"];
const UNNAMED_TRACK_ID: u32 = 0;
const DEADLINE_SLACK_FACTOR: usize = 4;
const DEADLINE_SLACK_FLOOR: usize = 64;

type LiveIds = Arc<Mutex<BTreeSet<u32>>>;
type Queued = (u64, HlsAction);

#[derive(Debug, Clone, Copy)]
enum Lane {
    Other,
    Retry,
    Fresh,
}

/// Connection-id allocator shared by every adapter of one session. An id stays reserved while its
/// [`ConnLease`] lives, so a counter that wraps never hands out an id still in use.
#[derive(Debug, Clone)]
pub(crate) struct ConnAlloc {
    next: Arc<AtomicU32>,
    live: LiveIds,
}

/// A reserved connection id, released on drop.
#[derive(Debug)]
pub(crate) struct ConnLease {
    id: u32,
    live: LiveIds,
}

impl ConnLease {
    fn id(&self) -> u32 {
        self.id
    }
}

impl Drop for ConnLease {
    fn drop(&mut self) {
        self.live
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&self.id);
    }
}

impl ConnAlloc {
    pub(crate) fn new() -> Self {
        Self {
            next: Arc::new(AtomicU32::new(FIRST_CONN_ID)),
            live: Arc::new(Mutex::new(BTreeSet::new())),
        }
    }

    fn lease(&self) -> ConnLease {
        let mut live = self.live.lock().unwrap_or_else(PoisonError::into_inner);
        loop {
            let candidate = self
                .next
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |c| {
                    Some(c.checked_add(1).unwrap_or(FIRST_CONN_ID))
                })
                .unwrap_or(FIRST_CONN_ID);
            if live.insert(candidate) {
                return ConnLease {
                    id: candidate,
                    live: self.live.clone(),
                };
            }
        }
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
    stale_replayed: bool,
    lease: Option<ConnLease>,
    started_us: u64,
    responded_ok: bool,
    expected_len: Option<u64>,
    slice: Option<(usize, usize)>,
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
            stale_replayed: false,
            lease: None,
            started_us: 0,
            responded_ok: false,
            expected_len: None,
            slice: None,
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
    reserved_id_reported: bool,
    dropped_reported: BTreeSet<TrackId>,
    skipped_reported: BTreeSet<(Option<u32>, String)>,
    skipped_kind: Option<ErrorKind>,
    playlist_timeout_us: u64,
    segment_timeout_us: u64,
    backoff_ms: Vec<u64>,
    backoff_cap_ms: u64,
    max_redirects: u32,
    max_inflight_segments: usize,
    reqs: HashMap<u32, Req>,
    deadlines: BinaryHeap<Reverse<(u64, u32)>>,
    parked: Vec<Req>,
    queue_seq: u64,
    pending: VecDeque<Queued>,
    retries: VecDeque<Queued>,
    fresh: VecDeque<Queued>,
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
        let (backoff_ms, backoff_cap_ms) = config.hls_backoff();
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
            reserved_id_reported: false,
            dropped_reported: BTreeSet::new(),
            skipped_reported: BTreeSet::new(),
            skipped_kind: None,
            playlist_timeout_us: config.hls_request_timeout_us,
            segment_timeout_us: config.hls_segment_timeout_us,
            backoff_ms,
            backoff_cap_ms,
            max_redirects: config.max_redirects,
            max_inflight_segments: config.hls_max_inflight_segments.max(1),
            reqs: HashMap::new(),
            deadlines: BinaryHeap::new(),
            parked: Vec::new(),
            queue_seq: 0,
            pending: VecDeque::new(),
            retries: VecDeque::new(),
            fresh: VecDeque::new(),
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
            if !Self::range_honoured(&mut req, status, headers) {
                self.actions.push_back(Action::Close { conn });
                self.fail_status(req, now, Some(status));
                if !auth.is_waiting() {
                    self.advance(now, auth);
                }
                return;
            }
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
            let mut req = req;
            if Self::slice_body(&mut req) {
                self.deliver(req, now);
            } else {
                self.fail(req, now);
            }
        }
        if !auth.is_waiting() {
            self.advance(now, auth);
        }
    }

    /// A request carrying a byte range must be answered with `206` and a `Content-Range` naming
    /// exactly that range, or with a `200` (the server ignored `Range`), from which the range is
    /// sliced out once the whole body arrived (the body cap applies to the whole file). Any other
    /// 2xx answer to a ranged request is a failure.
    fn range_honoured(req: &mut Req, status: u16, headers: &[(String, String)]) -> bool {
        let Some((start, len)) = req.byte_range else {
            return true;
        };
        match status {
            STATUS_PARTIAL_CONTENT => content_range_is(headers, start, len),
            STATUS_OK => {
                let bounds = usize::try_from(start)
                    .ok()
                    .zip(usize::try_from(len).ok())
                    .filter(|(s, l)| s.checked_add(*l).is_some());
                req.slice = bounds;
                bounds.is_some()
            }
            _ => false,
        }
    }

    fn slice_body(req: &mut Req) -> bool {
        let Some((start, len)) = req.slice else {
            return true;
        };
        let Some(end) = start.checked_add(len).filter(|end| *end <= req.body.len()) else {
            return false;
        };
        req.body.truncate(end);
        req.body.drain(..start);
        true
    }

    fn deliver(&mut self, req: Req, now: u64) {
        match req.purpose {
            Purpose::Resource(id) if id != ResourceId::Init => {
                self.held.insert(id, req.body);
                self.release_ready(now);
            }
            Purpose::Key(id) => self.deliver_key(id, &req.body, now),
            Purpose::Playlist => {
                self.client.set_playlist_base(req.url.as_str());
                self.deliver_now(Purpose::Playlist, &req.body, Some(req.original_url), now);
            }
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
        if self.failed.is_empty() {
            return;
        }
        let requested: BTreeSet<ResourceId> = self
            .retries
            .iter()
            .chain(self.fresh.iter())
            .filter_map(|(_, action)| match action {
                HlsAction::FetchResource { id, .. } => Some(*id),
                _ => None,
            })
            .collect();
        let failed: Vec<ResourceId> = self.failed.iter().copied().collect();
        for id in failed {
            if !requested.contains(&id) {
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
            && status != Some(STATUS_FORBIDDEN)
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
            _ if self.chain_alive() => None,
            Purpose::Playlist => retry_url,
            Purpose::Resource(_) | Purpose::Key(_) => Some(self.client.playlist_url().to_string()),
        };
        self.schedule_retry(now, refetch);
    }

    fn chain_alive(&self) -> bool {
        self.retry_refetch.is_some()
            || self
                .pending
                .iter()
                .any(|(_, a)| matches!(a, HlsAction::FetchPlaylist { .. }))
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

    fn timeout_us(&self, purpose: Purpose) -> u64 {
        match purpose {
            Purpose::Playlist => self.playlist_timeout_us,
            Purpose::Resource(_) => self.segment_timeout_us,
            Purpose::Key(_) => self.key_timeout_us,
        }
    }

    fn deadline_of(&self, req: &Req) -> Option<u64> {
        req.started_us.checked_add(self.timeout_us(req.purpose))
    }

    fn expire(&mut self, now: u64) {
        while let Some(Reverse((deadline, conn))) = self.deadlines.peek().copied() {
            if deadline > now {
                break;
            }
            self.deadlines.pop();
            let live = self
                .reqs
                .get(&conn)
                .is_some_and(|r| self.deadline_of(r) == Some(deadline));
            if !live {
                continue;
            }
            if let Some(req) = self.reqs.remove(&conn) {
                self.actions.push_back(Action::Close { conn });
                self.fail(req, now);
            }
        }
    }

    fn next_action(&mut self) -> Option<HlsAction> {
        let window_open = self.order.len() < self.max_inflight_segments;
        let head = |queue: &VecDeque<Queued>| queue.front().map(|(seq, _)| *seq);
        let lane = [
            (head(&self.pending), Lane::Other),
            (head(&self.retries), Lane::Retry),
            (head(&self.fresh).filter(|_| window_open), Lane::Fresh),
        ]
        .into_iter()
        .filter_map(|(seq, lane)| seq.map(|seq| (seq, lane)))
        .min_by_key(|(seq, _)| *seq)
        .map(|(_, lane)| lane)?;
        let queue = match lane {
            Lane::Other => &mut self.pending,
            Lane::Retry => &mut self.retries,
            Lane::Fresh => &mut self.fresh,
        };
        queue.pop_front().map(|(_, action)| action)
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
        req.authed = crate::auth::carries_credentials(&headers);
        req.started_us = now;
        req.responded_ok = false;
        req.expected_len = None;
        req.slice = None;
        req.body.clear();
        let lease = self.conn_alloc.lease();
        let conn = lease.id();
        req.lease = Some(lease);
        let timeout = self.timeout_us(req.purpose);
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
            self.deadlines.push(Reverse((deadline, conn)));
        }
        self.compact_deadlines();
    }

    fn compact_deadlines(&mut self) {
        let limit = self
            .reqs
            .len()
            .saturating_mul(DEADLINE_SLACK_FACTOR)
            .saturating_add(DEADLINE_SLACK_FLOOR);
        if self.deadlines.len() <= limit {
            return;
        }
        self.deadlines = self
            .reqs
            .iter()
            .filter_map(|(conn, r)| self.deadline_of(r).map(|d| Reverse((d, *conn))))
            .collect();
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
        self.pending.clear();
        self.retries.clear();
        self.fresh.clear();
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

    fn is_reserved(&self, id: TrackId) -> bool {
        self.role == Role::Main && id.value() >= AUDIO_TRACK_ID_OFFSET
    }

    fn pump_core(&mut self) {
        while let Some(action) = self.client.poll() {
            let seq = self.queue_seq;
            self.queue_seq = self.queue_seq.wrapping_add(1);
            let queue = match &action {
                HlsAction::FetchResource { id, .. } if *id != ResourceId::Init => {
                    if self.order.contains(id) {
                        &mut self.retries
                    } else {
                        &mut self.fresh
                    }
                }
                _ => &mut self.pending,
            };
            queue.push_back((seq, action));
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
                Output::Discontinuity => self.events.push_back(Event::Discontinuity),
                Output::EndOfStream => self.events.push_back(Event::Status(Status::Ended)),
                _ => {}
            }
        }
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

fn content_range_is(headers: &[(String, String)], start: u64, len: u64) -> bool {
    let Some(last) = len.checked_sub(1).and_then(|l| start.checked_add(l)) else {
        return false;
    };
    headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-range"))
        .and_then(|(_, v)| v.trim().strip_prefix("bytes "))
        .and_then(|v| v.split_once('/'))
        .and_then(|(range, _total)| range.split_once('-'))
        .is_some_and(|(a, b)| {
            a.trim().parse::<u64>().ok() == Some(start)
                && b.trim().parse::<u64>().ok() == Some(last)
        })
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

#[cfg(test)]
mod tests {
    use super::*;

    impl ConnAlloc {
        fn starting_at(n: u32) -> Self {
            let alloc = Self::new();
            alloc.next.store(n, Ordering::Relaxed);
            alloc
        }
    }

    #[test]
    fn lease_skips_ids_still_held_after_the_counter_wraps() {
        let alloc = ConnAlloc::starting_at(u32::MAX - 1);
        let a = alloc.lease();
        let b = alloc.lease();
        assert_eq!((a.id(), b.id()), (u32::MAX - 1, u32::MAX));
        let c = alloc.lease();
        assert_eq!(c.id(), FIRST_CONN_ID);

        alloc.next.store(u32::MAX - 1, Ordering::Relaxed);
        let d = alloc.lease();
        assert!(
            ![a.id(), b.id(), c.id()].contains(&d.id()),
            "reused live id {}",
            d.id()
        );

        drop(a);
        alloc.next.store(u32::MAX - 1, Ordering::Relaxed);
        assert_eq!(
            alloc.lease().id(),
            u32::MAX - 1,
            "a released id is reusable"
        );
    }
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
                Req::new(Purpose::Playlist, url.clone(), None),
                &mut auth,
                now,
            );
            let conn = source.reqs.keys().next().copied().unwrap();
            source.reqs.remove(&conn);
        }
        assert!(
            source.deadlines.len() <= DEADLINE_SLACK_FLOOR + DEADLINE_SLACK_FACTOR,
            "{} stale deadlines kept",
            source.deadlines.len()
        );
    }

    #[test]
    fn expire_fails_only_the_overdue_live_request() {
        let (mut source, mut auth, url) = source_with_auth();
        let timeout = source.playlist_timeout_us;
        source.send(Req::new(Purpose::Playlist, url.clone(), None), &mut auth, 0);
        let overdue = source.reqs.keys().next().copied().unwrap();
        source.send(
            Req::new(Purpose::Playlist, url.clone(), None),
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
        let timeout = source.playlist_timeout_us;
        source.send(Req::new(Purpose::Playlist, url.clone(), None), &mut auth, 0);
        let conn = source.reqs.keys().next().copied().unwrap();
        source.reqs.remove(&conn);
        let mut newer = Req::new(Purpose::Playlist, url, None);
        newer.started_us = timeout / 2;
        source.reqs.insert(conn, newer);
        source.take_actions();

        source.expire(timeout);
        assert!(source.take_actions().is_empty());
        assert!(source.reqs.contains_key(&conn));
    }
}
