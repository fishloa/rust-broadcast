//! RTSP source: DESCRIBE, per-track SETUP and PLAY over one TCP connection with RTP/RTCP
//! interleaved in `$` frames (RFC 2326 section 10.12), depayloaded by
//! [`transmux::RtpStreamDepacketiser`].
//!
//! The adapter is sans-IO: it asks the host for a TCP connection and byte writes through
//! [`Action::TcpConnect`] / [`Action::Send`] and reacts to the host's `on_connected` / `on_data` /
//! `on_closed`. The reconnect and stall policy is the TS-over-HTTP one (same [`TsConfig`]).

use alloc::collections::{BTreeSet, VecDeque};
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use rtsp_runtime::transport::{LowerTransport, Transport, TransportSpec};
use rtsp_runtime::{ClientEvent, ClientSession, Credentials, Method, StatusCode};
use transmux::{CodecConfig, RtpLossEvent, RtpStreamDepacketiser, RtpStreamTrack, Sample, TrackId};
use tshttp_runtime::{Config as TsConfig, StarveKind};
use url::{Host, Url};

use crate::auth::{AuthDecision, AuthState};
use crate::events::{Action, ErrorKind, Event, Status};
use crate::redirect;
use crate::source::rtsp_align::{Aligner, Out};
use crate::source::rtsp_sdp::{ParsedSdp, SdpError, SkippedMedia, TrackInit, parse_tracks};

const SCHEME_RTSPS: &str = "rtsps";
const RTSP_DEFAULT_PORT: u16 = 554;
const RTSPS_DEFAULT_PORT: u16 = 322;
const CONTROL_AGGREGATE: &str = "*";
const PATH_SEPARATOR: char = '/';
const FIRST_CONN_ID: u32 = 1;
const MAX_AUTH_REPLAYS: u32 = 2;
const MAX_STALL_NOTICES_PER_POLL: u32 = 64;
const US_PER_MS: u64 = 1_000;
const US_PER_SEC: u64 = 1_000_000;
const STATUS_UNAUTHORIZED: u16 = 401;
const STATUS_FORBIDDEN: u16 = 403;
const STATUS_NOT_FOUND: u16 = 404;
const STATUS_PROXY_AUTH_REQUIRED: u16 = 407;
const REDIRECT_STATUSES: [u16; 5] = [301, 302, 303, 307, 308];
const KEEPALIVE_DEFAULT_US: u64 = 30 * US_PER_SEC;
const KEEPALIVE_MAX_US: u64 = 3_600 * US_PER_SEC;
const KEEPALIVE_DIVISOR: u64 = 2;
const CHALLENGE_HEADER: &str = "WWW-Authenticate";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Req {
    Describe,
    Setup(usize),
    Play,
}

impl Req {
    fn answers(self, method: &Method) -> bool {
        match self {
            Self::Describe => *method == Method::Describe,
            Self::Setup(_) => *method == Method::Setup,
            Self::Play => *method == Method::Play,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Connecting,
    Awaiting(Req),
    Playing,
    Backoff(u64),
    WaitingAuth(Req),
    Closed,
}

pub(crate) struct RtspSource {
    original_url: Url,
    max_redirects: u32,
    redirects: u32,
    credentials_in_scope: bool,
    discontinuity_pending: bool,
    request_url: Url,
    uri: String,
    host: String,
    port: u16,
    tls: bool,
    cfg: TsConfig,
    phase: Phase,
    conn: Option<u32>,
    next_conn: u32,
    connections: u32,
    client: ClientSession,
    challenge: Option<String>,
    stale_replayed: bool,
    auth_replays: u32,
    tracks: Vec<TrackInit>,
    reported_drops: BTreeSet<(usize, Option<String>)>,
    aggregate: Option<String>,
    depa: Option<RtpStreamDepacketiser>,
    aligner: Aligner,
    pre_sample: Vec<Event>,
    tracks_dirty: bool,
    seen_first_sample: bool,
    stalled: bool,
    attempt: u64,
    connected_at: u64,
    last_byte_at: u64,
    last_sample_at: u64,
    next_notice_at: Option<u64>,
    starved_data_fired: bool,
    starved_nodata_fired: bool,
    next_keepalive: Option<u64>,
    keepalive_interval: u64,
    wake_pending: Option<u64>,
    loss_events: u64,
    malformed_frames: u64,
    rtcp_frames: u64,
    actions: VecDeque<Action>,
    events: VecDeque<Event>,
}

fn control_uri<'a>(aggregate: Option<&'a str>, request_uri: &'a str) -> &'a str {
    aggregate.unwrap_or(request_uri)
}

fn effective_port(url: &Url) -> u16 {
    url.port().unwrap_or(if url.scheme() == SCHEME_RTSPS {
        RTSPS_DEFAULT_PORT
    } else {
        RTSP_DEFAULT_PORT
    })
}

fn keepalive_interval_for(declared_secs: Option<u64>) -> u64 {
    match declared_secs {
        Some(secs) if secs > 0 => {
            (secs.saturating_mul(US_PER_SEC) / KEEPALIVE_DIVISOR).min(KEEPALIVE_MAX_US)
        }
        _ => KEEPALIVE_DEFAULT_US,
    }
}

impl RtspSource {
    pub(crate) fn new(url: &Url, cfg: &TsConfig, max_redirects: u32, now_us: u64) -> Self {
        let tls = url.scheme() == SCHEME_RTSPS;
        let default_port = if tls {
            RTSPS_DEFAULT_PORT
        } else {
            RTSP_DEFAULT_PORT
        };
        let host = match url.host() {
            Some(Host::Domain(d)) => d.to_string(),
            Some(Host::Ipv4(a)) => a.to_string(),
            Some(Host::Ipv6(a)) => a.to_string(),
            None => String::new(),
        };
        let mut request_url = url.clone();
        let _ = request_url.set_username("");
        let _ = request_url.set_password(None);
        let mut me = Self {
            original_url: request_url.clone(),
            max_redirects,
            redirects: 0,
            credentials_in_scope: true,
            discontinuity_pending: false,
            uri: request_url.to_string(),
            request_url,
            host,
            port: url.port().unwrap_or(default_port),
            tls,
            cfg: cfg.clone(),
            phase: Phase::Backoff(now_us),
            conn: None,
            next_conn: FIRST_CONN_ID,
            connections: 0,
            client: ClientSession::new(),
            challenge: None,
            stale_replayed: false,
            auth_replays: 0,
            tracks: Vec::new(),
            reported_drops: BTreeSet::new(),
            aggregate: None,
            depa: None,
            aligner: Aligner::default(),
            pre_sample: Vec::new(),
            tracks_dirty: false,
            seen_first_sample: false,
            stalled: false,
            attempt: 0,
            connected_at: now_us,
            last_byte_at: now_us,
            last_sample_at: now_us,
            next_notice_at: None,
            starved_data_fired: false,
            starved_nodata_fired: false,
            next_keepalive: None,
            keepalive_interval: KEEPALIVE_DEFAULT_US,
            wake_pending: None,
            loss_events: 0,
            malformed_frames: 0,
            rtcp_frames: 0,
            actions: VecDeque::new(),
            events: VecDeque::new(),
        };
        me.start_connect(now_us, None);
        me
    }

    pub(crate) fn take_actions(&mut self) -> VecDeque<Action> {
        core::mem::take(&mut self.actions)
    }

    pub(crate) fn take_events(&mut self) -> VecDeque<Event> {
        core::mem::take(&mut self.events)
    }

    pub(crate) fn loss_events(&self) -> u64 {
        self.loss_events
    }

    pub(crate) fn malformed_frames(&self) -> u64 {
        self.malformed_frames
    }

    pub(crate) fn rtcp_frames(&self) -> u64 {
        self.rtcp_frames
    }

    pub(crate) fn poll(&mut self, now: u64, auth: &AuthState) {
        if self.phase == Phase::Closed {
            return;
        }
        self.reconnect_if_due(now, auth);
        self.run_timers(now);
        self.reconnect_if_due(now, auth);
    }

    pub(crate) fn on_connected(&mut self, conn: u32, now: u64) {
        if self.conn != Some(conn) || self.phase != Phase::Connecting {
            return;
        }
        self.connections = self.connections.saturating_add(1);
        self.events.push_back(Event::Status(Status::Establishing));
        self.connected_at = now;
        self.last_byte_at = now;
        self.last_sample_at = now;
        if !self.send(Req::Describe) {
            self.fail_connection(now);
        }
    }

    pub(crate) fn on_data(&mut self, conn: u32, bytes: &[u8], now: u64, auth: &mut AuthState) {
        if self.conn != Some(conn) {
            return;
        }
        let outcome = self.client.handle_data(bytes);
        for event in outcome.events {
            if self.conn != Some(conn) {
                return;
            }
            self.on_client_event(event, now, auth);
        }
        if outcome.error.is_some() && self.conn == Some(conn) {
            self.connection_lost(now);
        }
    }

    pub(crate) fn on_closed(&mut self, conn: u32, now: u64) {
        if self.conn != Some(conn) {
            return;
        }
        match self.phase {
            Phase::WaitingAuth(_) => {
                self.conn = None;
            }
            Phase::Connecting | Phase::Awaiting(_) | Phase::Playing => {
                self.flush_depacketiser(now);
                self.conn = None;
                self.forget_connection_state();
                self.after_connection_end(now);
            }
            Phase::Backoff(_) | Phase::Closed => {}
        }
    }

    pub(crate) fn on_credentials(&mut self, now: u64, auth: &AuthState) {
        self.stale_replayed = false;
        self.auth_replays = 0;
        if let Some(creds) = auth
            .credentials()
            .filter(|_| self.credentials_in_scope)
            .cloned()
        {
            let client = core::mem::take(&mut self.client);
            self.client = Self::client_with_credentials(client, creds, self.challenge.as_deref());
        }
        let Phase::WaitingAuth(req) = self.phase else {
            return;
        };
        if self.conn.is_none() {
            self.start_connect(now, Some(auth));
            return;
        }
        self.connected_at = now;
        self.last_byte_at = now;
        self.last_sample_at = now;
        if !self.send(req) {
            self.fail_connection(now);
        }
    }

    pub(crate) fn reconnect(&mut self, now: u64) {
        match self.phase {
            Phase::Backoff(_) | Phase::Closed | Phase::WaitingAuth(_) => {}
            Phase::Connecting | Phase::Awaiting(_) | Phase::Playing => {
                self.flush_depacketiser(now);
                self.discontinuity_pending |= self.seen_first_sample;
                self.drop_connection(true);
                self.attempt = 0;
                self.phase = Phase::Backoff(now);
            }
        }
    }

    pub(crate) fn close(&mut self) {
        if self.phase == Phase::Closed {
            return;
        }
        self.drop_connection(true);
        self.phase = Phase::Closed;
    }

    pub(crate) fn sync_wake(&mut self, now: u64) {
        let Some(deadline) = self.next_deadline() else {
            return;
        };
        let outstanding = self
            .wake_pending
            .is_some_and(|w| w == deadline || (w > now && w < deadline));
        if !outstanding {
            self.actions.push_back(Action::WakeAt(deadline));
            self.wake_pending = Some(deadline);
        }
    }

    fn next_deadline(&self) -> Option<u64> {
        match self.phase {
            Phase::Backoff(due) => Some(due),
            Phase::Awaiting(_) | Phase::Playing => {
                let mut deadline = if self.seen_first_sample {
                    let notice = self.last_byte_at.saturating_add(self.notice_us());
                    let reconnect = self.last_byte_at.saturating_add(self.reconnect_us());
                    self.next_notice_at.unwrap_or(notice).min(reconnect)
                } else {
                    self.connected_at.saturating_add(self.start_us())
                };
                if let Some(k) = self.next_keepalive.filter(|_| self.phase == Phase::Playing) {
                    deadline = deadline.min(k);
                }
                Some(deadline)
            }
            Phase::Connecting | Phase::WaitingAuth(_) | Phase::Closed => None,
        }
    }

    fn start_us(&self) -> u64 {
        self.cfg.start_timeout_ms.saturating_mul(US_PER_MS)
    }

    fn notice_us(&self) -> u64 {
        self.cfg.stall_notice_ms.saturating_mul(US_PER_MS)
    }

    fn reconnect_us(&self) -> u64 {
        self.cfg.stall_reconnect_ms.saturating_mul(US_PER_MS)
    }

    fn backoff_after_played(&mut self, now: u64) {
        let lasted = now.saturating_sub(self.connected_at);
        let floor = self
            .cfg
            .min_reconnect_interval_ms
            .saturating_mul(US_PER_MS)
            .saturating_sub(lasted);
        self.attempt = 0;
        self.discontinuity_pending = true;
        self.events
            .push_back(Event::Status(Status::Reconnecting { attempt: 0 }));
        self.phase = Phase::Backoff(now.saturating_add(floor));
    }

    fn reconnect_if_due(&mut self, now: u64, auth: &AuthState) {
        if let Phase::Backoff(due) = self.phase
            && now >= due
        {
            self.retarget(self.original_url.clone());
            self.redirects = 0;
            self.start_connect(now, Some(auth));
        }
    }

    fn client_with_credentials(
        mut client: ClientSession,
        creds: Credentials,
        challenge: Option<&str>,
    ) -> ClientSession {
        if client.authenticate_with(creds.clone(), challenge).is_err() {
            client = client.with_credentials(creds);
        }
        client
    }

    fn start_connect(&mut self, now: u64, auth: Option<&AuthState>) {
        let mut client = ClientSession::new();
        if let Some(creds) = auth
            .and_then(AuthState::credentials)
            .filter(|_| self.credentials_in_scope)
        {
            client =
                Self::client_with_credentials(client, creds.clone(), self.challenge.as_deref());
        }
        self.client = client;
        self.stale_replayed = false;
        self.auth_replays = 0;
        self.forget_connection_state();
        self.seen_first_sample = false;
        self.connected_at = now;
        self.last_byte_at = now;
        self.last_sample_at = now;
        let conn = self.alloc_conn();
        self.conn = Some(conn);
        self.phase = Phase::Connecting;
        self.actions.push_back(Action::TcpConnect {
            conn,
            host: self.host.clone(),
            port: self.port,
            tls: self.tls,
        });
    }

    fn alloc_conn(&mut self) -> u32 {
        loop {
            let candidate = self.next_conn;
            self.next_conn = candidate.checked_add(1).unwrap_or(FIRST_CONN_ID);
            if Some(candidate) != self.conn {
                return candidate;
            }
        }
    }

    fn forget_connection_state(&mut self) {
        self.tracks.clear();
        self.aggregate = None;
        self.depa = None;
        self.aligner = Aligner::default();
        self.pre_sample.clear();
        self.tracks_dirty = false;
        self.stalled = false;
        self.next_notice_at = None;
        self.starved_data_fired = false;
        self.starved_nodata_fired = false;
        self.next_keepalive = None;
    }

    fn drop_connection(&mut self, teardown: bool) {
        if let Some(conn) = self.conn.take() {
            if teardown
                && self.client.session_id().is_some()
                && let Ok(bytes) = self
                    .client
                    .teardown(control_uri(self.aggregate.as_deref(), &self.uri))
            {
                self.actions.push_back(Action::Send { conn, bytes });
            }
            self.actions.push_back(Action::Close { conn });
        }
        self.forget_connection_state();
    }

    fn send(&mut self, req: Req) -> bool {
        let Some(conn) = self.conn else {
            return false;
        };
        let built = match req {
            Req::Describe => self.client.describe(&self.uri),
            Req::Setup(index) => {
                let Some(track) = self.tracks.get(index) else {
                    return false;
                };
                let transport = Transport::single(TransportSpec::rtp_avp_tcp_interleaved(
                    track.channel,
                    track.rtcp_channel,
                ));
                let uri = track.setup_uri.clone();
                let _ = self.client.take_negotiated_transport();
                self.client.setup(&uri, &transport)
            }
            Req::Play => self
                .client
                .play(control_uri(self.aggregate.as_deref(), &self.uri)),
        };
        match built {
            Ok(bytes) => {
                self.actions.push_back(Action::Send { conn, bytes });
                self.phase = Phase::Awaiting(req);
                true
            }
            Err(_) => false,
        }
    }

    fn base_url(&self) -> Url {
        self.client
            .content_base()
            .and_then(|base| self.request_url.join(base).ok())
            .unwrap_or_else(|| {
                let mut base = self.request_url.clone();
                if !base.path().ends_with(PATH_SEPARATOR) {
                    let path = alloc::format!("{}{PATH_SEPARATOR}", base.path());
                    base.set_path(&path);
                }
                base
            })
    }

    fn same_authority(&self, url: &Url) -> bool {
        let default_port = if self.tls {
            RTSPS_DEFAULT_PORT
        } else {
            RTSP_DEFAULT_PORT
        };
        url.scheme() == self.request_url.scheme()
            && url.host() == self.request_url.host()
            && url.port().unwrap_or(default_port) == self.port
    }

    fn resolve_control(&self, control: Option<&str>, base: &Url) -> Option<String> {
        match control {
            None => Some(self.uri.clone()),
            Some(CONTROL_AGGREGATE) => Some(base.to_string()),
            Some(c) => base
                .join(c)
                .ok()
                .filter(|u| self.same_authority(u))
                .map(|u| u.to_string()),
        }
    }

    fn on_client_event(&mut self, event: ClientEvent, now: u64, auth: &mut AuthState) {
        match event {
            ClientEvent::AuthRetry { request, .. } => {
                self.auth_replays = self.auth_replays.saturating_add(1);
                if self.auth_replays > MAX_AUTH_REPLAYS {
                    self.reject_repeated_challenge(auth);
                    return;
                }
                if let Some(conn) = self.conn {
                    self.actions.push_back(Action::Send {
                        conn,
                        bytes: request,
                    });
                }
            }
            ClientEvent::Response {
                method,
                status,
                body,
                ..
            } => self.on_response(&method, status, &body, now, auth),
            ClientEvent::MediaData { channel, data } => self.on_media(channel, &data, now),
            _ => {}
        }
    }

    fn on_response(
        &mut self,
        method: &Method,
        status: StatusCode,
        body: &[u8],
        now: u64,
        auth: &mut AuthState,
    ) {
        if self.phase == Phase::Playing {
            self.stale_replayed = false;
            self.auth_replays = 0;
            return;
        }
        let Phase::Awaiting(req) = self.phase else {
            return;
        };
        if !req.answers(method) {
            return;
        }
        let code = u16::from(status);
        if REDIRECT_STATUSES.contains(&code) {
            self.follow_redirect(now, auth);
            return;
        }
        if code == STATUS_UNAUTHORIZED || code == STATUS_PROXY_AUTH_REQUIRED {
            if !self.credentials_in_scope {
                self.fail_connection(now);
                return;
            }
            self.on_unauthorized(code, req, now, auth);
            return;
        }
        if code == STATUS_FORBIDDEN {
            self.events.push_back(Event::Forbidden { status: code });
            self.fail_connection(now);
            return;
        }
        if code == STATUS_NOT_FOUND && req == Req::Describe {
            self.events.push_back(Event::Status(Status::NotFound));
            self.fail_connection(now);
            return;
        }
        if !status.is_success() {
            self.fail_connection(now);
            return;
        }
        self.stale_replayed = false;
        self.auth_replays = 0;
        let proceeded = match req {
            Req::Describe => self.on_describe_ok(body),
            Req::Setup(index) => self.on_setup_ok(index),
            Req::Play => {
                self.on_play_ok(now);
                true
            }
        };
        if !proceeded {
            self.fail_connection(now);
        }
    }

    fn retarget(&mut self, url: Url) {
        let default_port = if url.scheme() == SCHEME_RTSPS {
            RTSPS_DEFAULT_PORT
        } else {
            RTSP_DEFAULT_PORT
        };
        self.tls = url.scheme() == SCHEME_RTSPS;
        self.host = match url.host() {
            Some(Host::Domain(d)) => d.to_string(),
            Some(Host::Ipv4(a)) => a.to_string(),
            Some(Host::Ipv6(a)) => a.to_string(),
            None => String::new(),
        };
        self.port = url.port().unwrap_or(default_port);
        self.uri = url.to_string();
        self.credentials_in_scope = url.scheme() == self.original_url.scheme()
            && url.host() == self.original_url.host()
            && effective_port(&url) == effective_port(&self.original_url);
        self.request_url = url;
    }

    fn follow_redirect(&mut self, now: u64, auth: &AuthState) {
        let target = self.client.redirect_location().and_then(|location| {
            redirect::resolve_rtsp(
                &self.request_url,
                location,
                self.redirects,
                self.max_redirects,
            )
        });
        let Some(target) = target else {
            self.fail_connection(now);
            return;
        };
        self.drop_connection(false);
        self.retarget(target);
        self.redirects = self.redirects.saturating_add(1);
        self.start_connect(now, Some(auth));
    }

    fn on_unauthorized(&mut self, code: u16, req: Req, now: u64, auth: &mut AuthState) {
        let had_credentials = auth.credentials().is_some();
        if let Some(line) = self.client.last_challenge() {
            self.challenge = Some(line.to_string());
        }
        let headers: Vec<(String, String)> = self
            .challenge
            .iter()
            .map(|line| (CHALLENGE_HEADER.to_string(), line.clone()))
            .collect();
        match auth.handle_unauthorized(code, &headers, had_credentials, self.stale_replayed) {
            AuthDecision::Wait(events) => {
                self.events.extend(events);
                self.phase = Phase::WaitingAuth(req);
            }
            AuthDecision::Replay => {
                if !self.send(req) {
                    self.fail_connection(now);
                }
            }
            AuthDecision::ReplayStale => {
                self.stale_replayed = true;
                if !self.send(req) {
                    self.fail_connection(now);
                }
            }
        }
    }

    fn reject_repeated_challenge(&mut self, auth: &mut AuthState) {
        let Phase::Awaiting(req) = self.phase else {
            return;
        };
        let headers: Vec<(String, String)> = self
            .client
            .last_challenge()
            .map(|line| (CHALLENGE_HEADER.to_string(), line.to_string()))
            .into_iter()
            .collect();
        if let AuthDecision::Wait(events) =
            auth.handle_unauthorized(STATUS_UNAUTHORIZED, &headers, true, true)
        {
            self.events.extend(events);
        }
        self.phase = Phase::WaitingAuth(req);
    }

    fn on_describe_ok(&mut self, body: &[u8]) -> bool {
        let base = self.base_url();
        let parsed = parse_tracks(body, |control| self.resolve_control(control, &base));
        let ParsedSdp {
            aggregate,
            tracks,
            skipped,
        } = match parsed {
            Ok(parsed) => parsed,
            Err(SdpError::Unparsable) => return false,
            Err(SdpError::NothingPlayable { skipped }) => {
                self.report_dropped(&skipped);
                let codec = skipped
                    .iter()
                    .find(|m| m.codec_unsupported)
                    .and_then(|m| m.codec.clone());
                let Some(fourcc) = codec else {
                    return false;
                };
                self.events.push_back(Event::Error {
                    kind: ErrorKind::UnsupportedCodec { fourcc },
                    fatal: true,
                });
                self.close();
                return true;
            }
        };
        self.report_dropped(&skipped);
        self.aggregate = aggregate;
        self.tracks = tracks;
        self.send(Req::Setup(0))
    }

    fn report_dropped(&mut self, skipped: &[SkippedMedia]) {
        let current: BTreeSet<(usize, Option<String>)> = skipped
            .iter()
            .map(|media| (media.media_index, media.codec.clone()))
            .collect();
        for media in skipped {
            if self
                .reported_drops
                .contains(&(media.media_index, media.codec.clone()))
            {
                continue;
            }
            let position = media.media_index.saturating_add(1);
            self.events.push_back(Event::Error {
                kind: ErrorKind::TrackDropped {
                    track_id: u32::try_from(position).unwrap_or(u32::MAX),
                    stream_type: None,
                    fourcc: None,
                },
                fatal: false,
            });
        }
        self.reported_drops = current;
    }

    fn on_setup_ok(&mut self, index: usize) -> bool {
        let Some((channel, rtcp_channel)) = self
            .client
            .take_negotiated_transport()
            .as_ref()
            .and_then(Transport::first)
            .filter(|spec| spec.lower_transport == Some(LowerTransport::Tcp))
            .and_then(|spec| spec.interleaved)
        else {
            return false;
        };
        if channel == rtcp_channel
            || self.tracks.iter().take(index).any(|t| {
                [t.channel, t.rtcp_channel].contains(&channel)
                    || [t.channel, t.rtcp_channel].contains(&rtcp_channel)
            })
        {
            return false;
        }
        let Some(track) = self.tracks.get_mut(index) else {
            return false;
        };
        track.channel = channel;
        track.rtcp_channel = rtcp_channel;
        let next = index.saturating_add(1);
        if next < self.tracks.len() {
            return self.send(Req::Setup(next));
        }
        self.depa = Some(RtpStreamDepacketiser::new(
            self.tracks
                .iter()
                .map(|t| RtpStreamTrack::new(t.track_id, t.kind, t.config.clone(), t.clock_rate))
                .collect(),
        ));
        self.aligner = Aligner::new(
            &self
                .tracks
                .iter()
                .map(|t| {
                    (
                        t.track_id,
                        matches!(t.config, CodecConfig::Avc { .. }),
                        t.clock_rate,
                    )
                })
                .collect::<Vec<_>>(),
        );
        self.send(Req::Play)
    }

    fn note_rtp_info(&mut self) {
        let times: Vec<(TrackId, u32)> = self
            .client
            .take_rtp_info()
            .into_iter()
            .filter_map(|entry| {
                let rtptime = entry.rtptime?;
                let uri = Url::parse(&entry.uri).ok()?;
                let track = self
                    .tracks
                    .iter()
                    .find(|t| Url::parse(&t.setup_uri).is_ok_and(|u| u == uri))?;
                Some((track.track_id, rtptime))
            })
            .collect();
        self.aligner.note_rtp_info(&times);
    }

    fn on_play_ok(&mut self, now: u64) {
        self.note_rtp_info();
        self.phase = Phase::Playing;
        self.redirects = 0;
        self.tracks_dirty = true;
        self.last_byte_at = now;
        self.last_sample_at = now;
        self.keepalive_interval = keepalive_interval_for(self.client.session_timeout());
        self.next_keepalive = Some(now.saturating_add(self.keepalive_interval));
    }

    fn route(&self, channel: u8) -> Option<(TrackId, bool)> {
        self.tracks.iter().find_map(|t| {
            if t.channel == channel {
                Some((t.track_id, false))
            } else if t.rtcp_channel == channel {
                Some((t.track_id, true))
            } else {
                None
            }
        })
    }

    fn on_media(&mut self, channel: u8, data: &[u8], now: u64) {
        if self.phase != Phase::Playing {
            return;
        }
        let Some((track, is_rtcp)) = self.route(channel) else {
            return;
        };
        let Some(depa) = self.depa.as_mut() else {
            return;
        };
        if is_rtcp {
            self.rtcp_frames = self.rtcp_frames.saturating_add(1);
            depa.push_rtcp(track, data);
            self.aligner.note_rtcp();
            return;
        }
        self.last_byte_at = now;
        if self.stalled {
            self.stalled = false;
            self.next_notice_at = None;
            self.starved_nodata_fired = false;
            self.events.push_back(Event::Status(Status::Live));
        }
        let samples = match depa.push(track, data) {
            Ok(samples) => samples,
            Err(_) => {
                self.malformed_frames = self.malformed_frames.saturating_add(1);
                Vec::new()
            }
        };
        while let Some(event) = depa.poll_loss_event() {
            if matches!(
                event,
                RtpLossEvent::SequenceGap { .. }
                    | RtpLossEvent::DamagedAccessUnit { .. }
                    | RtpLossEvent::DroppedAccessUnit { .. }
            ) {
                self.loss_events = self.loss_events.saturating_add(1);
            }
        }
        let produced = !samples.is_empty();
        for sample in samples {
            if !self.try_complete_avc_config(track, &sample) {
                continue;
            }
            self.maybe_update_avc_dims(track, &sample);
            self.align_and_emit(track, sample, now);
        }
        if !produced
            && !self.starved_data_fired
            && now.saturating_sub(self.last_sample_at) >= self.notice_us()
        {
            self.events.push_back(Event::Starved(StarveKind::Data));
            self.starved_data_fired = true;
        }
    }

    fn maybe_update_avc_dims(&mut self, track: TrackId, sample: &Sample) {
        let Some(track_init) = self.tracks.iter_mut().find(|t| t.track_id == track) else {
            return;
        };
        let is_unknown = matches!(
            &track_init.config,
            CodecConfig::Avc { width, height, .. } if *width == 0 && *height == 0
        );
        if !is_unknown {
            return;
        }
        if let Some((w, h)) = transmux::avc_access_unit_dimensions(&sample.data) {
            if let CodecConfig::Avc { width, height, .. } = &mut track_init.config {
                *width = w;
                *height = h;
            }
            let config = track_init.config.clone();
            if let Some(depa) = &mut self.depa {
                depa.set_track_config(track, config);
            }
            self.tracks_dirty = true;
        }
    }

    fn try_complete_avc_config(&mut self, track: TrackId, sample: &Sample) -> bool {
        let ready = {
            let Some(track_init) = self.tracks.iter().find(|t| t.track_id == track) else {
                return true;
            };
            match &track_init.config {
                CodecConfig::Avc { config, .. } => !config.config.sps.is_empty(),
                _ => true,
            }
        };
        if ready {
            return true;
        }
        let (sps, pps) = transmux::avc_access_unit_parameter_sets(&sample.data);
        if sps.is_empty() || pps.is_empty() {
            return false;
        }
        let Ok(avc) = transmux::avc_config_from_sps_pps(sps, pps) else {
            return false;
        };
        let (width, height) = avc
            .config
            .sps
            .first()
            .and_then(|s| transmux::avc_sps_dimensions(&s.0))
            .unwrap_or((0, 0));
        let Some(track_init) = self.tracks.iter_mut().find(|t| t.track_id == track) else {
            return false;
        };
        if let CodecConfig::Avc {
            config,
            width: w,
            height: h,
        } = &mut track_init.config
        {
            *config = avc;
            *w = width;
            *h = height;
        }
        let config = track_init.config.clone();
        if let Some(depa) = &mut self.depa {
            depa.set_track_config(track, config);
        }
        self.tracks_dirty = true;
        true
    }

    fn announce_discontinuity(&mut self) {
        if self.discontinuity_pending && !self.seen_first_sample {
            self.discontinuity_pending = false;
            self.events.push_back(Event::Discontinuity);
        }
    }

    fn announce_tracks(&mut self) {
        if self.tracks_dirty {
            let mut specs = self
                .depa
                .as_ref()
                .map(RtpStreamDepacketiser::track_specs)
                .unwrap_or_default();
            // Hardware decoders reject an empty avcC.
            specs.retain(|spec| match &spec.config {
                CodecConfig::Avc { config, .. } => !config.config.sps.is_empty(),
                _ => true,
            });
            if !specs.is_empty() {
                self.events.push_back(Event::Tracks(specs));
            }
            self.tracks_dirty = false;
        }
    }

    fn align_and_emit(&mut self, track: TrackId, sample: Sample, now: u64) {
        self.announce_discontinuity();
        self.announce_tracks();
        let Some(depa) = self.depa.as_ref() else {
            self.emit_sample(track, sample, now);
            return;
        };
        for out in self.aligner.push(depa, track, sample) {
            match out {
                Out::Sample(track, sample) => self.emit_sample(track, sample, now),
                Out::Event(event) => self.pre_sample.push(event),
            }
        }
    }

    fn emit_sample(&mut self, track: TrackId, sample: Sample, now: u64) {
        self.announce_discontinuity();
        self.announce_tracks();
        if !self.seen_first_sample {
            self.events.push_back(Event::Status(Status::Live));
            self.seen_first_sample = true;
            self.attempt = 0;
        }
        self.last_sample_at = now;
        self.starved_data_fired = false;
        self.starved_nodata_fired = false;
        self.next_notice_at = None;
        self.events.extend(self.pre_sample.drain(..));
        self.events.push_back(Event::Sample {
            track,
            sample,
            join: None,
        });
    }

    fn flush_depacketiser(&mut self, now: u64) {
        if self.phase != Phase::Playing {
            return;
        }
        let Some(depa) = self.depa.as_mut() else {
            return;
        };
        let ids: Vec<TrackId> = depa.track_specs().iter().map(|s| s.track_id).collect();
        let mut flushed = Vec::new();
        for id in ids {
            if let Ok(samples) = depa.flush(id) {
                flushed.extend(samples.into_iter().map(|sample| (id, sample)));
            }
        }
        for (id, sample) in flushed {
            self.align_and_emit(id, sample, now);
        }
    }

    fn run_timers(&mut self, now: u64) {
        match self.phase {
            Phase::Awaiting(_) | Phase::Playing => {}
            _ => return,
        }
        if self.seen_first_sample {
            if self.check_stall(now) {
                return;
            }
        } else if now.saturating_sub(self.connected_at) >= self.start_us() {
            self.fail_connection(now);
            return;
        }
        self.send_keepalive_if_due(now);
    }

    fn check_stall(&mut self, now: u64) -> bool {
        let idle = now.saturating_sub(self.last_byte_at);
        if idle < self.notice_us() {
            return false;
        }
        if !self.starved_nodata_fired {
            self.events.push_back(Event::Starved(StarveKind::NoData));
            self.starved_nodata_fired = true;
        }
        match self.next_notice_at {
            None => {
                self.next_notice_at = Some(now.saturating_add(US_PER_SEC));
                self.push_stalled(idle / US_PER_SEC);
            }
            Some(mut next) => {
                let mut emitted = 0u32;
                while now >= next && emitted < MAX_STALL_NOTICES_PER_POLL {
                    let elapsed = next.saturating_sub(self.last_byte_at);
                    self.push_stalled(elapsed / US_PER_SEC);
                    next = next.saturating_add(US_PER_SEC);
                    emitted = emitted.saturating_add(1);
                }
                if now >= next {
                    next = now.saturating_add(US_PER_SEC);
                }
                self.next_notice_at = Some(next);
            }
        }
        if idle >= self.reconnect_us() {
            self.flush_depacketiser(now);
            self.drop_connection(true);
            self.backoff_after_played(now);
            return true;
        }
        false
    }

    fn push_stalled(&mut self, secs: u64) {
        self.events
            .push_back(Event::Status(Status::Stalled { secs }));
        self.stalled = true;
    }

    fn send_keepalive_if_due(&mut self, now: u64) {
        if self.phase != Phase::Playing {
            return;
        }
        let Some(due) = self.next_keepalive else {
            return;
        };
        if now < due {
            return;
        }
        self.next_keepalive = now.checked_add(self.keepalive_interval);
        let Some(conn) = self.conn else {
            return;
        };
        if let Ok(bytes) = self
            .client
            .options(control_uri(self.aggregate.as_deref(), &self.uri))
        {
            self.actions.push_back(Action::Send { conn, bytes });
        }
    }

    fn fail_connection(&mut self, now: u64) {
        self.drop_connection(true);
        self.fail_before_playing(now);
    }

    fn connection_lost(&mut self, now: u64) {
        self.flush_depacketiser(now);
        self.drop_connection(false);
        self.after_connection_end(now);
    }

    fn after_connection_end(&mut self, now: u64) {
        if self.seen_first_sample {
            self.backoff_after_played(now);
        } else {
            self.fail_before_playing(now);
        }
    }

    fn fail_before_playing(&mut self, now: u64) {
        self.attempt = self.attempt.saturating_add(1);
        let after_ms = usize::try_from(self.attempt)
            .ok()
            .and_then(|n| n.checked_sub(1))
            .and_then(|i| self.cfg.backoff_ms.get(i).copied())
            .unwrap_or(self.cfg.backoff_cap_ms);
        self.events.push_back(Event::Status(Status::Reconnecting {
            attempt: self.attempt,
        }));
        self.phase = Phase::Backoff(now.saturating_add(after_ms.saturating_mul(US_PER_MS)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reconnect_never_reuses_the_live_connection_id() {
        let url = Url::parse("rtsp://cam.test/live/").unwrap();
        let mut source = RtspSource::new(&url, &TsConfig::default(), 0, 0);
        source.take_actions();
        source.next_conn = u32::MAX;
        source.conn = Some(u32::MAX);
        source.start_connect(1, None);
        assert_ne!(source.conn, Some(u32::MAX));
        assert!(source.conn.is_some());
    }

    #[test]
    fn connection_ids_wrap_to_one_and_skip_zero() {
        let url = Url::parse("rtsp://cam.test/live/").unwrap();
        let mut source = RtspSource::new(&url, &TsConfig::default(), 0, 0);
        source.conn = None;
        source.next_conn = u32::MAX;
        source.start_connect(1, None);
        assert_eq!(source.conn, Some(u32::MAX));
        source.conn = None;
        source.start_connect(2, None);
        assert_eq!(source.conn, Some(1));
    }
}
