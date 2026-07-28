//! RTMP push ingest source (issue #738; ported onto `media_plane::ingress`'s
//! [`Listener`] trait at issue #805 task 4 — the last of the nine input
//! kinds to move onto the driver-backed architecture; see this crate's
//! CHANGELOG and `docs/superpowers/specs/2026-07-26-media-plane-architecture.md`
//! §8 for why RTMP was deliberately left on the old `SourceConnector` path
//! until every other kind (and the registry reconciliation it depends on)
//! was already proven).
//!
//! # Why `Listener`, not `Dialer`
//!
//! Every other source in this module dials out. RTMP is the opposite: it
//! binds a listen port and *accepts* inbound publishers, which is exactly
//! what [`media_plane::ingress::Listener`] models — `poll_accept() ->
//! Result<Option<Session>>`, a non-blocking poll so
//! [`media_plane::ingress::ListenDriver`] can admit and drive up to
//! `max_sessions` connections **concurrently**.
//!
//! # Bridging a blocking-async `accept()` into a non-blocking `poll_accept()`
//!
//! `rtmp_runtime::io::AsyncRtmpServer::accept` is `pub async fn accept(&self)
//! -> io::Result<RtmpConnection>` — a blocking `.await`, no `try_accept`
//! variant. `RtmpRoute::ensure_infra` (private) bridges it with an
//! **accept-pump task**: spawned once (alongside the listen bind — see
//! "Bind once" below), it loops `server.accept().await` and sends each
//! accepted [`RtmpConnection`] into a bounded [`tokio::sync::mpsc`] channel;
//! `RtmpListener::poll_accept` drains it with `try_recv()` — `Empty` maps to
//! `Ok(None)`, which [`Listener::poll_accept`]'s own contract spells out as
//! "nothing waiting right now", not an error or end-of-input.
//!
//! # Why `RtmpIngestSession` isn't fed raw bytes
//!
//! Every other byte-stream `IngestSession` in this crate states `type In<'a>
//! = &'a [u8]` and does its own wire-protocol parsing inside `feed`. RTMP
//! does not, because [`RtmpConnection::next_events`] already **is** the
//! read-parse-reply cycle: it reads a socket chunk, drives the sans-IO
//! `rtmp_runtime::server::ServerSession` internally, writes whatever reply
//! bytes that produced straight back to the socket, and only then hands back
//! the resulting [`ServerEvent`]s. There is no raw `TcpStream` to extract
//! from an already-constructed `RtmpConnection` (its fields are private —
//! deliberately, since a caller reimplementing the read/reply loop over its
//! socket would just be recreating what `next_events` already does), so
//! [`RtmpIngestSession`] instead states `type In<'a> = &'a [ServerEvent]`:
//! [`run_rtmp`]'s own per-session task awaits `next_events()` (the genuinely
//! async, I/O-performing half) and hands the resulting events to
//! [`Stage::feed`] (the sans-IO translation half, no different in kind from
//! any other session's byte-parsing `feed`, just parsing pre-parsed events
//! instead of bytes). [`IngestSession::poll_transmit`] is therefore never
//! overridden here (its `None` default is exactly right): every reply RTMP
//! needs is already written by `next_events` itself before this session ever
//! sees the events it produced.
//!
//! Because `Stage::In` isn't `&[u8]`, [`media_plane::ingress::ListenDriver`]'s
//! own `feed`/`on_deadline`/`finish` convenience wrappers (pinned to `&[u8]`)
//! don't fit; [`run_rtmp`] instead uses `ListenDriver::driver_mut`/`driver`/
//! `reap_if_terminal` (added alongside this port — see that method's own doc
//! in `media-plane/src/ingress.rs` for why).
//!
//! # Why listener mode *is* a `Listener` here, unlike SRT
//!
//! `multimux::source::srt`'s module doc has a whole section, "Why listener
//! mode is not a `Listener` yet", explaining that `SrtListener::accept` takes
//! `&mut self` (so it cannot be shared without a `Mutex`) and — the harder
//! blocker — the listener and *every* accepted connection share one
//! `UdpSocket`, making "accept another while N are live" a demultiplexing
//! problem inside `srt-runtime` itself, not a `multimux` concern at all.
//! **Neither applies to RTMP**: `AsyncRtmpServer::accept` takes `&self` (so
//! the server can be `Arc`-shared across the accept-pump task and this
//! module without any additional locking), and each accepted connection owns
//! its own `TcpStream` — concurrent accept is not a demultiplexing problem
//! here, it is exactly what `AsyncRtmpServer`/`RtmpConnection` were already
//! built to support. The blocker SRT's doc describes is specific to
//! `srt-runtime`'s current public API, not a general objection to a push
//! source implementing `Listener` — RTMP is the proof.
//!
//! # This fixes a real, documented defect
//!
//! Before this port, `crate::origin::supervisor::supervise` awaited
//! `RtmpSource::connect()` **sequentially** against a bind-once, reused
//! listener: a publisher that completed the handshake and then went idle
//! (never sending a media tag, or going silent mid-stream) held `connect()`/
//! `next_samples()` open for up to its own timeout, and because `supervise`
//! never called `connect()` again until that one call returned, **no other
//! publisher could be accepted in the meantime** — one stalled connection
//! wedged the entire route (`#738 T11b review, Critical`; previously
//! mitigated only by `IngestTimeouts` eventually giving up on that one
//! connection). [`run_rtmp`]'s `ListenDriver` admits and drives up to
//! `max_sessions` sessions **concurrently** — each session's read is its own
//! independent `tokio` task, raced via `FuturesUnordered`, so one stalled
//! publisher's read future simply never resolves while every other session's
//! read future keeps making progress. See this module's
//! `second_publisher_is_served_while_first_is_stalled_at_handshake` test.
//!
//! # Preserved behaviours (unchanged by this port)
//!
//! - **Bind once, reuse forever.** `RtmpRoute::ensure_infra` binds the
//!   listen socket lazily via a [`tokio::sync::OnceCell`] and spawns the
//!   accept-pump task exactly once; every subsequent [`run_rtmp`] call (a new
//!   attempt from `crate::origin::supervisor::supervise_driver`, e.g. after a
//!   listen-socket-level failure) reuses the same bound socket and pump —
//!   never a `:1935` re-bind race against the supervisor's backoff loop.
//! - **`Established` gates on the first `Sample`, not the first
//!   `TrackAdded`.** FLV carries no PMT-style "this many tracks are coming"
//!   declaration (see [`transmux::flv_stream`]'s module doc, "No
//!   `TracksResolved`"), so [`RtmpIngestSession`] leans on that module's own
//!   documented **ordering assumption** (Annex E: a track's AVC/AAC
//!   sequence-header tag always precedes that track's media tags) and waits
//!   for the first [`transmux::DemuxEvent::Sample`] before announcing
//!   [`SessionEvent::NewProgram`]/[`SessionEvent::Established`] — exactly
//!   what [`SessionEvent::Established`]'s own doc in `media-plane`
//!   prescribes, naming RTMP as this case verbatim ("gating on the first
//!   `DemuxEvent::Sample`, exactly as those docs suggest").
//! - **The first sample is never dropped.** Every [`DemuxEvent::Sample`]
//!   observed *before* the first-sample gate fires is buffered
//!   (`newly_seen_samples`) and re-emitted as [`SessionEvent::Sample`]
//!   immediately after the [`SessionEvent::NewProgram`]/[`SessionEvent::Established`]
//!   pair that same `feed` call produces — nothing observed during
//!   establishment is silently discarded. See this module's
//!   `first_sample_observed_during_establishment_is_not_dropped` test.
//! - **Timeouts still bound reads.** [`IngestTimeouts::read`] wraps every
//!   [`RtmpConnection::next_events`] call (`with_timeouts`, mirroring every
//!   other source in this module); on expiry that session is ended (treated
//!   the same as a clean EOF — see `read_one`'s doc for why the exact
//!   `HealthState` distinction from a genuine transport error isn't
//!   preserved, only the "reads don't hang forever" property), freeing its
//!   `max_sessions` slot for the next publisher.

use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

use broadcast_common::{Demand, Stage, Timestamp};
use futures_util::StreamExt;
use futures_util::stream::FuturesUnordered;
use rtmp_runtime::io::{AsyncRtmpServer, RtmpConnection};
use rtmp_runtime::server::{ServerConfig, ServerEvent};
use tokio::sync::{Mutex as TokioMutex, OnceCell, mpsc};
use transmux::pipeline::{Sample, TrackSpec};
use transmux::{DemuxEvent, StreamingFlvDemux};

use media_plane::ingress::{
    AcceptOutcome, HandshakePolicy, IngestSession, ListenDriver, Listener, ProgramId, SessionEvent,
    SessionId,
};
use media_plane::trunk::{RetentionClass, TrunkConfig};

use crate::error::{MultimuxError, Result};
use crate::route::RouteHandle;
use crate::source::segment::ProgramSegmenter;
use crate::source::{IngestTimeouts, Source};

/// Per-session `report_driver_progress` bookkeeping — one `HashSet` per
/// admitted [`SessionId`], mirroring every other driver-backed source's own
/// (single-session) `published: HashSet<ProgramId>`, just keyed one level
/// deeper since [`run_rtmp`] drives many sessions at once.
type PublishedBySession = HashMap<SessionId, HashSet<ProgramId>>;

/// Per-session [`ProgramSegmenter`] bookkeeping — the same one-level-deeper
/// mirror of every other driver-backed source's own `HashMap<ProgramId,
/// ProgramSegmenter>` as [`PublishedBySession`].
type SegmentersBySession = HashMap<SessionId, HashMap<ProgramId, ProgramSegmenter>>;

/// One [`read_one`] call, boxed so [`run_rtmp`] can hold many of them (one
/// per admitted session) in a single [`FuturesUnordered`].
type BoxedRead = Pin<Box<dyn Future<Output = (SessionId, ReadOutcome)> + Send>>;

/// Default cap on concurrently admitted RTMP publishers per route
/// ([`Listener::max_sessions`]) — generous for a single origin's worth of
/// cameras/encoders while still bounding a flood of inbound TCP connections
/// (the same class of unbounded-allocation vector `media_plane::ingress`'s
/// own `max_sessions`/`max_programs` docs describe).
pub const DEFAULT_RTMP_MAX_SESSIONS: usize = 16;

/// How often [`run_rtmp`]'s driving loop polls [`ListenDriver::poll_accept`]
/// for a newly accepted connection, while no session currently has a read in
/// flight to race it against. Small enough that a new publisher is admitted
/// promptly; a fixed poll rather than a wake-on-send `Notify` because
/// `RtmpListener::poll_accept` is itself required to be non-blocking and O(1)
/// — polling it on a short tick is exactly as cheap as a real wake-up and
/// avoids a second synchronization primitive for no behavioural difference.
const ACCEPT_POLL_INTERVAL: Duration = Duration::from_millis(20);

/// Bound on the accept-pump task's channel — see [`RtmpRoute::ensure_infra`].
/// A publisher that completes its TCP handshake is queued here until
/// [`RtmpListener::poll_accept`] drains it (whether or not a `max_sessions`
/// slot is free); this is *separate* from `max_sessions` (which bounds
/// concurrently **admitted** sessions) and exists so a burst of connects
/// arriving faster than they're drained cannot grow this channel without
/// bound.
const ACCEPT_QUEUE_CAPACITY: usize = 32;

/// The bind-once, reuse-forever infrastructure [`RtmpRoute::ensure_infra`]
/// builds exactly once and every [`run_rtmp`] attempt shares — see the
/// module doc's "Bind once" note.
struct RtmpInfra {
    /// Shared with every `run_rtmp` attempt via `Arc`; guarded by a
    /// synchronous [`StdMutex`] (never held across an `.await`) so
    /// [`RtmpListener::poll_accept`] — which [`Listener::poll_accept`]
    /// requires to be non-blocking — can `try_recv()` without an executor
    /// bridge.
    accept_rx: Arc<StdMutex<mpsc::Receiver<RtmpConnection>>>,
}

/// An RTMP push-ingest route: binds `listen` once and accepts publishers
/// against it, gated by an optional `app`/`stream_key` (see
/// [`Self::with_app`]/[`Self::with_stream_key`]). Replaces the pre-#805
/// task-4 `RtmpSource`; [`run_rtmp`] is the new, `Listener`-backed drive loop.
pub struct RtmpRoute {
    name: String,
    listen: String,
    app: Option<String>,
    stream_key: Option<String>,
    timeouts: IngestTimeouts,
    max_sessions: usize,
    /// Bind-once, reuse-forever — see the module doc.
    infra: OnceCell<RtmpInfra>,
}

/// Manual `Debug` (rather than `#[derive(Debug)]`), mirroring
/// [`crate::source::ts_http::TsHttpRoute`]'s: `stream_key` is redacted even
/// though it's a shared secret rather than a password, for the same reason
/// — it should never turn up verbatim in a log line.
impl std::fmt::Debug for RtmpRoute {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RtmpRoute")
            .field("name", &self.name)
            .field("listen", &self.listen)
            .field("app", &self.app)
            .field("stream_key", &self.stream_key.as_ref().map(|_| "***"))
            .field("max_sessions", &self.max_sessions)
            .finish()
    }
}

impl RtmpRoute {
    /// Build a route listening on `listen` (e.g. `"0.0.0.0:1935"`, or
    /// `"127.0.0.1:0"` for an ephemeral test port).
    pub fn new(name: impl Into<String>, listen: impl Into<String>) -> Self {
        RtmpRoute {
            name: name.into(),
            listen: listen.into(),
            app: None,
            stream_key: None,
            timeouts: IngestTimeouts::default(),
            max_sessions: DEFAULT_RTMP_MAX_SESSIONS,
            infra: OnceCell::new(),
        }
    }

    /// Overrides the default [`IngestTimeouts`]: `connect` bounds how long a
    /// still-establishing session (no `Sample` observed yet) may sit before
    /// [`HandshakePolicy::establish_by`] reaps it; `read` bounds every
    /// individual [`RtmpConnection::next_events`] call.
    #[must_use]
    pub fn with_timeouts(mut self, timeouts: IngestTimeouts) -> Self {
        self.timeouts = timeouts;
        self
    }

    /// Requires the publisher's `connect` `app` name to match exactly, if
    /// set — enforced by [`RtmpIngestSession::feed`] once the first
    /// `Connected { app }` event arrives.
    #[must_use]
    pub fn with_app(mut self, app: Option<String>) -> Self {
        self.app = app;
        self
    }

    /// Requires the publisher's `publish` stream key to match exactly, if
    /// set — passed straight through as
    /// [`ServerConfig::expected_stream_key`], so a mismatched key is
    /// rejected by the sans-IO session itself (`NetStream.Publish.BadName`)
    /// before this source ever sees a `Publish`/`Media` event.
    #[must_use]
    pub fn with_stream_key(mut self, stream_key: Option<String>) -> Self {
        self.stream_key = stream_key;
        self
    }

    /// Overrides [`DEFAULT_RTMP_MAX_SESSIONS`] — the [`Listener::max_sessions`]
    /// bound.
    #[must_use]
    pub fn with_max_sessions(mut self, max_sessions: usize) -> Self {
        self.max_sessions = max_sessions;
        self
    }

    /// Binds the listen socket and spawns the accept-pump task on the first
    /// call only (`OnceCell`); every later call — a fresh [`run_rtmp`]
    /// attempt — is handed a clone of the same shared receiver, never
    /// re-binding. See the module doc's "Bind once" note.
    async fn ensure_infra(&self) -> Result<Arc<StdMutex<mpsc::Receiver<RtmpConnection>>>> {
        let infra = self
            .infra
            .get_or_try_init(|| async {
                let config =
                    ServerConfig::default().with_expected_stream_key(self.stream_key.clone());
                let server = AsyncRtmpServer::bind(self.listen.as_str(), config)
                    .await
                    .map(Arc::new)
                    .map_err(|e| MultimuxError::Connect {
                        reason: format!("rtmp: bind {}: {e}", self.listen),
                    })?;
                let (tx, rx) = mpsc::channel(ACCEPT_QUEUE_CAPACITY);
                tokio::spawn(async move {
                    loop {
                        match server.accept().await {
                            Ok(conn) => {
                                if tx.send(conn).await.is_err() {
                                    // No `RtmpListener` is receiving any more
                                    // (every `run_rtmp` attempt using this
                                    // route has ended) — nothing left to pump
                                    // for.
                                    break;
                                }
                            }
                            Err(e) => {
                                tracing::warn!(
                                    error = %e,
                                    "rtmp: accept-pump ending after a listen-socket error"
                                );
                                break;
                            }
                        }
                    }
                });
                Ok::<RtmpInfra, MultimuxError>(RtmpInfra {
                    accept_rx: Arc::new(StdMutex::new(rx)),
                })
            })
            .await?;
        Ok(Arc::clone(&infra.accept_rx))
    }
}

impl Source for RtmpRoute {
    fn stream_name(&self) -> &str {
        &self.name
    }
}

/// The non-blocking [`Listener`] bridge over the accept-pump's channel — see
/// the module doc.
struct RtmpListener {
    accept_rx: Arc<StdMutex<mpsc::Receiver<RtmpConnection>>>,
    app: Option<String>,
    max_sessions: usize,
}

impl Listener for RtmpListener {
    type Session = RtmpIngestSession;
    type Error = MultimuxError;

    fn max_sessions(&self) -> usize {
        self.max_sessions
    }

    fn poll_accept(&mut self) -> Result<Option<RtmpIngestSession>> {
        let mut rx = self
            .accept_rx
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match rx.try_recv() {
            Ok(conn) => Ok(Some(RtmpIngestSession::new(conn, self.app.clone()))),
            Err(mpsc::error::TryRecvError::Empty) => Ok(None),
            Err(mpsc::error::TryRecvError::Disconnected) => Err(MultimuxError::Connect {
                reason: "rtmp: accept-pump task ended (listen socket failure)".into(),
            }),
        }
    }
}

/// An admitted RTMP publisher: a shared handle to its accepted
/// [`RtmpConnection`] (read by [`run_rtmp`]'s own per-session task — see the
/// module doc's "Why `RtmpIngestSession` isn't fed raw bytes") plus the FLV
/// demux state that translates [`ServerEvent`]s into [`SessionEvent`]s.
pub struct RtmpIngestSession {
    conn: Arc<TokioMutex<RtmpConnection>>,
    app: Option<String>,
    demux: StreamingFlvDemux,
    /// Track specs resolved so far — frozen into `known_track_ids` once
    /// established (see the module doc's "`Established` gates on the first
    /// `Sample`" note).
    specs: Vec<TrackSpec>,
    known_track_ids: BTreeSet<u32>,
    established: bool,
    pending: VecDeque<SessionEvent>,
}

impl RtmpIngestSession {
    fn new(conn: RtmpConnection, app: Option<String>) -> Self {
        RtmpIngestSession {
            conn: Arc::new(TokioMutex::new(conn)),
            app,
            demux: StreamingFlvDemux::new(),
            specs: Vec::new(),
            known_track_ids: BTreeSet::new(),
            established: false,
            pending: VecDeque::new(),
        }
    }

    /// A cheap `Arc` clone of the accepted connection — [`run_rtmp`]'s own
    /// per-session read task uses this to call
    /// [`RtmpConnection::next_events`] concurrently with every other
    /// session's, entirely outside this sans-IO [`Stage`] impl. See the
    /// module doc.
    pub fn conn_handle(&self) -> Arc<TokioMutex<RtmpConnection>> {
        Arc::clone(&self.conn)
    }

    /// Drains every [`DemuxEvent`] the FLV demux has ready, translating it
    /// into [`SessionEvent`]s queued on `pending` — shared by [`Self::feed`]
    /// and [`Self::finish`]. See the module doc's "`Established` gates on the
    /// first `Sample`" / "first sample is never dropped" notes for exactly
    /// what this does before vs. after establishment.
    fn drain_demux(&mut self) {
        let mut newly_seen_samples: Vec<(u32, Sample)> = Vec::new();
        while let Some(ev) = self.demux.poll_event() {
            match ev {
                DemuxEvent::TrackAdded(spec) => {
                    if !self.established {
                        self.specs.push(spec);
                    }
                    // A track resolving *after* establishment is a
                    // genuinely non-conformant encoder (Annex E requires the
                    // sequence header before any media for that track) —
                    // ignored, mirroring `ts_program::ProgramTracker`'s own
                    // "unrouted track" policy for a post-connect PMT bump.
                }
                DemuxEvent::Sample {
                    track_id, sample, ..
                } => {
                    if !self.established {
                        // Buffered, not dropped — re-emitted below the
                        // moment establishment fires, whichever sample(s)
                        // triggered it included.
                        newly_seen_samples.push((track_id, sample));
                    } else if self.known_track_ids.contains(&track_id) {
                        self.pending.push_back(SessionEvent::Sample {
                            program: ProgramId(0),
                            track_id,
                            retention: RetentionClass::Timed,
                            sample,
                        });
                    }
                    // A sample for a track not in `known_track_ids` (only
                    // possible for the non-conformant late-track case noted
                    // above) is dropped, exactly like `ts_program`'s
                    // unrouted-track policy.
                }
                _ => {}
            }
        }

        if !self.established && !newly_seen_samples.is_empty() {
            self.known_track_ids = self.specs.iter().map(|s| s.track_id).collect();
            self.pending.push_back(SessionEvent::NewProgram {
                program: ProgramId(0),
                tracks: self.specs.clone(),
            });
            self.pending.push_back(SessionEvent::Established);
            self.established = true;
            for (track_id, sample) in newly_seen_samples {
                if self.known_track_ids.contains(&track_id) {
                    self.pending.push_back(SessionEvent::Sample {
                        program: ProgramId(0),
                        track_id,
                        retention: RetentionClass::Timed,
                        sample,
                    });
                }
            }
        }
    }
}

impl Stage for RtmpIngestSession {
    type In<'a> = &'a [ServerEvent];
    type Out = SessionEvent;
    type Error = MultimuxError;

    fn feed(&mut self, events: &[ServerEvent], _now: Timestamp) -> Result<()> {
        for event in events {
            match event {
                ServerEvent::Connected { app } => {
                    if let Some(expected) = &self.app {
                        if app != expected {
                            return Err(MultimuxError::Connect {
                                reason: format!(
                                    "rtmp: app {app:?} does not match configured app {expected:?}"
                                ),
                            });
                        }
                    }
                }
                ServerEvent::Media { flv } => {
                    self.demux.feed(flv).map_err(|e| MultimuxError::Depay {
                        reason: format!("rtmp/flv: {e}"),
                    })?;
                }
                // `ServerEvent` is `#[non_exhaustive]`; `Publish`/`Eof` need
                // no reaction here — `Eof` is observed at the transport
                // level (`RtmpConnection::next_events` returning `Ok(None)`)
                // and drives `Self::finish` instead.
                _ => {}
            }
        }
        self.drain_demux();
        Ok(())
    }

    fn poll(&mut self) -> Option<SessionEvent> {
        self.pending.pop_front()
    }

    fn finish(&mut self) -> Result<()> {
        self.demux.finish();
        self.drain_demux();
        Ok(())
    }

    fn next_deadline(&self) -> Option<Timestamp> {
        None
    }

    fn on_deadline(&mut self, _now: Timestamp) {}

    fn demand(&self) -> Demand {
        Demand::new(4096)
    }
}

impl IngestSession for RtmpIngestSession {
    /// Uninhabited: `RtmpConnection::next_events` already writes every reply
    /// byte the RTMP protocol needs (see the module doc), so this session
    /// never has anything of its own to send — `poll_transmit`'s `None`
    /// default is exactly right, and there is no real value to name here.
    type Request = Infallible;
}

/// What one [`read_one`] call observed.
enum ReadOutcome {
    /// A batch of parsed (and already replied-to) events, ready to `feed`.
    Events(Vec<ServerEvent>),
    /// The connection ended cleanly.
    Eof,
    /// The underlying read/parse failed.
    TransportError(String),
    /// No data within [`IngestTimeouts::read`] — see the module doc's
    /// "Timeouts still bound reads" note for why this is treated the same
    /// as [`Self::Eof`] rather than surfacing a distinct `HealthState::Failed`.
    TimedOut,
}

/// Awaits the next batch of events for session `id`, bounded by
/// `read_timeout` — the async half of driving an [`RtmpIngestSession`] (see
/// the module doc). Boxed so [`run_rtmp`] can hold many of these,
/// one per admitted session, in one [`FuturesUnordered`].
fn read_one(
    id: SessionId,
    conn: Arc<TokioMutex<RtmpConnection>>,
    read_timeout: Duration,
) -> BoxedRead {
    Box::pin(async move {
        let mut guard = conn.lock().await;
        let outcome = match tokio::time::timeout(read_timeout, guard.next_events()).await {
            Ok(Ok(Some(events))) => ReadOutcome::Events(events),
            Ok(Ok(None)) => ReadOutcome::Eof,
            Ok(Err(e)) => ReadOutcome::TransportError(e.to_string()),
            Err(_) => ReadOutcome::TimedOut,
        };
        (id, outcome)
    })
}

/// Publishes session `id`'s progress (registry + segmenters) and, if that
/// left it terminal, reaps it — the per-session equivalent of every other
/// driver-backed source's single-`IngestDriver` "feed, report, check
/// `is_running`" sequence, reassembled from
/// [`ListenDriver::driver`]/[`ListenDriver::reap_if_terminal`] because
/// [`ListenDriver::feed`]'s `&[u8]`-pinned convenience wrapper doesn't fit
/// [`RtmpIngestSession`]'s `Stage::In` (see the module doc).
fn report_and_maybe_reap(
    driver: &mut ListenDriver<RtmpListener>,
    id: SessionId,
    route_handle: &Arc<RouteHandle>,
    published: &mut PublishedBySession,
    segmenters: &mut SegmentersBySession,
) -> bool {
    if let Some(d) = driver.driver(id) {
        crate::source::report_driver_progress(d, route_handle, published.entry(id).or_default());
        crate::source::segment::drive_program_segmenters(
            d,
            route_handle,
            segmenters.entry(id).or_default(),
        );
    }
    let reaped = driver.reap_if_terminal(id).is_some();
    if reaped {
        published.remove(&id);
        segmenters.remove(&id);
    }
    reaped
}

/// Binds `route` (once ever — see the module doc), then admits and drives up
/// to [`Listener::max_sessions`] RTMP publishers **concurrently** until a
/// listen-socket-level failure occurs. Never returns in ordinary operation —
/// unlike every dial-based `run_*` in this module (one connection per
/// attempt), a `Listener`-backed source's whole point is to keep accepting
/// across the lifetime of the route, so `crate::origin::supervisor::supervise_driver`
/// calling this again after an `Err` is the "the listen socket itself died,
/// try rebinding" path, not "the one publisher disconnected, get the next
/// one" (that no longer needs a fresh attempt at all — see concurrent
/// admission above).
///
/// `route_handle` is the driver-backed registry side of issue #805 task 2 —
/// each session's own `media_plane::ingress::IngestDriver` publishes its
/// programs and drives its segmenters exactly like every other `run_*` entry
/// point, just once per admitted session rather than once for the whole
/// call (see `report_and_maybe_reap`).
pub async fn run_rtmp(
    route: &RtmpRoute,
    trunk_config: TrunkConfig,
    handshake: HandshakePolicy,
    route_handle: &Arc<RouteHandle>,
) -> MultimuxError {
    let accept_rx = match route.ensure_infra().await {
        Ok(rx) => rx,
        Err(e) => return e,
    };
    let listener = RtmpListener {
        accept_rx,
        app: route.app.clone(),
        max_sessions: route.max_sessions,
    };
    let mut driver: ListenDriver<RtmpListener> = ListenDriver::new(
        listener,
        trunk_config,
        handshake,
        media_plane::DEFAULT_MAX_PROGRAMS,
    );
    let start = Instant::now();
    let read_timeout = route.timeouts.read;

    let mut published: PublishedBySession = HashMap::new();
    let mut segmenters: SegmentersBySession = HashMap::new();
    let mut reads: FuturesUnordered<BoxedRead> = FuturesUnordered::new();

    loop {
        tokio::select! {
            () = tokio::time::sleep(ACCEPT_POLL_INTERVAL) => {
                loop {
                    match driver.poll_accept() {
                        AcceptOutcome::Idle => break,
                        AcceptOutcome::Refused => {
                            tracing::warn!("rtmp: connection refused, max_sessions reached");
                        }
                        AcceptOutcome::Error(e) => return e,
                        AcceptOutcome::Admitted(id) => {
                            let conn = driver
                                .driver(id)
                                .expect("just admitted by poll_accept")
                                .session()
                                .conn_handle();
                            published.insert(id, HashSet::new());
                            segmenters.insert(id, HashMap::new());
                            reads.push(read_one(id, conn, read_timeout));
                        }
                        // `AcceptOutcome` is `#[non_exhaustive]`: a future
                        // variant this loop has no reaction to yet is
                        // treated like `Idle` (stop draining for this tick)
                        // rather than looping forever on an unrecognized
                        // outcome.
                        _ => break,
                    }
                }
            }
            Some((id, outcome)) = reads.next(), if !reads.is_empty() => {
                let now = Timestamp::from_instant(start, Instant::now());
                match outcome {
                    ReadOutcome::Events(events) => {
                        if let Some(d) = driver.driver_mut(id) {
                            d.feed(&events[..], now);
                        }
                        let reaped =
                            report_and_maybe_reap(&mut driver, id, route_handle, &mut published, &mut segmenters);
                        if !reaped {
                            if let Some(d) = driver.driver(id) {
                                let conn = d.session().conn_handle();
                                reads.push(read_one(id, conn, read_timeout));
                            }
                        }
                    }
                    ReadOutcome::Eof => {
                        if let Some(d) = driver.driver_mut(id) {
                            d.finish();
                        }
                        report_and_maybe_reap(&mut driver, id, route_handle, &mut published, &mut segmenters);
                    }
                    ReadOutcome::TransportError(reason) => {
                        tracing::warn!(error = %reason, "rtmp: session read failed");
                        if let Some(d) = driver.driver_mut(id) {
                            d.finish();
                        }
                        report_and_maybe_reap(&mut driver, id, route_handle, &mut published, &mut segmenters);
                    }
                    ReadOutcome::TimedOut => {
                        tracing::warn!("rtmp: session idle past read timeout");
                        if let Some(d) = driver.driver_mut(id) {
                            d.finish();
                        }
                        report_and_maybe_reap(&mut driver, id, route_handle, &mut published, &mut segmenters);
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::ts_program::test_support::{handshake, trunk_config};
    use rtmp_runtime::server::ServerSession;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};
    use transmux::pipeline::CodecConfig;

    /// The real ffmpeg-captured RTMP publish (`app=live`, `stream_key=testkey`,
    /// H.264+AAC) — copied into this crate for hermeticity, see
    /// `tests/fixtures/PROVENANCE.md`. Original: `rtmp-runtime/tests/fixtures/obs-publish.bin`.
    const FIXTURE: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/rtmp-obs-publish.bin"
    );

    fn load_fixture() -> Vec<u8> {
        std::fs::read(FIXTURE).expect("read tests/fixtures/rtmp-obs-publish.bin")
    }

    /// Reserves a free loopback TCP port then immediately releases it, so a
    /// caller can pass the exact address to something that binds it shortly
    /// after — the same technique `ts_udp`'s own loopback test uses.
    async fn reserve_addr() -> std::net::SocketAddr {
        let reserved = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("reserve port");
        let addr = reserved.local_addr().expect("local addr");
        drop(reserved);
        addr
    }

    /// Replays `fixture` byte-by-byte through a fresh sans-IO `ServerSession`,
    /// returning the fixture offset at which the *first* `ServerEvent::Publish`
    /// fires — i.e. the handshake/connect/createStream/publish dance has
    /// completed but no `Media` tag has been sent yet. A client that sends
    /// only `fixture[..offset]` has therefore genuinely completed the RTMP
    /// handshake and then gone silent, never publishing any media — computed
    /// empirically (not hand-picked) so this stays correct if the fixture is
    /// ever swapped for a different capture.
    fn fixture_offset_after_publish_before_any_media(fixture: &[u8]) -> usize {
        let mut session = ServerSession::new(ServerConfig::default());
        for (i, byte) in fixture.iter().enumerate() {
            let (_, events) = session
                .handle_data(std::slice::from_ref(byte))
                .expect("handle_data must not error while replaying a real capture");
            if events
                .iter()
                .any(|e| matches!(e, ServerEvent::Publish { .. }))
            {
                return i + 1;
            }
        }
        panic!("fixture never reaches ServerEvent::Publish");
    }

    /// Connects to `addr` and writes `bytes`, draining (and discarding)
    /// whatever the server replies with so its writes never block — shared
    /// shape by every test below's client task.
    async fn connect_and_write(addr: std::net::SocketAddr, bytes: Vec<u8>) -> TcpStream {
        let mut stream = None;
        for _ in 0..200 {
            match TcpStream::connect(addr).await {
                Ok(s) => {
                    stream = Some(s);
                    break;
                }
                Err(_) => tokio::time::sleep(Duration::from_millis(10)).await,
            }
        }
        let mut stream = stream.expect("connect to the RTMP listener");
        stream.write_all(&bytes).await.expect("write publish bytes");
        stream
    }

    /// Drains (and discards) replies from `stream` until it stalls for
    /// `idle_for` or the connection closes — draining is required so the
    /// server's own writes (handshake S0/S1/S2, command-response acks) never
    /// block on an unread socket buffer.
    async fn drain_for(mut stream: TcpStream, idle_for: Duration) {
        let mut sink = [0u8; 8192];
        let _ = tokio::time::timeout(idle_for, async {
            loop {
                match stream.read(&mut sink).await {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
            }
        })
        .await;
    }

    /// Loopback biting test (issue #805 task 4): a real TCP client plays back
    /// the captured ffmpeg publish against a real [`RtmpRoute`] bound to an
    /// ephemeral loopback port, driven through the real [`run_rtmp`] ->
    /// `ListenDriver` machinery into a real `RouteHandle` — proving the whole
    /// chain (`AsyncRtmpServer` accept -> `StreamingFlvDemux` ->
    /// `LlHlsSegmenter` -> `RouteHandle`) actually moves real H.264/AAC
    /// samples end to end on the new `Listener`-backed path, and that both
    /// resolved tracks' specs land.
    ///
    /// MUTATION VERIFIED: removing the `newly_seen_samples.push((track_id,
    /// sample))` line in [`RtmpIngestSession::drain_demux`] (so every sample
    /// observed before establishment is discarded instead of buffered)
    /// makes this test fail with `panicked at
    /// multimux/src/source/rtmp.rs:851:9: RTMP publish must resolve both
    /// tracks into a registry-resolvable Trunk` — the fixture's very first
    /// media tags all arrive before the FLV demux has finished resolving
    /// both tracks (both sequence headers land in the same initial
    /// `next_events()` batch as several samples), so with the buffering
    /// removed, `established` never flips (no `NewProgram`/`Established`
    /// ever fires) and `resolve_route_trunk` never succeeds within the 10 s
    /// hang guard. Rebuilt with the mutation in place, ran `cargo test -p
    /// multimux --lib source::rtmp::tests::loopback_rtmp_publish_lands_media_in_the_trunk`,
    /// confirmed that exact panic, then reverted.
    #[tokio::test]
    async fn loopback_rtmp_publish_lands_media_in_the_trunk() {
        let addr = reserve_addr().await;
        let route = RtmpRoute::new("cam-rtmp", addr.to_string());
        let fixture = load_fixture();

        let route_handle = Arc::new(RouteHandle::new(1.0, 500, 8));
        let route_handle_for_task = Arc::clone(&route_handle);
        let run_task = tokio::spawn(async move {
            let _ = run_rtmp(&route, trunk_config(), handshake(), &route_handle_for_task).await;
        });

        let client = tokio::spawn(async move {
            let stream = connect_and_write(addr, fixture).await;
            drain_for(stream, Duration::from_secs(3)).await;
        });

        // Wait for the driver-minted Trunk to appear under SPTS_PROGRAM_ID
        // and carry both resolved track specs — the registry-resolvable
        // proof, mirroring every other driver-backed source's own loopback
        // test.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        let landed = loop {
            if let Ok(resolved) = crate::http::resolve_route_trunk(&route_handle) {
                if resolved.tracks().len() == 2 {
                    break true;
                }
            }
            if tokio::time::Instant::now() >= deadline {
                break false;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        };
        assert!(
            landed,
            "RTMP publish must resolve both tracks into a registry-resolvable Trunk"
        );

        let resolved = crate::http::resolve_route_trunk(&route_handle).expect("resolved");
        let specs = resolved.tracks();
        assert!(
            specs
                .iter()
                .any(|s| matches!(s.config, CodecConfig::Avc { .. })),
            "expected an AVC (video) track among {specs:?}"
        );
        assert!(
            specs
                .iter()
                .any(|s| matches!(s.config, CodecConfig::Aac { .. })),
            "expected an AAC (audio) track among {specs:?}"
        );

        client.abort();
        run_task.abort();
    }

    /// Biting test (issue #805 task 4, the concurrency fix): a first
    /// publisher completes the RTMP handshake (`connect`/`createStream`/
    /// `publish` all succeed — proven by
    /// `fixture_offset_after_publish_before_any_media`) and then sends
    /// nothing further; a second, well-behaved publisher connects afterward
    /// and publishes the whole real fixture. Both connect against the *same*
    /// [`RtmpRoute`]/[`run_rtmp`] call. Asserts the second publisher's media
    /// still lands — proving one stalled publisher does not block another,
    /// the defect `crate::origin::supervisor::supervise`'s sequential
    /// `connect()` await had before this port (see the module doc).
    ///
    /// MUTATION VERIFIED: changing the `AcceptOutcome::Admitted(id)` arm in
    /// `run_rtmp` so a read is only ever queued for the *first* admitted
    /// session (`if reads.is_empty() { reads.push(read_one(id, conn,
    /// read_timeout)); }`, every later session admitted but never driven)
    /// reproduces the pre-port wedge at the read-scheduling level: the
    /// second (well-behaved) publisher is admitted but its `next_events()`
    /// is never even called, so its media can never reach the demux. This
    /// makes the test fail with `panicked at multimux/src/source/rtmp.rs:967:9:
    /// the second (well-behaved) publisher's media must land even while the
    /// first publisher is stalled at handshake — one stalled publisher must
    /// not block another`. Rebuilt with the mutation in place, ran `cargo
    /// test -p multimux --lib source::rtmp::tests::second_publisher_is_served_while_first_is_stalled_at_handshake`,
    /// confirmed that exact panic, then reverted.
    #[tokio::test]
    async fn second_publisher_is_served_while_first_is_stalled_at_handshake() {
        let addr = reserve_addr().await;
        let route = RtmpRoute::new("cam-rtmp-concurrent", addr.to_string())
            .with_max_sessions(4)
            .with_timeouts(IngestTimeouts {
                connect: IngestTimeouts::default().connect,
                // Long enough that the mutation above would still be running
                // its sequential wait when this test's own (much shorter)
                // hang guard gives up.
                read: Duration::from_secs(30),
            });
        let fixture = load_fixture();
        let stall_prefix_len = fixture_offset_after_publish_before_any_media(&fixture);
        assert!(
            stall_prefix_len > 0 && stall_prefix_len < fixture.len(),
            "the publish-before-media offset must be strictly inside the fixture"
        );
        let stall_prefix = fixture[..stall_prefix_len].to_vec();

        let route_handle = Arc::new(RouteHandle::new(1.0, 500, 8));
        let route_handle_for_task = Arc::clone(&route_handle);
        let run_task = tokio::spawn(async move {
            let _ = run_rtmp(&route, trunk_config(), handshake(), &route_handle_for_task).await;
        });

        // First client: completes the handshake/connect/publish dance, then
        // holds the socket open and sends nothing further.
        let stalled_client = tokio::spawn(async move {
            let stream = connect_and_write(addr, stall_prefix).await;
            drain_for(stream, Duration::from_secs(20)).await;
        });

        // Give the stalled publisher a head start so it is genuinely
        // admitted (and stuck) before the well-behaved one connects — this
        // is what makes the test exercise "one already-stalled session"
        // rather than "two sessions racing to connect at the same instant".
        tokio::time::sleep(Duration::from_millis(200)).await;

        let good_client = tokio::spawn(async move {
            let stream = connect_and_write(addr, fixture).await;
            drain_for(stream, Duration::from_secs(3)).await;
        });

        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        let landed = loop {
            if let Ok(resolved) = crate::http::resolve_route_trunk(&route_handle) {
                if resolved.tracks().len() == 2 {
                    break true;
                }
            }
            if tokio::time::Instant::now() >= deadline {
                break false;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        };
        assert!(
            landed,
            "the second (well-behaved) publisher's media must land even while \
             the first publisher is stalled at handshake — one stalled publisher \
             must not block another"
        );

        run_task.abort();
        stalled_client.abort();
        good_client.abort();
    }

    /// Biting test (issue #805 task 4): splitting the real fixture so
    /// video's and audio's sequence headers land in SEPARATE
    /// `next_events()` reads (with a real delay in between) must still
    /// resolve BOTH tracks and land BOTH tracks' samples in the Trunk — the
    /// regression `RtmpIngestSession::drain_demux`'s buffer-before-established
    /// design exists to prevent (see `connect_resolves_both_tracks...` in
    /// this crate's pre-#805-task-4 history for the original of this test).
    ///
    /// MUTATION VERIFIED: the same removal as
    /// `loopback_rtmp_publish_lands_media_in_the_trunk`'s own mutation
    /// (dropping `newly_seen_samples.push((track_id, sample))` in
    /// `RtmpIngestSession::drain_demux` instead of buffering it) makes this
    /// test fail too, with `panicked at multimux/src/source/rtmp.rs:1063:9:
    /// both tracks must resolve even when their sequence headers land in
    /// separate reads, and neither track's samples may be dropped` — chunk 1
    /// only ever carries the first track's sequence header (by construction,
    /// split at `first_track_offset`), so with the pre-establishment buffer
    /// removed, no `Sample` from either chunk ever contributes to
    /// establishing the session and `resolve_route_trunk` never returns
    /// `Ok`. Rebuilt with the mutation in place, ran `cargo test -p multimux
    /// --lib source::rtmp::tests::first_sample_observed_during_establishment_is_not_dropped`,
    /// confirmed that exact panic, then reverted.
    #[tokio::test]
    async fn first_sample_observed_during_establishment_is_not_dropped() {
        let fixture = load_fixture();

        // Find the fixture byte offset at which the FIRST track's
        // `DemuxEvent::TrackAdded` fires, and the offset for the SECOND.
        let (first_track_offset, _second_track_offset) = {
            let mut session = ServerSession::new(ServerConfig::default());
            let mut demux = StreamingFlvDemux::new();
            let mut offsets = Vec::new();
            for (i, byte) in fixture.iter().enumerate() {
                let (_, events) = session
                    .handle_data(std::slice::from_ref(byte))
                    .expect("handle_data must not error while replaying a real capture");
                for event in events {
                    if let ServerEvent::Media { flv } = event {
                        demux
                            .feed(&flv)
                            .expect("feed must not error on a real capture");
                    }
                }
                while let Some(ev) = demux.poll_event() {
                    if matches!(ev, DemuxEvent::TrackAdded(_)) {
                        offsets.push(i + 1);
                    }
                }
                if offsets.len() >= 2 {
                    break;
                }
            }
            assert_eq!(
                offsets.len(),
                2,
                "fixture must resolve exactly two tracks (video+audio) for this test"
            );
            (offsets[0], offsets[1])
        };

        let (chunk1, chunk2) = fixture.split_at(first_track_offset);
        let chunk1 = chunk1.to_vec();
        let chunk2 = chunk2.to_vec();

        let addr = reserve_addr().await;
        let route = RtmpRoute::new("cam-rtmp-split", addr.to_string());
        let route_handle = Arc::new(RouteHandle::new(1.0, 500, 8));
        let route_handle_for_task = Arc::clone(&route_handle);
        let run_task = tokio::spawn(async move {
            let _ = run_rtmp(&route, trunk_config(), handshake(), &route_handle_for_task).await;
        });

        let client = tokio::spawn(async move {
            let mut stream = None;
            for _ in 0..200 {
                match TcpStream::connect(addr).await {
                    Ok(s) => {
                        stream = Some(s);
                        break;
                    }
                    Err(_) => tokio::time::sleep(Duration::from_millis(10)).await,
                }
            }
            let mut stream = stream.expect("connect to the RTMP listener");
            stream
                .write_all(&chunk1)
                .await
                .expect("write chunk 1 (through the first track's sequence header)");
            tokio::time::sleep(Duration::from_millis(150)).await;
            stream
                .write_all(&chunk2)
                .await
                .expect("write chunk 2 (the second track's sequence header onward)");
            drain_for(stream, Duration::from_secs(3)).await;
        });

        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        let landed = loop {
            if let Ok(resolved) = crate::http::resolve_route_trunk(&route_handle) {
                if resolved.tracks().len() == 2 {
                    break true;
                }
            }
            if tokio::time::Instant::now() >= deadline {
                break false;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        };
        assert!(
            landed,
            "both tracks must resolve even when their sequence headers land in \
             separate reads, and neither track's samples may be dropped"
        );

        run_task.abort();
        client.abort();
    }

    /// Biting test (issue #805 task 4 — preserved behaviour: "Timeouts still
    /// bound reads"): a publisher that connects and never sends a single
    /// byte (not even the RTMP handshake) must not pin its `max_sessions`
    /// slot forever — every [`RtmpConnection::next_events`] call is wrapped
    /// in [`IngestTimeouts::read`], even before establishment. With
    /// `max_sessions` set to 1, a second, well-behaved publisher can only
    /// ever be admitted once the first's slot is freed, so a passing
    /// `landed` here is only possible if the read timeout genuinely reaped
    /// the idle session — not merely that `next_events` eventually returned
    /// (which nothing here would otherwise observe).
    ///
    /// MUTATION VERIFIED: changing [`read_one`]'s
    /// `tokio::time::timeout(read_timeout, ...)` to
    /// `tokio::time::timeout(Duration::from_secs(3600), ...)` (i.e.
    /// effectively removing the read bound) makes this test fail: the idle
    /// session's read future never resolves, `max_sessions == 1` is never
    /// freed, the second publisher's connection is refused and dropped
    /// before a single byte is fed to it, and `landed` stays `false` until
    /// this test's own 10 s hang guard elapses. Rebuilt with that change, ran
    /// `cargo test -p multimux --lib source::rtmp::tests::idle_publisher_is_reaped_via_read_timeout_freeing_its_slot`,
    /// confirmed the exact `assert!(landed, ...)` panic, then reverted.
    #[tokio::test]
    async fn idle_publisher_is_reaped_via_read_timeout_freeing_its_slot() {
        let addr = reserve_addr().await;
        let route = RtmpRoute::new("cam-rtmp-idle", addr.to_string())
            .with_max_sessions(1)
            .with_timeouts(IngestTimeouts {
                connect: IngestTimeouts::default().connect,
                read: Duration::from_millis(200),
            });
        let route_handle = Arc::new(RouteHandle::new(1.0, 500, 8));
        let route_handle_for_task = Arc::clone(&route_handle);
        let run_task = tokio::spawn(async move {
            let _ = run_rtmp(&route, trunk_config(), handshake(), &route_handle_for_task).await;
        });

        // First client: connects and holds the socket open, sending
        // nothing at all -- occupies the one available slot.
        let idle_client = tokio::spawn(async move {
            let mut stream = None;
            for _ in 0..200 {
                match TcpStream::connect(addr).await {
                    Ok(s) => {
                        stream = Some(s);
                        break;
                    }
                    Err(_) => tokio::time::sleep(Duration::from_millis(10)).await,
                }
            }
            let stream = stream.expect("connect to the RTMP listener");
            tokio::time::sleep(Duration::from_secs(5)).await;
            drop(stream);
        });

        // Comfortably longer than the 200ms read timeout, so the idle
        // session has certainly already been reaped by the time the second
        // client tries to connect -- a generous margin (issue #807: not a
        // speed assertion), not a tight race.
        tokio::time::sleep(Duration::from_millis(800)).await;

        let fixture = load_fixture();
        let good_client = tokio::spawn(async move {
            let stream = connect_and_write(addr, fixture).await;
            drain_for(stream, Duration::from_secs(3)).await;
        });

        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        let landed = loop {
            if let Ok(resolved) = crate::http::resolve_route_trunk(&route_handle) {
                if resolved.tracks().len() == 2 {
                    break true;
                }
            }
            if tokio::time::Instant::now() >= deadline {
                break false;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        };
        assert!(
            landed,
            "with max_sessions == 1, the second publisher can only be served \
             once the read timeout reaps the first (idle) session's slot -- \
             if reads were unbounded, this would hang forever"
        );

        run_task.abort();
        idle_client.abort();
        good_client.abort();
    }
}
