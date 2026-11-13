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

use broadcast_client::{Action, Config, Event, Session, Status};
use broadcast_common::{Demand, Stage, Timestamp};
use futures_util::StreamExt;
use media_plane::ingress::{IngestDriver, IngestSession, SessionEvent};
use tokio::sync::mpsc;
use tokio::task::AbortHandle;
use tokio::time::Instant;
use transmux::{TrackId, TrackSpec};

use crate::error::MultimuxError;
use crate::source::driver::{IngestStep, StepOutcome, ts};
use crate::source::ts_program::ProgramTracker;

const MICROS_PER_NANO_DIVISOR: u128 = 1_000;
const MESSAGE_QUEUE: usize = 64;

/// Splits the session's track and sample events per programme, with the same programme policy
/// every transport shares ([`ProgramTracker`]).
pub(crate) struct EventIngest {
    tracker: ProgramTracker,
    known: BTreeMap<TrackId, TrackSpec>,
}

impl EventIngest {
    pub(crate) fn new() -> Self {
        Self {
            tracker: ProgramTracker::new(),
            known: BTreeMap::new(),
        }
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
            Event::Tracks(specs) => self.tracks(specs),
            Event::Sample { track, sample, .. } => self.tracker.sample(track, sample),
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
    pub(crate) what: &'static str,
    pub(crate) connect: Duration,
    pub(crate) read: Duration,
}

pub(crate) struct SessionStep {
    session: Session,
    client: reqwest::Client,
    labels: SessionLabels,
    start: Instant,
    tx: mpsc::Sender<Message>,
    rx: mpsc::Receiver<Message>,
    requests: BTreeMap<u32, AbortHandle>,
    wake_at: Option<u64>,
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
        config: Config,
        credentials: Option<broadcast_client::Credentials>,
        labels: SessionLabels,
        route_handle: std::sync::Arc<crate::route::RouteHandle>,
        feed_si: bool,
    ) -> Result<Self, MultimuxError> {
        let start = Instant::now();
        let mut session = Session::open(url, config, 0).map_err(|e| MultimuxError::Connect {
            reason: format!("{}: {e}", labels.what),
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
        Ok(Self {
            session,
            client,
            labels,
            start,
            tx,
            rx,
            requests: BTreeMap::new(),
            wake_at: None,
            route_handle,
            feed_si,
            media_seen: false,
            stalled_seen: false,
            finite: false,
            transport_error: None,
        })
    }

    fn now_us(&self) -> u64 {
        let micros = self.start.elapsed().as_nanos() / MICROS_PER_NANO_DIVISOR;
        u64::try_from(micros).unwrap_or(u64::MAX)
    }

    fn perform_actions(&mut self) -> Result<(), MultimuxError> {
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
                Action::Close { conn } => {
                    if let Some(task) = self.requests.remove(&conn) {
                        task.abort();
                    }
                }
                Action::WakeAt(at) => {
                    self.wake_at = Some(self.wake_at.map_or(at, |w| w.min(at)));
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
                self.session.on_data(conn, &bytes, now);
                true
            }
            Message::Closed { conn, error } => {
                self.requests.remove(&conn);
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
                Event::Status(Status::Stalled { .. }) => self.stalled_seen = true,
                Event::Status(Status::Reconnecting { .. }) => {
                    verdict = verdict.max(self.reconnect_verdict());
                }
                Event::Finite { .. } => self.finite = true,
                Event::Status(Status::Ended) => verdict = verdict.max(Verdict::Ended),
                Event::Status(Status::NotFound) => {
                    verdict = verdict.max(Verdict::Failed(self.http_error(404)));
                }
                Event::ServerRefused { status, .. }
                | Event::Forbidden { status }
                | Event::AuthRejected { status } => {
                    verdict = verdict.max(Verdict::Failed(self.http_error(status)));
                }
                Event::AuthRequired { .. } => {
                    verdict = verdict.max(Verdict::Failed(MultimuxError::Auth {
                        reason: format!("{}: authentication required", self.labels.what),
                    }));
                }
                Event::Error { kind, fatal: true } => {
                    verdict = verdict.max(Verdict::Failed(MultimuxError::Connect {
                        reason: format!("{}: {kind:?}", self.labels.what),
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
        if let Err(e) = self.perform_actions() {
            return StepOutcome::Failed(e);
        }
        let mut received = false;
        let mut verdict = self.drain_events(driver);
        if matches!(verdict, Verdict::Continue) {
            let until_wake = self.wake_at.map_or(window, |at| {
                Duration::from_micros(at.saturating_sub(self.now_us()))
            });
            match tokio::time::timeout(window.min(until_wake), self.rx.recv()).await {
                Ok(Some(message)) => received = self.apply(message),
                Ok(None) => {}
                Err(_) => self.wake_at = None,
            }
            if let Err(e) = self.perform_actions() {
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
