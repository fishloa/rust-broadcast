//! RTSP source: DESCRIBE, per-track SETUP and PLAY over one TCP connection with RTP/RTCP
//! interleaved in `$` frames (RFC 2326 section 10.12), depayloaded by
//! [`transmux::RtpStreamDepacketiser`].
//!
//! The adapter is sans-IO: it asks the host for a TCP connection and byte writes through
//! [`Action::TcpConnect`] / [`Action::Send`] and reacts to the host's `on_connected` / `on_data` /
//! `on_closed`. The reconnect and stall policy is the TS-over-HTTP one (same [`TsConfig`]).

use alloc::collections::VecDeque;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use rtsp_runtime::transport::{LowerTransport, Transport, TransportSpec};
use rtsp_runtime::{ClientEvent, ClientSession, Credentials, Method, StatusCode};
use transmux::{RtpLossEvent, RtpStreamDepacketiser, RtpStreamTrack, Sample, TrackId};
use tshttp_runtime::{Config as TsConfig, StarveKind};
use url::{Host, Url};

use crate::auth::{AuthDecision, AuthState};
use crate::events::{Action, Event, Status};
use crate::source::rtsp_sdp::{TrackInit, parse_tracks};

const SCHEME_RTSPS: &str = "rtsps";
const RTSP_DEFAULT_PORT: u16 = 554;
const RTSPS_DEFAULT_PORT: u16 = 322;
const CONTROL_AGGREGATE: &str = "*";
const RTCP_CHANNEL_OFFSET: u8 = 1;
const US_PER_MS: u64 = 1_000;
const US_PER_SEC: u64 = 1_000_000;
const STATUS_UNAUTHORIZED: u16 = 401;
const STATUS_FORBIDDEN: u16 = 403;
const STATUS_NOT_FOUND: u16 = 404;
const STATUS_PROXY_AUTH_REQUIRED: u16 = 407;
const KEEPALIVE_DEFAULT_US: u64 = 30 * US_PER_SEC;
const KEEPALIVE_MIN_US: u64 = 5 * US_PER_SEC;
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
    tracks: Vec<TrackInit>,
    depa: Option<RtpStreamDepacketiser>,
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
    actions: VecDeque<Action>,
    events: VecDeque<Event>,
}

fn keepalive_interval_for(declared_secs: Option<u64>) -> u64 {
    match declared_secs {
        Some(secs) => (secs.saturating_mul(US_PER_SEC) / KEEPALIVE_DIVISOR)
            .clamp(KEEPALIVE_MIN_US, KEEPALIVE_MAX_US),
        None => KEEPALIVE_DEFAULT_US,
    }
}

impl RtspSource {
    pub(crate) fn new(url: &Url, cfg: &TsConfig, now_us: u64) -> Self {
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
            uri: request_url.to_string(),
            request_url,
            host,
            port: url.port().unwrap_or(default_port),
            tls,
            cfg: cfg.clone(),
            phase: Phase::Backoff(now_us),
            conn: None,
            next_conn: 1,
            connections: 0,
            client: ClientSession::new(),
            challenge: None,
            tracks: Vec::new(),
            depa: None,
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
        if self.connections > 1 {
            self.events.push_back(Event::Discontinuity);
        }
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
        if let Some(creds) = auth.credentials().cloned() {
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
        let outstanding = self.wake_pending.is_some_and(|w| w > now && w <= deadline);
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

    fn reconnect_if_due(&mut self, now: u64, auth: &AuthState) {
        if let Phase::Backoff(due) = self.phase
            && now >= due
        {
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
        if let Some(creds) = auth.and_then(AuthState::credentials) {
            client =
                Self::client_with_credentials(client, creds.clone(), self.challenge.as_deref());
        }
        self.client = client;
        self.forget_connection_state();
        self.seen_first_sample = false;
        self.connected_at = now;
        self.last_byte_at = now;
        self.last_sample_at = now;
        let conn = self.next_conn;
        self.next_conn = self.next_conn.saturating_add(1).max(1);
        self.conn = Some(conn);
        self.phase = Phase::Connecting;
        self.actions.push_back(Action::TcpConnect {
            conn,
            host: self.host.clone(),
            port: self.port,
            tls: self.tls,
        });
    }

    fn forget_connection_state(&mut self) {
        self.tracks.clear();
        self.depa = None;
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
                && let Ok(bytes) = self.client.teardown(&self.uri)
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
                let Some(uri) = self.control_uri(track.control.as_deref()) else {
                    return false;
                };
                let transport = Transport::single(TransportSpec::rtp_avp_tcp_interleaved(
                    track.channel,
                    track.channel.saturating_add(RTCP_CHANNEL_OFFSET),
                ));
                self.client.setup(&uri, &transport)
            }
            Req::Play => self.client.play(&self.uri),
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

    fn control_uri(&self, control: Option<&str>) -> Option<String> {
        match control {
            None | Some(CONTROL_AGGREGATE) => Some(self.uri.clone()),
            Some(c) => self.request_url.join(c).ok().map(|u| u.to_string()),
        }
    }

    fn on_client_event(&mut self, event: ClientEvent, now: u64, auth: &mut AuthState) {
        match event {
            ClientEvent::AuthRetry { request, .. } => {
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
        let Phase::Awaiting(req) = self.phase else {
            return;
        };
        if !req.answers(method) {
            return;
        }
        let code = u16::from(status);
        if code == STATUS_UNAUTHORIZED || code == STATUS_PROXY_AUTH_REQUIRED {
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
        match auth.handle_unauthorized(code, &headers, had_credentials) {
            AuthDecision::Wait(events) => {
                self.events.extend(events);
                self.phase = Phase::WaitingAuth(req);
            }
            AuthDecision::Replay => {
                if !self.send(req) {
                    self.fail_connection(now);
                }
            }
        }
    }

    fn on_describe_ok(&mut self, body: &[u8]) -> bool {
        let Ok(tracks) = parse_tracks(body) else {
            return false;
        };
        self.tracks = tracks;
        self.send(Req::Setup(0))
    }

    fn on_setup_ok(&mut self, index: usize) -> bool {
        let Some(channel) = self
            .client
            .negotiated_transport()
            .and_then(Transport::first)
            .filter(|spec| spec.lower_transport == Some(LowerTransport::Tcp))
            .and_then(|spec| spec.interleaved)
            .map(|(rtp, _rtcp)| rtp)
        else {
            return false;
        };
        let Some(track) = self.tracks.get_mut(index) else {
            return false;
        };
        track.channel = channel;
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
        self.send(Req::Play)
    }

    fn on_play_ok(&mut self, now: u64) {
        self.phase = Phase::Playing;
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
            } else if t.channel.checked_add(RTCP_CHANNEL_OFFSET) == Some(channel) {
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
            depa.push_rtcp(track, data);
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
                RtpLossEvent::SequenceGap { .. } | RtpLossEvent::DamagedAccessUnit { .. }
            ) {
                self.loss_events = self.loss_events.saturating_add(1);
            }
        }
        let produced = !samples.is_empty();
        for sample in samples {
            self.emit_sample(track, sample, now);
        }
        if !produced
            && !self.starved_data_fired
            && now.saturating_sub(self.last_sample_at) >= self.notice_us()
        {
            self.events.push_back(Event::Starved(StarveKind::Data));
            self.starved_data_fired = true;
        }
    }

    fn emit_sample(&mut self, track: TrackId, sample: Sample, now: u64) {
        if self.tracks_dirty {
            let specs = self
                .depa
                .as_ref()
                .map(RtpStreamDepacketiser::track_specs)
                .unwrap_or_default();
            self.events.push_back(Event::Tracks(specs));
            self.tracks_dirty = false;
        }
        if !self.seen_first_sample {
            self.events.push_back(Event::Status(Status::Live));
            self.seen_first_sample = true;
            self.attempt = 0;
        }
        self.last_sample_at = now;
        self.starved_data_fired = false;
        self.starved_nodata_fired = false;
        self.next_notice_at = None;
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
            self.emit_sample(id, sample, now);
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
                while now >= next {
                    let elapsed = next.saturating_sub(self.last_byte_at);
                    self.push_stalled(elapsed / US_PER_SEC);
                    next = next.saturating_add(US_PER_SEC);
                }
                self.next_notice_at = Some(next);
            }
        }
        if idle >= self.reconnect_us() {
            self.flush_depacketiser(now);
            self.drop_connection(true);
            self.attempt = 0;
            self.phase = Phase::Backoff(now);
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
        self.next_keepalive = Some(now.saturating_add(self.keepalive_interval));
        let Some(conn) = self.conn else {
            return;
        };
        if let Ok(bytes) = self.client.options(&self.uri) {
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
            self.attempt = 0;
            self.phase = Phase::Backoff(now);
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
