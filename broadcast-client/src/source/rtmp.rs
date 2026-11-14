//! RTMP pull source: `rtmp://` and `rtmps://` play over one TCP connection the host opens, on
//! `rtmp-runtime`'s sans-IO [`ClientSession`] in play mode (Adobe RTMP 1.0 7.2.1.1 `connect`,
//! 7.2.1.3 `createStream`, 7.2.2.1 `play`).
//!
//! No RTMP wire format is written here: the client returns FLV tags, which
//! [`transmux::StreamingFlvDemux`] turns into tracks and samples. The start, stall, backoff and
//! reconnect rules are those of [`crate::tshttp::TsHttpPolicy`], the demux-free policy core of the
//! TS-over-HTTP source: `Open` becomes a `TcpConnect`, a connected socket plays the part of the
//! `200` response, `Abort` becomes a `Close` and `Reconnect` a `WakeAt`.
//!
//! Spec: `private/specs-verified/rtmp/rtmp-runtime/rtmp.md` (sections 7.1 to 7.2.1 for the command
//! flow, 6.7 for the user control events the client answers).

use alloc::collections::{BTreeMap, VecDeque};
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use crate::tshttp::TsHttpConfig as TsConfig;
use crate::tshttp::TsHttpPolicy;
use rtmp_runtime::client::{ClientEvent, ClientMode, ClientSession, RtmpClientConfig};
use rtmp_runtime::target::RtmpTarget;
use transmux::{DemuxEvent, StreamingFlvDemux, TrackId, TrackSpec};
use url::{Host, Url};

use crate::ConnId;
use crate::error::{Error, TransportError};
use crate::events::{Action, ErrorKind, Event, MAX_SERVER_REFUSED_REASON_BYTES, Status};
use crate::source::fetch::deadline;
use broadcast_common::stage::Timestamp;

pub(crate) const SCHEME_RTMP: &str = "rtmp";
pub(crate) const SCHEME_RTMPS: &str = "rtmps";
const FIRST_CONN_ID: u32 = 1;
const STATUS_STREAM_NOT_FOUND: &str = "NetStream.Play.StreamNotFound";

/// The parsed target: [`rtmp_runtime::target::RtmpTarget`] (which owns the RTMP URL grammar:
/// `rtmp`/`rtmps`, default ports, percent-decoding) with the host as the text a connect
/// action carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Target {
    pub(crate) host: String,
    pub(crate) port: u16,
    pub(crate) tls: bool,
    pub(crate) app: String,
    pub(crate) stream: String,
    pub(crate) tc_url: String,
}

pub(crate) fn parse_target(url: &Url) -> Result<Target, Error> {
    let target = RtmpTarget::parse(url.as_str()).map_err(|e| Error::InvalidUrl(e.to_string()))?;
    let host = match &target.host {
        Host::Domain(d) => d.clone(),
        Host::Ipv4(a) => a.to_string(),
        Host::Ipv6(a) => a.to_string(),
    };
    Ok(Target {
        host,
        port: target.port,
        tls: target.tls,
        app: target.app,
        stream: target.stream_key,
        tc_url: target.tc_url,
    })
}

fn sanitize_code(code: &str) -> String {
    let mut out = String::new();
    for ch in code.chars().filter(|c| !c.is_control()) {
        if out.len() + ch.len_utf8() > MAX_SERVER_REFUSED_REASON_BYTES {
            break;
        }
        out.push(ch);
    }
    out
}

pub(crate) struct RtmpSource {
    target: Target,
    policy: TsHttpPolicy,
    demux: StreamingFlvDemux,
    client: Option<ClientSession>,
    conn: Option<ConnId>,
    next_conn: u32,
    recorded: bool,
    tracks: BTreeMap<TrackId, TrackSpec>,
    tracks_dirty: bool,
    seen_first_sample: bool,
    connection_count: u32,
    stalled: bool,
    failure_announced: bool,
    ended: bool,
    closed: bool,
    wake_pending: Option<u64>,
    now_us: u64,
    actions: VecDeque<Action>,
    events: VecDeque<Event>,
}

impl RtmpSource {
    pub(crate) fn new(url: &Url, cfg: &TsConfig, now_us: u64) -> Result<Self, Error> {
        let target = parse_target(url)?;
        let policy = TsHttpPolicy::new(url.as_str(), cfg.clone(), Timestamp::from_micros(now_us));
        let mut source = Self {
            target,
            policy,
            demux: StreamingFlvDemux::new(),
            client: None,
            conn: None,
            next_conn: FIRST_CONN_ID,
            recorded: false,
            tracks: BTreeMap::new(),
            tracks_dirty: false,
            seen_first_sample: false,
            connection_count: 0,
            stalled: false,
            failure_announced: false,
            ended: false,
            closed: false,
            wake_pending: None,
            now_us,
            actions: VecDeque::new(),
            events: VecDeque::new(),
        };
        source.pump_policy(now_us);
        Ok(source)
    }

    pub(crate) fn take_actions(&mut self) -> VecDeque<Action> {
        core::mem::take(&mut self.actions)
    }

    pub(crate) fn take_events(&mut self) -> VecDeque<Event> {
        core::mem::take(&mut self.events)
    }

    pub(crate) fn conn(&self) -> Option<ConnId> {
        self.conn
    }

    pub(crate) fn poll(&mut self, now_us: u64) {
        if self.closed || self.ended {
            return;
        }
        self.now_us = now_us;
        let demux = &mut self.demux;
        self.policy
            .poll_timeout_with(Timestamp::from_micros(now_us), |events| {
                demux.finish();
                let mut sampled = false;
                while let Some(ev) = demux.poll_event() {
                    sampled |= matches!(ev, DemuxEvent::Sample { .. });
                    events.push_back(crate::tshttp::Event::Demux(Box::new(ev)));
                }
                sampled
            });
        self.pump_policy(now_us);
    }

    pub(crate) fn on_connected(&mut self, conn: ConnId, now_us: u64) {
        if self.closed || self.ended || self.conn != Some(conn) || self.client.is_some() {
            return;
        }
        self.now_us = now_us;
        let mut client = ClientSession::new(self.client_config());
        let hello = client.start();
        self.client = Some(client);
        self.actions.push_back(Action::Send { conn, bytes: hello });
        self.policy
            .on_response(200, &[], Timestamp::from_micros(now_us));
        self.pump_policy(now_us);
    }

    pub(crate) fn on_data(&mut self, conn: ConnId, bytes: &[u8], now_us: u64) {
        if self.closed || self.ended || self.conn != Some(conn) {
            return;
        }
        self.now_us = now_us;
        let Some(client) = self.client.as_mut() else {
            return;
        };
        let (out, events) = match client.handle_data(bytes) {
            Ok(r) => r,
            Err(_) => {
                self.lose_connection(true, now_us);
                return;
            }
        };
        if !out.is_empty() {
            self.actions.push_back(Action::Send { conn, bytes: out });
        }
        let mut sampled = false;
        for event in events {
            match event {
                ClientEvent::Playing { recorded } => self.recorded = recorded,
                ClientEvent::Media { flv } => {
                    if self.demux.feed(&flv).is_err() {
                        self.lose_connection(true, now_us);
                        return;
                    }
                    sampled |= self.drain_demux();
                }
                ClientEvent::Error { code, .. } => {
                    self.refused(&code, now_us);
                    return;
                }
                ClientEvent::StreamEnded => {
                    self.stream_ended(now_us);
                    return;
                }
                _ => {}
            }
        }
        self.policy
            .on_bytes(bytes.len(), Timestamp::from_micros(now_us), sampled);
        self.pump_policy(now_us);
    }

    /// Whether the live connection ended and the next connection's media will be preceded by
    /// [`Event::Discontinuity`].
    pub(crate) fn on_closed(
        &mut self,
        conn: ConnId,
        error: Option<&TransportError>,
        now_us: u64,
    ) -> bool {
        if self.closed || self.ended || self.conn != Some(conn) {
            return false;
        }
        self.now_us = now_us;
        self.conn = None;
        self.client = None;
        self.flush_demux(now_us);
        if error.is_some() {
            self.policy.on_error(Timestamp::from_micros(now_us));
        } else {
            self.policy.on_eof(Timestamp::from_micros(now_us));
        }
        self.pump_policy(now_us);
        self.connection_count > 0
    }

    pub(crate) fn reconnect(&mut self, now_us: u64) {
        if self.closed {
            return;
        }
        self.now_us = now_us;
        self.ended = false;
        self.flush_demux(now_us);
        self.policy.reconnect(Timestamp::from_micros(now_us));
        self.pump_policy(now_us);
    }

    pub(crate) fn close(&mut self) {
        if self.closed {
            return;
        }
        self.closed = true;
        self.drop_connection();
    }

    pub(crate) fn sync_wake(&mut self, now_us: u64) {
        if self.closed || self.ended {
            return;
        }
        let Some(deadline) = self.policy.next_deadline().map(|d| d.as_micros()) else {
            return;
        };
        let outstanding = self
            .wake_pending
            .is_some_and(|w| w == deadline || (w > now_us && w < deadline));
        if !outstanding {
            self.actions
                .push_back(Action::WakeAt(Timestamp::from_micros(deadline)));
            self.wake_pending = Some(deadline);
        }
    }

    fn client_config(&self) -> RtmpClientConfig {
        let mut config = RtmpClientConfig::default();
        config.mode = ClientMode::Play;
        config.app.clone_from(&self.target.app);
        config.stream_key.clone_from(&self.target.stream);
        config.tc_url = Some(self.target.tc_url.clone());
        config
    }

    fn alloc_conn(&mut self) -> ConnId {
        loop {
            let candidate = self.next_conn;
            self.next_conn = candidate.checked_add(1).unwrap_or(FIRST_CONN_ID);
            if Some(ConnId::new(candidate)) != self.conn {
                return ConnId::new(candidate);
            }
        }
    }

    fn drop_connection(&mut self) {
        if let Some(conn) = self.conn.take() {
            self.actions.push_back(Action::Close { conn });
        }
        self.client = None;
    }

    fn flush_demux(&mut self, now_us: u64) {
        self.demux.finish();
        if self.drain_demux() {
            self.policy.on_sample(Timestamp::from_micros(now_us));
        }
    }

    fn drain_demux(&mut self) -> bool {
        let mut sampled = false;
        while let Some(ev) = self.demux.poll_event() {
            sampled |= matches!(ev, DemuxEvent::Sample { .. });
            self.policy
                .push_event(crate::tshttp::Event::Demux(Box::new(ev)));
        }
        sampled
    }

    fn lose_connection(&mut self, error: bool, now_us: u64) {
        self.drop_connection();
        self.flush_demux(now_us);
        if error {
            self.policy.on_error(Timestamp::from_micros(now_us));
        } else {
            self.policy.on_eof(Timestamp::from_micros(now_us));
        }
        self.pump_policy(now_us);
    }

    fn refused(&mut self, code: &str, now_us: u64) {
        if code == STATUS_STREAM_NOT_FOUND {
            self.events.push_back(Event::Status(Status::NotFound));
        } else {
            self.events.push_back(Event::Error {
                kind: ErrorKind::RtmpStatus {
                    code: sanitize_code(code),
                },
                fatal: false,
            });
        }
        self.lose_connection(true, now_us);
    }

    fn stream_ended(&mut self, now_us: u64) {
        if !self.recorded {
            self.lose_connection(false, now_us);
            return;
        }
        self.drop_connection();
        self.flush_demux(now_us);
        self.pump_policy(now_us);
        self.drain_pending_tracks();
        self.events.push_back(Event::Status(Status::Ended));
        self.ended = true;
    }

    fn drain_pending_tracks(&mut self) {
        if self.tracks_dirty {
            self.announce_tracks();
        }
    }

    fn announce_tracks(&mut self) {
        let specs: Vec<TrackSpec> = self.tracks.values().cloned().collect();
        self.events.push_back(Event::Tracks(specs));
        self.tracks_dirty = false;
    }

    fn pump_policy(&mut self, now_us: u64) {
        loop {
            let mut progressed = false;
            while let Some(event) = self.policy.poll_event() {
                progressed = true;
                self.map_event(event);
            }
            while let Some(action) = self.policy.poll() {
                progressed = true;
                self.map_action(action, now_us);
            }
            if !progressed {
                break;
            }
        }
    }

    fn map_action(&mut self, action: crate::tshttp::Action, now_us: u64) {
        match action {
            crate::tshttp::Action::Open { .. } => {
                self.drop_connection();
                self.demux = StreamingFlvDemux::new();
                self.recorded = false;
                self.client = None;
                let conn = self.alloc_conn();
                self.conn = Some(conn);
                self.actions.push_back(Action::TcpConnect {
                    conn,
                    host: self.target.host.clone(),
                    port: self.target.port,
                    tls: self.target.tls,
                    identity: None,
                });
            }
            crate::tshttp::Action::Abort => self.drop_connection(),
            crate::tshttp::Action::Reconnect { after } => {
                if self.failure_announced {
                    self.failure_announced = false;
                } else {
                    self.events
                        .push_back(Event::Status(Status::Reconnecting { attempt: 0 }));
                }
                // A deadline past the end of the clock can never be reached: no wake for it.
                if let Some(at) = deadline(Timestamp::from_micros(now_us), after) {
                    self.actions.push_back(Action::WakeAt(at));
                }
            }
        }
    }

    fn map_event(&mut self, event: crate::tshttp::Event) {
        match event {
            crate::tshttp::Event::Connected => {
                self.tracks.clear();
                self.tracks_dirty = true;
                self.connection_count = self.connection_count.saturating_add(1);
                if self.connection_count > 1 {
                    self.events.push_back(Event::Discontinuity);
                }
                self.events.push_back(Event::Status(Status::Establishing));
                self.seen_first_sample = false;
                self.stalled = false;
            }
            crate::tshttp::Event::Demux(ev) => match *ev {
                DemuxEvent::TrackAdded(spec) | DemuxEvent::TrackUpdated(spec) => {
                    self.tracks.insert(spec.track_id, spec);
                    self.tracks_dirty = true;
                }
                DemuxEvent::TrackRemoved { track_id, .. } => {
                    self.tracks.remove(&track_id);
                    self.tracks_dirty = true;
                }
                DemuxEvent::Sample {
                    track_id, sample, ..
                } => {
                    if !self.seen_first_sample {
                        self.seen_first_sample = true;
                        if self.tracks_dirty {
                            self.announce_tracks();
                        }
                        self.events.push_back(Event::Status(Status::Live));
                    } else if self.tracks_dirty {
                        self.announce_tracks();
                    }
                    if self.stalled {
                        self.stalled = false;
                        self.events.push_back(Event::Status(Status::Live));
                    }
                    self.events.push_back(Event::Sample {
                        track: track_id,
                        sample,
                        join: None,
                    });
                }
                DemuxEvent::DisplayChanged {
                    track_id,
                    display,
                    pts,
                    ..
                } => {
                    if self.tracks_dirty {
                        self.announce_tracks();
                    }
                    self.events.push_back(Event::DisplayChanged {
                        track: track_id,
                        pts,
                        display,
                    });
                }
                _ => {}
            },
            crate::tshttp::Event::StallNotice { secs } => {
                self.events
                    .push_back(Event::Status(Status::Stalled { secs }));
                self.stalled = true;
            }
            crate::tshttp::Event::Reconnecting { attempt } => {
                self.failure_announced = true;
                self.events
                    .push_back(Event::Status(Status::Reconnecting { attempt }));
            }
            crate::tshttp::Event::NotFound => {
                self.events.push_back(Event::Status(Status::NotFound));
            }
            crate::tshttp::Event::Starved(kind) => {
                self.events.push_back(Event::Starved(kind));
            }
            _ => {}
        }
    }
}
