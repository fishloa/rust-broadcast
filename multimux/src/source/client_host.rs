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

use broadcast_client::{Action, Config, Error, ErrorKind, Event, Session, Status, UdpMode};
use broadcast_common::{Demand, Stage, Timestamp};
use futures_util::StreamExt;
use media_plane::ingress::{IngestDriver, IngestSession, SessionEvent};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpStream, UdpSocket};
use tokio::sync::mpsc;
use tokio::task::AbortHandle;
use tokio::time::Instant;
use transmux::{TrackId, TrackSpec};

use crate::clock::{HostClock, SystemHostClock};
use crate::error::MultimuxError;
use crate::source::driver::{IngestStep, StepOutcome, ts};
use crate::source::stitch::Stitcher;
use crate::source::ts_program::ProgramTracker;

const MESSAGE_QUEUE: usize = 64;

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
            tracker: ProgramTracker::new(),
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
        conn: u32,
        status: u16,
        headers: Vec<(String, String)>,
    },
    Data {
        conn: u32,
        bytes: Vec<u8>,
    },
    Closed {
        conn: u32,
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
    pub(crate) fn new(what: &'static str, timeouts: crate::source::IngestTimeouts) -> Self {
        Self {
            what,
            connect: timeouts.connect,
            read: timeouts.read,
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
    resync_us: Option<u64>,
    next_anchor_us: u64,
    tx: mpsc::Sender<Message>,
    rx: mpsc::Receiver<Message>,
    requests: BTreeMap<u32, AbortHandle>,
    sockets: BTreeMap<u32, std::sync::Arc<UdpSocket>>,
    writers: BTreeMap<u32, BoxedWrite>,
    udp: UdpHost,
    tls: TlsHost,
    wakes: std::collections::BTreeSet<u64>,
    route_handle: std::sync::Arc<crate::route::RouteHandle>,
    feed_si: bool,
    media_seen: bool,
    stalled_seen: bool,
    finite: bool,
    transport_error: Option<String>,
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
        let mut session = Session::open(url, config, 0).map_err(|e| match e {
            Error::InvalidConfig { field } => MultimuxError::ConfigInvalid {
                field,
                reason: format!("{}: {e}", labels.what),
            },
            other => MultimuxError::Connect {
                reason: format!("{}: {other}", labels.what),
            },
        })?;
        if let Some(credentials) = credentials {
            session
                .set_credentials(credentials)
                .map_err(|e| MultimuxError::Auth {
                    reason: format!("{}: {e}", labels.what),
                })?;
        }
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| MultimuxError::Connect {
                reason: format!("reqwest client: {e}"),
            })?;
        let (tx, rx) = mpsc::channel(MESSAGE_QUEUE);
        let mut step = Self {
            session,
            client,
            labels,
            start,
            clock: std::sync::Arc::new(SystemHostClock::new()),
            resync_us: None,
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
        };
        step.anchor_wall_clock();
        Ok(step)
    }

    fn now_us(&self) -> u64 {
        self.clock.mono_us()
    }

    /// Takes the session's instants from `clock` (monotonic microseconds since the source opened,
    /// and the UTC clock the session's wall-clock anchor is read from).
    pub(crate) fn with_clock(mut self, clock: std::sync::Arc<dyn HostClock>) -> Self {
        self.clock = clock;
        self.anchor_wall_clock();
        self
    }

    /// Re-reads the UTC clock into the session every `resync_us` microseconds, so the session's
    /// wall-clock anchor follows the host clock's own corrections.
    pub(crate) fn with_clock_resync(mut self, resync_us: u64) -> Self {
        self.resync_us = Some(resync_us);
        self.next_anchor_us = self.now_us().saturating_add(resync_us);
        self
    }

    fn anchor_wall_clock(&mut self) {
        let now = self.now_us();
        let unix = self.clock.unix_us();
        self.session.set_wall_clock(now, unix);
        self.next_anchor_us = now.saturating_add(self.resync_us.unwrap_or(u64::MAX));
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
        let now = self.now_us();
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
                    host,
                    port,
                    mode: UdpMode::Receive,
                    multicast,
                    recv_buffer_bytes,
                } => {
                    let socket = self
                        .udp
                        .bind(&host, port, multicast.as_ref(), recv_buffer_bytes)
                        .await?;
                    self.register_socket(conn, socket);
                    self.session.on_connected(conn);
                }
                Action::UdpBind {
                    conn,
                    host,
                    port,
                    mode: UdpMode::Connect,
                    ..
                } => match connect_udp(&host, port).await {
                    Ok(socket) => {
                        self.register_socket(conn, socket);
                        self.session.on_connected(conn);
                    }
                    Err(e) => {
                        self.transport_error = Some(e.to_string());
                        let now = self.now_us();
                        self.session.on_closed(conn, Some(e.to_string()), now);
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
                    self.wakes.insert(at);
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

    fn fail_connection(&mut self, conn: u32, error: String) {
        self.transport_error = Some(error.clone());
        self.writers.remove(&conn);
        let now = self.now_us();
        self.session.on_closed(conn, Some(error), now);
    }

    fn register_socket(&mut self, conn: u32, socket: UdpSocket) {
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
        let now = self.now_us();
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
                self.session.on_closed(conn, error, now);
                false
            }
        }
    }

    fn drain_events(&mut self, driver: &mut IngestDriver<EventIngest>) -> Verdict {
        let now = ts(self.start, Instant::now());
        let mut verdict = Verdict::Continue;
        while let Some(event) = self.session.next_event() {
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
                            phase,
                            reason: "non-success status Not Found".into(),
                        },
                        None => self.http_error(404),
                    }));
                }
                Event::Forbidden { status } if self.labels.session_retries => {
                    verdict = verdict.max(Verdict::Failed(MultimuxError::Auth {
                        reason: format!("{}: HTTP {status}", self.labels.what),
                    }));
                }
                Event::ServerRefused { status, .. } if !self.labels.session_retries => {
                    verdict = verdict.max(Verdict::Failed(self.http_error(status)));
                }
                Event::Forbidden { status } | Event::AuthRejected { status } => {
                    verdict = verdict.max(Verdict::Failed(self.http_error(status)));
                }
                Event::AuthRequired { .. } => {
                    verdict = verdict.max(Verdict::Failed(MultimuxError::Auth {
                        reason: format!("{}: authentication required", self.labels.what),
                    }));
                }
                Event::Error {
                    kind: ErrorKind::RequestRefused { method, status },
                    fatal: false,
                } => {
                    verdict = verdict.max(Verdict::Failed(MultimuxError::Protocol {
                        phase: method,
                        reason: format!("non-success status {status}"),
                    }));
                }
                Event::Error { kind, fatal: false } if kind.ends_connection() => {
                    verdict = verdict.max(Verdict::Failed(MultimuxError::Connect {
                        reason: format!("{}: {}", self.labels.what, kind.name()),
                    }));
                }
                Event::Error { kind, fatal: false }
                    if self.labels.session_retries
                        && matches!(kind.refusal_status(), Some(401 | 403)) =>
                {
                    verdict = verdict.max(Verdict::Failed(MultimuxError::Auth {
                        reason: format!("{}: {}", self.labels.what, kind.name()),
                    }));
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

    fn http_error(&self, status: u16) -> MultimuxError {
        if status == reqwest::StatusCode::UNAUTHORIZED.as_u16() {
            MultimuxError::Auth {
                reason: format!("{}: {status}", self.labels.what),
            }
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
        if self.resync_us.is_some() && self.now_us() >= self.next_anchor_us {
            self.anchor_wall_clock();
        }
        if let Err(e) = self.perform_actions().await {
            return StepOutcome::Failed(e);
        }
        let mut received = false;
        let mut verdict = self.drain_events(driver);
        if matches!(verdict, Verdict::Continue) {
            let now = self.now_us();
            while self.wakes.first().is_some_and(|at| *at <= now) {
                self.wakes.pop_first();
            }
            let until_wake = self
                .wakes
                .first()
                .map_or(window, |at| Duration::from_micros(at.saturating_sub(now)));
            if let Ok(Some(message)) =
                tokio::time::timeout(window.min(until_wake), self.rx.recv()).await
            {
                received = self.apply(message);
            }
            if let Err(e) = self.perform_actions().await {
                return StepOutcome::Failed(e);
            }
            verdict = self.drain_events(driver);
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
        self.session.close();
        if let Err(e) = self.perform_actions().await {
            tracing::debug!(error = %e, "{}: farewell not delivered", self.labels.what);
        }
    }
}

fn http_request(
    client: &reqwest::Client,
    method: &str,
    url: &str,
    headers: &[(String, String)],
    byte_range: Option<(u64, u64)>,
) -> Result<reqwest::RequestBuilder, String> {
    let method = reqwest::Method::from_bytes(method.as_bytes()).map_err(|e| e.to_string())?;
    let mut request = client.request(method, url);
    for (name, value) in headers {
        request = request.header(name, value);
    }
    if let Some((offset, length)) = byte_range {
        let last = offset.saturating_add(length.saturating_sub(1));
        request = request.header(reqwest::header::RANGE, format!("bytes={offset}-{last}"));
    }
    Ok(request)
}

async fn fetch(request: reqwest::RequestBuilder, conn: u32, tx: mpsc::Sender<Message>) {
    let closed = |error: Option<String>| Message::Closed { conn, error };
    let response = match request.send().await {
        Ok(response) => response,
        Err(e) => {
            let _ = tx.send(closed(Some(e.to_string()))).await;
            return;
        }
    };
    let headers = response
        .headers()
        .iter()
        .filter_map(|(name, value)| {
            value
                .to_str()
                .ok()
                .map(|v| (name.as_str().to_owned(), v.to_owned()))
        })
        .collect();
    let head = Message::Response {
        conn,
        status: response.status().as_u16(),
        headers,
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
    pub(crate) options: crate::source::udp::UdpBindOptions,
}

impl UdpHost {
    async fn bind(
        &mut self,
        host: &str,
        port: u16,
        multicast: Option<&broadcast_client::udp::MulticastJoin>,
        recv_buffer_bytes: Option<u32>,
    ) -> Result<UdpSocket, MultimuxError> {
        if let Some(socket) = self.prebound.take() {
            return Ok(socket);
        }
        let mut options = self.options.clone();
        if let Some(bytes) = recv_buffer_bytes {
            options.recv_buffer_bytes = usize::try_from(bytes).ok();
        }
        let (addr, group) = match multicast {
            Some(join) => {
                if let Some(interface) = join.interface {
                    options.multicast_interface = Some(interface.to_string());
                } else if let Some(index) = join.interface_index {
                    options.multicast_interface = Some(index.to_string());
                }
                let any = if host.contains(':') {
                    "[::]"
                } else {
                    "0.0.0.0"
                };
                (format!("{any}:{port}"), Some(host))
            }
            None if host.contains(':') => (format!("[{host}]:{port}"), None),
            None => (format!("{host}:{port}"), None),
        };
        crate::source::udp::bind_udp(&addr, group, options).await
    }
}

async fn receive(socket: std::sync::Arc<UdpSocket>, conn: u32, tx: mpsc::Sender<Message>) {
    let mut buf = vec![0u8; crate::source::MAX_TS_READ];
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

async fn read_stream(mut reader: BoxedRead, conn: u32, tx: mpsc::Sender<Message>) {
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
        let config = self
            .config
            .clone()
            .unwrap_or_else(rtsp_runtime::io::default_tls_client_config);
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

    #[test]
    fn a_config_the_session_refuses_is_the_route_config_error_with_its_field() {
        let mut config = Config::relay();
        config.kind = broadcast_client::SourceKind::Udp;
        config.udp.reorder_packets = 0;
        let labels = SessionLabels::new("ts/udp", crate::source::IngestTimeouts::default());
        let route_handle = std::sync::Arc::new(crate::route::RouteHandle::new(1.0, 250, 8));
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
                MultimuxError::ConfigInvalid { field, .. } if *field == "udp.reorder_packets"
            ),
            "{error:?}"
        );
    }
}
