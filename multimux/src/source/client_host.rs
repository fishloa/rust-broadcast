//! The one async host of a [`broadcast_client::Session`] for every pull input.
//!
//! The session is sans-IO: it names the requests to make and the timers to arm, and the host
//! performs them with `reqwest`/`tokio` and reports the results with the session's own callbacks.
//! [`SessionStep`] is that host, shaped as the per-source half of
//! [`run_ingest_scaffold`](crate::source::driver::run_ingest_scaffold), so cancellation, the read
//! stall clock, route publication and the terminal health mapping stay in one place. What the
//! session emits is fed to an [`EventIngest`], which splits it per programme into the
//! [`SessionEvent`]s the `Trunk` is written from.
//!
//! The session owns the protocol: redirects, cookies, Digest/Bearer auth, resume and request
//! retry. A session that asks to reconnect ends the attempt here, and the route supervisor owns
//! the backoff between attempts.

use std::collections::BTreeMap;
use std::convert::Infallible;
use std::time::Duration;
use url::Url;

use broadcast_client::{
    Action, Config, ConnId, Error, ErrorKind, Event, Session, Status, TransportError,
};
use broadcast_common::{Demand, Stage, Timestamp};
use futures_util::StreamExt;
use media_plane::ingress::{IngestDriver, IngestSession, SessionEvent};
use rustls_platform_verifier::BuilderVerifierExt;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpStream, UdpSocket};
use tokio::sync::mpsc;
use tokio::task::AbortHandle;
use tokio::time::Instant;
use transmux::{TrackId, TrackSpec};

use crate::clock::{HostClock, SystemHostClock};
use crate::error::{ConfigField, MultimuxError, Phase, ProtocolFailure};
use crate::source::driver::{IngestStep, StepOutcome, ts};
use http;
use media_plane::ts_session::ProgramTracker;
use transmux::stitch::Stitcher;

const MESSAGE_QUEUE: usize = 64;

/// The one HTTP client every pull uses: redirects are the session's to follow, and TLS is
/// ring-backed with the platform trust roots (reqwest's own default, minus the aws-lc-rs provider).
pub(crate) fn http_client() -> Result<reqwest::Client, String> {
    let provider = std::sync::Arc::new(rustls::crypto::ring::default_provider());
    let tls = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| e.to_string())?
        .with_platform_verifier()
        .map_err(|e| e.to_string())?
        .with_no_client_auth();
    reqwest::Client::builder()
        .use_preconfigured_tls(tls)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| e.to_string())
}

/// Events handed to the driver between two yields to the runtime, so a large queued burst cannot
/// hold a worker thread (or starve a current-thread runtime) for its whole length.
const EVENT_DRAIN_YIELD_EVERY: usize = 256;

/// Splits the session's track and sample events per programme, with the same programme policy
/// every transport shares ([`ProgramTracker`]).
pub(crate) struct EventIngest {
    tracker: ProgramTracker,
    known: BTreeMap<TrackId, TrackSpec>,
    stitch: Option<Stitcher>,
}

impl EventIngest {
    pub(crate) fn new() -> Self {
        Self {
            tracker: ProgramTracker::new()
                .with_backlog_drop_observer(crate::source::count_ts_backlog_dropped),
            known: BTreeMap::new(),
            stitch: None,
        }
    }

    /// Keeps one continuous sample timeline across the source's discontinuities (a DASH Period
    /// that restarts its clock, a skipped segment): the first sample after one is placed where
    /// the previous content ended. A `Trunk` feeds segmenters that expect time to move forward.
    pub(crate) fn stitching(mut self) -> Self {
        self.stitch = Some(Stitcher::default());
        self
    }

    fn tracks(&mut self, specs: Vec<TrackSpec>) {
        let current: BTreeMap<TrackId, TrackSpec> =
            specs.into_iter().map(|s| (s.track_id, s)).collect();
        let removed: Vec<TrackId> = self
            .known
            .keys()
            .filter(|id| !current.contains_key(id))
            .copied()
            .collect();
        for id in removed {
            self.known.remove(&id);
            self.tracker.track_removed(id);
        }
        for (id, spec) in current {
            match self.known.get(&id) {
                None => self.tracker.track_added(spec.clone()),
                Some(old) if *old != spec => self.tracker.track_updated(spec.clone()),
                Some(_) => continue,
            }
            self.known.insert(id, spec);
        }
        self.tracker.tracks_resolved();
    }
}

impl Stage for EventIngest {
    type In<'a> = Event;
    type Out = SessionEvent;
    type Error = Infallible;

    fn feed(&mut self, input: Event, _now: Timestamp) -> Result<(), Infallible> {
        match input {
            Event::Tracks(specs) => {
                if let Some(stitch) = self.stitch.as_mut() {
                    stitch.set_tracks(&specs);
                }
                self.tracks(specs);
            }
            Event::Sample {
                track, mut sample, ..
            } => {
                if let Some(stitch) = self.stitch.as_mut() {
                    stitch.apply(track, std::slice::from_mut(&mut sample));
                }
                self.tracker.sample(track, sample);
            }
            Event::Discontinuity => {
                if let Some(stitch) = self.stitch.as_mut() {
                    stitch.discontinuity();
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn poll(&mut self) -> Option<SessionEvent> {
        self.tracker.poll()
    }

    fn finish(&mut self) -> Result<(), Infallible> {
        Ok(())
    }

    fn demand(&self) -> Demand {
        Demand::new(1)
    }
}

impl IngestSession for EventIngest {
    type Request = bytes::Bytes;
}

enum Message {
    Response {
        conn: ConnId,
        status: http::StatusCode,
        headers: http::HeaderMap,
    },
    Data {
        conn: ConnId,
        bytes: Vec<u8>,
    },
    Closed {
        conn: ConnId,
        error: Option<String>,
    },
}

/// What a failed or stalled attempt is reported as, chosen by the source.
pub(crate) struct SessionLabels {
    what: &'static str,
    connect: Duration,
    read: Duration,
    not_found_phase: Option<&'static str>,
    session_retries: bool,
}

impl SessionLabels {
    pub(crate) fn new(what: &'static str, timeouts: broadcast_common::timeouts::Timeouts) -> Self {
        Self {
            what,
            connect: timeouts.connect,
            read: timeouts.read_idle,
            not_found_phase: None,
            session_retries: false,
        }
    }

    /// Reports a "not found" answer as the protocol error of `phase` that the route supervisor
    /// treats as permanent after repeated attempts, instead of an ordinary connect failure.
    pub(crate) fn not_found_in_phase(mut self, phase: &'static str) -> Self {
        self.not_found_phase = Some(phase);
        self
    }

    /// Leaves request retries, backoff and reconnects to the session (a segmented source such
    /// as HLS retries each resource itself): a refused request, a not-found answer or a
    /// reconnect does not end the attempt. Only an authentication or permission refusal and a
    /// fatal session error do.
    pub(crate) fn session_retries(mut self) -> Self {
        self.session_retries = true;
        self
    }
}

pub(crate) struct SessionStep {
    session: Session,
    client: reqwest::Client,
    labels: SessionLabels,
    start: Instant,
    clock: std::sync::Arc<dyn HostClock>,
    resync: Option<Duration>,
    next_anchor_us: u64,
    tx: mpsc::Sender<Message>,
    rx: mpsc::Receiver<Message>,
    requests: BTreeMap<ConnId, AbortHandle>,
    sockets: BTreeMap<ConnId, std::sync::Arc<UdpSocket>>,
    writers: BTreeMap<ConnId, BoxedWrite>,
    udp: UdpHost,
    tls: TlsHost,
    wakes: std::collections::BTreeSet<u64>,
    route_handle: std::sync::Arc<crate::route::RouteHandle>,
    feed_si: bool,
    media_seen: bool,
    stalled_seen: bool,
    finite: bool,
    transport_error: Option<String>,
    #[cfg(all(feature = "test-seams", test))]
    injected_events: std::collections::VecDeque<Event>,
    #[cfg(all(feature = "test-seams", test))]
    events_drained: usize,
}

impl SessionStep {
    pub(crate) fn open(
        url: &str,
        mut config: Config,
        credentials: Option<broadcast_client::Credentials>,
        labels: SessionLabels,
        route_handle: std::sync::Arc<crate::route::RouteHandle>,
        feed_si: bool,
    ) -> Result<Self, MultimuxError> {
        config.preemptive_basic = true;
        let start = Instant::now();
        let now = Timestamp::from_micros(0);
        let mut session = Session::open(
            Url::parse(url).map_err(|e| MultimuxError::Connect {
                reason: format!("{}: {e}", labels.what),
            })?,
            config,
            Timestamp::from_micros(0),
        )
        .map_err(|e| match e {
            Error::InvalidConfig(field) => MultimuxError::ConfigInvalid {
                field: ConfigField::RoutesInput,
                reason: format!("{}: {}: {e}", labels.what, field.name()),
            },
            other => MultimuxError::Connect {
                reason: format!("{}: {other}", labels.what),
            },
        })?;
        if let Some(credentials) = credentials {
            session.set_credentials(now, credentials).map_err(|e| {
                MultimuxError::Auth(broadcast_auth::Error::ChallengeParse(
                    broadcast_auth::HttpAuthError(format!("{labels}: {e}", labels = labels.what)),
                ))
            })?;
        }
        let client = http_client().map_err(|e| MultimuxError::Connect {
            reason: format!("reqwest client: {e}"),
        })?;
        let (tx, rx) = mpsc::channel(MESSAGE_QUEUE);
        let mut step = Self {
            session,
            client,
            labels,
            start,
            clock: std::sync::Arc::new(SystemHostClock::new()),
            resync: None,
            next_anchor_us: 0,
            tx,
            rx,
            requests: BTreeMap::new(),
            sockets: BTreeMap::new(),
            writers: BTreeMap::new(),
            udp: UdpHost::default(),
            tls: TlsHost::default(),
            wakes: std::collections::BTreeSet::new(),
            route_handle,
            feed_si,
            media_seen: false,
            stalled_seen: false,
            finite: false,
            transport_error: None,
            #[cfg(all(feature = "test-seams", test))]
            injected_events: std::collections::VecDeque::new(),
            #[cfg(all(feature = "test-seams", test))]
            events_drained: 0,
        };
        step.anchor_wall_clock();
        Ok(step)
    }

    fn now_us(&self) -> u64 {
        self.clock.mono_us()
    }

    /// The host clock as the session's `now`.
    fn now(&self) -> Timestamp {
        Timestamp::from_micros(self.now_us())
    }

    /// Takes the session's instants from `clock` (monotonic microseconds since the source opened,
    /// and the UTC clock the session's wall-clock anchor is read from).
    pub(crate) fn with_clock(mut self, clock: std::sync::Arc<dyn HostClock>) -> Self {
        self.clock = clock;
        self.anchor_wall_clock();
        self
    }

    /// Re-reads the UTC clock into the session every `resync`, so the session's wall-clock anchor
    /// follows the host clock's own corrections.
    pub(crate) fn with_clock_resync(mut self, resync: Option<Duration>) -> Self {
        self.resync = resync;
        self.next_anchor_us = match resync {
            Some(span) => Timestamp::from_micros(self.now_us())
                .checked_add(span)
                .map_or(u64::MAX, Timestamp::as_micros),
            None => u64::MAX,
        };
        self
    }

    fn anchor_wall_clock(&mut self) {
        let now = self.now_us();
        let unix = self.clock.unix_us();
        if let Ok(read) = jiff::Timestamp::from_microsecond(i64::try_from(unix).unwrap_or(i64::MAX))
        {
            self.session
                .set_wall_clock(Timestamp::from_micros(now), read);
        }
        self.next_anchor_us = match self.resync {
            Some(span) => Timestamp::from_micros(now)
                .checked_add(span)
                .map_or(u64::MAX, Timestamp::as_micros),
            None => u64::MAX,
        };
    }

    pub(crate) fn with_udp(mut self, udp: UdpHost) -> Self {
        self.udp = udp;
        self
    }

    pub(crate) fn with_tls(mut self, tls: TlsHost) -> Self {
        self.tls = tls;
        self
    }

    async fn perform_actions(&mut self) -> Result<(), MultimuxError> {
        let now = self.now();
        while let Some(action) = self.session.poll(now) {
            match action {
                Action::HttpRequest {
                    conn,
                    method,
                    url,
                    headers,
                    byte_range,
                    ..
                } => {
                    let request = http_request(&self.client, &method, &url, &headers, byte_range)
                        .map_err(|reason| MultimuxError::Connect {
                        reason: format!("{}: {reason}", self.labels.what),
                    })?;
                    let tx = self.tx.clone();
                    let task = tokio::spawn(fetch(request, conn, tx));
                    self.requests.insert(conn, task.abort_handle());
                }
                Action::UdpBind {
                    conn,
                    addr,
                    options,
                } => {
                    // The route's own socket options are the base when the
                    // session leaves a value unset (the receive-buffer default
                    // and any configured reuse); the session's join is the
                    // route's when the route configured one.
                    let mut merged = self.udp.options.clone();
                    if options.recv_buffer_bytes.is_some() {
                        merged.recv_buffer_bytes = options.recv_buffer_bytes;
                    }
                    merged.reuse_address |= options.reuse_address;
                    merged.reuse_port |= options.reuse_port;
                    if let Some(join) = options.join {
                        merged = merged.with_join(join);
                    }
                    let socket = self.udp.bind(addr, merged).await?;
                    self.register_socket(conn, socket);
                    self.session.on_connected(conn);
                }
                Action::UdpConnect {
                    conn, host, port, ..
                } => match connect_udp(&host.to_string(), port).await {
                    Ok(socket) => {
                        self.register_socket(conn, socket);
                        self.session.on_connected(conn);
                    }
                    Err(e) => {
                        self.transport_error = Some(e.to_string());
                        self.session.on_closed(
                            conn,
                            Some(TransportError::Other {
                                reason: e.to_string(),
                            }),
                            self.now(),
                        );
                    }
                },
                Action::TcpConnect {
                    conn,
                    host,
                    port,
                    tls,
                    ..
                } => match self
                    .tls
                    .connect(&host, port, tls, self.labels.connect)
                    .await
                {
                    Ok((reader, writer)) => {
                        let task = tokio::spawn(read_stream(reader, conn, self.tx.clone()));
                        self.requests.insert(conn, task.abort_handle());
                        self.writers.insert(conn, writer);
                        self.session.on_connected(conn);
                    }
                    Err(e) => self.fail_connection(conn, e),
                },
                Action::Send { conn, bytes } => {
                    if let Some(writer) = self.writers.get_mut(&conn) {
                        if let Err(e) = write_bounded(writer, &bytes).await {
                            self.fail_connection(conn, e);
                        }
                    } else if let Some(socket) = self.sockets.get(&conn)
                        && let Err(e) = socket.send(&bytes).await
                    {
                        self.transport_error = Some(e.to_string());
                    }
                }
                Action::Close { conn } => {
                    self.writers.remove(&conn);
                    self.sockets.remove(&conn);
                    if let Some(task) = self.requests.remove(&conn) {
                        task.abort();
                    }
                }
                Action::WakeAt(at) => {
                    self.wakes.insert(at.as_micros());
                }
                other => {
                    return Err(MultimuxError::Connect {
                        reason: format!(
                            "{}: session asked for an action this host does not perform: {other:?}",
                            self.labels.what
                        ),
                    });
                }
            }
        }
        Ok(())
    }

    fn fail_connection(&mut self, conn: ConnId, error: String) {
        self.transport_error = Some(error.clone());
        self.writers.remove(&conn);
        self.session.on_closed(
            conn,
            Some(TransportError::Other {
                reason: error.clone(),
            }),
            self.now(),
        );
    }

    fn register_socket(&mut self, conn: ConnId, socket: UdpSocket) {
        let socket = std::sync::Arc::new(socket);
        let task = tokio::spawn(receive(
            std::sync::Arc::clone(&socket),
            conn,
            self.tx.clone(),
        ));
        self.requests.insert(conn, task.abort_handle());
        self.sockets.insert(conn, socket);
    }

    fn apply(&mut self, message: Message) -> bool {
        let now = self.now();
        match message {
            Message::Response {
                conn,
                status,
                headers,
            } => {
                self.session.on_response(conn, status, &headers, now);
                false
            }
            Message::Data { conn, bytes } => {
                if self.feed_si {
                    self.route_handle.feed_si_ts(&bytes);
                }
                let before = self.session.payload_bytes();
                self.session.on_data(conn, &bytes, now);
                self.session.payload_bytes() > before
            }
            Message::Closed { conn, error } => {
                self.requests.remove(&conn);
                self.sockets.remove(&conn);
                self.writers.remove(&conn);
                if error.is_some() {
                    self.transport_error.clone_from(&error);
                }
                self.session
                    .on_closed(conn, error.map(TransportError::from), now);
                false
            }
        }
    }

    fn next_event(&mut self) -> Option<Event> {
        #[cfg(all(feature = "test-seams", test))]
        if let Some(event) = self.injected_events.pop_front() {
            return Some(event);
        }
        self.session.next_event()
    }

    #[cfg(all(feature = "test-seams", test))]
    pub(crate) fn inject_events(&mut self, events: impl IntoIterator<Item = Event>) {
        self.injected_events.extend(events);
    }

    async fn drain_events(&mut self, driver: &mut IngestDriver<EventIngest>) -> Verdict {
        let now = ts(self.start, Instant::now());
        let mut verdict = Verdict::Continue;
        let mut drained = 0usize;
        while let Some(event) = self.next_event() {
            drained += 1;
            #[cfg(all(feature = "test-seams", test))]
            {
                self.events_drained = drained;
            }
            if drained.is_multiple_of(EVENT_DRAIN_YIELD_EVERY) {
                tokio::task::yield_now().await;
            }
            match event {
                Event::Tracks(_) | Event::Sample { .. } => {
                    self.media_seen = true;
                    verdict = verdict.max(Verdict::Fed);
                    driver.feed(event, now);
                }
                Event::Discontinuity => driver.feed(event, now),
                Event::Status(Status::Stalled { .. }) => self.stalled_seen = true,
                Event::Status(Status::Reconnecting { .. }) if !self.labels.session_retries => {
                    verdict = verdict.max(self.reconnect_verdict());
                }
                Event::Finite { .. } => self.finite = true,
                Event::Status(Status::Ended) => verdict = verdict.max(Verdict::Ended),
                Event::Status(Status::NotFound) if !self.labels.session_retries => {
                    verdict = verdict.max(Verdict::Failed(match self.labels.not_found_phase {
                        Some(phase) => MultimuxError::Protocol {
                            phase: match phase {
                                "DESCRIBE" => Phase::Describe,
                                "SETUP" => Phase::Setup,
                                "PLAY" => Phase::Play,
                                "recv" => Phase::Recv,
                                other => Phase::Other(other),
                            },
                            kind: ProtocolFailure::Status(http::StatusCode::NOT_FOUND),
                        },
                        None => self.http_error(http::StatusCode::NOT_FOUND),
                    }));
                }
                Event::Forbidden { status } if self.labels.session_retries => {
                    verdict = verdict.max(Verdict::Failed(MultimuxError::Auth(
                        broadcast_auth::Error::ChallengeParse(broadcast_auth::HttpAuthError(
                            format!("{}: HTTP {status}", self.labels.what),
                        )),
                    )));
                }
                Event::ServerRefused { status, .. } if !self.labels.session_retries => {
                    verdict = verdict.max(Verdict::Failed(self.http_error(status)));
                }
                Event::Forbidden { status } | Event::AuthRejected { status } => {
                    verdict = verdict.max(Verdict::Failed(self.http_error(status)));
                }
                Event::AuthRequired { .. } => {
                    verdict = verdict.max(Verdict::Failed(MultimuxError::Auth(
                        broadcast_auth::Error::ChallengeParse(broadcast_auth::HttpAuthError(
                            format!("{}: authentication required", self.labels.what),
                        )),
                    )));
                }
                Event::Error {
                    kind: ErrorKind::RequestRefused { method, status },
                    fatal: false,
                } => {
                    verdict = verdict.max(Verdict::Failed(MultimuxError::Protocol {
                        phase: match method {
                            "DESCRIBE" => Phase::Describe,
                            "SETUP" => Phase::Setup,
                            "PLAY" => Phase::Play,
                            "recv" => Phase::Recv,
                            other => Phase::Other(other),
                        },
                        kind: ProtocolFailure::Other {
                            reason: format!("non-success status {status}"),
                        },
                    }));
                }
                Event::Error { kind, fatal: false } if kind.ends_connection() => {
                    verdict = verdict.max(Verdict::Failed(MultimuxError::Connect {
                        reason: format!("{}: {}", self.labels.what, kind.name()),
                    }));
                }
                Event::Error { kind, fatal: false }
                    if self.labels.session_retries
                        && matches!(
                            kind.refusal_status(),
                            Some(http::StatusCode::UNAUTHORIZED | http::StatusCode::FORBIDDEN)
                        ) =>
                {
                    verdict = verdict.max(Verdict::Failed(MultimuxError::Auth(
                        broadcast_auth::Error::ChallengeParse(broadcast_auth::HttpAuthError(
                            format!("{}: {}", self.labels.what, kind.name()),
                        )),
                    )));
                }
                Event::Error { kind, fatal: false }
                    if self.labels.session_retries && kind.is_permanent() =>
                {
                    verdict = verdict.max(Verdict::Failed(MultimuxError::Session {
                        what: self.labels.what,
                        kind,
                    }));
                }
                Event::Error { kind, fatal: true } => {
                    verdict = verdict.max(Verdict::Failed(MultimuxError::Session {
                        what: self.labels.what,
                        kind,
                    }));
                }
                _ => {}
            }
        }
        verdict
    }

    fn http_error(&self, status: http::StatusCode) -> MultimuxError {
        if status == http::StatusCode::UNAUTHORIZED {
            MultimuxError::Auth(broadcast_auth::Error::ChallengeParse(
                broadcast_auth::HttpAuthError(format!("{}: {status}", self.labels.what)),
            ))
        } else {
            MultimuxError::Connect {
                reason: format!("{}: HTTP {status}", self.labels.what),
            }
        }
    }

    /// What a session's request to reconnect means for this attempt: a live body that simply
    /// ends is a clean end, everything else is a failed attempt.
    fn reconnect_verdict(&self) -> Verdict {
        let failed = |reason: String| Verdict::Failed(MultimuxError::Connect { reason });
        let what = self.labels.what;
        if let Some(error) = &self.transport_error {
            failed(format!("{what} stream read: {error}"))
        } else if !self.media_seen {
            failed(format!(
                "{what} handshake timed out: no media within {:?}",
                self.labels.connect
            ))
        } else if self.stalled_seen {
            failed(format!(
                "{what} stream read: no data within {:?}",
                self.labels.read
            ))
        } else if self.finite {
            failed(format!("{what} stream read: truncated transfer"))
        } else {
            Verdict::Ended
        }
    }
}

impl Drop for SessionStep {
    fn drop(&mut self) {
        for task in self.requests.values() {
            task.abort();
        }
    }
}

enum Verdict {
    Continue,
    Fed,
    Ended,
    Failed(MultimuxError),
}

impl Verdict {
    fn rank(&self) -> u8 {
        match self {
            Verdict::Continue => 0,
            Verdict::Fed => 1,
            Verdict::Ended => 2,
            Verdict::Failed(_) => 3,
        }
    }

    fn max(self, other: Verdict) -> Verdict {
        if other.rank() > self.rank() {
            other
        } else {
            self
        }
    }
}

impl IngestStep<EventIngest> for SessionStep {
    async fn step(
        &mut self,
        driver: &mut IngestDriver<EventIngest>,
        window: Duration,
    ) -> StepOutcome {
        if self.resync.is_some() && self.now_us() >= self.next_anchor_us {
            self.anchor_wall_clock();
        }
        if let Err(e) = self.perform_actions().await {
            return StepOutcome::Failed(e);
        }
        let mut received = false;
        let mut verdict = self.drain_events(driver).await;
        if matches!(verdict, Verdict::Continue) {
            let now = self.now_us();
            while self.wakes.first().is_some_and(|at| *at <= now) {
                self.wakes.pop_first();
            }
            let until_wake = self.wakes.first().map_or(window, |at| {
                self.clock
                    .real_duration(Duration::from_micros(at.saturating_sub(now)))
            });
            if let Ok(Some(message)) =
                tokio::time::timeout(window.min(until_wake), self.rx.recv()).await
            {
                received = self.apply(message);
            }
            if let Err(e) = self.perform_actions().await {
                return StepOutcome::Failed(e);
            }
            verdict = self.drain_events(driver).await;
        }
        match verdict {
            Verdict::Failed(e) => StepOutcome::Failed(e),
            Verdict::Ended => StepOutcome::Eof,
            Verdict::Fed => StepOutcome::Received,
            Verdict::Continue if received => StepOutcome::Received,
            Verdict::Continue => StepOutcome::Idle,
        }
    }

    fn stalled(&self, read: Duration) -> MultimuxError {
        MultimuxError::Connect {
            reason: format!("{} stream read: no data within {read:?}", self.labels.what),
        }
    }

    async fn farewell(&mut self) {
        self.session.close(self.now());
        if let Err(e) = self.perform_actions().await {
            tracing::debug!(error = %e, "{}: farewell not delivered", self.labels.what);
        }
    }
}

fn http_request(
    client: &reqwest::Client,
    method: &http::Method,
    url: &url::Url,
    headers: &http::HeaderMap,
    byte_range: Option<broadcast_common::range::ByteSpan>,
) -> Result<reqwest::RequestBuilder, String> {
    let method =
        reqwest::Method::from_bytes(method.as_str().as_bytes()).map_err(|e| e.to_string())?;
    let mut request = client.request(method, url.clone());
    for (name, value) in headers {
        request = request.header(name, value);
    }
    if let Some(span) = byte_range {
        request = request.header(reqwest::header::RANGE, span.range_header());
    }
    Ok(request)
}

async fn fetch(request: reqwest::RequestBuilder, conn: ConnId, tx: mpsc::Sender<Message>) {
    let closed = |error: Option<String>| Message::Closed { conn, error };
    let response = match request.send().await {
        Ok(response) => response,
        Err(e) => {
            let _ = tx.send(closed(Some(e.to_string()))).await;
            return;
        }
    };
    let head = Message::Response {
        conn,
        status: response.status(),
        headers: response.headers().clone(),
    };
    if tx.send(head).await.is_err() {
        return;
    }
    let mut body = response.bytes_stream();
    while let Some(chunk) = body.next().await {
        let message = match chunk {
            Ok(bytes) => Message::Data {
                conn,
                bytes: bytes.to_vec(),
            },
            Err(e) => {
                let _ = tx.send(closed(Some(e.to_string()))).await;
                return;
            }
        };
        if tx.send(message).await.is_err() {
            return;
        }
    }
    let _ = tx.send(closed(None)).await;
}

/// How the host obtains the datagram socket a session asks for with [`Action::UdpBind`]: a
/// socket the caller already bound is used once, every other bind follows the route's options.
#[derive(Default)]
pub(crate) struct UdpHost {
    pub(crate) prebound: Option<UdpSocket>,
    pub(crate) options: broadcast_common::net::UdpOptions,
}

impl UdpHost {
    async fn bind(
        &mut self,
        addr: std::net::SocketAddr,
        options: broadcast_common::net::UdpOptions,
    ) -> Result<UdpSocket, MultimuxError> {
        if let Some(socket) = self.prebound.take() {
            return Ok(socket);
        }
        // A multicast bind joins the group named in `options.join` from the
        // unspecified address of the group's family; a unicast bind is the
        // literal `addr` itself.
        let bind_addr = match &options.join {
            Some(_) => {
                let any = if addr.is_ipv6() {
                    std::net::Ipv6Addr::UNSPECIFIED.into()
                } else {
                    std::net::Ipv4Addr::UNSPECIFIED.into()
                };
                std::net::SocketAddr::new(any, addr.port())
            }
            None => addr,
        };
        crate::source::udp::bind_udp(&bind_addr.to_string(), None, options).await
    }
}

async fn receive(socket: std::sync::Arc<UdpSocket>, conn: ConnId, tx: mpsc::Sender<Message>) {
    let mut buf = vec![0u8; media_plane::ts_session::MAX_TS_READ];
    loop {
        let message = match socket.recv(&mut buf).await {
            Ok(n) => Message::Data {
                conn,
                bytes: buf[..n].to_vec(),
            },
            Err(e) => {
                let _ = tx
                    .send(Message::Closed {
                        conn,
                        error: Some(e.to_string()),
                    })
                    .await;
                return;
            }
        };
        if tx.send(message).await.is_err() {
            return;
        }
    }
}

async fn connect_udp(host: &str, port: u16) -> std::io::Result<UdpSocket> {
    let peer = tokio::net::lookup_host((host, port))
        .await?
        .next()
        .ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotFound, format!("{host}: no address"))
        })?;
    let local: std::net::SocketAddr = if peer.is_ipv6() {
        (std::net::Ipv6Addr::UNSPECIFIED, 0).into()
    } else {
        (std::net::Ipv4Addr::UNSPECIFIED, 0).into()
    };
    let socket = UdpSocket::bind(local).await?;
    socket.connect(peer).await?;
    Ok(socket)
}

type BoxedRead = Box<dyn AsyncRead + Send + Unpin>;
type BoxedWrite = Box<dyn AsyncWrite + Send + Unpin>;

const TCP_READ_BUFFER: usize = 64 * 1024;

async fn write_bounded(writer: &mut BoxedWrite, bytes: &[u8]) -> Result<(), String> {
    let write_timeout = crate::source::driver::DEFAULT_WRITE_TIMEOUT;
    match tokio::time::timeout(write_timeout, writer.write_all(bytes)).await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) => Err(format!("write: {e}")),
        Err(_) => Err(format!(
            "write: one protocol message not accepted within {write_timeout:?}"
        )),
    }
}

async fn read_stream(mut reader: BoxedRead, conn: ConnId, tx: mpsc::Sender<Message>) {
    let mut buf = vec![0u8; TCP_READ_BUFFER];
    loop {
        let message = match reader.read(&mut buf).await {
            Ok(0) => Message::Closed { conn, error: None },
            Ok(n) => Message::Data {
                conn,
                bytes: buf[..n].to_vec(),
            },
            Err(e) => Message::Closed {
                conn,
                error: Some(e.to_string()),
            },
        };
        let end = matches!(message, Message::Closed { .. });
        if tx.send(message).await.is_err() || end {
            return;
        }
    }
}

/// How the host opens the TCP connection a session asks for with [`Action::TcpConnect`]: plain,
/// or TLS verified against `config` (the public-CA bundle when none is given).
#[derive(Default)]
pub(crate) struct TlsHost {
    #[cfg(feature = "tls")]
    pub(crate) config: Option<rustls::ClientConfig>,
}

impl TlsHost {
    async fn connect(
        &self,
        host: &str,
        port: u16,
        tls: bool,
        connect_timeout: Duration,
    ) -> Result<(BoxedRead, BoxedWrite), String> {
        let addr = if host.contains(':') {
            format!("[{host}]:{port}")
        } else {
            format!("{host}:{port}")
        };
        let opened = async {
            let tcp = TcpStream::connect(&addr)
                .await
                .map_err(|e| format!("connect {addr}: {e}"))?;
            if tls {
                self.handshake(host, &addr, tcp).await
            } else {
                let (reader, writer) = tcp.into_split();
                Ok((
                    Box::new(reader) as BoxedRead,
                    Box::new(writer) as BoxedWrite,
                ))
            }
        };
        tokio::time::timeout(connect_timeout, opened)
            .await
            .map_err(|_| format!("connect {addr}: no response within {connect_timeout:?}"))?
    }

    #[cfg(feature = "tls")]
    async fn handshake(
        &self,
        host: &str,
        addr: &str,
        tcp: TcpStream,
    ) -> Result<(BoxedRead, BoxedWrite), String> {
        let config = match self.config.clone() {
            Some(config) => config,
            None => rtsp_runtime::io::default_tls_client_config()
                .map_err(|e| format!("TLS client configuration: {e}"))?,
        };
        let connector = tokio_rustls::TlsConnector::from(std::sync::Arc::new(config));
        let name = rustls::pki_types::ServerName::try_from(host.to_string())
            .map_err(|e| format!("invalid TLS server name {host:?}: {e}"))?;
        let stream = connector
            .connect(name, tcp)
            .await
            .map_err(|e| format!("TLS handshake {addr}: {e}"))?;
        let (reader, writer) = tokio::io::split(stream);
        Ok((Box::new(reader), Box::new(writer)))
    }

    #[cfg(not(feature = "tls"))]
    async fn handshake(
        &self,
        _host: &str,
        addr: &str,
        _tcp: TcpStream,
    ) -> Result<(BoxedRead, BoxedWrite), String> {
        Err(format!(
            "TLS requires multimux's `tls` feature; cannot connect to {addr}"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::route::RouteConfig;

    #[test]
    fn a_config_the_session_refuses_is_the_route_config_error_with_its_field() {
        let mut config = Config::relay();
        config.kind = broadcast_client::SourceKind::Udp;
        config.udp.reorder_packets = 0;
        let labels = SessionLabels::new("ts/udp", broadcast_common::timeouts::Timeouts::default());
        let route_handle = std::sync::Arc::new(crate::route::RouteHandle::new(
            RouteConfig::default()
                .with_target_duration(std::time::Duration::from_secs(1))
                .with_part_target(Duration::from_millis(250))
                .with_window_segments(8),
        ));
        let error = SessionStep::open(
            "udp://127.0.0.1:5000",
            config,
            None,
            labels,
            route_handle,
            false,
        )
        .err()
        .expect("the session refuses the config");
        assert!(
            matches!(
                &error,
                MultimuxError::ConfigInvalid { field, .. } if *field == ConfigField::RoutesInput
            ),
            "{error:?}"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_large_queued_event_burst_does_not_starve_another_task() {
        use broadcast_client::Action;
        use media_plane::ts_session::test_support::{build_ts_bytes, handshake};
        use transmux::TrackId;

        let ring = |n: usize| std::num::NonZeroUsize::new(n).expect("non-zero ring");
        let labels = SessionLabels::new("ts/http", broadcast_common::timeouts::Timeouts::default());
        let route_handle = std::sync::Arc::new(crate::route::RouteHandle::new(
            RouteConfig::default()
                .with_target_duration(std::time::Duration::from_secs(1))
                .with_part_target(Duration::from_millis(250))
                .with_window_segments(8),
        ));
        let mut step = SessionStep::open(
            "http://127.0.0.1:1/stream.ts",
            Config::relay(),
            None,
            labels,
            route_handle,
            false,
        )
        .expect("open");
        let conn = loop {
            match step.session.poll(Timestamp::from_micros(0)) {
                Some(Action::HttpRequest { conn, .. }) => break conn,
                Some(_) => {}
                None => panic!("the session issued no request"),
            }
        };
        let frames = 4 * EVENT_DRAIN_YIELD_EVERY as u32;
        step.session.on_response(
            conn,
            http::StatusCode::OK,
            &http::HeaderMap::new(),
            Timestamp::from_micros(1),
        );
        step.session.on_data(
            conn,
            &build_ts_bytes(TrackId::new(1), 0xAB, frames),
            Timestamp::from_micros(2),
        );
        let mut driver = IngestDriver::new(
            EventIngest::new(),
            media_plane::IngestConfig::new(handshake()).with_trunk(
                media_plane::trunk::TrunkConfig::default()
                    .with_timed_capacity(ring(8192))
                    .with_sparse_capacity(ring(64))
                    .with_segment_capacity(ring(8))
                    .with_event_capacity(ring(8))
                    .with_part_capacity(ring(8)),
            ),
        );
        let ran = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let flag = std::sync::Arc::clone(&ran);
        tokio::spawn(async move { flag.store(true, std::sync::atomic::Ordering::SeqCst) });
        let verdict = step.drain_events(&mut driver).await;
        assert!(matches!(verdict, Verdict::Fed));
        assert!(
            ran.load(std::sync::atomic::Ordering::SeqCst),
            "draining a large burst must yield to the runtime"
        );
    }

    fn events(ingest: &mut EventIngest) -> Vec<SessionEvent> {
        std::iter::from_fn(|| Stage::poll(ingest)).collect()
    }

    fn spec(track: u32, changed: bool) -> TrackSpec {
        let mut spec = media_plane::ts_session::test_support::track_spec(TrackId::new(track));
        if changed {
            spec.es_info_descriptors = vec![0x00];
        }
        spec
    }

    #[test]
    fn a_track_set_change_is_reported_once_and_a_removed_track_dropped() {
        let mut ingest = EventIngest::new();
        ingest
            .feed(
                Event::Tracks(vec![spec(1, false), spec(2, false)]),
                Timestamp::ZERO,
            )
            .expect("feed");
        let first = events(&mut ingest);
        assert_eq!(first.len(), 2, "Established then NewProgram");
        assert!(matches!(&first[0], SessionEvent::Established));
        assert!(matches!(
            &first[1],
            SessionEvent::NewProgram { tracks, .. } if tracks.len() == 2
        ));

        ingest
            .feed(Event::Tracks(vec![spec(1, false)]), Timestamp::ZERO)
            .expect("feed");
        let removed = events(&mut ingest);
        assert_eq!(removed.len(), 1, "a removed track changes the set");
        match &removed[0] {
            SessionEvent::TracksChanged { tracks, .. } => {
                assert_eq!(tracks.len(), 1, "only track 1 remains");
                assert_eq!(tracks[0].track_id, TrackId::new(1));
            }
            other => panic!("expected TracksChanged, got {other:?}"),
        }

        ingest
            .feed(Event::Tracks(vec![spec(1, false)]), Timestamp::ZERO)
            .expect("feed");
        assert!(
            events(&mut ingest).is_empty(),
            "an unchanged track set is not a change"
        );

        ingest
            .feed(Event::Tracks(vec![spec(1, true)]), Timestamp::ZERO)
            .expect("feed");
        let updated = events(&mut ingest);
        assert_eq!(updated.len(), 1, "a changed spec is announced once");
        match &updated[0] {
            SessionEvent::TracksChanged { tracks, .. } => {
                assert_eq!(tracks.len(), 1);
                assert_eq!(tracks[0].es_info_descriptors, vec![0x00]);
            }
            other => panic!("expected TracksChanged, got {other:?}"),
        }
    }

    #[test]
    fn event_ingest_demands_another_event() {
        let ingest = EventIngest::new();
        assert_eq!(Stage::demand(&ingest), Demand::new(1));
    }

    fn test_route() -> std::sync::Arc<crate::route::RouteHandle> {
        std::sync::Arc::new(crate::route::RouteHandle::new(
            RouteConfig::default()
                .with_target_duration(std::time::Duration::from_secs(1))
                .with_part_target(Duration::from_millis(250))
                .with_window_segments(8),
        ))
    }

    fn test_driver() -> IngestDriver<EventIngest> {
        IngestDriver::new(
            EventIngest::new(),
            media_plane::IngestConfig::new(media_plane::ts_session::test_support::handshake())
                .with_trunk(media_plane::trunk::TrunkConfig::default()),
        )
    }

    fn open_step(retries: bool) -> SessionStep {
        let mut labels = SessionLabels::new("test", broadcast_common::timeouts::Timeouts::default());
        if retries {
            labels = labels.session_retries();
        }
        SessionStep::open(
            "http://127.0.0.1:1/test.ts",
            Config::relay(),
            None,
            labels,
            test_route(),
            false,
        )
        .expect("open")
    }

    fn connect_reason(verdict: &Verdict) -> Option<&str> {
        match verdict {
            Verdict::Failed(MultimuxError::Connect { reason }) => Some(reason),
            _ => None,
        }
    }

    #[cfg(feature = "test-seams")]
    #[tokio::test]
    async fn a_truncated_finite_body_ending_the_connection_is_a_failure() {
        let mut step = open_step(false);
        let mut driver = test_driver();
        step.inject_events([
            Event::Tracks(vec![spec(1, false)]),
            Event::Finite { expected_bytes: 10 },
            Event::Status(Status::Reconnecting { attempt: 1 }),
        ]);
        let verdict = step.drain_events(&mut driver).await;
        assert!(
            connect_reason(&verdict).is_some_and(|r| r.contains("truncated transfer")),
            "a finite body that ended early is a truncated transfer"
        );
    }

    #[cfg(feature = "test-seams")]
    #[tokio::test]
    async fn a_reconnect_is_terminal_without_session_retries_and_retried_with_them() {
        let mut driver = test_driver();
        let mut plain = open_step(false);
        plain.inject_events([
            Event::Tracks(vec![spec(1, false)]),
            Event::Status(Status::Stalled {
                stalled: Duration::from_millis(1),
            }),
            Event::Status(Status::Reconnecting { attempt: 1 }),
        ]);
        assert!(
            connect_reason(&plain.drain_events(&mut driver).await).is_some(),
            "a route that owns reconnection fails the attempt"
        );

        let mut driver = test_driver();
        let mut retrying = open_step(true);
        retrying.inject_events([
            Event::Tracks(vec![spec(1, false)]),
            Event::Status(Status::Reconnecting { attempt: 1 }),
        ]);
        assert!(
            !matches!(retrying.drain_events(&mut driver).await, Verdict::Failed(_)),
            "a segmented source retries the reconnect itself"
        );
    }

    #[cfg(feature = "test-seams")]
    #[tokio::test]
    async fn a_not_found_answer_is_terminal_without_session_retries_only() {
        let mut driver = test_driver();
        let mut plain = open_step(false);
        plain.inject_events([Event::Status(Status::NotFound)]);
        assert!(matches!(
            plain.drain_events(&mut driver).await,
            Verdict::Failed(_)
        ));

        let mut driver = test_driver();
        let mut retrying = open_step(true);
        retrying.inject_events([Event::Status(Status::NotFound)]);
        assert!(
            !matches!(retrying.drain_events(&mut driver).await, Verdict::Failed(_)),
            "a segmented source treats a missing resource as its own retry"
        );
    }

    #[cfg(feature = "test-seams")]
    #[tokio::test]
    async fn a_forbidden_answer_is_auth_with_retries_and_connect_without() {
        let mut driver = test_driver();
        let mut retrying = open_step(true);
        retrying.inject_events([Event::Forbidden {
            status: http::StatusCode::FORBIDDEN,
        }]);
        assert!(matches!(
            retrying.drain_events(&mut driver).await,
            Verdict::Failed(MultimuxError::Auth(_))
        ));

        let mut driver = test_driver();
        let mut plain = open_step(false);
        plain.inject_events([Event::Forbidden {
            status: http::StatusCode::FORBIDDEN,
        }]);
        assert!(matches!(
            plain.drain_events(&mut driver).await,
            Verdict::Failed(MultimuxError::Connect { .. })
        ));
    }

    #[cfg(feature = "test-seams")]
    #[tokio::test]
    async fn a_server_refusal_is_terminal_without_session_retries_only() {
        let mut driver = test_driver();
        let mut plain = open_step(false);
        plain.inject_events([Event::ServerRefused {
            status: http::StatusCode::SERVICE_UNAVAILABLE,
            retry_after: None,
            reason: None,
        }]);
        assert!(matches!(
            plain.drain_events(&mut driver).await,
            Verdict::Failed(_)
        ));

        let mut driver = test_driver();
        let mut retrying = open_step(true);
        retrying.inject_events([Event::ServerRefused {
            status: http::StatusCode::SERVICE_UNAVAILABLE,
            retry_after: None,
            reason: None,
        }]);
        assert!(
            !matches!(retrying.drain_events(&mut driver).await, Verdict::Failed(_)),
            "the session applies the retry delay itself"
        );
    }

    #[cfg(feature = "test-seams")]
    #[tokio::test]
    async fn a_connection_ending_error_fails_the_attempt_and_a_recoverable_one_does_not() {
        let mut driver = test_driver();
        let mut plain = open_step(false);
        plain.inject_events([Event::Error {
            kind: ErrorKind::SrtPeerIdle,
            fatal: false,
        }]);
        assert!(
            connect_reason(&plain.drain_events(&mut driver).await).is_some(),
            "an error that ended the connection is a failed attempt"
        );

        let mut driver = test_driver();
        let mut plain = open_step(false);
        plain.inject_events([Event::Error {
            kind: ErrorKind::BadKey,
            fatal: false,
        }]);
        assert!(
            !matches!(plain.drain_events(&mut driver).await, Verdict::Failed(_)),
            "a recoverable error does not end the attempt"
        );
    }

    #[cfg(feature = "test-seams")]
    #[tokio::test]
    async fn a_permanent_error_fails_a_retrying_attempt_and_a_recoverable_one_does_not() {
        let mut driver = test_driver();
        let mut retrying = open_step(true);
        retrying.inject_events([Event::Error {
            kind: ErrorKind::DecryptFailed,
            fatal: false,
        }]);
        assert!(matches!(
            retrying.drain_events(&mut driver).await,
            Verdict::Failed(MultimuxError::Session { .. })
        ));

        let mut driver = test_driver();
        let mut retrying = open_step(true);
        retrying.inject_events([Event::Error {
            kind: ErrorKind::BadKey,
            fatal: false,
        }]);
        assert!(
            !matches!(retrying.drain_events(&mut driver).await, Verdict::Failed(_)),
            "a retryable error is left to the session"
        );

        let mut driver = test_driver();
        let mut plain = open_step(false);
        plain.inject_events([Event::Error {
            kind: ErrorKind::DecryptFailed,
            fatal: false,
        }]);
        assert!(
            !matches!(plain.drain_events(&mut driver).await, Verdict::Failed(_)),
            "a route that owns reconnection does not pre-empt the session's permanent classification"
        );
    }

    #[cfg(feature = "test-seams")]
    #[tokio::test]
    async fn the_drain_counter_counts_every_event() {
        let mut step = open_step(false);
        let mut driver = test_driver();
        step.inject_events([
            Event::Status(Status::Stalled {
                stalled: Duration::from_millis(1),
            }),
            Event::Status(Status::Ended),
            Event::Discontinuity,
        ]);
        let _ = step.drain_events(&mut driver).await;
        assert_eq!(step.events_drained, 3);
    }

    #[tokio::test]
    async fn dropping_the_step_aborts_outstanding_tasks() {
        let mut step = open_step(false);
        let task = tokio::spawn(std::future::pending::<()>());
        step.requests
            .insert(ConnId::new(9), task.abort_handle());
        drop(step);
        let joined = tokio::time::timeout(Duration::from_secs(5), task).await;
        assert!(
            matches!(joined, Ok(Err(e)) if e.is_cancelled()),
            "dropping the host must abort the connection tasks it owns"
        );
    }

    #[cfg(feature = "test-seams")]
    #[tokio::test]
    async fn a_failed_connection_records_the_transport_error_the_reconnect_reports() {
        let mut step = open_step(false);
        step.fail_connection(ConnId::new(1), "boom".to_string());
        let mut driver = test_driver();
        step.inject_events([
            Event::Tracks(vec![spec(1, false)]),
            Event::Status(Status::Reconnecting { attempt: 1 }),
        ]);
        let verdict = step.drain_events(&mut driver).await;
        assert!(
            connect_reason(&verdict).is_some_and(|r| r.contains("boom")),
            "the recorded transport error is the reconnect's reason"
        );
    }

    #[tokio::test]
    async fn apply_reports_whether_the_payload_advanced() {
        let mut step = open_step(false);
        let conn = loop {
            match step.session.poll(Timestamp::ZERO) {
                Some(Action::HttpRequest { conn, .. }) => break conn,
                Some(_) => {}
                None => panic!("the session issued no request"),
            }
        };
        assert!(
            !step.apply(Message::Response {
                conn,
                status: http::StatusCode::OK,
                headers: http::HeaderMap::new(),
            }),
            "a response head is not payload"
        );
        assert!(
            !step.apply(Message::Data {
                conn: ConnId::new(4242),
                bytes: vec![0x47; 10],
            }),
            "data for a connection the session does not know advances nothing"
        );
        assert!(
            step.apply(Message::Data {
                conn,
                bytes: vec![0x47; 10],
            }),
            "accepted body bytes advance the payload counter"
        );
    }

    #[tokio::test]
    async fn the_session_socket_options_are_ored_onto_the_route_options() {
        let mut config = Config::relay();
        config.kind = broadcast_client::SourceKind::Udp;
        config.udp.options.reuse_address = false;
        config.udp.options.reuse_port = false;
        let step = SessionStep::open(
            "udp://127.0.0.1:58000",
            config,
            None,
            SessionLabels::new("udp", broadcast_common::timeouts::Timeouts::default()),
            test_route(),
            false,
        )
        .expect("open");
        let mut step = step.with_udp(UdpHost {
            prebound: None,
            options: broadcast_common::net::UdpOptions::new()
                .with_reuse_address(true)
                .with_reuse_port(true),
        });
        step.perform_actions().await.expect("bind");
        let socket = step.sockets.values().next().expect("a bound socket");
        let sock = socket2::SockRef::from(socket.as_ref());
        assert!(
            sock.reuse_address().expect("reuse_address"),
            "the route's SO_REUSEADDR must survive a session that left it unset"
        );
        assert!(
            sock.reuse_port().expect("reuse_port"),
            "the route's SO_REUSEPORT must survive a session that left it unset"
        );
    }

    struct ManualClock(std::sync::atomic::AtomicU64);

    impl ManualClock {
        fn new(us: u64) -> Self {
            Self(std::sync::atomic::AtomicU64::new(us))
        }
        fn set(&self, us: u64) {
            self.0.store(us, std::sync::atomic::Ordering::SeqCst);
        }
    }

    impl HostClock for ManualClock {
        fn mono_us(&self) -> u64 {
            self.0.load(std::sync::atomic::Ordering::SeqCst)
        }
        fn unix_us(&self) -> u64 {
            0
        }
    }

    #[tokio::test]
    async fn the_wall_clock_is_re_read_only_when_the_resync_is_due() {
        let clock = std::sync::Arc::new(ManualClock::new(1_000));
        let mut step = open_step(false)
            .with_clock(clock.clone())
            .with_clock_resync(Some(Duration::from_micros(5_000)));
        assert_eq!(step.next_anchor_us, 6_000);
        let mut driver = test_driver();

        clock.set(3_000);
        let _ = step.step(&mut driver, Duration::ZERO).await;
        assert_eq!(step.next_anchor_us, 6_000, "not due yet");

        clock.set(6_000);
        let _ = step.step(&mut driver, Duration::ZERO).await;
        assert_eq!(step.next_anchor_us, 11_000, "due at the deadline");
    }

    #[tokio::test]
    async fn a_response_yields_idle_and_advanced_payload_yields_received() {
        let mut step = open_step(false);
        let mut driver = test_driver();
        step.perform_actions().await.expect("request");
        let conn = *step.requests.keys().next().expect("a pending request");

        step.tx
            .send(Message::Response {
                conn,
                status: http::StatusCode::OK,
                headers: http::HeaderMap::new(),
            })
            .await
            .expect("queue a response");
        let outcome = step.step(&mut driver, Duration::from_millis(5)).await;
        assert!(
            matches!(outcome, StepOutcome::Idle),
            "a message that advanced no payload is idle"
        );

        step.tx
            .send(Message::Data {
                conn,
                bytes: vec![0x47; 10],
            })
            .await
            .expect("queue data");
        let outcome = step.step(&mut driver, Duration::from_millis(5)).await;
        assert!(
            matches!(outcome, StepOutcome::Received),
            "accepted body bytes are received"
        );
    }

    #[tokio::test]
    async fn a_large_read_is_delivered_whole() {
        use tokio::io::AsyncWriteExt as _;

        let (client, mut server) = tokio::io::duplex(8192);
        let (tx, mut rx) = mpsc::channel(4);
        let task = tokio::spawn(read_stream(Box::new(client), ConnId::new(1), tx));
        let payload = vec![0xAB; 2_000];
        server.write_all(&payload).await.expect("write");
        match rx.recv().await.expect("a message") {
            Message::Data { bytes, .. } => {
                assert_eq!(bytes.len(), 2_000, "the read buffer must hold a whole read")
            }
            _ => panic!("expected data"),
        }
        drop(server);
        let _ = rx.recv().await;
        let _ = tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .expect("the read loop ends at EOF");
    }

    #[tokio::test]
    async fn a_closed_read_stops_the_loop() {
        let (client, server) = tokio::io::duplex(64);
        drop(server);
        let (tx, mut rx) = mpsc::channel(1);
        let task = tokio::spawn(read_stream(Box::new(client), ConnId::new(1), tx));
        assert!(matches!(
            rx.recv().await.expect("a message"),
            Message::Closed { error: None, .. }
        ));
        let finished = tokio::time::timeout(Duration::from_secs(5), task).await;
        assert!(
            finished.is_ok(),
            "the read loop must return after delivering the close"
        );
    }

    #[cfg(feature = "tls")]
    #[tokio::test]
    async fn a_failed_tls_handshake_is_an_error() {
        use tokio::io::AsyncWriteExt as _;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("listener");
        let port = listener.local_addr().expect("addr").port();
        let server = tokio::spawn(async move {
            if let Ok((mut socket, _)) = listener.accept().await {
                let _ = socket.write_all(b"not a tls server\r\n").await;
                let _ = socket.shutdown().await;
            }
        });
        let result = TlsHost::default()
            .connect("127.0.0.1", port, true, Duration::from_secs(5))
            .await;
        assert!(
            result.is_err(),
            "a peer that does not speak TLS must fail the handshake"
        );
        let _ = server.await;
    }
}
