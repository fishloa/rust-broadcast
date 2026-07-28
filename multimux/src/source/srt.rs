//! SRT (Secure Reliable Transport) ingest source (issue #739; ported onto
//! the media-plane ingress traits at plan step 5a): an [`srt_runtime::io`]
//! socket (listener or caller mode) carrying an MPEG-2 Transport Stream,
//! feeding the shared [`crate::source::ts_program::TsIngestSession`].
//!
//! Like [`crate::source::ts_udp`] and [`crate::source::ts_http`], this module
//! owns **only the transport** — all PAT/PMT/PES demuxing and
//! `DemuxEvent`→`SessionEvent` translation (including the B5 mid-stream
//! `NewProgram` handling) lives in [`crate::source::ts_program`]. Before this
//! port this file carried its own near-identical copy of that drain loop.
//!
//! SRT has two connection modes ([`draft-sharabayko-srt-01`] §3), and after
//! this port they are **no longer symmetric in how well they fit the plane**:
//!
//! - **Caller** ([`SrtRoute::new_caller`], [`run_srt_caller`]): dials out to
//!   a remote SRT listener. Fits [`Dialer`] cleanly — see below.
//! - **Listener** ([`SrtRoute::new_listener`], [`run_srt_listener_once`]):
//!   binds a UDP port and accepts inbound Callers. **Deliberately does not
//!   implement [`media_plane::ingress::Listener`]** — see
//!   [Why listener mode is not a `Listener` yet](#why-listener-mode-is-not-a-listener-yet).
//!
//! # Where the I/O boundary falls (caller mode)
//!
//! `SrtSocket::connect` **is** a genuine multi-round-trip handshake
//! (INDUCTION → CONCLUSION), and unlike [`crate::source::rtsp`] it cannot be
//! driven through `poll_transmit`/`feed`, because `srt-runtime` does not
//! expose a usable sans-IO connection type — see
//! [The srt-runtime sans-IO gap](#the-srt-runtime-sans-io-gap). But that
//! handshake is entirely *transport-opening*: it establishes the SRT
//! connection the way a TLS handshake establishes a `TlsStream`, and once it
//! completes, the whole remaining protocol is "payloads arrive, demux them".
//! So [`SrtDialer::dial`] constructs the sans-IO session and performs no
//! I/O, and [`connect_caller`]/[`run_srt_caller`] — the tokio-side driver —
//! opens the socket and feeds payloads in, exactly as
//! [`crate::source::ts_http`] does with its GET.
//!
//! # The `srt-runtime` sans-IO gap
//!
//! This is worth recording precisely, because at first glance SRT *looks*
//! like the RTSP case (where a sans-IO engine already existed and the whole
//! handshake moved onto `poll_transmit`/`feed` with no executor bridge).
//! `srt-runtime` **does** ship a large sans-IO core —
//! `srt_runtime::caller::CallerHandshake` / `listener::ListenerHandshake`
//! (`start()`/`feed(&ControlPacket)`/`tick()` → `HandshakeOutput::Send(..)`),
//! plus sans-IO `arq::Sender`/`arq::Receiver`, `tsbpd::TsbpdScheduler` and
//! `livecc::LiveCC`. But `srt_runtime::io` composes all of those inside a
//! **private** `Driver` struct owning the `UdpSocket` on a spawned task, and
//! the only public handle, `SrtSocket`, is a pair of `mpsc` channels — its
//! `recv()` is a channel read, not a byte-level API. There is **no public
//! "feed a datagram in, get a payload out" SRT connection type**, so an
//! `IngestSession` that owned the SRT protocol itself would have to
//! re-compose `ArqReceiver` + `TsbpdScheduler` + `LiveCC` and duplicate
//! `Driver` wholesale. That is a real, bounded piece of work in
//! `srt-runtime` (not `multimux`), and it is the prerequisite for both a
//! genuinely sans-IO SRT caller and the listener redesign below.
//!
//! # Why listener mode is not a `Listener` yet
//!
//! [`media_plane::ingress::Listener`] is `poll_accept() -> Result<Option<Session>>`
//! — a **non-blocking** poll, so [`media_plane::ingress::ListenDriver`] can
//! admit and drive up to `max_sessions` concurrently. `SrtListener::accept`
//! is `pub async fn accept(&mut self)` — a blocking `.await` with no
//! `poll_accept`/`try_accept` variant — and it takes `&mut self`, so it
//! cannot even be shared without a `Mutex`. Worse, internally the listener
//! and *every* accepted connection share one `UdpSocket` (`drain_completed`
//! hands each new connection an `Arc::clone` of it), so "accept another
//! while N are live" is not a `multimux` concern at all: it is a
//! demultiplexing responsibility inside `srt-runtime`.
//!
//! So this module keeps listener mode at today's semantics —
//! [`run_srt_listener_once`] accepts exactly one Caller and drives it, which
//! is precisely what the pre-5a `connect()` did — and does **not** pretend
//! to satisfy a trait it cannot honour. The scoped cost of closing this is
//! recorded in this crate's CHANGELOG rather than guessed at here.
//!
//! Encrypted SRT (the SEK-wrapped payload encryption `draft-sharabayko-srt-01`
//! §6 negotiates) remains **out of scope**: [`srt_runtime::io`] does not
//! apply the SEK to decrypt DATA payloads, so this source carries no
//! passphrase field.
//!
//! [`draft-sharabayko-srt-01`]: https://datatracker.ietf.org/doc/html/draft-sharabayko-srt-01

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use broadcast_common::Timestamp;
use media_plane::ingress::{Dialer, HandshakePolicy, IngestDriver};
use media_plane::trunk::TrunkConfig;
use srt_runtime::HandshakeConfig;
use srt_runtime::io::{SrtListener, SrtSocket};
use tokio::sync::{Mutex, OnceCell};

use crate::error::{MultimuxError, Result};
use crate::source::ts_program::TsIngestSession;
use crate::source::{IngestTimeouts, Source};

/// An SRT-over-MPEG-TS route, in either listener (bind + accept) or caller
/// (dial out) mode — see the module doc. Replaces the old (pre-5a)
/// `SrtSource`.
pub struct SrtRoute {
    name: String,
    /// Listener bind address (`Some`) — mutually exclusive with `remote`.
    /// Enforced by [`crate::config::InputSpec::Srt`]'s `validate`.
    listen: Option<String>,
    /// Caller dial-out address (`Some`) — mutually exclusive with `listen`.
    remote: Option<String>,
    stream_id: Option<String>,
    latency_ms: Option<u16>,
    timeouts: IngestTimeouts,
    /// Bind-once, reuse-forever (listener mode only) — see the module doc.
    /// [`SrtListener::accept`] takes `&mut self`, hence the `Mutex` (an
    /// `OnceCell` alone only gives shared access).
    listener: OnceCell<Arc<Mutex<SrtListener>>>,
}

/// Manual `Debug`: `listener`'s inner `SrtListener` has no `Debug` impl of
/// its own to derive over.
impl std::fmt::Debug for SrtRoute {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SrtRoute")
            .field("name", &self.name)
            .field("listen", &self.listen)
            .field("remote", &self.remote)
            .field("stream_id", &self.stream_id)
            .field("latency_ms", &self.latency_ms)
            .finish()
    }
}

impl SrtRoute {
    /// Build a listener-mode route: binds `listen` (lazily, on first accept)
    /// and accepts the next Caller — see the module doc.
    pub fn new_listener(name: impl Into<String>, listen: impl Into<String>) -> Self {
        SrtRoute {
            name: name.into(),
            listen: Some(listen.into()),
            remote: None,
            stream_id: None,
            latency_ms: None,
            timeouts: IngestTimeouts::default(),
            listener: OnceCell::new(),
        }
    }

    /// Build a caller-mode route: dials `remote` fresh on every connect.
    pub fn new_caller(name: impl Into<String>, remote: impl Into<String>) -> Self {
        SrtRoute {
            name: name.into(),
            listen: None,
            remote: Some(remote.into()),
            stream_id: None,
            latency_ms: None,
            timeouts: IngestTimeouts::default(),
            listener: OnceCell::new(),
        }
    }

    /// Sets the Stream ID to advertise (`draft-sharabayko-srt-01` §3.2.1.3)
    /// — caller mode only.
    #[must_use]
    pub fn with_stream_id(mut self, id: Option<String>) -> Self {
        self.stream_id = id;
        self
    }

    /// Overrides the negotiated TSBPD latency (`draft-sharabayko-srt-01`
    /// §4.3.1.2); `None` keeps [`HandshakeConfig::default`]'s value.
    #[must_use]
    pub fn with_latency_ms(mut self, ms: Option<u16>) -> Self {
        self.latency_ms = ms;
        self
    }

    /// Overrides the default [`IngestTimeouts`].
    #[must_use]
    pub fn with_timeouts(mut self, timeouts: IngestTimeouts) -> Self {
        self.timeouts = timeouts;
        self
    }

    fn handshake_config(&self) -> HandshakeConfig {
        let mut cfg = HandshakeConfig {
            stream_id: self.stream_id.clone(),
            ..HandshakeConfig::default()
        };
        if let Some(latency) = self.latency_ms {
            cfg.latency_ms = latency;
        }
        cfg
    }
}

impl Source for SrtRoute {
    fn stream_name(&self) -> &str {
        &self.name
    }
}

/// Constructs a [`TsIngestSession`] — performs **no I/O**. The SRT
/// handshake lives in [`connect_caller`]/[`accept_listener`]; see the module
/// doc for why it cannot move onto `poll_transmit`/`feed`.
#[derive(Clone, Copy, Debug, Default)]
pub struct SrtDialer;

impl Dialer for SrtDialer {
    type Session = TsIngestSession;
    /// Construction cannot fail — the dial itself belongs to
    /// [`connect_caller`], the I/O side.
    type Error = Infallible;

    fn dial(&mut self) -> core::result::Result<TsIngestSession, Infallible> {
        Ok(TsIngestSession::new())
    }
}

/// Dials `route`'s remote SRT listener (caller mode), bounded by
/// [`IngestTimeouts::connect`] — **transport-opening I/O, deliberately
/// outside `dial()`**.
///
/// The whole dial is wrapped in the timeout (rather than the timeout only
/// bounding a subsequent read) because `srt-runtime` runs its own internal
/// handshake-retry budget: a `tokio::time::timeout` around a later step
/// would never get a chance to interrupt a dial against a blackholed remote,
/// since the dial future runs to its own completion first (issue #739
/// review).
pub async fn connect_caller(route: &SrtRoute) -> Result<SrtSocket> {
    let remote = route
        .remote
        .as_ref()
        .ok_or_else(|| MultimuxError::Connect {
            reason: "srt: connect_caller on a listener-mode route".into(),
        })?;
    let cfg = route.handshake_config();
    let connect_timeout = route.timeouts.connect;
    match tokio::time::timeout(connect_timeout, SrtSocket::connect(remote.as_str(), cfg)).await {
        Ok(Ok(sock)) => Ok(sock),
        Ok(Err(e)) => Err(MultimuxError::Connect {
            reason: format!("srt: connect {remote}: {e}"),
        }),
        Err(_) => Err(MultimuxError::Connect {
            reason: format!("srt: connect {remote}: no response within {connect_timeout:?}"),
        }),
    }
}

/// Binds `route`'s listener (once ever, lazily) and accepts the next inbound
/// Caller.
///
/// Deliberately **unbounded**: waiting for an inbound Caller to show up is
/// idle (nothing is wrong), not stalled — the same reasoning the pre-5a
/// `connect()` used, preserved so the accept side's behaviour is unchanged
/// by this port. The post-accept read is bounded by
/// [`IngestTimeouts::read`] in [`recv_and_feed`].
pub async fn accept_listener(route: &SrtRoute) -> Result<SrtSocket> {
    let listen = route
        .listen
        .as_ref()
        .ok_or_else(|| MultimuxError::Connect {
            reason: "srt: accept_listener on a caller-mode route".into(),
        })?;
    let cfg = route.handshake_config();
    let listener = route
        .listener
        .get_or_try_init(|| async {
            SrtListener::bind(listen.as_str(), cfg.clone())
                .await
                .map(|l| Arc::new(Mutex::new(l)))
                .map_err(|e| MultimuxError::Connect {
                    reason: format!("srt: bind {listen}: {e}"),
                })
        })
        .await?;

    // Guard dropped at the end of this call, before any read loop — an
    // accepted-but-unresolved connection must never hold the listener lock.
    let mut guard = listener.lock().await;
    guard.accept().await.map_err(|e| MultimuxError::Connect {
        reason: format!("srt: accept: {e}"),
    })
}

/// What one [`recv_and_feed`] call observed on the SRT socket.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum StreamStatus {
    /// A payload was read and fed to the driver.
    Fed,
    /// The peer shut the connection down cleanly — not an error.
    Ended,
}

/// Reads the next SRT payload (bounded by `read_timeout`) and feeds it to
/// `driver`.
///
/// # Errors
/// A read stall or a socket error — both genuine failures, distinct from the
/// clean [`StreamStatus::Ended`].
pub async fn recv_and_feed(
    sock: &mut SrtSocket,
    driver: &mut IngestDriver<TsIngestSession>,
    read_timeout: Duration,
    now: Timestamp,
) -> Result<StreamStatus> {
    let payload = tokio::time::timeout(read_timeout, sock.recv())
        .await
        .map_err(|_| MultimuxError::Connect {
            reason: format!("srt recv: no data within {read_timeout:?}"),
        })?
        .map_err(|e| MultimuxError::Connect {
            reason: format!("srt recv: {e}"),
        })?;
    let Some(bytes) = payload else {
        return Ok(StreamStatus::Ended);
    };
    driver.feed(&bytes, now);
    Ok(StreamStatus::Fed)
}

/// Drives an already-open `sock` through a fresh [`TsIngestSession`] until
/// the peer shuts down or a read fails — shared by both modes, since once
/// the socket exists caller and listener are identical.
///
/// `on_driver` is invoked once, immediately after the **first** payload has
/// been fed, so a caller can subscribe to a [`media_plane::trunk::Trunk`]
/// the moment one exists: a `Trunk` subscription starts from *now* and sees
/// no backlog, so subscribing only after this function returns would observe
/// nothing.
///
/// Returns `Ok(())` on a clean peer shutdown (the driver is left
/// [`media_plane::ingress::HealthState::Ended`]) and `Err` on a genuine
/// failure.
///
/// `route_handle` is the driver-backed registry side of issue #805 task 2 —
/// see `crate::source::rtsp::run_rtsp`'s own doc for what
/// `crate::source::report_driver_progress` does with it each iteration
/// (called alongside, not instead of, `on_driver`).
pub async fn drive_socket(
    mut sock: SrtSocket,
    read_timeout: Duration,
    trunk_config: TrunkConfig,
    handshake: HandshakePolicy,
    route_handle: &std::sync::Arc<crate::route::RouteHandle>,
    on_driver: impl FnOnce(&IngestDriver<TsIngestSession>),
) -> Result<()> {
    let mut dialer = SrtDialer;
    let session = dialer
        .dial()
        .unwrap_or_else(|never: Infallible| match never {});
    let mut driver = IngestDriver::new(
        session,
        trunk_config,
        handshake,
        media_plane::DEFAULT_MAX_PROGRAMS,
    );
    let start = std::time::Instant::now();
    let mut handoff = Some(on_driver);
    let mut published = std::collections::HashSet::new();
    loop {
        let now = Timestamp::from_instant(start, std::time::Instant::now());
        let status = recv_and_feed(&mut sock, &mut driver, read_timeout, now).await?;
        crate::source::report_driver_progress(&driver, route_handle, &mut published);
        if let Some(f) = handoff.take() {
            f(&driver);
        }
        if status == StreamStatus::Ended {
            driver.finish();
            return Ok(());
        }
    }
}

/// Caller mode: dial `route`'s remote and drive it — the new drive loop,
/// replacing the pre-5a `SrtSource::connect`/`SrtSession::next_samples`
/// pair.
pub async fn run_srt_caller(
    route: &SrtRoute,
    trunk_config: TrunkConfig,
    handshake: HandshakePolicy,
    route_handle: &std::sync::Arc<crate::route::RouteHandle>,
) -> Result<()> {
    let sock = connect_caller(route).await?;
    drive_socket(
        sock,
        route.timeouts.read,
        trunk_config,
        handshake,
        route_handle,
        |_| {},
    )
    .await
}

/// Listener mode: accept **exactly one** inbound Caller and drive it, then
/// return — today's semantics, unchanged by this port. See
/// [Why listener mode is not a `Listener` yet](self#why-listener-mode-is-not-a-listener-yet)
/// for why this is not `poll_accept`-shaped.
pub async fn run_srt_listener_once(
    route: &SrtRoute,
    trunk_config: TrunkConfig,
    handshake: HandshakePolicy,
    route_handle: &std::sync::Arc<crate::route::RouteHandle>,
) -> Result<()> {
    let sock = accept_listener(route).await?;
    drive_socket(
        sock,
        route.timeouts.read,
        trunk_config,
        handshake,
        route_handle,
        |_| {},
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::ts_program::test_support::{build_ts_bytes, handshake, trunk_config};
    use media_plane::ingress::ProgramId;
    use media_plane::trunk::{SampleCursor, SampleCursorItem};
    use std::sync::Mutex as StdMutex;

    /// Counts every sample a cursor yields — the `Trunk`-side replacement
    /// for the pre-5a tests' `MediaStore::init_bytes()`/`window_segments()`
    /// assertions. The property under test is unchanged (real H.264 samples
    /// travelled the whole chain: SRT socket → `StreamingTsDemux` →
    /// `SessionEvent::Sample` → `Trunk`); only the sink moved, because step
    /// 4 deleted `MediaStore` and a step-5a source publishes samples, not
    /// segments (see this crate's CHANGELOG on the segmenter gap).
    fn drain(cursor: &mut SampleCursor) -> usize {
        let mut n = 0;
        while let Some(item) = cursor.poll() {
            if matches!(item, SampleCursorItem::Timed { .. }) {
                n += 1;
            }
        }
        n
    }

    /// Loopback biting test (issue #739): a real `SrtSocket` caller connects
    /// to a real listener-mode `SrtRoute` bound to an ephemeral loopback
    /// port and sends a muxed TS stream in SRT-payload-sized (~1316-byte,
    /// TS-packet-aligned) chunks — proving the whole chain
    /// (`SrtListener::accept` → `StreamingTsDemux` → `Trunk`) actually moves
    /// real H.264 samples, not just that the track set resolves.
    ///
    /// MUTATION-CHECKED: make `recv_and_feed` drop the payload (skip
    /// `driver.feed`) and this test's sample-count assertion fails — nothing
    /// ever reaches the `Trunk`.
    #[tokio::test]
    async fn loopback_srt_listener_publish_lands_samples_in_trunk() {
        let reserved = tokio::net::UdpSocket::bind("127.0.0.1:0")
            .await
            .expect("reserve port");
        let addr = reserved.local_addr().expect("local addr");
        drop(reserved);

        // A short read timeout: `SrtSocket::drop` does not send the peer an
        // explicit SRT Shutdown control packet, so once the client stops
        // sending, the drive loop would otherwise wait the full default
        // 30 s read timeout before returning.
        let route =
            SrtRoute::new_listener("cam-srt", addr.to_string()).with_timeouts(IngestTimeouts {
                connect: IngestTimeouts::default().connect,
                read: Duration::from_millis(300),
            });
        let ts_bytes = build_ts_bytes(1, 0xAA, 40);

        let client = tokio::spawn(async move {
            let cfg = HandshakeConfig::default();
            let mut sock = None;
            for _ in 0..200 {
                match SrtSocket::connect(addr, cfg.clone()).await {
                    Ok(s) => {
                        sock = Some(s);
                        break;
                    }
                    Err(_) => tokio::time::sleep(Duration::from_millis(10)).await,
                }
            }
            let mut sock = sock.expect("connect to SrtRoute's listener");
            for chunk in ts_bytes.chunks(7 * 188) {
                sock.send(chunk).await.expect("send TS payload");
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        });

        let cursor: Arc<StdMutex<Option<SampleCursor>>> = Arc::new(StdMutex::new(None));
        let cursor_for_cb = Arc::clone(&cursor);
        let sock = tokio::time::timeout(Duration::from_secs(10), accept_listener(&route))
            .await
            .expect("accept must not hang")
            .expect("accept");
        let route_handle = Arc::new(crate::route::RouteHandle::new(4.0, 500, 4));
        // Drive to completion: the client stops sending, so the 300 ms read
        // timeout ends the loop with an `Err` — expected here, and the
        // samples have already landed by then.
        let _ = tokio::time::timeout(
            Duration::from_secs(10),
            drive_socket(
                sock,
                Duration::from_millis(300),
                trunk_config(),
                handshake(),
                &route_handle,
                move |driver| {
                    if let Some(t) = driver.trunk(ProgramId(0)) {
                        *cursor_for_cb.lock().unwrap() = Some(t.subscribe());
                    }
                },
            ),
        )
        .await;

        let mut guard = cursor.lock().unwrap();
        let total = guard.as_mut().map(drain).unwrap_or(0);
        assert!(
            total > 0,
            "SRT publish must land real samples in the Trunk, got {total}"
        );
        client.abort();
    }

    /// Loopback test for caller mode (issue #739): a real test-owned
    /// `SrtListener` accepts, and a caller-mode `SrtRoute` dials out to it —
    /// proving the caller-mode dial path also resolves real tracks and
    /// delivers real samples into the `Trunk`.
    #[tokio::test]
    async fn loopback_srt_caller_connects_and_yields_samples() {
        let listener_addr = "127.0.0.1:0".parse::<std::net::SocketAddr>().unwrap();
        let mut listener = SrtListener::bind(listener_addr, HandshakeConfig::default())
            .await
            .expect("listener bind");
        let bound_addr = listener.local_addr().expect("listener local addr");

        let ts_bytes = build_ts_bytes(1, 0xBB, 40);
        let server = tokio::spawn(async move {
            let mut sock = listener.accept().await.expect("listener accept");
            for chunk in ts_bytes.chunks(7 * 188) {
                sock.send(chunk).await.expect("send TS payload");
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        });

        let route = SrtRoute::new_caller("cam-srt-caller", bound_addr.to_string()).with_timeouts(
            IngestTimeouts {
                connect: Duration::from_secs(5),
                read: Duration::from_millis(300),
            },
        );
        let sock = tokio::time::timeout(Duration::from_secs(10), connect_caller(&route))
            .await
            .expect("caller dial must not hang")
            .expect("caller dial");

        let cursor: Arc<StdMutex<Option<SampleCursor>>> = Arc::new(StdMutex::new(None));
        let cursor_for_cb = Arc::clone(&cursor);
        let route_handle = Arc::new(crate::route::RouteHandle::new(4.0, 500, 4));
        let _ = tokio::time::timeout(
            Duration::from_secs(10),
            drive_socket(
                sock,
                Duration::from_millis(300),
                trunk_config(),
                handshake(),
                &route_handle,
                move |driver| {
                    if let Some(t) = driver.trunk(ProgramId(0)) {
                        *cursor_for_cb.lock().unwrap() = Some(t.subscribe());
                    }
                },
            ),
        )
        .await;

        let mut guard = cursor.lock().unwrap();
        let total = guard.as_mut().map(drain).unwrap_or(0);
        assert!(
            total > 0,
            "caller-mode SRT must land real samples in the Trunk, got {total}"
        );
        server.abort();
    }

    /// A publisher that holds the socket open but goes idle must not hang
    /// the drive loop forever — issue #663 P5.2, preserved from the pre-5a
    /// test of the same intent (now asserting on `run_srt_caller`'s `Err`
    /// rather than `next_samples`'s).
    #[tokio::test]
    async fn drive_socket_times_out_when_source_goes_silent() {
        const READ_TIMEOUT: Duration = Duration::from_millis(150);

        let listener_addr = "127.0.0.1:0".parse::<std::net::SocketAddr>().unwrap();
        let mut listener = SrtListener::bind(listener_addr, HandshakeConfig::default())
            .await
            .expect("listener bind");
        let bound_addr = listener.local_addr().expect("listener local addr");

        let ts_bytes = build_ts_bytes(1, 0xCC, 10);
        let server = tokio::spawn(async move {
            let mut sock = listener.accept().await.expect("listener accept");
            for chunk in ts_bytes.chunks(7 * 188) {
                sock.send(chunk).await.expect("send TS payload");
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            // Hold the socket open, but send nothing further.
            tokio::time::sleep(Duration::from_secs(30)).await;
        });

        let route = SrtRoute::new_caller("cam-srt-idle", bound_addr.to_string()).with_timeouts(
            IngestTimeouts {
                connect: Duration::from_secs(5),
                read: READ_TIMEOUT,
            },
        );
        let route_handle = Arc::new(crate::route::RouteHandle::new(4.0, 500, 4));
        let outcome = tokio::time::timeout(
            Duration::from_secs(10),
            run_srt_caller(&route, trunk_config(), handshake(), &route_handle),
        )
        .await
        .expect("run_srt_caller must return via IngestTimeouts::read, not hang");
        assert!(
            outcome.is_err(),
            "an idle-but-open publisher must surface a read-timeout error"
        );
        server.abort();
    }

    /// A caller dialling a bound-but-never-replying remote must fail within
    /// the *configured* connect timeout, not srt-runtime's own internal
    /// handshake-retry budget — the biting test for wrapping the dial in the
    /// timeout (issue #739 review). Preserved in intent from the pre-5a
    /// suite, retargeted at `connect_caller`.
    #[tokio::test]
    async fn caller_connect_times_out_against_an_unreachable_remote() {
        const CONNECT_TIMEOUT: Duration = Duration::from_millis(200);
        // A bound UDP socket that never answers the SRT handshake.
        let blackhole = tokio::net::UdpSocket::bind("127.0.0.1:0")
            .await
            .expect("bind blackhole");
        let addr = blackhole.local_addr().expect("local addr");

        let route =
            SrtRoute::new_caller("cam-srt-dead", addr.to_string()).with_timeouts(IngestTimeouts {
                connect: CONNECT_TIMEOUT,
                read: Duration::from_secs(30),
            });
        let started = std::time::Instant::now();
        let outcome = tokio::time::timeout(Duration::from_secs(10), connect_caller(&route))
            .await
            .expect("connect_caller must return on its own, not hang");
        assert!(outcome.is_err(), "a blackholed remote must fail the dial");
        assert!(
            started.elapsed() < CONNECT_TIMEOUT * 4,
            "the dial must be bounded by the CONFIGURED connect timeout, not \
             srt-runtime's internal retry budget: took {:?}",
            started.elapsed()
        );
    }
}
