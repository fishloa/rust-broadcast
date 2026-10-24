//! Public [`Session`] implementation.

use alloc::collections::VecDeque;
use alloc::string::String;
use alloc::vec::Vec;
use std::collections::HashMap;

use url::Url;

use crate::Hint;
use crate::auth::AuthState;
use crate::error::{Error, TransportError};
use crate::events::{Action, Config, Event, SourceKind};
use crate::redirect;
use crate::source::hls::HlsSource;
use crate::source::ts_http::TsHttpSource;
use transmux::TrackId;

use tshttp_runtime::Action as TsAction;

/// Generic sans-IO client session.
pub struct Session {
    closed: bool,
    now_us: u64,
    config: Config,
    auth: AuthState,
    source: TsHttpSource,
    hls: Option<Box<HlsSource>>,
    redirect_count: u32,
    next_conn: u32,
    live_conn: Option<u32>,
    known_conns: HashMap<u32, Url>,
    conn_auth: HashMap<u32, bool>,
    actions: VecDeque<Action>,
    events: VecDeque<Event>,
    buffer: crate::buffer::Feeder,
}

impl Session {
    /// Open a session.
    pub fn open(url: &str, config: Config, now_us: u64) -> Result<Self, Error> {
        Self::open_with_memory(url, config, now_us, None)
    }

    /// Open a session seeded with stall memory.
    pub fn open_with_memory(
        url: &str,
        config: Config,
        now_us: u64,
        seed: Option<buffering_runtime::StallMemorySnapshot>,
    ) -> Result<Self, Error> {
        let parsed = Url::parse(url).map_err(|e| Error::InvalidUrl(e.to_string()))?;
        let scheme = parsed.scheme();
        if scheme != "http" && scheme != "https" {
            return Err(Error::UnsupportedScheme(scheme.to_string()));
        }
        let use_hls = match config.kind {
            SourceKind::Hls => true,
            SourceKind::Auto => has_playlist_extension(&parsed),
            _ => false,
        };
        let buffer_cfg = config.buffer;
        let auth = AuthState::new(parsed.clone(), config.extra_headers.clone());
        let source = TsHttpSource::new(parsed.clone(), config.ts_http.clone(), now_us);
        let hls = use_hls.then(|| Box::new(HlsSource::new(&parsed, &config)));
        Ok(Self {
            closed: false,
            now_us,
            config,
            auth,
            source,
            hls,
            redirect_count: 0,
            next_conn: 1,
            live_conn: None,
            known_conns: HashMap::new(),
            conn_auth: HashMap::new(),
            actions: VecDeque::new(),
            events: VecDeque::new(),
            buffer: crate::buffer::Feeder::new(buffer_cfg, seed),
        })
    }

    /// Poll next action.
    pub fn poll(&mut self, now_us: u64) -> Option<Action> {
        if self.closed && self.actions.is_empty() {
            return None;
        }
        if !self.closed {
            self.now_us = now_us;
            if let Some(h) = self.hls.as_mut() {
                h.poll(now_us, &mut self.auth);
                self.flush_hls();
            } else {
                if !self.auth.is_waiting() {
                    self.source.poll_timeout(now_us);
                }
                self.drain_source();
            }
        }
        self.actions.pop_front()
    }

    /// Poll next event.
    pub fn next_event(&mut self) -> Option<Event> {
        if self.closed && self.events.is_empty() {
            return None;
        }
        if !self.closed && self.hls.is_none() {
            self.drain_source();
        }
        self.events.pop_front()
    }

    /// Response callback.
    pub fn on_response(&mut self, conn: u32, status: u16, headers: &[(String, String)]) {
        if self.closed {
            return;
        }
        if let Some(h) = self.hls.as_mut() {
            h.on_response(conn, status, headers, self.now_us, &mut self.auth);
            self.flush_hls();
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
        if let Some(h) = self.hls.as_mut() {
            self.now_us = now_us;
            h.on_data(conn, bytes, now_us, &mut self.auth);
            self.flush_hls();
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
        if !bytes.is_empty() {
            let pcr = self.source.take_last_pcr();
            self.buffer.on_chunk(now_us, bytes.len(), pcr);
            let evs = self.buffer.drain();
            for e in evs {
                self.events.push_back(e);
            }
        }
    }

    /// Closed callback.
    pub fn on_closed(&mut self, conn: u32, error: Option<TransportError>, now_us: u64) {
        if self.closed {
            return;
        }
        if let Some(h) = self.hls.as_mut() {
            self.now_us = now_us;
            h.on_closed(conn, error.as_deref(), now_us, &mut self.auth);
            self.flush_hls();
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
        let hint = if error.is_some() {
            Hint::SourceError
        } else {
            Hint::SourceEnded
        };
        self.buffer.hint(now_us, hint);
        self.source.on_closed(error, now_us);
        self.drain_source();
        let evs = self.buffer.drain();
        for e in evs {
            self.events.push_back(e);
        }
    }

    /// Set credentials.
    pub fn set_credentials(&mut self, creds: crate::Credentials) {
        if self.closed {
            return;
        }
        let was_waiting = self.auth.set_credentials(creds);
        if was_waiting {
            if let Some(h) = self.hls.as_mut() {
                h.resume(self.now_us, &mut self.auth);
                self.flush_hls();
            } else {
                self.source.reconnect(self.now_us);
                self.drain_source();
            }
        }
    }

    /// Caller-initiated reconnect.
    pub fn reconnect(&mut self, now_us: u64) {
        if self.closed {
            return;
        }
        self.now_us = now_us;
        if let Some(h) = self.hls.as_mut() {
            h.reconnect(now_us, &mut self.auth);
            self.flush_hls();
            return;
        }
        self.source.reconnect(now_us);
        self.drain_source();
    }

    /// Switch to another HLS variant of the last master playlist.
    ///
    /// # Errors
    /// [`Error::NotHls`] for a non-HLS session, [`Error::NoSuchVariant`] for an unknown index.
    pub fn select_variant(&mut self, index: usize) -> Result<(), Error> {
        let Some(h) = self.hls.as_mut() else {
            return Err(Error::NotHls);
        };
        if self.closed {
            return Ok(());
        }
        h.select_variant(index, self.now_us, &mut self.auth)?;
        self.flush_hls();
        Ok(())
    }

    /// Playback position update.
    pub fn on_playback_position(&mut self, now_us: u64, pts_27mhz: Option<u64>) {
        if self.closed {
            return;
        }
        self.now_us = now_us;
        self.buffer.on_playback_position(now_us, pts_27mhz);
        let evs = self.buffer.drain();
        for e in evs {
            self.events.push_back(e);
        }
    }

    /// Set rendered tracks.
    pub fn set_rendered_tracks(&mut self, tracks: &[TrackId]) {
        if self.closed {
            return;
        }
        self.buffer.set_rendered_tracks(tracks);
    }

    /// Hint passthrough.
    pub fn hint(&mut self, h: Hint) {
        if self.closed {
            return;
        }
        self.buffer.hint(self.now_us, h);
        let evs = self.buffer.drain();
        for e in evs {
            self.events.push_back(e);
        }
    }

    /// Re-arm join state without PcrDiscontinuity hint or controller reset.
    pub fn rejoin(&mut self) {
        if self.closed {
            return;
        }
        self.buffer.rejoin();
    }

    /// Inputs fed to the buffering controller so far.
    #[cfg(feature = "test-seams")]
    #[doc(hidden)]
    pub fn buffer_input_log(&self) -> &[crate::BufferInput] {
        &self.buffer.log
    }

    /// Stall memory snapshot.
    pub fn stall_memory(&self) -> buffering_runtime::StallMemorySnapshot {
        self.buffer.stall_memory()
    }

    /// Terminal close.
    pub fn close(&mut self) {
        if self.closed {
            return;
        }
        self.closed = true;
        let mut conns: Vec<u32> = self.known_conns.keys().copied().collect();
        if let Some(h) = self.hls.as_ref() {
            conns.extend(h.conns());
            conns.sort_unstable();
        }
        for c in conns {
            self.actions.push_back(Action::Close { conn: c });
        }
        self.known_conns.clear();
        self.conn_auth.clear();
        self.live_conn = None;
        self.events.clear();
    }

    fn flush_hls(&mut self) {
        if let Some(h) = self.hls.as_mut() {
            self.actions.extend(h.take_actions());
            self.events.extend(h.take_events());
        }
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
        self.buffer.hint(self.now_us, Hint::SourceError);
        self.drain_source();
        let evs = self.buffer.drain();
        for e in evs {
            self.events.push_back(e);
        }
    }

    fn drain_source(&mut self) {
        while let Some(item) = self.source.poll_item() {
            let Some(e) = item else {
                self.buffer.on_demux_discontinuity(self.now_us);
                continue;
            };
            match e {
                Event::Finite { .. } => {
                    self.buffer.set_finite();
                    self.events.push_back(e);
                }
                Event::Tracks(specs) => {
                    for spec in &specs {
                        self.buffer.on_track_added(spec);
                    }
                    self.events.push_back(Event::Tracks(specs));
                }
                Event::Sample { track, sample, .. } => {
                    let join = self.buffer.join_sample(track, &sample);
                    self.buffer.on_sample(track, &sample, join);
                    self.events.push_back(Event::Sample {
                        track,
                        sample,
                        join,
                    });
                }
                Event::Discontinuity => {
                    self.buffer.hint(self.now_us, Hint::Reconnected);
                    self.events.push_back(e);
                }
                other => {
                    self.events.push_back(other);
                }
            }
        }
        let evs = self.buffer.drain();
        for e in evs {
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

fn has_playlist_extension(url: &Url) -> bool {
    std::path::Path::new(url.path())
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("m3u8") || e.eq_ignore_ascii_case("m3u"))
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
