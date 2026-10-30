//! Public [`Session`] implementation.

use alloc::collections::{BTreeSet, VecDeque};
use alloc::string::String;
use alloc::vec::Vec;
use std::collections::HashMap;

use percent_encoding::percent_decode_str;
use url::Url;

use crate::Credentials;
use crate::Hint;
use crate::auth::AuthState;
use crate::error::{Error, TransportError};
use crate::events::{Action, Config, ErrorKind, Event, SourceKind, Status};
use crate::headers;
use crate::playout::{Playout, Source};
use crate::queue::EventQueue;
use crate::redirect;
use crate::source::adaptive::Adaptive;
use crate::source::dash::DashSource;
use crate::source::fetch::conn::ConnAlloc;
use crate::source::fetch::wall::WallClock;
use crate::source::hls_session::HlsSession;
use crate::source::rtsp::RtspSource;
use crate::source::ts_http::TsHttpSource;
use crate::source::ts_http_hold::{HoldLimits, StreamKind};
use crate::timeline::JumpDetector;
use transmux::{JoinDecision, Sample, TrackId, TrackSpec};

use tshttp_runtime::Action as TsAction;

const SCHEME_HTTP: &str = "http";
const SCHEME_HTTPS: &str = "https";
const SCHEME_RTSP: &str = "rtsp";
const SCHEME_RTSPS: &str = "rtsps";
const MICROS_PER_MILLI: u64 = 1_000;
const TS_WAKE_TICK_US: u64 = 1_000_000;
const FIRST_CONN_ID: u32 = 1;

const MAX_UNAUTHENTICATED_REPLAYS: u32 = 2;

#[derive(Debug, Clone, Copy, Default)]
struct ConnAuth {
    authed: bool,
    stale_replayed: bool,
    replays: u32,
}

fn replay_allowed(lineage: ConnAuth, replay_is_authenticated: bool) -> bool {
    replay_is_authenticated || lineage.replays < MAX_UNAUTHENTICATED_REPLAYS
}

/// Generic sans-IO client session.
///
/// The host calls [`Session::poll`] until it returns `None` and drains [`Session::next_event`]
/// after every callback (`on_response`, `on_data`, `on_closed`, `on_connected`,
/// `on_playback_position`, `set_credentials`, `reconnect`): actions and events are produced
/// inside the callbacks themselves, not only by `poll`.
pub struct Session {
    closed: bool,
    now_us: u64,
    config: Config,
    auth: AuthState,
    source: TsHttpSource,
    adaptive: Option<Adaptive>,
    rtsp: Option<Box<RtspSource>>,
    redirect_count: u32,
    next_conn: u32,
    live_conn: Option<u32>,
    known_conns: HashMap<u32, Url>,
    conn_auth: HashMap<u32, ConnAuth>,
    ts_wake: Option<u64>,
    actions: VecDeque<Action>,
    events: EventQueue,
    buffer: crate::buffer::Feeder,
    playout: Playout,
    jump: JumpDetector,
    declaration: crate::rendered::Declaration,
    stale_dropped: u64,
    hls_joins: crate::source::hls_join::HlsJoins,
    wall: Option<WallClock>,
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
        let mut parsed = Url::parse(url).map_err(|e| Error::InvalidUrl(e.to_string()))?;
        let userinfo = take_userinfo(&mut parsed)?;
        let scheme = parsed.scheme();
        let is_rtsp_scheme = scheme == SCHEME_RTSP || scheme == SCHEME_RTSPS;
        let is_http_scheme = scheme == SCHEME_HTTP || scheme == SCHEME_HTTPS;
        let rtsp_allowed = matches!(config.kind, SourceKind::Auto | SourceKind::Rtsp);
        let http_allowed = !matches!(config.kind, SourceKind::Rtsp);
        if !((is_rtsp_scheme && rtsp_allowed) || (is_http_scheme && http_allowed)) {
            return Err(Error::UnsupportedScheme(scheme.to_string()));
        }
        if is_rtsp_scheme && parsed.host().is_none() {
            return Err(Error::InvalidUrl("missing host".to_string()));
        }
        let use_hls = !is_rtsp_scheme
            && match config.kind {
                SourceKind::Hls => true,
                SourceKind::Auto => has_playlist_extension(&parsed),
                _ => false,
            };
        let use_dash = !is_rtsp_scheme
            && match config.kind {
                SourceKind::Dash => true,
                SourceKind::Auto => has_mpd_extension(&parsed),
                _ => false,
            };
        headers::validate(&config.extra_headers)?;
        headers::validate_cookies(&config.initial_cookies)?;
        config.validate()?;
        let buffer_cfg = config.buffer;
        let start_pending_wait_us = config.start_pending_track_wait_us;
        let start_pending_hard_cap_us = config.start_pending_track_hard_cap_us;
        let jump = JumpDetector::new(config.timeline_jump);
        let event_cap = config.max_queued_events;
        let mut playout = Playout::new(config.playout);
        let mut auth = AuthState::new(parsed.clone(), config.extra_headers.clone());
        auth.seed_cookies(&config.initial_cookies);
        if let Some(creds) = userinfo {
            auth.set_credentials(creds);
        }
        let source = TsHttpSource::new(parsed.clone(), config.ts_http.clone(), now_us)
            .with_hold_limits(HoldLimits::from_config(&config));
        let adaptive = if use_hls {
            Some(Adaptive::Hls(Box::new(HlsSession::new(&parsed, &config))))
        } else if use_dash {
            Some(Adaptive::Dash(Box::new(DashSource::new(
                &parsed,
                &config,
                ConnAlloc::new(),
            ))))
        } else {
            None
        };
        let mut rtsp = is_rtsp_scheme.then(|| {
            Box::new(RtspSource::new(
                &parsed,
                &config.ts_http.clamped(),
                config.max_redirects,
                now_us,
            ))
        });
        if let Some(r) = rtsp.as_mut() {
            r.on_credentials(now_us, &auth);
        }
        if use_hls || use_dash {
            playout.engage(Source::Live {
                target_duration_secs: None,
            });
        }
        Ok(Self {
            closed: false,
            now_us,
            config,
            auth,
            source,
            adaptive,
            rtsp,
            redirect_count: 0,
            next_conn: FIRST_CONN_ID,
            live_conn: None,
            known_conns: HashMap::new(),
            conn_auth: HashMap::new(),
            ts_wake: None,
            actions: VecDeque::new(),
            events: EventQueue::new(event_cap),
            buffer: crate::buffer::Feeder::new(
                buffer_cfg,
                seed,
                start_pending_wait_us,
                start_pending_hard_cap_us,
            ),
            playout,
            jump,
            declaration: crate::rendered::Declaration::default(),
            stale_dropped: 0,
            hls_joins: crate::source::hls_join::HlsJoins::default(),
            wall: None,
        })
    }

    /// Poll next action.
    pub fn poll(&mut self, now_us: u64) -> Option<Action> {
        if self.closed && self.actions.is_empty() {
            return None;
        }
        if !self.closed {
            self.now_us = now_us;
            if let Some(r) = self.rtsp.as_mut() {
                r.poll(now_us, &self.auth);
                self.flush_rtsp();
            } else if let Some(h) = self.adaptive.as_mut() {
                h.poll(now_us, &mut self.auth);
                self.flush_adaptive();
            } else {
                if !self.auth.is_waiting() {
                    self.source.poll_timeout(now_us);
                }
                self.drain_source();
            }
        }
        while let Some(action) = self.actions.pop_front() {
            if matches!(action, Action::WakeAt(at) if at <= now_us) {
                continue;
            }
            return Some(action);
        }
        None
    }

    /// Poll next event.
    pub fn next_event(&mut self) -> Option<Event> {
        if self.closed && self.events.is_empty() {
            return None;
        }
        if !self.closed && self.adaptive.is_none() && self.rtsp.is_none() {
            self.drain_source();
        }
        self.events.pop_front()
    }

    /// Response callback; `now_us` is the host clock when the response head arrived.
    pub fn on_response(
        &mut self,
        conn: u32,
        status: u16,
        headers: &[(String, String)],
        now_us: u64,
    ) {
        if self.closed || self.rtsp.is_some() {
            return;
        }
        self.now_us = now_us;
        if let Some(h) = self.adaptive.as_mut() {
            h.on_response(
                conn,
                status,
                headers,
                self.now_us,
                self.wall.as_ref(),
                &mut self.auth,
            );
            self.flush_adaptive();
            return;
        }
        let Some(req_url) = self.known_conns.get(&conn).cloned() else {
            return;
        };
        self.auth.receive_cookies(&req_url, headers, self.now_us);
        let lineage = self.conn_auth.get(&conn).copied().unwrap_or_default();

        if status == 301 || status == 302 || status == 303 || status == 307 || status == 308 {
            if let Some(target) = redirect::resolve(
                &req_url,
                headers,
                self.redirect_count,
                self.config.max_redirects,
            ) {
                self.redirect_count = self.redirect_count.saturating_add(1);
                self.known_conns.remove(&conn);
                let lineage = self.conn_auth.remove(&conn).unwrap_or_default();
                if self.live_conn == Some(conn) {
                    self.live_conn = None;
                }
                self.actions.push_back(Action::Close { conn });
                let new_conn = self.alloc_conn(target.clone());
                let new_headers = self.auth.request_headers(&target, self.now_us);
                let authed = has_authorization(&new_headers);
                self.conn_auth.insert(
                    new_conn,
                    ConnAuth {
                        authed,
                        stale_replayed: lineage.stale_replayed,
                        replays: lineage.replays,
                    },
                );
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
            if req_url.origin() != self.auth.origin().origin() {
                self.fail_connect(conn);
                return;
            }
            self.known_conns.remove(&conn);
            self.conn_auth.remove(&conn);
            if self.live_conn == Some(conn) {
                self.live_conn = None;
            }
            self.actions.push_back(Action::Close { conn });

            let decision = self.auth.handle_unauthorized(
                status,
                headers,
                lineage.authed,
                lineage.stale_replayed,
            );
            match decision {
                crate::auth::AuthDecision::Replay | crate::auth::AuthDecision::ReplayStale => {
                    let stale_replayed = lineage.stale_replayed
                        || matches!(decision, crate::auth::AuthDecision::ReplayStale);
                    let new_headers = self.auth.request_headers(&req_url, self.now_us);
                    let authed = has_authorization(&new_headers);
                    if !replay_allowed(lineage, authed) {
                        self.signal_connect_failure();
                        return;
                    }
                    let new_conn = self.alloc_conn(req_url.clone());
                    self.conn_auth.insert(
                        new_conn,
                        ConnAuth {
                            authed,
                            stale_replayed,
                            replays: lineage.replays.saturating_add(1),
                        },
                    );
                    self.actions.push_back(Action::HttpRequest {
                        conn: new_conn,
                        method: "GET".to_string(),
                        url: req_url.to_string(),
                        headers: new_headers,
                        byte_range: None,
                    });
                }
                crate::auth::AuthDecision::Wait(events) => {
                    self.events.extend(events);
                    self.actions.retain(|a| !matches!(a, Action::WakeAt(_)));
                    self.ts_wake = None;
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
        } else {
            self.known_conns.remove(&conn);
            self.conn_auth.remove(&conn);
            if self.live_conn == Some(conn) {
                self.live_conn = None;
            }
            self.actions.push_back(Action::Close { conn });
        }
        self.source.on_response(status, headers, self.now_us);
        self.drain_source();
    }

    /// Data callback.
    pub fn on_data(&mut self, conn: u32, bytes: &[u8], now_us: u64) {
        if self.closed {
            return;
        }
        if let Some(r) = self.rtsp.as_mut() {
            self.now_us = now_us;
            r.on_data(conn, bytes, now_us, &mut self.auth);
            self.flush_rtsp();
            if !bytes.is_empty() {
                self.buffer.on_chunk(now_us, bytes.len(), None);
                let evs = self.buffer.drain();
                for e in evs {
                    self.events.push_back(e);
                }
            }
            return;
        }
        if let Some(h) = self.adaptive.as_mut() {
            self.now_us = now_us;
            h.on_data(conn, bytes, now_us, &mut self.auth);
            self.flush_adaptive();
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
        if let Some(r) = self.rtsp.as_mut() {
            self.now_us = now_us;
            r.on_closed(conn, now_us);
            self.flush_rtsp();
            return;
        }
        if let Some(h) = self.adaptive.as_mut() {
            self.now_us = now_us;
            h.on_closed(conn, error.as_deref(), now_us, &mut self.auth);
            self.flush_adaptive();
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

    /// The host established the TCP connection requested by [`Action::TcpConnect`] (RTSP).
    ///
    /// A failed connect is reported with [`Session::on_closed`] and an error instead.
    pub fn on_connected(&mut self, conn: u32) {
        if self.closed {
            return;
        }
        if let Some(r) = self.rtsp.as_mut() {
            r.on_connected(conn, self.now_us);
            self.flush_rtsp();
        }
    }

    /// RTP packets the RTSP depayloader reported as lost or damaged so far (sequence gaps and
    /// unrecoverable access units); the stream continues past them.
    pub fn rtp_loss_count(&self) -> u64 {
        self.rtsp.as_ref().map_or(0, |r| r.loss_events())
    }

    /// Interleaved RTP frames the RTSP depayloader rejected as malformed so far; each costs only
    /// itself.
    pub fn rtp_malformed_count(&self) -> u64 {
        self.rtsp.as_ref().map_or(0, |r| r.malformed_frames())
    }

    /// Interleaved RTCP frames routed to the RTSP depayloader so far (sender reports etc.).
    pub fn rtcp_packet_count(&self) -> u64 {
        self.rtsp.as_ref().map_or(0, |r| r.rtcp_frames())
    }

    /// Set credentials.
    ///
    /// # Errors
    /// [`Error::InvalidCredentials`] when the token, user name or password cannot be sent in an
    /// HTTP header; the previous credentials stay in force.
    pub fn set_credentials(&mut self, creds: crate::Credentials) -> Result<(), Error> {
        if self.closed {
            return Ok(());
        }
        headers::validate_credentials(&creds)?;
        let was_waiting = self.auth.set_credentials(creds);
        if let Some(r) = self.rtsp.as_mut() {
            r.on_credentials(self.now_us, &self.auth);
            self.flush_rtsp();
            return Ok(());
        }
        if was_waiting {
            if let Some(h) = self.adaptive.as_mut() {
                h.resume(self.now_us, &mut self.auth);
                self.flush_adaptive();
            } else {
                self.source.reconnect(self.now_us);
                self.drain_source();
            }
        }
        Ok(())
    }

    /// Tell the session the UTC time: at the monotonic instant `now_us` (the clock every other
    /// call takes) the host's UTC clock read `unix_us` microseconds since 1970-01-01T00:00:00Z
    /// (POSIX time, no leap seconds). The host reads both clocks back to back, so the pair is as
    /// tight as the host can make it; the session never reads a clock itself.
    ///
    /// Callable at any time and repeatedly (at open, every ten minutes, on a system clock-change
    /// notification or after a device wakes): each call replaces the previous anchor, a call
    /// whose `now_us` is earlier than the last one is accepted as it stands, and no value is
    /// rejected (`unix_us` is a `u64`, so dates past the year 584 000 are representable).
    /// [`Session::open`] performs no I/O, so a host calls this between `open` and the first
    /// [`Session::poll`].
    ///
    /// Sources that do not play DASH ignore the anchor, with one exception: an HTTP-date
    /// `Retry-After` on a `429`/`503` of an HLS request is counted against it (RFC 9110 10.2.3),
    /// and without an anchor against the response's own `Date` header. The anchor is also kept
    /// for the DASH source, which needs a clock only for dynamic MPDs (availability and the live
    /// edge) and ranks this one below a measured `UTCTiming` unless
    /// [`crate::DashConfig::clock_policy`] says otherwise.
    pub fn set_wall_clock(&mut self, now_us: u64, unix_us: u64) {
        self.wall = Some(WallClock::new(now_us, unix_us));
        if self.closed {
            return;
        }
        if let Some(a) = self.adaptive.as_mut() {
            a.set_wall_clock(now_us, unix_us);
            self.flush_adaptive();
        }
    }

    /// Live cookies as `(name, value)` pairs, for the host to persist and pass back as
    /// `Config::initial_cookies`. The values are secrets.
    pub fn cookies(&self) -> Vec<(String, String)> {
        self.auth.cookies(self.now_us)
    }

    /// Caller-initiated reconnect.
    ///
    /// A finite TS source restarts even after [`Status::Ended`]; an HLS session that has reported
    /// `Ended` ignores the call.
    pub fn reconnect(&mut self, now_us: u64) {
        if self.closed {
            return;
        }
        self.now_us = now_us;
        if let Some(r) = self.rtsp.as_mut() {
            r.reconnect(now_us);
            self.flush_rtsp();
            return;
        }
        if let Some(h) = self.adaptive.as_mut() {
            h.reconnect(now_us, &mut self.auth);
            self.flush_adaptive();
            return;
        }
        self.source.reconnect(now_us);
        self.drain_source();
    }

    /// Switch to another variant of the last [`Event::Variants`]: an HLS variant of the master
    /// playlist, or a video Representation of the current DASH Period (the switch takes effect at the
    /// next segment boundary and is kept in later Periods). After [`Status::Ended`] a known index is
    /// accepted and ignored: nothing is started and nothing is emitted.
    ///
    /// # Errors
    /// [`Error::NotHls`] for a session that is neither HLS nor DASH, [`Error::NoSuchVariant`] for an
    /// unknown index.
    pub fn select_variant(&mut self, index: usize) -> Result<(), Error> {
        let Some(h) = self.adaptive.as_mut() else {
            return Err(Error::NotHls);
        };
        if self.closed {
            return Ok(());
        }
        let ended = h.is_ended();
        h.select_variant(index, self.now_us, &mut self.auth)?;
        if !ended && h.variant_switch_restarts_playout() {
            self.playout.reset();
        }
        self.flush_adaptive();
        Ok(())
    }

    /// Select the separate audio rendition, or `None` for no separate audio.
    ///
    /// For HLS `index` addresses the renditions of the last [`Event::Variants`] filtered to the
    /// chosen variant's `AUDIO` group and to audio type, in master-playlist order. A rendition
    /// without a `URI` (carried inside the variant) is selectable but never fetched. For DASH it
    /// addresses every audio AdaptationSet of the current Period in MPD order, `None` stops requesting
    /// audio and a later `Some` starts it again. A manual selection replaces automatic selection
    /// until the session ends. After [`Status::Ended`] the call is accepted and ignored: nothing is
    /// started and nothing more is emitted.
    ///
    /// # Errors
    /// [`Error::NotHls`] for a session that is neither HLS nor DASH, [`Error::NoSuchRendition`] for
    /// an out-of-range index or when no master playlist (or MPD selection) was seen yet.
    pub fn select_audio_rendition(&mut self, index: Option<usize>) -> Result<(), Error> {
        let Some(h) = self.adaptive.as_mut() else {
            return Err(Error::NotHls);
        };
        if self.closed {
            return Ok(());
        }
        h.select_audio(index, self.now_us, &mut self.auth)?;
        self.flush_adaptive();
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
        self.playout.on_position(pts_27mhz);
        self.drain_playout();
    }

    /// Set rendered tracks.
    pub fn set_rendered_tracks(&mut self, tracks: &[TrackId]) {
        if self.closed {
            return;
        }
        if self.adaptive.is_some() || self.rtsp.is_some() {
            self.buffer.set_rendered(Some(tracks));
            self.jump.set_rendered(Some(tracks));
            self.playout.set_rendered(Some(tracks));
            self.drain_playout();
            return;
        }
        let resolution = self.declaration.declare(tracks);
        self.apply_rendered(resolution);
        self.sync_expected();
    }

    fn sync_expected(&mut self) {
        let missing = self.source.unresolved_kinds();
        let absent = missing.iter().any(|kind| match kind {
            StreamKind::Video => !self.buffer.has_video(),
            StreamKind::Audio => !self.buffer.has_audio(),
        });
        let audio_pending = missing.contains(&StreamKind::Audio) && !self.buffer.has_audio();
        let rendered = !self.buffer.rendered_explicit() || self.declaration.has_pending();
        self.buffer
            .set_expecting(self.now_us, absent && rendered, audio_pending);
    }

    fn apply_rendered(&mut self, resolution: crate::rendered::Resolution) {
        if resolution.changed {
            let rendered = resolution.rendered.as_deref();
            self.buffer.set_rendered(rendered);
            self.jump.set_rendered(rendered);
            self.playout.set_rendered(rendered);
        }
        for track in resolution.unknown {
            self.events.push_back(Event::Error {
                kind: ErrorKind::UnknownRenderedTrack {
                    track_id: track.value(),
                },
                fatal: false,
            });
        }
        self.drain_playout();
    }

    /// The default rendered set: the lowest [`TrackId`] of each kind among the tracks announced so
    /// far, `(video, audio)`. It is what the buffer controller judges until
    /// [`set_rendered_tracks`](Self::set_rendered_tracks) is called, and it can change when a
    /// track with a lower id is announced later.
    #[must_use]
    pub fn default_rendered_tracks(&self) -> (Option<TrackId>, Option<TrackId>) {
        self.buffer.default_rendered()
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
        self.hls_joins.rejoin();
        self.playout.rejoin();
        self.jump.clear();
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

    /// Buffer controller statistics (stalls, reprimes, discontinuities, longest freeze).
    #[must_use]
    pub fn buffer_stats(&self) -> crate::events::BufferStats {
        self.buffer.buffer_stats()
    }

    /// Terminal close.
    ///
    /// Queued actions other than [`Action::Close`] are dropped and every live connection gets
    /// exactly one `Close`; an RTSP session with a server-side session id first queues one
    /// [`Action::Send`] carrying `TEARDOWN`. Afterwards `poll` and `next_event` return `None`.
    pub fn close(&mut self) {
        if self.closed {
            return;
        }
        self.closed = true;
        let mut seen: BTreeSet<u32> = BTreeSet::new();
        let mut order: Vec<u32> = Vec::new();
        for action in self.actions.drain(..) {
            if let Action::Close { conn } = action
                && seen.insert(conn)
            {
                order.push(conn);
            }
        }
        let mut live: Vec<u32> = self.known_conns.keys().copied().collect();
        if let Some(h) = self.adaptive.as_ref() {
            live.extend(h.conns());
        }
        live.sort_unstable();
        order.extend(live.into_iter().filter(|c| seen.insert(*c)));
        self.actions
            .extend(order.into_iter().map(|conn| Action::Close { conn }));
        if let Some(r) = self.rtsp.as_mut() {
            r.take_actions();
            r.close();
            self.actions.extend(r.take_actions());
        }
        self.known_conns.clear();
        self.conn_auth.clear();
        self.live_conn = None;
        self.events.clear();
    }

    fn flush_rtsp(&mut self) {
        if let Some(r) = self.rtsp.as_mut() {
            r.sync_wake(self.now_us);
            self.actions.extend(r.take_actions());
            let events = r.take_events();
            for e in events {
                self.screen_passthrough(e);
            }
            let evs = self.buffer.drain();
            for e in evs {
                self.events.push_back(e);
            }
        }
    }

    fn screen_passthrough(&mut self, mut e: Event) {
        match &mut e {
            Event::Tracks(specs) => {
                self.jump.on_tracks(specs);
                self.buffer_tracks(specs);
                let resolution = self.declaration.on_tracks(specs);
                self.apply_rendered(resolution);
            }
            Event::Discontinuity => {
                self.jump.clear();
                self.buffer_reconnected();
            }
            Event::Sample {
                track,
                sample,
                join,
            } => match self.buffer_sample(*track, sample) {
                Some(j) => *join = j,
                None => return,
            },
            _ => {}
        }
        self.events.push_back(e);
    }

    fn buffer_tracks(&mut self, specs: &[TrackSpec]) {
        self.buffer.retain_tracks(specs);
        for spec in specs {
            self.buffer.on_track_added(spec);
        }
    }

    fn buffer_reconnected(&mut self) {
        self.buffer.rejoin();
        self.buffer.hint(self.now_us, Hint::Reconnected);
    }

    fn buffer_sample(&mut self, track: TrackId, sample: &Sample) -> Option<Option<JoinDecision>> {
        if !self.jump.screen(track, sample) {
            self.stale_dropped = self.stale_dropped.saturating_add(1);
            return None;
        }
        let mut join = self.buffer.join_sample(track, sample);
        if self.jump.is_jump(track, sample, join, self.now_us) {
            let handled = self.jump.already_handled();
            self.emit(Event::Discontinuity);
            self.jump.begin_epoch(track, sample);
            if !handled {
                self.buffer.on_demux_discontinuity(self.now_us);
                join = self.buffer.join_sample(track, sample);
            }
        }
        self.jump.record(track, sample, join, self.now_us);
        self.buffer.on_sample(track, sample, join);
        Some(join)
    }

    fn flush_adaptive(&mut self) {
        let Some(h) = self.adaptive.as_mut() else {
            return;
        };
        self.actions.extend(h.take_actions());
        self.playout.set_source(h.playout_source());
        let events = h.take_events();
        for mut e in events {
            match &mut e {
                Event::Tracks(specs) => self.hls_joins.on_tracks(specs),
                Event::Discontinuity => self.hls_joins.rejoin(),
                Event::Sample {
                    track,
                    sample,
                    join,
                } => {
                    if !self.jump.screen(*track, sample) {
                        self.stale_dropped = self.stale_dropped.saturating_add(1);
                        continue;
                    }
                    *join = self.hls_joins.push(*track, sample);
                    if self.jump.is_jump(*track, sample, *join, self.now_us) {
                        self.emit(Event::Discontinuity);
                        self.jump.begin_epoch(*track, sample);
                        self.hls_joins.rejoin();
                        *join = self.hls_joins.push(*track, sample);
                    }
                    self.jump.record(*track, sample, *join, self.now_us);
                }
                _ => {}
            }
            self.emit(e);
        }
    }

    fn emit(&mut self, e: Event) {
        match &e {
            Event::Tracks(specs) => {
                self.jump.on_tracks(specs);
                self.playout.on_tracks(specs);
            }
            Event::Sample {
                track,
                sample,
                join,
            } => self.playout.on_sample(*track, sample, *join),
            Event::Discontinuity => {
                self.jump.clear();
                self.playout.reset();
            }
            Event::Status(Status::Ended) => {
                self.playout.on_ended();
                self.drain_playout();
                self.events.push_back(e);
                return;
            }
            _ => {}
        }
        let announced = match &e {
            Event::Tracks(specs) if self.adaptive.is_none() => Some(specs.clone()),
            _ => None,
        };
        self.events.push_back(e);
        if let Some(specs) = announced {
            let resolution = self.declaration.on_tracks(&specs);
            self.apply_rendered(resolution);
        }
        self.drain_playout();
    }

    fn drain_playout(&mut self) {
        let evs = self.playout.drain();
        self.events.extend(evs);
    }

    fn alloc_conn(&mut self, url: Url) -> u32 {
        let id = loop {
            let candidate = self.next_conn;
            self.next_conn = candidate.checked_add(1).unwrap_or(FIRST_CONN_ID);
            if !self.known_conns.contains_key(&candidate) {
                break candidate;
            }
        };
        self.known_conns.insert(id, url);
        id
    }

    fn fail_connect(&mut self, conn: u32) {
        self.known_conns.remove(&conn);
        self.conn_auth.remove(&conn);
        if self.live_conn == Some(conn) {
            self.live_conn = None;
        }
        self.actions.push_back(Action::Close { conn });
        self.signal_connect_failure();
    }

    fn signal_connect_failure(&mut self) {
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
                self.playout.reset();
                self.jump.note_reset();
                continue;
            };
            match e {
                Event::Finite { .. } => {
                    self.buffer.set_finite();
                    self.playout.engage(Source::Vod);
                    self.emit(e);
                }
                Event::Status(Status::Establishing) => {
                    self.buffer.new_connection();
                    self.playout.disengage();
                    self.emit(e);
                }
                Event::Tracks(specs) => {
                    self.buffer_tracks(&specs);
                    self.emit(Event::Tracks(specs));
                }
                Event::Sample { track, sample, .. } => {
                    let Some(join) = self.buffer_sample(track, &sample) else {
                        continue;
                    };
                    self.emit(Event::Sample {
                        track,
                        sample,
                        join,
                    });
                }
                Event::Discontinuity => {
                    self.buffer_reconnected();
                    self.emit(e);
                }
                other => {
                    self.emit(other);
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
        self.sync_expected();
        self.sync_ts_wake();
    }

    fn sync_ts_wake(&mut self) {
        if self.closed || self.auth.is_waiting() {
            return;
        }
        let Some(deadline) = self.source.next_deadline_us() else {
            return;
        };
        let at = if deadline > self.now_us {
            deadline
        } else {
            self.now_us.saturating_add(TS_WAKE_TICK_US)
        };
        let outstanding = self
            .ts_wake
            .is_some_and(|w| w == at || (w > self.now_us && w < at));
        if !outstanding {
            self.actions.push_back(Action::WakeAt(at));
            self.ts_wake = Some(at);
        }
    }

    fn map_source_action(&mut self, a: TsAction) {
        match a {
            TsAction::Open { .. } => {
                self.redirect_count = 0;
                let target = self.source.url().clone();
                let conn = self.alloc_conn(target.clone());
                let headers = self.auth.request_headers(&target, self.now_us);
                let authed = has_authorization(&headers);
                self.conn_auth.insert(
                    conn,
                    ConnAuth {
                        authed,
                        stale_replayed: false,
                        replays: 0,
                    },
                );
                self.actions.push_back(Action::HttpRequest {
                    conn,
                    method: "GET".to_string(),
                    url: target.to_string(),
                    headers,
                    byte_range: None,
                });
            }
            TsAction::Abort => {
                let mut conns: Vec<u32> = self.known_conns.keys().copied().collect();
                conns.sort_unstable();
                for conn in conns {
                    self.actions.push_back(Action::Close { conn });
                }
                self.known_conns.clear();
                self.conn_auth.clear();
                self.live_conn = None;
            }
            TsAction::Reconnect { after_ms } => {
                self.actions.push_back(Action::WakeAt(
                    self.now_us
                        .saturating_add(after_ms.saturating_mul(MICROS_PER_MILLI)),
                ));
            }
            _ => {}
        }
    }

    /// Samples dropped because they still carried the timeline that a detected jump left: after
    /// [`Event::Discontinuity`] from a jump, every other track delivers nothing until its first
    /// sample lies within [`TimelineJumpConfig::forward_slack_us`](crate::TimelineJumpConfig) of
    /// the new timeline's first sample.
    pub fn stale_samples_dropped(&self) -> u64 {
        self.stale_dropped
    }

    /// Samples dropped because the host did not drain the queue within
    /// [`Config::max_queued_events`]. A dropped sample leaves an [`Event::Discontinuity`] in its
    /// place; coalesced [`Event::Starved`] and transient [`Status`] events are not counted.
    pub fn dropped_events(&self) -> u64 {
        self.events.dropped()
    }
}

fn take_userinfo(url: &mut Url) -> Result<Option<Credentials>, Error> {
    if url.username().is_empty() && url.password().is_none() {
        return Ok(None);
    }
    let decode = |raw: &str| {
        percent_decode_str(raw)
            .decode_utf8()
            .map(|s| s.into_owned())
            .map_err(|_| Error::InvalidUrl("userinfo is not valid UTF-8".to_string()))
    };
    let username = decode(url.username())?;
    let password = decode(url.password().unwrap_or_default())?;
    let _ = url.set_username("");
    let _ = url.set_password(None);
    let creds = Credentials::new(username, password);
    headers::validate_credentials(&creds)?;
    Ok(Some(creds))
}

fn has_authorization(headers: &[(String, String)]) -> bool {
    crate::auth::carries_credentials(headers)
}

fn has_mpd_extension(url: &Url) -> bool {
    std::path::Path::new(url.path())
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("mpd"))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unauthenticated_replay_lineage_is_capped() {
        let fresh = ConnAuth::default();
        assert!(replay_allowed(fresh, false));
        let spent = ConnAuth {
            replays: MAX_UNAUTHENTICATED_REPLAYS,
            ..ConnAuth::default()
        };
        assert!(!replay_allowed(spent, false));
        assert!(
            replay_allowed(spent, true),
            "an authenticated replay is not capped"
        );
    }

    #[test]
    fn alloc_conn_skips_live_ids_when_the_counter_wraps() {
        let mut session = Session::open("http://example/live.ts", Config::default(), 0).unwrap();
        let url = Url::parse("http://example/live.ts").unwrap();
        session.next_conn = u32::MAX;
        let a = session.alloc_conn(url.clone());
        assert_eq!(a, u32::MAX);
        let b = session.alloc_conn(url.clone());
        assert_eq!(b, FIRST_CONN_ID);
        session.next_conn = u32::MAX;
        let c = session.alloc_conn(url);
        assert!(c != a && c != b, "reused live id {c}");
        assert_ne!(c, 0);
    }

    #[test]
    fn a_tracks_event_without_a_track_removes_it_from_the_buffer_feeder() {
        use broadcast_common::Unpackage;
        let data = test_fixtures::read("ts/h264_aac.ts");
        let media = transmux::TsDemux::new().unpackage(&data[..]).unwrap();
        let specs: Vec<transmux::TrackSpec> = media.tracks.into_iter().map(|t| t.spec).collect();
        assert_eq!(specs.len(), 2);
        let mut session = Session::open("http://example/live.ts", Config::default(), 0).unwrap();
        session.source.inject(Some(Event::Tracks(specs.clone())));
        session.drain_source();
        assert!(specs.iter().all(|s| session.buffer.has_track(s.track_id)));
        session
            .source
            .inject(Some(Event::Tracks(specs[1..].to_vec())));
        session.drain_source();
        assert!(!session.buffer.has_track(specs[0].track_id));
        assert!(session.buffer.has_track(specs[1].track_id));
    }

    #[test]
    fn default_rendered_tracks_is_the_lowest_id_whatever_the_resolution_order() {
        use broadcast_common::Unpackage;
        const LOW_AUDIO: u32 = 20;
        const HIGH_AUDIO: u32 = 30;
        let data = test_fixtures::read("ts/h264_aac.ts");
        let media = transmux::TsDemux::new().unpackage(&data[..]).unwrap();
        let video = media
            .tracks
            .iter()
            .find(|t| transmux::LiveJoin::for_config(&t.spec.config).is_some())
            .unwrap()
            .spec
            .clone();
        let audio = media
            .tracks
            .iter()
            .find(|t| t.spec.config.audio_sample_rate().is_some())
            .unwrap()
            .spec
            .clone();
        let with_id = |spec: &transmux::TrackSpec, id: u32| {
            let mut s = spec.clone();
            s.track_id = TrackId::new(id);
            s
        };
        let low = with_id(&audio, LOW_AUDIO);
        let high = with_id(&audio, HIGH_AUDIO);
        let mut session = Session::open("http://example/live.ts", Config::default(), 0).unwrap();
        assert_eq!(session.default_rendered_tracks(), (None, None));
        session
            .source
            .inject(Some(Event::Tracks(vec![high.clone()])));
        session.drain_source();
        assert_eq!(
            session.default_rendered_tracks(),
            (None, Some(high.track_id))
        );
        session.source.inject(Some(Event::Tracks(vec![
            high.clone(),
            video.clone(),
            low.clone(),
        ])));
        session.drain_source();
        assert_eq!(
            session.default_rendered_tracks(),
            (Some(video.track_id), Some(low.track_id))
        );
    }

    #[cfg(feature = "test-seams")]
    fn pcr_hints(session: &Session) -> usize {
        session
            .buffer_input_log()
            .iter()
            .filter(|l| {
                matches!(
                    l,
                    crate::BufferInput::Hint {
                        hint: Hint::PcrDiscontinuity,
                        ..
                    }
                )
            })
            .count()
    }

    #[cfg(feature = "test-seams")]
    #[test]
    fn a_pcr_discontinuity_coinciding_with_a_jump_hints_and_rearms_once() {
        use broadcast_common::Unpackage;
        const TICKS_PER_FRAME: i64 = 3_600;
        const JUMP_TICKS: i64 = 30 * 90_000;
        let data = test_fixtures::read("ts/h264_aac.ts");
        let media = transmux::TsDemux::new().unpackage(&data[..]).unwrap();
        let video = media
            .tracks
            .into_iter()
            .find(|t| transmux::LiveJoin::for_config(&t.spec.config).is_some())
            .unwrap();
        let id = video.spec.track_id;
        let at = |i: usize, pts: i64| {
            let mut s = video.samples[i].clone();
            s.pts = Some(pts);
            s.dts = Some(pts);
            Event::Sample {
                track: id,
                sample: s,
                join: None,
            }
        };
        let mut session = Session::open("http://example/live.ts", Config::default(), 0).unwrap();
        session
            .source
            .inject(Some(Event::Tracks(vec![video.spec.clone()])));
        for i in 0..6 {
            session
                .source
                .inject(Some(at(i, i as i64 * TICKS_PER_FRAME)));
        }
        session.source.inject(None);
        session.source.inject(Some(at(0, 6 * TICKS_PER_FRAME)));
        session
            .source
            .inject(Some(at(1, 7 * TICKS_PER_FRAME + JUMP_TICKS)));
        session.drain_source();
        let mut discontinuities = 0;
        while let Some(e) = session.events.pop_front() {
            if matches!(e, Event::Discontinuity) {
                discontinuities += 1;
            }
        }
        assert_eq!(discontinuities, 1, "the host is told once");
        assert_eq!(
            pcr_hints(&session),
            1,
            "the demuxer already hinted and re-armed; the jump must not do it again"
        );
    }
}
