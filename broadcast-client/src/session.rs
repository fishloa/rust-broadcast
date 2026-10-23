//! Public [`Session`] implementation.

use alloc::collections::{BTreeMap, VecDeque};
use alloc::string::String;
use alloc::vec::Vec;
use std::collections::HashMap;

use url::Url;

use broadcast_auth::challenge::parse_challenges;
use broadcast_auth::{Authenticator, Credentials};
use transmux::{TrackId, TrackSpec};
use tshttp_runtime::TsHttpClient;

use crate::{Action, Config, ConnId, Error, Event, SourceKind, Status, TransportError};

/// Generic sans-IO client session.
pub struct Session {
    closed: bool,
    now_ms: u64,
    config: Config,
    origin: Url,
    current_url: Url,
    credentials: Option<Credentials>,
    authenticator: Option<Authenticator>,
    waiting_for_credentials: bool,
    ts: TsHttpClient,
    redirects_taken: u32,
    next_conn: ConnId,
    live_conn: Option<ConnId>,
    known_conns: HashMap<ConnId, Url>,
    conn_auth: HashMap<ConnId, bool>,
    connection_count: u32,
    conn_seen_first_sample: bool,
    conn_connected: bool,
    tracks: BTreeMap<TrackId, TrackSpec>,
    tracks_dirty: bool,
    stalled: bool,
    actions: VecDeque<Action>,
    events: VecDeque<Event>,
}

impl Session {
    /// Open a session.
    pub fn open(url: &str, config: Config, now_ms: u64) -> Result<Self, Error> {
        let parsed = Url::parse(url).map_err(|e| Error::InvalidUrl(e.to_string()))?;
        let scheme = parsed.scheme();
        if scheme != "http" && scheme != "https" {
            return Err(Error::UnsupportedScheme(scheme.to_string()));
        }
        if config.kind == SourceKind::TsHttp {
            // accepted
        }
        let ts = TsHttpClient::new(parsed.as_str(), config.ts_http.clone(), now_ms);
        Ok(Self {
            closed: false,
            now_ms,
            config,
            origin: parsed.clone(),
            current_url: parsed,
            credentials: None,
            authenticator: None,
            waiting_for_credentials: false,
            ts,
            redirects_taken: 0,
            next_conn: 1,
            live_conn: None,
            known_conns: HashMap::new(),
            conn_auth: HashMap::new(),
            connection_count: 0,
            conn_seen_first_sample: false,
            conn_connected: false,
            tracks: BTreeMap::new(),
            tracks_dirty: false,
            stalled: false,
            actions: VecDeque::new(),
            events: VecDeque::new(),
        })
    }

    /// Poll next action.
    pub fn poll(&mut self, now_ms: u64) -> Option<Action> {
        if self.closed {
            return None;
        }
        self.now_ms = now_ms;
        self.ts.poll_timeout(now_ms);
        self.drain_ts();
        self.actions.pop_front()
    }

    /// Poll next event.
    pub fn next_event(&mut self) -> Option<Event> {
        if self.closed {
            return None;
        }
        self.drain_ts();
        self.events.pop_front()
    }

    /// Response callback.
    pub fn on_response(&mut self, conn: ConnId, status: u16, headers: &[(String, String)]) {
        if self.closed {
            return;
        }
        let Some(req_url) = self.known_conns.get(&conn).cloned() else {
            return;
        };
        let had_auth = self.conn_auth.get(&conn).copied().unwrap_or(false);

        if status == 301 || status == 302 || status == 303 || status == 307 || status == 308 {
            let loc = headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case("location"))
                .map(|(_, v)| v.clone());
            let Some(loc) = loc else {
                self.fail_connect(conn);
                return;
            };
            if self.redirects_taken >= self.config.max_redirects {
                self.fail_connect(conn);
                return;
            }
            let target = match req_url.join(&loc) {
                Ok(u) => u,
                Err(_) => {
                    self.fail_connect(conn);
                    return;
                }
            };
            self.known_conns.remove(&conn);
            self.conn_auth.remove(&conn);
            if self.live_conn == Some(conn) {
                self.live_conn = None;
            }
            self.actions.push_back(Action::Close { conn });
            self.redirects_taken = self.redirects_taken.saturating_add(1);
            self.current_url = target.clone();
            let new_conn = self.alloc_conn(target.clone());
            let headers = self.request_headers(&target);
            let auth = headers
                .iter()
                .any(|(k, _)| k.eq_ignore_ascii_case("authorization"));
            self.conn_auth.insert(new_conn, auth);
            self.actions.push_back(Action::HttpRequest {
                conn: new_conn,
                method: "GET".to_string(),
                url: target.to_string(),
                headers,
                byte_range: None,
            });
            return;
        }

        if status == 401 || status == 407 {
            self.known_conns.remove(&conn);
            self.conn_auth.remove(&conn);
            if self.live_conn == Some(conn) {
                self.live_conn = None;
            }
            self.actions.push_back(Action::Close { conn });

            let lines: Vec<&str> = headers
                .iter()
                .filter(|(k, _)| k.eq_ignore_ascii_case("www-authenticate"))
                .map(|(_, v)| v.as_str())
                .collect();

            let stale = headers
                .iter()
                .filter(|(k, _)| k.eq_ignore_ascii_case("www-authenticate"))
                .flat_map(|(_, v)| parse_challenges(&[v.as_str()]).unwrap_or_default())
                .any(|c| {
                    c.other_params.iter().any(|(k, v)| {
                        k.eq_ignore_ascii_case("stale") && v.eq_ignore_ascii_case("true")
                    })
                });

            if self.credentials.is_none() || (!had_auth && !stale) {
                if (self.credentials.is_some() && !had_auth) || (had_auth && stale) {
                    // build authenticator and replay
                    if let (Some(creds), Some(line)) =
                        (self.credentials.clone(), lines.first().copied())
                        && let Ok(auth) = Authenticator::from_challenge(line, creds)
                    {
                        self.authenticator = Some(auth);
                    }
                    // replay same request
                    let target = self.current_url.clone();
                    let new_conn = self.alloc_conn(target.clone());
                    let headers = self.request_headers(&target);
                    let auth = headers
                        .iter()
                        .any(|(k, _)| k.eq_ignore_ascii_case("authorization"));
                    self.conn_auth.insert(new_conn, auth);
                    self.actions.push_back(Action::HttpRequest {
                        conn: new_conn,
                        method: "GET".to_string(),
                        url: target.to_string(),
                        headers,
                        byte_range: None,
                    });
                    return;
                }
                // no credentials: AuthRequired
                let chosen = lines
                    .iter()
                    .flat_map(|l| parse_challenges(&[*l]).unwrap_or_default())
                    .find(|c| c.resource_metadata.is_some())
                    .or_else(|| {
                        lines
                            .first()
                            .and_then(|l| parse_challenges(&[*l]).ok())
                            .and_then(|v| v.into_iter().next())
                    });
                let scheme = chosen
                    .as_ref()
                    .map(|c| c.scheme.clone())
                    .unwrap_or(broadcast_auth::Scheme::Bearer);
                let realm = chosen.as_ref().and_then(|c| c.realm.clone());
                let resource_metadata = chosen.as_ref().and_then(|c| c.resource_metadata.clone());
                self.events.push_back(Event::AuthRequired {
                    scheme,
                    realm,
                    origin: self.origin.clone(),
                    resource_metadata,
                });
                self.events
                    .push_back(Event::Status(Status::WaitingForCredentials));
                self.waiting_for_credentials = true;
                self.authenticator = None;
            } else {
                self.events.push_back(Event::AuthRejected { status });
                self.events
                    .push_back(Event::Status(Status::WaitingForCredentials));
                self.waiting_for_credentials = true;
                self.authenticator = None;
            }
            return;
        }

        if status == 403 {
            self.events.push_back(Event::Forbidden { status });
            self.fail_connect(conn);
            return;
        }

        if (200..300).contains(&status) {
            self.redirects_taken = 0;
            self.live_conn = Some(conn);
            self.ts.on_response(status, self.now_ms);
            self.drain_ts();
        } else {
            self.ts.on_response(status, self.now_ms);
            self.drain_ts();
        }
    }

    /// Data callback.
    pub fn on_data(&mut self, conn: ConnId, bytes: &[u8], now_ms: u64) {
        if self.closed {
            return;
        }
        if !self.known_conns.contains_key(&conn) {
            return;
        }
        if self.live_conn != Some(conn) {
            return;
        }
        self.now_ms = now_ms;
        if self.stalled && !bytes.is_empty() {
            self.events.push_back(Event::Status(Status::Live));
            self.stalled = false;
        }
        self.ts.feed(bytes, now_ms);
        self.drain_ts();
    }

    /// Closed callback.
    pub fn on_closed(&mut self, conn: ConnId, error: Option<TransportError>, now_ms: u64) {
        if self.closed {
            return;
        }
        if !self.known_conns.contains_key(&conn) {
            return;
        }
        self.known_conns.remove(&conn);
        self.conn_auth.remove(&conn);
        if self.live_conn == Some(conn) {
            self.live_conn = None;
        }
        self.now_ms = now_ms;
        if error.is_some() {
            self.ts.on_error(now_ms);
        } else {
            self.ts.on_eof(now_ms);
        }
        self.drain_ts();
    }

    /// Set credentials.
    pub fn set_credentials(&mut self, creds: Credentials) {
        if self.closed {
            return;
        }
        self.credentials = Some(creds);
        if self.waiting_for_credentials {
            self.waiting_for_credentials = false;
            self.authenticator = None;
            self.ts.reconnect(self.now_ms);
            self.drain_ts();
        } else {
            self.authenticator = None;
        }
    }

    /// Caller-initiated reconnect.
    pub fn reconnect(&mut self, now_ms: u64) {
        if self.closed {
            return;
        }
        self.now_ms = now_ms;
        self.ts.reconnect(now_ms);
        self.drain_ts();
    }

    /// Terminal close.
    pub fn close(&mut self) {
        if self.closed {
            return;
        }
        self.closed = true;
        let conns: Vec<ConnId> = self.known_conns.keys().copied().collect();
        for c in conns {
            self.actions.push_back(Action::Close { conn: c });
        }
        self.known_conns.clear();
        self.conn_auth.clear();
        self.live_conn = None;
    }

    fn alloc_conn(&mut self, url: Url) -> ConnId {
        let id = self.next_conn;
        self.next_conn = self.next_conn.saturating_add(1);
        if self.next_conn == 0 {
            self.next_conn = 1;
        }
        self.known_conns.insert(id, url);
        id
    }

    fn request_headers(&mut self, url: &Url) -> Vec<(String, String)> {
        let mut headers = self.config.extra_headers.clone();
        if url.origin() == self.origin.origin()
            && let Some(creds) = &self.credentials
        {
            match creds {
                Credentials::Bearer { token } => {
                    headers.push(("Authorization".to_string(), format!("Bearer {token}")));
                }
                Credentials::Basic { .. } | Credentials::Digest { .. } => {
                    if let Some(auth) = &mut self.authenticator {
                        let ctx = broadcast_auth::RequestContext::new("GET", url.as_str());
                        if let Ok(val) = auth.authorization(&ctx) {
                            headers.push(("Authorization".to_string(), val));
                        }
                    }
                }
                _ => {}
            }
        }
        headers
    }

    fn fail_connect(&mut self, conn: ConnId) {
        self.known_conns.remove(&conn);
        self.conn_auth.remove(&conn);
        if self.live_conn == Some(conn) {
            self.live_conn = None;
        }
        self.ts.on_error(self.now_ms);
        self.drain_ts();
    }

    fn drain_ts(&mut self) {
        while let Some(ev) = self.ts.poll_event() {
            self.map_ts_event(ev);
        }
        while let Some(act) = self.ts.poll() {
            self.map_ts_action(act);
        }
    }

    fn map_ts_action(&mut self, a: tshttp_runtime::Action) {
        match a {
            tshttp_runtime::Action::Open { .. } => {
                self.redirects_taken = 0;
                self.connection_count = self.connection_count.saturating_add(1);
                let target = self.current_url.clone();
                let conn = self.alloc_conn(target.clone());
                let headers = self.request_headers(&target);
                let auth = headers
                    .iter()
                    .any(|(k, _)| k.eq_ignore_ascii_case("authorization"));
                self.conn_auth.insert(conn, auth);
                self.actions.push_back(Action::HttpRequest {
                    conn,
                    method: "GET".to_string(),
                    url: target.to_string(),
                    headers,
                    byte_range: None,
                });
            }
            tshttp_runtime::Action::Abort => {
                if let Some(conn) = self.live_conn {
                    self.known_conns.remove(&conn);
                    self.conn_auth.remove(&conn);
                    self.live_conn = None;
                    self.actions.push_back(Action::Close { conn });
                }
            }
            tshttp_runtime::Action::Reconnect { after_ms } => {
                self.actions
                    .push_back(Action::WakeAt(self.now_ms.saturating_add(after_ms)));
            }
            _ => {}
        }
    }

    fn map_ts_event(&mut self, e: tshttp_runtime::Event) {
        match e {
            tshttp_runtime::Event::Connected => {
                self.events.push_back(Event::Status(Status::Establishing));
                self.conn_connected = true;
                self.conn_seen_first_sample = false;
            }
            tshttp_runtime::Event::Demux(transmux::DemuxEvent::TrackAdded(spec)) => {
                self.tracks.insert(spec.track_id, spec);
                self.tracks_dirty = true;
            }
            tshttp_runtime::Event::Demux(transmux::DemuxEvent::TrackUpdated(spec)) => {
                self.tracks.insert(spec.track_id, spec);
                self.tracks_dirty = true;
            }
            tshttp_runtime::Event::Demux(transmux::DemuxEvent::TrackRemoved {
                track_id, ..
            }) => {
                self.tracks.remove(&track_id);
                self.tracks_dirty = true;
            }
            tshttp_runtime::Event::Demux(transmux::DemuxEvent::Sample {
                track_id, sample, ..
            }) => {
                if !self.conn_seen_first_sample {
                    if self.connection_count > 1 {
                        self.events.push_back(Event::Discontinuity);
                    }
                    if self.tracks_dirty {
                        let vals: Vec<TrackSpec> = self.tracks.values().cloned().collect();
                        self.events.push_back(Event::Tracks(vals));
                        self.tracks_dirty = false;
                    }
                    if self.conn_connected {
                        self.events.push_back(Event::Status(Status::Live));
                    }
                    self.conn_seen_first_sample = true;
                } else if self.tracks_dirty {
                    let vals: Vec<TrackSpec> = self.tracks.values().cloned().collect();
                    self.events.push_back(Event::Tracks(vals));
                    self.tracks_dirty = false;
                }
                self.events.push_back(Event::Sample {
                    track: track_id,
                    sample,
                });
            }
            tshttp_runtime::Event::Demux(transmux::DemuxEvent::DisplayChanged {
                track_id,
                display,
                pts,
                ..
            }) => {
                if self.tracks_dirty {
                    let vals: Vec<TrackSpec> = self.tracks.values().cloned().collect();
                    self.events.push_back(Event::Tracks(vals));
                    self.tracks_dirty = false;
                }
                self.events.push_back(Event::DisplayChanged {
                    track: track_id,
                    pts,
                    display,
                });
            }
            tshttp_runtime::Event::StallNotice { secs } => {
                self.events
                    .push_back(Event::Status(Status::Stalled { secs }));
                self.stalled = true;
            }
            tshttp_runtime::Event::Reconnecting { attempt } => {
                self.events
                    .push_back(Event::Status(Status::Reconnecting { attempt }));
            }
            tshttp_runtime::Event::NotFound { status: _ } => {
                self.events.push_back(Event::Status(Status::NotFound));
                // tshttp also emits Reconnecting separately
            }
            tshttp_runtime::Event::Starved(k) => {
                self.events.push_back(Event::Starved(k));
            }
            _ => {}
        }
    }
}

impl core::fmt::Debug for Session {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Session")
            .field("closed", &self.closed)
            .field("now_ms", &self.now_ms)
            .field("origin", &self.origin)
            .field("current_url", &self.current_url)
            .field("credentials", &self.credentials)
            .field("waiting_for_credentials", &self.waiting_for_credentials)
            .finish_non_exhaustive()
    }
}
