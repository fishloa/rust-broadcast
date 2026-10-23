//! Public [`Session`] implementation.

use alloc::collections::VecDeque;
use alloc::string::String;
use alloc::vec::Vec;
use std::collections::HashMap;

use url::Url;

use crate::auth::AuthState;
use crate::error::{Error, TransportError};
use crate::events::{Action, Config, Event, SourceKind};
use crate::redirect;
use crate::source::ts_http::TsHttpSource;

use tshttp_runtime::Action as TsAction;

/// Generic sans-IO client session.
pub struct Session {
    closed: bool,
    now_us: u64,
    config: Config,
    auth: AuthState,
    source: TsHttpSource,
    redirect_count: u32,
    next_conn: u32,
    live_conn: Option<u32>,
    known_conns: HashMap<u32, Url>,
    conn_auth: HashMap<u32, bool>,
    actions: VecDeque<Action>,
    events: VecDeque<Event>,
}

impl Session {
    /// Open a session.
    pub fn open(url: &str, config: Config, now_us: u64) -> Result<Self, Error> {
        let parsed = Url::parse(url).map_err(|e| Error::InvalidUrl(e.to_string()))?;
        let scheme = parsed.scheme();
        if scheme != "http" && scheme != "https" {
            return Err(Error::UnsupportedScheme(scheme.to_string()));
        }
        if config.kind == SourceKind::TsHttp || config.kind == SourceKind::Auto {
            // accepted
        }
        let auth = AuthState::new(parsed.clone(), config.extra_headers.clone());
        let source = TsHttpSource::new(parsed.clone(), config.ts_http.clone(), now_us);
        Ok(Self {
            closed: false,
            now_us,
            config,
            auth,
            source,
            redirect_count: 0,
            next_conn: 1,
            live_conn: None,
            known_conns: HashMap::new(),
            conn_auth: HashMap::new(),
            actions: VecDeque::new(),
            events: VecDeque::new(),
        })
    }

    /// Poll next action.
    pub fn poll(&mut self, now_us: u64) -> Option<Action> {
        if self.closed && self.actions.is_empty() {
            return None;
        }
        if !self.closed {
            self.now_us = now_us;
            if !self.auth.is_waiting() {
                self.source.poll_timeout(now_us);
            }
            self.drain_source();
        }
        self.actions.pop_front()
    }

    /// Poll next event.
    pub fn next_event(&mut self) -> Option<Event> {
        if self.closed && self.events.is_empty() {
            return None;
        }
        if !self.closed {
            self.drain_source();
        }
        self.events.pop_front()
    }

    /// Response callback.
    pub fn on_response(&mut self, conn: u32, status: u16, headers: &[(String, String)]) {
        if self.closed {
            return;
        }
        let Some(req_url) = self.known_conns.get(&conn).cloned() else {
            return;
        };
        let had_auth = self.conn_auth.get(&conn).copied().unwrap_or(false);

        if status == 301 || status == 302 || status == 303 || status == 307 || status == 308 {
            if let Some(target) = redirect::resolve(
                &req_url,
                headers,
                self.redirect_count,
                self.config.max_redirects,
            ) {
                self.redirect_count = self.redirect_count.saturating_add(1);
                self.source.set_url(target.clone());
                self.known_conns.remove(&conn);
                self.conn_auth.remove(&conn);
                if self.live_conn == Some(conn) {
                    self.live_conn = None;
                }
                self.actions.push_back(Action::Close { conn });
                let new_conn = self.alloc_conn(target.clone());
                let new_headers = self.auth.request_headers(&target);
                let authed = new_headers
                    .iter()
                    .any(|(k, _)| k.eq_ignore_ascii_case("authorization"));
                self.conn_auth.insert(new_conn, authed);
                self.actions.push_back(Action::HttpRequest {
                    conn: new_conn,
                    method: "GET".to_string(),
                    url: target.to_string(),
                    headers: new_headers,
                    byte_range: None,
                });
            } else {
                self.fail_connect(conn);
            }
            return;
        }

        if status == 401 || status == 407 {
            self.known_conns.remove(&conn);
            self.conn_auth.remove(&conn);
            if self.live_conn == Some(conn) {
                self.live_conn = None;
            }
            self.actions.push_back(Action::Close { conn });

            match self.auth.handle_unauthorized(status, headers, had_auth) {
                crate::auth::AuthDecision::Replay => {
                    let target = self.source.url().clone();
                    let new_conn = self.alloc_conn(target.clone());
                    let new_headers = self.auth.request_headers(&target);
                    let authed = new_headers
                        .iter()
                        .any(|(k, _)| k.eq_ignore_ascii_case("authorization"));
                    self.conn_auth.insert(new_conn, authed);
                    self.actions.push_back(Action::HttpRequest {
                        conn: new_conn,
                        method: "GET".to_string(),
                        url: target.to_string(),
                        headers: new_headers,
                        byte_range: None,
                    });
                }
                crate::auth::AuthDecision::Wait(events) => {
                    for e in events {
                        self.events.push_back(e);
                    }
                }
            }
            return;
        }

        if status == 403 {
            self.events.push_back(Event::Forbidden { status });
            self.fail_connect(conn);
            return;
        }

        if (200..300).contains(&status) {
            self.redirect_count = 0;
            self.live_conn = Some(conn);
        }
        self.source.on_response(status, headers, self.now_us);
        self.drain_source();
    }

    /// Data callback.
    pub fn on_data(&mut self, conn: u32, bytes: &[u8], now_us: u64) {
        if self.closed {
            return;
        }
        if !self.known_conns.contains_key(&conn) {
            return;
        }
        if self.live_conn != Some(conn) {
            return;
        }
        self.now_us = now_us;
        self.source.on_data(bytes, now_us);
        self.drain_source();
    }

    /// Closed callback.
    pub fn on_closed(&mut self, conn: u32, error: Option<TransportError>, now_us: u64) {
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
        self.now_us = now_us;
        self.source.on_closed(error, now_us);
        self.drain_source();
    }

    /// Set credentials.
    pub fn set_credentials(&mut self, creds: crate::Credentials) {
        if self.closed {
            return;
        }
        let was_waiting = self.auth.set_credentials(creds);
        if was_waiting {
            self.source.reconnect(self.now_us);
            self.drain_source();
        }
    }

    /// Caller-initiated reconnect.
    pub fn reconnect(&mut self, now_us: u64) {
        if self.closed {
            return;
        }
        self.now_us = now_us;
        self.source.reconnect(now_us);
        self.drain_source();
    }

    /// Terminal close.
    pub fn close(&mut self) {
        if self.closed {
            return;
        }
        self.closed = true;
        let conns: Vec<u32> = self.known_conns.keys().copied().collect();
        for c in conns {
            self.actions.push_back(Action::Close { conn: c });
        }
        self.known_conns.clear();
        self.conn_auth.clear();
        self.live_conn = None;
        self.events.clear();
    }

    fn alloc_conn(&mut self, url: Url) -> u32 {
        let id = self.next_conn;
        self.next_conn = self.next_conn.saturating_add(1);
        if self.next_conn == 0 {
            self.next_conn = 1;
        }
        self.known_conns.insert(id, url);
        id
    }

    fn fail_connect(&mut self, conn: u32) {
        self.known_conns.remove(&conn);
        self.conn_auth.remove(&conn);
        if self.live_conn == Some(conn) {
            self.live_conn = None;
        }
        self.source.on_error(self.now_us);
        self.drain_source();
    }

    fn drain_source(&mut self) {
        while let Some(e) = self.source.poll_event() {
            self.events.push_back(e);
        }
        while let Some(ta) = self.source.poll_action() {
            self.map_source_action(ta);
        }
    }

    fn map_source_action(&mut self, a: TsAction) {
        match a {
            TsAction::Open { .. } => {
                self.redirect_count = 0;
                let target = self.source.url().clone();
                let conn = self.alloc_conn(target.clone());
                let headers = self.auth.request_headers(&target);
                let authed = headers
                    .iter()
                    .any(|(k, _)| k.eq_ignore_ascii_case("authorization"));
                self.conn_auth.insert(conn, authed);
                self.actions.push_back(Action::HttpRequest {
                    conn,
                    method: "GET".to_string(),
                    url: target.to_string(),
                    headers,
                    byte_range: None,
                });
            }
            TsAction::Abort => {
                if let Some(conn) = self.live_conn {
                    self.known_conns.remove(&conn);
                    self.conn_auth.remove(&conn);
                    self.live_conn = None;
                    self.actions.push_back(Action::Close { conn });
                }
            }
            TsAction::Reconnect { after_ms } => {
                self.actions
                    .push_back(Action::WakeAt(self.now_us.saturating_add(after_ms * 1000)));
            }
            _ => {}
        }
    }
}

impl core::fmt::Debug for Session {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Session")
            .field("closed", &self.closed)
            .field("now_us", &self.now_us)
            .field("auth", &self.auth)
            .finish_non_exhaustive()
    }
}
