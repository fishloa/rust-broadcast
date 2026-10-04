//! WHIP (RFC 9725) push ingest source (issue #740).
//!
//! Accepts an inbound WHIP publisher: an HTTP `POST` carrying an SDP offer
//! (`application/sdp`), answered with a `201 Created` carrying this side's
//! SDP answer, after which media flows over ICE + DTLS-SRTP —
//! [`webrtc_runtime::media::MediaTransport`] is the transport this module
//! drives; SDP itself (offer parsing, answer construction) is out of that
//! crate's scope (see its module doc) and is handled here, over
//! [`sdp_types::Session`] (the same SDP crate `crate::source::sdp` uses for
//! RTSP DESCRIBE bodies) for structure, plus a small amount of manual line
//! scanning for the WebRTC-specific attributes (`a=ice-ufrag`, `a=setup`,
//! `a=candidate`, …) neither `sdp_types` nor `crate::source::sdp` models.
//!
//! # Scope: video (H.264) only in this cut
//!
//! A WHIP publisher's audio track is essentially always Opus (RFC 7587) —
//! browsers do not send AAC over RTP. `transmux::RtpStreamDepacketiser` only
//! depayloads RFC 6184 (H.264) and RFC 3640 (AAC); there is no Opus RTP
//! depacketiser anywhere in this workspace, and adding one is out of scope
//! for this crate (`transmux` is not touched here). An offer containing no
//! `m=video` section — or more than one `m=` section at all — is rejected
//! with [`MultimuxError::Sdp`] rather than silently dropping the media it
//! can't carry. Follow-up: an RFC 7587 Opus depacketiser in `transmux`.
//!
//! # Why the `avcC` config is captured from the bitstream, not the SDP
//!
//! [`crate::source::sdp::parse_sdp_tracks`] (RTSP/raw-RTP-over-UDP) builds
//! `CodecConfig::Avc` from the SDP's `a=fmtp` `sprop-parameter-sets`
//! (RFC 6184 §8.2) — the RTSP-world convention. A browser's WHIP offer
//! generally omits it: WebRTC senders carry SPS/PPS **in-band** instead, as
//! a STAP-A aggregation packet ahead of each IDR (the same RFC 6184 §5.1,
//! just the other of its two documented ways to convey parameter sets).
//! Requiring `sprop-parameter-sets` here would reject a real browser's offer
//! outright. Instead, [`WhipIngestSession`] depayloads immediately (its
//! `avcC` config is never consulted by `RtpStreamDepacketiser::push` itself —
//! only carried through to the caller, per that type's own doc), and defers
//! announcing [`SessionEvent::NewProgram`] until it has scanned a real IDR's
//! length-prefixed NAL units for a genuine SPS (type 7) + PPS (type 8) pair,
//! from which [`transmux::avc_config_from_sps_pps`] builds the real
//! [`transmux::AVCConfigurationBox`] — mirrors `crate::source::rtmp`'s own
//! "`Established` gates on the first `Sample`" precedent (see that module's
//! doc), just gated on "first sample with real parameter sets" rather than
//! merely "first sample". Every sample observed before that gate fires is
//! buffered and re-emitted immediately after, exactly like RTMP's
//! `newly_seen_samples` — nothing is ever dropped waiting for it. This
//! crate's own engineering discipline (never fabricate a spec value) rules
//! out shipping a placeholder/empty `avcC` in the announced `TrackSpec`.
//!
//! # Listener shape
//!
//! Exactly like `crate::source::rtmp`: WHIP is an inbound publisher, so this
//! is a [`Listener`], not a `media_plane::ingress::Dialer`. The HTTP
//! POST/201 exchange is the "handshake"; `WhipRoute::ensure_infra` (private)
//! binds the listen socket once and spawns an accept-pump task, mirroring
//! `RtmpRoute::ensure_infra` almost exactly. Unlike RTMP, the accepted
//! "connection" the pump hands off is not a live socket still being read —
//! by the time an admitted session reaches the channel, the SDP exchange is
//! already complete (a real HTTP request/response, entirely orthogonal to
//! the media session that follows) and what is queued is the *result*: a
//! bound ephemeral [`UdpSocket`], the negotiated [`MediaTransport`], and the
//! offer's track set.
//!
//! [`WhipIngestSession::feed`]'s `Stage::In` is `&'a [u8]` — one *decrypted*
//! RTP packet's reconstructed wire bytes (this module's own private
//! `rebuild_rtp_wire`), the same shape `crate::source::rtp_udp` feeds its
//! depacketiser with — so, unlike RTMP (`Stage::In = &'a [ServerEvent]`),
//! [`run_whip`] can use [`ListenDriver::feed`]'s plain `&[u8]` convenience
//! wrapper rather than the `driver_mut`/`driver`/`reap_if_terminal` triple.
//! The ICE/DTLS/SRTP machinery that produces those bytes from a raw UDP
//! datagram lives entirely in [`run_whip`]'s own private per-session read
//! loop (`read_one`), never in the [`Stage`] impl itself — exactly the same
//! split RTMP draws between `RtmpConnection::next_events` (I/O) and
//! `RtmpIngestSession::feed` (sans-IO translation).

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

use broadcast_common::{Demand, Stage, Timestamp};
use futures_util::StreamExt;
use futures_util::stream::FuturesUnordered;
use tokio::net::{TcpListener, UdpSocket};
use tokio::sync::{Notify, OnceCell, mpsc};

use media_plane::ingress::{
    AcceptOutcome, HandshakePolicy, IngestSession, ListenDriver, Listener, ProgramId, SessionEvent,
    SessionId,
};
use media_plane::trunk::{RetentionClass, TrunkConfig};

use transmux::pipeline::{CodecConfig, Sample, TrackSpec};
use transmux::{
    AVCConfigurationBox, AVCDecoderConfigurationRecord, AvcPps, AvcSps, RtpStreamDepacketiser,
    RtpStreamTrack, avc_config_from_sps_pps,
};

use webrtc_runtime::media::{
    Datagram, MAX_REMOTE_CANDIDATES, MediaEvent, MediaTransport, MediaTransportConfig, SetupRole,
    parse_remote_fingerprint,
};

use crate::error::{MultimuxError, Result};
use crate::route::RouteHandle;
use crate::source::SessionClocks;
#[cfg(feature = "test-hooks")]
use crate::source::handshake_policy;
use crate::source::{DriverProgress, IngestTimeouts, Source};
use axum::response::IntoResponse as _;

/// Unknown coded dimensions — the SDP/RTP path gives no frame geometry at
/// all, matching `crate::source::sdp`'s own `UNKNOWN_DIMENSION` placeholder
/// for `CodecConfig::Avc` (that field genuinely isn't derivable here either;
/// this is the same accepted convention, not a new fabrication).
const UNKNOWN_DIMENSION: u16 = 0;

/// H.264 NAL unit type field mask (ISO/IEC 14496-10 §7.3.1 `nal_unit_type`,
/// 5 bits).
const NAL_TYPE_MASK: u8 = 0x1F;
/// Sequence parameter set NAL unit type (ISO/IEC 14496-10 Table 7-1).
const NAL_TYPE_SPS: u8 = 7;
/// Picture parameter set NAL unit type (ISO/IEC 14496-10 Table 7-1).
const NAL_TYPE_PPS: u8 = 8;
/// AVCC-style NAL length-prefix width (`length_size_minus_one` = 3, i.e. a
/// 4-byte length) — matches `transmux::rtp::reassemble_video`'s own encoding
/// of the AUs [`RtpStreamDepacketiser::push`] emits.
const NAL_LENGTH_PREFIX: usize = 4;

/// RFC 3550 §5.1 fixed RTP header length (before any CSRC/extension) — used
/// only to read the payload-type byte before depayloading.
const RTP_MIN_HEADER_LEN: usize = 12;
/// Mask for the 7-bit payload-type field (RTP header byte 1, bit 7 is the
/// marker bit).
const RTP_PT_MASK: u8 = 0x7F;
/// Mask for the RTP version+padding+extension+CSRC-count byte (header byte
/// 0) this module preserves verbatim when rebuilding wire bytes from a
/// decrypted, already-parsed [`webrtc_runtime::media::DecryptedRtp`] — always
/// `2` (version) with no padding/extension/CSRC beyond what the CSRC list
/// itself already carries, per RFC 3550 §5.1.
const RTP_VERSION_BYTE: u8 = 0x80;
/// RFC 3550 §5.1 `X` (extension) bit in the first header byte.
const RTP_EXTENSION_BIT: u8 = 0x10;
/// RFC 3550 §5.3.1: the extension `length` field counts 32-bit words.
const RTP_EXTENSION_WORD_LEN: usize = 4;

/// Default cap on concurrently admitted WHIP publishers per route, mirroring
/// [`crate::source::rtmp::DEFAULT_RTMP_MAX_SESSIONS`]'s own reasoning: generous
/// for one origin's worth of encoders while still bounding a flood of inbound
/// connections.
pub const DEFAULT_WHIP_MAX_SESSIONS: usize = 16;

/// Bound on the accept-pump task's channel — see
/// `crate::source::rtmp::ACCEPT_QUEUE_CAPACITY`'s identical reasoning.
const ACCEPT_QUEUE_CAPACITY: usize = 32;

/// Max UDP datagram this source reads in one `recv` — matches
/// `crate::source::rtp_udp::MAX_UDP_DATAGRAM`.
const MAX_UDP_DATAGRAM: usize = 65_536;

/// One negotiated WHIP session, handed from the HTTP accept-pump to
/// [`WhipListener::poll_accept`]: the SDP exchange is already complete by
/// the time this exists (see the module doc) — what remains is the media
/// session itself. The [`MediaTransport`] is owned outright (SP6.3): the
/// session task's read loop takes it by value, never through a lock.
struct AdmittedWhip {
    socket: Arc<UdpSocket>,
    media: MediaTransport,
    tracks: Vec<WhipTrack>,
}

/// One `m=video` track resolved from a WHIP offer.
#[derive(Clone)]
struct WhipTrack {
    track_id: u32,
    payload_type: u8,
    clock_rate: u32,
}

/// Bind-once, reuse-forever infrastructure — see [`WhipRoute::ensure_infra`]
/// and `crate::source::rtmp::RtmpInfra`'s identical shape.
struct WhipInfra {
    accept_rx: Arc<StdMutex<mpsc::Receiver<AdmittedWhip>>>,
    /// Sessions admitted (SDP answered, media socket bound) but not yet
    /// reaped — checked against `max_sessions` in `whip_post`
    /// *before* any of that per-connection work happens (issue r07-C11):
    /// `media_plane::ingress::ListenDriver`'s own `max_sessions` cap only
    /// takes effect once a session reaches `poll_accept`, by which point the
    /// UDP bind and ICE/DTLS setup already ran for nothing.
    active_sessions: Arc<AtomicUsize>,
    /// Cancels the signalling server when the route is dropped, so the bound
    /// port is released.
    cancel: tokio_util::sync::CancellationToken,
    /// Tracks the signalling-server task.
    tracker: tokio_util::task::TaskTracker,
    /// Count of `MediaTransport::handle_timeout` fires driven by the timer
    /// arm (per route — a global would leak across tests).
    timer_fires: Arc<AtomicU64>,
    /// Wakes a `wait_timer_fire` waiter when the counter bumps.
    timer_notify: Arc<Notify>,
    /// Wakes [`run_whip`]'s accept drain when a session is admitted (I2).
    admit_notify: Arc<Notify>,
    /// Monotonic count of sessions the DRIVER (`poll_accept`) has admitted
    /// (N2): distinct from `active_sessions`, which the HTTP handler also
    /// touches — a test that reverts the I2 admit-drain fix would leave this
    /// at 0 and fail, whereas `active_sessions` stays 2 from the handler.
    admitted_total: Arc<AtomicUsize>,
    /// A SEPARATE wake for the test observer of `admitted_total` (mirrors
    /// RTMP's `admit_count_notify`), so the observer is never woken on the
    /// driver's own `admit_notify`.
    admit_count_notify: Arc<Notify>,
    /// Number of live `last_datagram` (idle-clock) entries the driver holds
    /// (minor review finding): a reaped session must drop its entry, else this
    /// gauge climbs one per reaped session. Test-only observable.
    last_datagram_entries: Arc<AtomicUsize>,
    /// Test-only: staging a [`MediaEvent::TimerError`] to be surfaced on the
    /// next `handle_timeout` drive of a freshly-admitted session's transport
    /// (finding N3's session-end test seam — a real ICE/DTLS timer failure is
    /// not stageable through the public API). Consumed once by `run_whip`.
    #[cfg(feature = "test-hooks")]
    force_timer_error: Arc<StdMutex<Option<String>>>,
}

impl WhipInfra {
    /// Stop the signalling server and wait for it to drain (releasing the
    /// bound port). Used by `Drop` and by the accept-lifecycle tests.
    #[allow(dead_code)]
    async fn shutdown(&self) {
        self.cancel.cancel();
        self.tracker.close();
        self.tracker.wait().await;
    }
}

/// A WHIP push-ingest route: binds an HTTP listen socket once and accepts
/// publishers against it. See the module doc.
pub struct WhipRoute {
    name: String,
    listen: String,
    /// A caller-supplied, already-bound listener (tests that must know the
    /// port before the route starts). `None` binds `listen` on first use.
    prebound: StdMutex<Option<TcpListener>>,
    timeouts: IngestTimeouts,
    max_sessions: usize,
    infra: OnceCell<Arc<WhipInfra>>,
}

impl std::fmt::Debug for WhipRoute {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WhipRoute")
            .field("name", &self.name)
            .field("listen", &self.listen)
            .field("max_sessions", &self.max_sessions)
            .finish()
    }
}

impl WhipRoute {
    /// Build a route whose WHIP publish endpoint listens on `listen` (e.g.
    /// `"0.0.0.0:8080"`, or `"127.0.0.1:0"` for an ephemeral test port). A
    /// publisher `POST`s its SDP offer to `http://<listen>/whip` (only the
    /// `/whip` path is answered).
    pub fn new(name: impl Into<String>, listen: impl Into<String>) -> Self {
        WhipRoute {
            name: name.into(),
            listen: listen.into(),
            prebound: StdMutex::new(None),
            timeouts: IngestTimeouts::default(),
            max_sessions: DEFAULT_WHIP_MAX_SESSIONS,
            infra: OnceCell::new(),
        }
    }

    /// Build a route over an already-bound listener (W2a SP7.1: a test binds
    /// port 0, learns the port, and hands the bound listener in, rather than
    /// racing reserve-then-rebind).
    pub fn with_listener(
        name: impl Into<String>,
        listener: TcpListener,
        max_sessions: usize,
    ) -> Self {
        let listen = listener
            .local_addr()
            .map(|a| a.to_string())
            .unwrap_or_else(|_| "127.0.0.1:0".to_string());
        WhipRoute {
            name: name.into(),
            listen,
            prebound: StdMutex::new(Some(listener)),
            timeouts: IngestTimeouts::default(),
            max_sessions,
            infra: OnceCell::new(),
        }
    }

    /// Overrides the default [`IngestTimeouts`].
    #[must_use]
    pub fn with_timeouts(mut self, timeouts: IngestTimeouts) -> Self {
        self.timeouts = timeouts;
        self
    }

    /// Overrides [`DEFAULT_WHIP_MAX_SESSIONS`].
    #[must_use]
    pub fn with_max_sessions(mut self, max_sessions: usize) -> Self {
        self.max_sessions = max_sessions;
        self
    }

    /// Binds the HTTP listen socket and spawns the accept-pump task on the
    /// first call only — see `crate::source::rtmp::RtmpRoute::ensure_infra`'s
    /// identical "bind once" reasoning.
    async fn ensure_infra(&self) -> Result<Arc<WhipInfra>> {
        let max_sessions = self.max_sessions;
        let infra = self
            .infra
            .get_or_try_init(|| async {
                // Recover the slot even from a poisoned lock (a `with_listener`
                // caller panicking while holding it must not abort startup).
                let prebound = match self.prebound.lock() {
                    Ok(mut slot) => slot.take(),
                    Err(poisoned) => poisoned.into_inner().take(),
                };
                let listener = match prebound {
                    Some(listener) => listener,
                    None => TcpListener::bind(&self.listen).await.map_err(|e| {
                        MultimuxError::Connect {
                            reason: format!("whip: bind {}: {e}", self.listen),
                        }
                    })?,
                };
                let (tx, rx) = mpsc::channel(ACCEPT_QUEUE_CAPACITY);
                let active_sessions = Arc::new(AtomicUsize::new(0));
                let admit_notify = Arc::new(Notify::new());
                // SP2.1: serve through `serve_hyper_util` (header-read
                // timeout, connection cap, graceful drain) rather than
                // `axum::serve`. The `WhipInfra` holds the cancel token so
                // dropping the route stops the listener and releases the port.
                let state = Arc::new(WhipServeState {
                    tx,
                    active_sessions: Arc::clone(&active_sessions),
                    max_sessions,
                    admit_notify: Arc::clone(&admit_notify),
                });
                let cancel = tokio_util::sync::CancellationToken::new();
                let serve_cancel = cancel.clone();
                let tracked = tokio_util::task::TaskTracker::new();
                tracked.spawn(async move {
                    if let Err(e) =
                        crate::origin::serve_hyper_util(listener, whip_router(state), serve_cancel)
                            .await
                    {
                        tracing::warn!(error = %e, "whip: signalling server ended");
                    }
                });
                Ok::<Arc<WhipInfra>, MultimuxError>(Arc::new(WhipInfra {
                    accept_rx: Arc::new(StdMutex::new(rx)),
                    active_sessions,
                    cancel,
                    tracker: tracked,
                    timer_fires: Arc::new(AtomicU64::new(0)),
                    timer_notify: Arc::new(Notify::new()),
                    admit_notify,
                    admitted_total: Arc::new(AtomicUsize::new(0)),
                    admit_count_notify: Arc::new(Notify::new()),
                    last_datagram_entries: Arc::new(AtomicUsize::new(0)),
                    #[cfg(feature = "test-hooks")]
                    force_timer_error: Arc::new(StdMutex::new(None)),
                }))
            })
            .await?;
        Ok(Arc::clone(infra))
    }
}

/// Test handle over a running `WhipRoute`: the live session count and the
/// transport-timer fire counter, for bounded condition waits.
#[doc(hidden)]
pub struct WhipRouteShared {
    infra: Arc<WhipInfra>,
}

impl WhipRouteShared {
    /// The live admitted-session count.
    pub fn active_sessions(&self) -> usize {
        self.infra.active_sessions.load(Ordering::SeqCst)
    }

    /// The number of `handle_timeout` fires driven by the transport-deadline
    /// timer arm.
    pub fn timer_fires(&self) -> u64 {
        self.infra.timer_fires.load(Ordering::Relaxed)
    }

    /// Waits until the transport-deadline timer has fired at least `n` times
    /// (or forever if it never does — the caller bounds it). Registers the
    /// `Notified` BEFORE re-checking the counter.
    pub async fn wait_timer_fire(&self, n: u64) {
        loop {
            let notified = self.infra.timer_notify.notified();
            if self.timer_fires() >= n {
                return;
            }
            notified.await;
        }
    }

    /// The number of sessions the DRIVER (`poll_accept`) has admitted (N2):
    /// the driver-side observable a test must assert, so reverting the I2
    /// admit-drain fix fails it even though the HTTP handler still bumps
    /// `active_sessions`.
    pub fn admitted_total(&self) -> usize {
        self.infra.admitted_total.load(Ordering::SeqCst)
    }

    /// Waits until the driver has admitted at least `n` sessions within
    /// `bound` (registering the `Notified` BEFORE the first check).
    pub async fn wait_for_admissions(&self, n: usize, bound: Duration) -> usize {
        use tokio::time::Instant;
        let deadline = Instant::now() + bound;
        loop {
            let notified = self.infra.admit_count_notify.notified(); // register FIRST
            let now = self.admitted_total();
            if now >= n {
                return now;
            }
            tokio::select! {
                _ = notified => {}
                _ = tokio::time::sleep_until(deadline) => return now,
            }
        }
    }

    /// Number of live `last_datagram` (idle-clock) entries the driver holds
    /// (minor review finding): a reaped session must drop its entry, so this
    /// returns to 0 once every session is reaped.
    pub fn last_datagram_entries(&self) -> usize {
        self.infra.last_datagram_entries.load(Ordering::Relaxed)
    }

    /// Stage a [`MediaEvent::TimerError`] to be surfaced by the next admitted
    /// session's transport (test seam): `run_whip` consumes it when the
    /// session is admitted, ending that session on its next timer drive.
    #[cfg(feature = "test-hooks")]
    pub fn force_next_timer_error(&self, err: impl Into<String>) {
        *self
            .infra
            .force_timer_error
            .lock()
            .expect("force_timer_error") = Some(err.into());
    }
}

/// Test harness: bind an ephemeral listener, start the WHIP signalling
/// server over it, and return the bound address + a shared handle + the
/// cancel token.
#[doc(hidden)]
#[cfg(feature = "test-hooks")]
pub async fn serve_for_test() -> (
    std::net::SocketAddr,
    WhipRouteShared,
    tokio_util::sync::CancellationToken,
) {
    serve_for_test_with_read_timeout(IngestTimeouts::default().read).await
}

/// [`serve_for_test`] with an explicit session read timeout.
#[doc(hidden)]
#[cfg(feature = "test-hooks")]
pub async fn serve_for_test_with_read_timeout(
    read: Duration,
) -> (
    std::net::SocketAddr,
    WhipRouteShared,
    tokio_util::sync::CancellationToken,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let mut route = WhipRoute::with_listener("cam", listener, DEFAULT_WHIP_MAX_SESSIONS);
    route.timeouts.read = read;
    let infra = route.ensure_infra().await.expect("ensure_infra");
    let token = infra.cancel.clone();
    let shared = WhipRouteShared {
        infra: Arc::clone(&infra),
    };
    // `shared.infra` keeps the server alive; dropping `route` only drops its
    // `OnceCell`'s `Arc` clone.
    drop(route);
    (addr, shared, token)
}

/// Test harness that ALSO spawns [`run_whip`] against the route, so the media
/// driver drains the admit channel (defect 2's "publisher B admitted under a
/// steady read" needs the read loop running). Returns the signalling address,
/// the shared handle and a cancel token that stops both the server and the
/// forever-looping `run_whip` task.
#[doc(hidden)]
#[cfg(feature = "test-hooks")]
pub async fn serve_for_test_with_read_load() -> (
    std::net::SocketAddr,
    WhipRouteShared,
    tokio_util::sync::CancellationToken,
) {
    serve_for_test_with_read_load_and_timeout(IngestTimeouts::default().read).await
}

/// [`serve_for_test_with_read_load`] with an explicit session read timeout
/// (test-only): a short timeout lets a test observe an idle session being
/// reaped — and its `last_datagram` entry dropped — without waiting out the
/// default.
#[doc(hidden)]
#[cfg(feature = "test-hooks")]
pub async fn serve_for_test_with_read_load_and_timeout(
    read: Duration,
) -> (
    std::net::SocketAddr,
    WhipRouteShared,
    tokio_util::sync::CancellationToken,
) {
    // A handshake deadline that never fires in practice: the production
    // `run_whip` default (an effectively unbounded establish budget).
    serve_for_test_with_read_load_timeout_and_policy(read, Duration::from_secs(10_000)).await
}

/// [`serve_for_test_with_read_load_and_timeout`] with an explicit handshake
/// deadline (test-only): a short `establish_by` deadline lets a test reap a
/// session through the `Events` path — a datagram feed advancing past the
/// deadline turns the session terminal and drops its `last_datagram` entry —
/// rather than through the read-timeout path.
#[doc(hidden)]
#[cfg(feature = "test-hooks")]
pub async fn serve_for_test_with_read_load_timeout_and_policy(
    read: Duration,
    handshake_timeout: Duration,
) -> (
    std::net::SocketAddr,
    WhipRouteShared,
    tokio_util::sync::CancellationToken,
) {
    use std::num::NonZeroUsize;
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let mut route = WhipRoute::with_listener("cam", listener, DEFAULT_WHIP_MAX_SESSIONS);
    route.timeouts.read = read;
    let route = Arc::new(route);
    let infra = route.ensure_infra().await.expect("ensure_infra");
    let token = infra.cancel.clone();

    let policy = handshake_policy(handshake_timeout);
    let route_handle = Arc::new(RouteHandle::new(1.0, 500, 8));
    let route_clone = Arc::clone(&route);
    let abort_token = token.clone();
    let run_task = tokio::spawn(async move {
        let _ = run_whip(
            &route_clone,
            TrunkConfig::new(
                NonZeroUsize::new(64).unwrap(),
                NonZeroUsize::new(16).unwrap(),
                NonZeroUsize::new(8).unwrap(),
                NonZeroUsize::new(8).unwrap(),
                NonZeroUsize::new(8).unwrap(),
            ),
            policy,
            &route_handle,
        )
        .await;
    });
    tokio::spawn(async move {
        abort_token.cancelled().await;
        run_task.abort();
    });

    let shared = WhipRouteShared {
        infra: Arc::clone(&infra),
    };
    (addr, shared, token)
}

/// `#[doc(hidden)]` test hook: parse a WHIP offer's ICE credentials (media
/// level first, session level fallback) so `tests/whip_whep_sdp.rs` can pin
/// the media-first decision.
#[doc(hidden)]
#[cfg(feature = "test-hooks")]
pub fn parse_offer_for_test(offer: &str) -> (String, String) {
    let parsed = parse_whip_offer(offer).expect("parse offer");
    (parsed.remote_ufrag, parsed.remote_pwd)
}

/// `#[doc(hidden)]` test hook: render a WHIP SDP answer deterministically —
/// the offer, local address, ICE credentials, fingerprint and candidate lines
/// are all caller-supplied, so `tests/whip_whep_sdp.rs` can golden
/// `sdp_types::Session::write`'s line ordering (the review's answer-golden
/// gap) without the random cert/candidate values.
#[doc(hidden)]
#[cfg(feature = "test-hooks")]
pub fn render_answer_for_test(
    offer: &str,
    local_addr: std::net::SocketAddr,
    local_ice_ufrag: &str,
    local_ice_pwd: &str,
    fingerprint: &str,
    candidates: &[String],
) -> String {
    let parsed = parse_whip_offer(offer).expect("parse offer");
    render_answer(
        &parsed,
        local_addr,
        local_ice_ufrag,
        local_ice_pwd,
        fingerprint,
        candidates,
    )
}

impl Source for WhipRoute {
    fn stream_name(&self) -> &str {
        &self.name
    }
}

/// One parsed WHIP SDP offer: session-level ICE credentials + candidates
/// (taken from wherever in the SDP text they appear — a bundled offer
/// signals `a=ice-ufrag`/`a=ice-pwd` identically on every `m=` section, and
/// this cut requires exactly one anyway), plus the single `m=video` track.
struct ParsedOffer {
    remote_ufrag: String,
    remote_pwd: String,
    /// The offer's `a=fingerprint` value (RFC 8122 §5) — required; without
    /// it there is nothing to authenticate the peer's DTLS certificate
    /// against (RFC 5764 §5), so an offer lacking one is rejected like any
    /// other malformed offer.
    remote_fingerprint: String,
    mid: String,
    candidates: Vec<String>,
    payload_type: u8,
    clock_rate: u32,
    /// The chosen `m=video` media section, cloned so the answer reconstructs
    /// `m=`/`c=`/codec attributes from the typed offer rather than verbatim
    /// echoed text.
    media: sdp_types::Media,
}

/// Parses a WHIP offer for its single supported case: exactly one `m=video`
/// section, H.264 payload types only (matched by SDP encoding name, not
/// assumed) — see the module doc's "Scope" section for why audio (Opus) and
/// multi-track offers are rejected rather than silently dropped.
fn parse_whip_offer(offer: &str) -> Result<ParsedOffer> {
    let session = sdp_types::Session::parse(offer.as_bytes()).map_err(|e| MultimuxError::Sdp {
        reason: format!("whip: parse offer: {e}"),
    })?;
    let video_medias: Vec<_> = session
        .medias
        .iter()
        .filter(|m| m.media == "video")
        .collect();
    if session.medias.len() != video_medias.len() || video_medias.len() != 1 {
        return Err(MultimuxError::Sdp {
            reason: format!(
                "whip: this route accepts exactly one m=video section and nothing else \
                 (Opus audio has no RTP depacketiser in this workspace yet); offer had {} \
                 total section(s), {} of them video",
                session.medias.len(),
                video_medias.len()
            ),
        });
    }
    let media = video_medias[0];

    // ICE credentials are read at media level FIRST, falling back to session
    // level (RFC 8839 §5.4): a bundled offer may signal them only once, on
    // the first m= section, and a media section's own pair must win over a
    // stale session-level decoy (the old flat scan took the first match
    // anywhere). `session.get_first_attribute_value` exists on sdp-types 0.2.
    let remote_ufrag = ice_attr(&session, media, "ice-ufrag")
        .ok_or_else(|| MultimuxError::Sdp {
            reason: "whip: offer has no a=ice-ufrag".into(),
        })?
        .to_string();
    let remote_pwd = ice_attr(&session, media, "ice-pwd")
        .ok_or_else(|| MultimuxError::Sdp {
            reason: "whip: offer has no a=ice-pwd".into(),
        })?
        .to_string();
    let remote_fingerprint = parse_remote_fingerprint(offer).ok_or_else(|| MultimuxError::Sdp {
        reason: "whip: offer has no a=fingerprint".into(),
    })?;
    let mid = media
        .get_first_attribute_value("mid")
        .flatten()
        .unwrap_or("0")
        .to_string();
    let candidates: Vec<String> = media
        .attributes
        .iter()
        .filter(|a| a.attribute == "candidate" && a.value.is_some())
        .map(|a| a.value.clone().unwrap_or_default())
        .collect();

    // Pick the first payload type in `m=video`'s `fmt` list whose
    // `a=rtpmap` names H.264 — skips a paired `rtx` payload type (RFC 4588)
    // offered alongside it, which this route neither requests nor handles.
    let mut chosen: Option<(u8, u32)> = None;
    for tok in media.fmt.split_whitespace() {
        let Ok(pt) = tok.parse::<u8>() else { continue };
        let rtpmap = media.attributes.iter().find(|a| {
            a.attribute == "rtpmap"
                && a.value
                    .as_deref()
                    .and_then(|v| v.split_whitespace().next())
                    .and_then(|pt| pt.parse::<u8>().ok())
                    == Some(pt)
        });
        let Some(rtpmap) = rtpmap else { continue };
        let Some(value) = rtpmap.value.as_deref() else {
            continue;
        };
        if !value.to_ascii_uppercase().contains("H264") {
            continue;
        }
        let clock_rate = transmux::rtpmap_clock_rate(value).unwrap_or(90_000);
        chosen = Some((pt, clock_rate));
        break;
    }
    let (payload_type, clock_rate) = chosen.ok_or_else(|| MultimuxError::Sdp {
        reason: "whip: m=video has no H.264 (a=rtpmap naming H264) payload type".into(),
    })?;

    Ok(ParsedOffer {
        remote_ufrag,
        remote_pwd,
        remote_fingerprint,
        mid,
        candidates,
        payload_type,
        clock_rate,
        media: media.clone(),
    })
}

/// The first `name` attribute value, taken from the media section (media
/// level) and falling back to the session level (RFC 8839 §5.4). The typed
/// sdp-types attribute accessors make this resolve `Option<Option<&str>>`.
fn ice_attr<'a>(
    session: &'a sdp_types::Session,
    media: &'a sdp_types::Media,
    name: &'a str,
) -> Option<&'a str> {
    media
        .get_first_attribute_value(name)
        .flatten()
        .or_else(|| session.get_first_attribute_value(name).flatten())
}

/// Builds this side's SDP answer via `sdp-types`' own `Session::write`
/// (Task 8): [`SetupRole::Passive`] (this route is always the DTLS server),
/// `a=recvonly` (a WHIP publish endpoint never sends media back),
/// `a=rtcp-mux` (required — [`MediaTransport`] demuxes RTP/RTCP on one
/// 5-tuple per RFC 5761 §4, never a separate RTCP port). ICE candidate lines
/// come from [`MediaTransport::local_candidates`], and the codec attributes
/// (`a=rtpmap`/`a=fmtp`/`a=rtcp-fb`) are echoed from the offer's typed media.
fn build_answer(
    offer: &ParsedOffer,
    media: &MediaTransport,
    local_addr: std::net::SocketAddr,
    local_ice_ufrag: &str,
    local_ice_pwd: &str,
) -> String {
    render_answer(
        offer,
        local_addr,
        local_ice_ufrag,
        local_ice_pwd,
        media.local_fingerprint(),
        &media.local_candidates(),
    )
}

/// [`build_answer`]'s deterministic core (test-only export): the fingerprint
/// and candidate lines are parameters, so a golden can pin `Session::write`'s
/// line ordering without the random cert/candidate values.
fn render_answer(
    offer: &ParsedOffer,
    local_addr: std::net::SocketAddr,
    local_ice_ufrag: &str,
    local_ice_pwd: &str,
    fingerprint: &str,
    candidates: &[String],
) -> String {
    use sdp_types::{Attribute, Connection, Media, Origin, Session};

    let mut session = Session::new(Origin::with_ip_addr("0", 0, local_addr.ip()), "-");

    // Reconstruct the m= section from the offer's typed media (echo the
    // negotiated fmt list, RFC 3264 §6.1), with this side's c= line.
    let mut m = Media {
        media: offer.media.media.clone(),
        port: offer.media.port,
        num_ports: offer.media.num_ports,
        proto: offer.media.proto.clone(),
        fmt: offer.payload_type.to_string(),
        media_title: None,
        connections: Vec::new(),
        bandwidths: Vec::new(),
        key: None,
        attributes: Vec::new(),
    };
    m.add_connection(Connection::from_ip_addr(local_addr.ip()));
    m.add_attribute_with_value("rtcp", "9 IN IP4 0.0.0.0");
    // Echo the codec attributes naming the chosen payload type.
    for attr in &offer.media.attributes {
        if !matches!(attr.attribute.as_str(), "rtpmap" | "fmtp" | "rtcp-fb") {
            continue;
        }
        let names_pt = attr
            .value
            .as_deref()
            .and_then(|v| v.split_whitespace().next())
            .and_then(|pt| pt.parse::<u8>().ok())
            == Some(offer.payload_type);
        if names_pt {
            m.attributes.push(Attribute {
                attribute: attr.attribute.clone(),
                value: attr.value.clone(),
            });
        }
    }
    m.add_attribute(sdp_types::Attribute::new("recvonly"));
    m.add_attribute_with_value("mid", &offer.mid);
    m.add_attribute(sdp_types::Attribute::new("rtcp-mux"));
    m.add_attribute_with_value("ice-ufrag", local_ice_ufrag);
    m.add_attribute_with_value("ice-pwd", local_ice_pwd);
    m.add_attribute_with_value("fingerprint", format!("sha-256 {}", fingerprint));
    m.add_attribute_with_value("setup", "passive");
    for cand in candidates {
        m.add_attribute_with_value("candidate", cand);
    }
    m.add_attribute(sdp_types::Attribute::new("end-of-candidates"));
    session.medias.push(m);

    let mut buf = Vec::new();
    session
        .write(&mut buf)
        .expect("writing an SDP answer to a Vec cannot fail");
    String::from_utf8(buf).expect("SDP answer is UTF-8")
}

/// Handles one accepted TCP connection end-to-end: read the POST, parse and
/// answer the offer, bind this session's own ephemeral media socket, build
/// the [`MediaTransport`], write the `201 Created` response, and hand the
/// negotiated session off to `tx`. An `OPTIONS` preflight (browsers send one
/// ahead of a cross-origin `POST` with a non-simple `Content-Type`) gets a
/// permissive CORS response and no session.
/// Shared state for the four WHIP signalling handlers.
#[derive(Clone)]
struct WhipServeState {
    tx: mpsc::Sender<AdmittedWhip>,
    active_sessions: Arc<AtomicUsize>,
    max_sessions: usize,
    /// Fired after a session is handed to the admit channel, so [`run_whip`]
    /// drains it immediately rather than waiting out a fixed poll interval a
    /// steady publish load can starve (defect 2 — the I2 fix).
    admit_notify: Arc<Notify>,
}

/// The WHIP signalling router: `POST /whip` (offer), `OPTIONS /whip`
/// (preflight) and the session resource's `PATCH`/`DELETE` (`/whip/session`).
/// The tower layers give it the concurrency bound, the 64 KiB body cap, the
/// 10 s request timeout and the `CorsLayer` the raw reader used to hand-roll
/// (SP2.1).
fn whip_router(state: Arc<WhipServeState>) -> axum::Router {
    whip_router_with_cap(state, crate::webrtc_session::MAX_PENDING_HTTP_CONNECTIONS)
}

/// [`whip_router`] with an explicit concurrency cap (test-only): the shared
/// pool across this router's routes is what a test dials low to observe the
/// `GlobalConcurrencyLimitLayer` property through the production router.
fn whip_router_with_cap(state: Arc<WhipServeState>, max_pending: usize) -> axum::Router {
    use axum::routing::{patch, post};

    axum::Router::new()
        .route("/whip", post(whip_post).options(whip_options))
        .route("/whip/session", patch(whip_patch).delete(whip_delete))
        // Mirror `output::whep`'s `ensure_connect_info`: a test harness driving
        // the router with `oneshot` has no accepted stream, so `ConnectInfo`
        // (and the `LocalAddr` extension) is absent and `whip_post`'s
        // `ConnectInfo(peer)` extractor would reject; inject a default so the
        // production router is driveable by `whip_router_for_test`.
        .layer(axum::middleware::from_fn(ensure_connect_info))
        // ONE shared concurrency pool across every route on this listener (not
        // one semaphore per route/method, which `ConcurrencyLimitLayer` would
        // build): `GlobalConcurrencyLimitLayer` owns the `Arc<Semaphore>` and
        // every `layer()` clone shares it.
        .layer(tower::limit::GlobalConcurrencyLimitLayer::new(max_pending))
        .layer(tower_http::limit::RequestBodyLimitLayer::new(
            crate::webrtc_session::MAX_HTTP_BODY_BYTES,
        ))
        .layer(tower_http::timeout::TimeoutLayer::with_status_code(
            axum::http::StatusCode::REQUEST_TIMEOUT,
            crate::webrtc_session::HTTP_READ_TIMEOUT,
        ))
        // DEVIATION: `tower_http::cors::CorsLayer` intercepts EVERY `OPTIONS`
        // preflight itself and answers `200 OK`, so the route's own 204
        // handler would never run; the WHIP OPTIONS contract (and the golden)
        // is `204 No Content`. CORS headers are therefore added by this
        // middleware, mirroring `origin::add_response_headers`, and the
        // handler stays reachable.
        .layer(axum::middleware::from_fn(add_whip_cors))
        .with_state(state)
}

/// The production [`whip_router`] built through its real [`WhipServeState`]
/// (test-only): a test that drives BOTH the WHIP and WHEP routers through the
/// shared-pool property must call the same `whip_router` production code, not
/// hand-build a `Router` with `GlobalConcurrencyLimitLayer` (N2b). `max_pending`
/// overrides [`whip_router_with_cap`]'s concurrency cap so the test can dial it
/// low and observe the shared pool.
#[doc(hidden)]
#[cfg(feature = "test-hooks")]
pub fn whip_router_for_test(max_sessions: usize, max_pending: usize) -> axum::Router {
    let (tx, _rx) = mpsc::channel(ACCEPT_QUEUE_CAPACITY);
    let state = Arc::new(WhipServeState {
        tx,
        active_sessions: Arc::new(AtomicUsize::new(0)),
        max_sessions,
        admit_notify: Arc::new(Notify::new()),
    });
    whip_router_with_cap(state, max_pending)
}

/// Add the permissive CORS headers the WHIP signalling endpoint needs to
/// every response (`CorsLayer` would intercept OPTIONS; see the router).
async fn add_whip_cors(
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use axum::http::{HeaderValue, header};
    let mut resp = next.run(req).await;
    let headers = resp.headers_mut();
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static("POST, PATCH, DELETE, OPTIONS"),
    );
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_HEADERS,
        HeaderValue::from_static("Authorization, Content-Type, If-Match"),
    );
    // Main exposed `Location` on the 201 so a cross-origin browser can read it
    // and DELETE by it (RFC 9725 §4.x); keep that, lost in the axum move.
    headers.insert(
        header::ACCESS_CONTROL_EXPOSE_HEADERS,
        HeaderValue::from_static("Location"),
    );
    resp
}

/// Inject a default `ConnectInfo`/`LocalAddr` when absent (a `oneshot`-driven
/// test has no accepted stream) — the exact counterpart to
/// `output::whep::ensure_connect_info`, so `whip_post`'s `ConnectInfo(peer)`
/// extractor never rejects a request driven through `whip_router_for_test`.
async fn ensure_connect_info(
    mut req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    if req
        .extensions()
        .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
        .is_none()
    {
        req.extensions_mut()
            .insert(axum::extract::ConnectInfo(std::net::SocketAddr::from((
                std::net::Ipv4Addr::UNSPECIFIED,
                0,
            ))));
    }
    if req
        .extensions()
        .get::<Option<crate::origin::LocalAddr>>()
        .is_none()
    {
        req.extensions_mut()
            .insert(None::<crate::origin::LocalAddr>);
    }
    next.run(req).await
}

/// Convert a `webrtc_runtime` [`HttpResponse`](webrtc_runtime::whip::server::HttpResponse)
/// into an axum response. The typed status line and headers are already on
/// the wire form `webrtc-runtime` produced; only the body needs wrapping.
fn to_axum(resp: webrtc_runtime::whip::server::HttpResponse) -> axum::response::Response {
    let mut out = axum::response::Response::new(axum::body::Body::from(resp.body));
    *out.status_mut() = resp.status;
    *out.headers_mut() = resp.headers;
    out
}

/// Which IP [`whip_post_inner`] binds this session's media socket on: the
/// signalling connection's LOCAL address when one is known (correct even when
/// peer != local — a non-loopback publisher reaches the server at the local
/// side of its own connection, never at its own peer address), else the
/// peer's family's unspecified address.
fn local_bind_ip(
    local: Option<std::net::SocketAddr>,
    peer: std::net::SocketAddr,
) -> std::net::IpAddr {
    local.map(|a| a.ip()).unwrap_or_else(|| match peer {
        std::net::SocketAddr::V4(_) => std::net::Ipv4Addr::UNSPECIFIED.into(),
        std::net::SocketAddr::V6(_) => std::net::Ipv6Addr::UNSPECIFIED.into(),
    })
}

/// `OPTIONS /whip`: 204; `CorsLayer` supplies the `Access-Control-*` headers.
async fn whip_options() -> axum::http::StatusCode {
    axum::http::StatusCode::NO_CONTENT
}

/// `POST /whip`: parse the offer, bind this session's media socket, build the
/// [`MediaTransport`], answer `201 Created`, and hand the session to the
/// driver over the admit channel.
async fn whip_post(
    axum::extract::State(state): axum::extract::State<Arc<WhipServeState>>,
    ext: axum::extract::Extension<Option<crate::origin::LocalAddr>>,
    axum::extract::ConnectInfo(peer): axum::extract::ConnectInfo<std::net::SocketAddr>,
    body: axum::body::Bytes,
) -> axum::response::Response {
    let local = ext.0.map(|l| l.0);
    match whip_post_inner(state, peer, local, &body).await {
        Ok(resp) => resp,
        Err(e) => {
            tracing::warn!(error = %e, "whip: offer rejected");
            axum::http::StatusCode::BAD_REQUEST.into_response()
        }
    }
}

async fn whip_post_inner(
    state: Arc<WhipServeState>,
    peer: std::net::SocketAddr,
    local: Option<std::net::SocketAddr>,
    body: &[u8],
) -> Result<axum::response::Response> {
    let offer_sdp = String::from_utf8_lossy(body).into_owned();
    let parsed = parse_whip_offer(&offer_sdp)?;

    // Capacity check (issue r07-C11) before any of the expensive
    // per-connection work below (UDP bind, ICE/DTLS setup): refuse here, not
    // after. `slot` is an RAII guard (issue r07-C11 follow-up): every `?`
    // between here and the successful `tx.send` below releases it
    // automatically on drop, so a client that resets (or any other mid-setup
    // failure) can no longer leak the reservation forever. `slot.disarm()`
    // right before the successful send hands the slot's lifetime off to the
    // admitted session — `report_and_maybe_reap` is the new owner from then
    // on.
    let Some(slot) =
        crate::webrtc_session::SessionSlot::acquire(&state.active_sessions, state.max_sessions)
    else {
        return Ok(axum::http::StatusCode::SERVICE_UNAVAILABLE.into_response());
    };

    // The IP to advertise as this session's ICE host candidate: not
    // `0.0.0.0` (unreachable), but the local side of the signalling
    // connection the client used to reach this endpoint — a real,
    // client-reachable address by construction (whatever routing/NAT let the
    // signalling connection through already applies to it). No STUN/TURN
    // reflexive/relay candidate is gathered here (`stun_server: None` below).
    // A `oneshot` test harness has no accepted stream, so fall back to the
    // peer's family's unspecified address (the media socket still binds).
    let advertise_ip = local_bind_ip(local, peer);
    let socket = UdpSocket::bind((advertise_ip, 0))
        .await
        .map_err(|e| MultimuxError::Connect {
            reason: format!("whip: bind media socket on {advertise_ip}: {e}"),
        })?;
    let local_addr = socket.local_addr().map_err(|e| MultimuxError::Connect {
        reason: format!("whip: media socket local_addr: {e}"),
    })?;

    let local_ice_ufrag = rand_token(8);
    let local_ice_pwd = rand_token(24);
    let mut media = MediaTransport::new(
        MediaTransportConfig {
            local_addr,
            local_ice_ufrag: local_ice_ufrag.clone(),
            local_ice_pwd: local_ice_pwd.clone(),
            remote_ice_ufrag: parsed.remote_ufrag.clone(),
            remote_ice_pwd: parsed.remote_pwd.clone(),
            remote_fingerprint: parsed.remote_fingerprint.clone(),
            is_controlling: false,
            local_setup: SetupRole::Passive,
            stun_server: None,
            max_remote_candidates: MAX_REMOTE_CANDIDATES,
        },
        Instant::now(),
    )
    .map_err(|e| MultimuxError::Connect {
        reason: format!("whip: build media transport: {e}"),
    })?;

    for raw in &parsed.candidates {
        // A candidate this side can't parse is skipped, not fatal — ICE
        // connectivity checks simply never nominate a pair for it.
        let _ = media.add_remote_candidate(raw);
    }

    let answer = build_answer(
        &parsed,
        &media,
        local_addr,
        &local_ice_ufrag,
        &local_ice_pwd,
    );

    // The typed state machine owns the 201's Location/ETag/Content-Type over
    // `http`/`headers` (SP2.2), not a hand-formatted response string.
    let mut session = webrtc_runtime::whip::server::WhipSession::new("/whip/session".to_string());
    let _ = session.on_post(Vec::new());
    let etag = rand_token(16);
    let response = session.accept(answer.into_bytes(), etag);

    let admitted = AdmittedWhip {
        socket: Arc::new(socket),
        media,
        tracks: vec![WhipTrack {
            track_id: 1,
            payload_type: parsed.payload_type,
            clock_rate: parsed.clock_rate,
        }],
    };
    if state.tx.send(admitted).await.is_ok() {
        // Ownership of the slot passes to the admitted session from here.
        slot.disarm();
        // Wake the run loop so it drains this admission at once (I2), instead
        // of waiting out a fixed poll interval a steady publish load starves.
        state.admit_notify.notify_waiters();
    }
    // If the send failed (channel closed), `slot` drops here and releases it.
    Ok(to_axum(response))
}

/// `PATCH /whip/session`: not implemented in this cut (no trickle-ICE /
/// ICE-restart handling). Main answered every non-POST method `405 Method Not
/// Allowed`; keep that parity rather than a stub that acks without effect
/// (RFC 9110 §15.5.6 requires an `Allow` header on a 405).
async fn whip_patch() -> axum::response::Response {
    let mut resp = axum::http::StatusCode::METHOD_NOT_ALLOWED.into_response();
    resp.headers_mut().insert(
        axum::http::header::ALLOW,
        axum::http::HeaderValue::from_static("POST, OPTIONS"),
    );
    resp
}

/// `DELETE /whip/session`: not implemented in this cut (no session teardown
/// path). Main answered `405` for every non-POST method; keep parity.
async fn whip_delete() -> axum::response::Response {
    let mut resp = axum::http::StatusCode::METHOD_NOT_ALLOWED.into_response();
    resp.headers_mut().insert(
        axum::http::header::ALLOW,
        axum::http::HeaderValue::from_static("POST, OPTIONS"),
    );
    resp
}

/// A short pseudo-random token for ICE ufrag/pwd — see
/// `webrtc_runtime`'s `whip_media_smoke` example for the identical
/// technique (OS-random `RandomState`, no `rand` dependency for one call
/// site).
fn rand_token(len: usize) -> String {
    use std::collections::hash_map::RandomState;
    use std::hash::BuildHasher;
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let state = RandomState::new();
    (0..len)
        .map(|i| {
            let idx = (state.hash_one(i) as usize) % CHARS.len();
            CHARS[idx] as char
        })
        .collect()
}

/// The non-blocking [`Listener`] bridge over the accept-pump's channel — see
/// the module doc and `crate::source::rtmp::RtmpListener`'s identical shape.
struct WhipListener {
    accept_rx: Arc<StdMutex<mpsc::Receiver<AdmittedWhip>>>,
    max_sessions: usize,
}

impl Listener for WhipListener {
    type Session = WhipIngestSession;
    type Error = MultimuxError;

    fn max_sessions(&self) -> usize {
        self.max_sessions
    }

    fn poll_accept(&mut self) -> Result<Option<WhipIngestSession>> {
        let mut rx = self
            .accept_rx
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match rx.try_recv() {
            Ok(admitted) => Ok(Some(WhipIngestSession::new(admitted))),
            Err(mpsc::error::TryRecvError::Empty) => Ok(None),
            Err(mpsc::error::TryRecvError::Disconnected) => Err(MultimuxError::Connect {
                reason: "whip: accept-pump task ended (listen socket failure)".into(),
            }),
        }
    }
}

/// This track's captured real decoder config, once seen — `None` until a
/// real SPS+PPS pair has been scanned out of an actual IDR (see the module
/// doc's "Why the `avcC` config is captured from the bitstream" section).
type CapturedConfig = Option<AVCConfigurationBox>;

/// An admitted WHIP publisher: the depacketiser plus payload-type routing
/// (identical shape to `crate::source::rtp_udp::RtpUdpIngestSession`), plus
/// the deferred-`avcC`-capture state this route needs because the SDP offer
/// alone doesn't carry real parameter sets (see the module doc). Also holds
/// the shared handles [`run_whip`]'s own read loop needs — the socket and
/// the sans-IO [`MediaTransport`] — mirroring
/// `crate::source::rtmp::RtmpIngestSession::conn_handle`.
pub struct WhipIngestSession {
    socket: Arc<UdpSocket>,
    /// The sans-IO [`MediaTransport`], owned outright (SP6.3): the read loop
    /// takes it by value via [`Self::take_transport`] and holds no lock
    /// across any `send_to().await`. Held behind a *synchronous* mutex only
    /// so [`WhipIngestSession`] stays `Send + Sync` (a raw `Option<T>` would
    /// make it `!Sync`, since `MediaTransport` is `Send + !Sync`); the guard
    /// is taken within a single synchronous `take`, never across an await.
    media: std::sync::Mutex<Option<MediaTransport>>,
    depacketiser: RtpStreamDepacketiser,
    pt_to_track: HashMap<u8, u32>,
    clock_rate_by_track: HashMap<u32, u32>,
    captured: HashMap<u32, CapturedConfig>,
    /// Samples observed before every track's config was captured — replayed
    /// immediately once the establishment gate fires. See the module doc.
    buffered: Vec<(u32, Sample)>,
    announced: bool,
    pending: std::collections::VecDeque<SessionEvent>,
}

impl WhipIngestSession {
    fn new(admitted: AdmittedWhip) -> Self {
        let mut pt_to_track = HashMap::new();
        let mut clock_rate_by_track = HashMap::new();
        let mut captured = HashMap::new();
        let mut tracks = Vec::new();
        for t in &admitted.tracks {
            pt_to_track.insert(t.payload_type, t.track_id);
            clock_rate_by_track.insert(t.track_id, t.clock_rate);
            captured.insert(t.track_id, None);
            // The placeholder handed to `RtpStreamTrack::new` is never
            // observed externally: `RtpStreamDepacketiser::push` never
            // consults a track's `config` (only carries it through to
            // `track_specs()`, which this module deliberately never calls —
            // see the module doc), and no `TrackSpec` is announced until
            // `captured` holds every track's real config.
            let placeholder = CodecConfig::Avc {
                config: AVCConfigurationBox::new(AVCDecoderConfigurationRecord {
                    configuration_version: 1,
                    profile_indication: 0,
                    profile_compatibility: 0,
                    level_indication: 0,
                    length_size_minus_one: 3,
                    sps: Vec::new(),
                    pps: Vec::new(),
                    chroma_format: None,
                    bit_depth_luma_minus8: None,
                    bit_depth_chroma_minus8: None,
                    sps_ext: Vec::new(),
                }),
                width: UNKNOWN_DIMENSION,
                height: UNKNOWN_DIMENSION,
            };
            tracks.push(RtpStreamTrack::new(
                t.track_id,
                transmux::rtp::RtpMediaKind::H264,
                placeholder,
                t.clock_rate,
            ));
        }
        WhipIngestSession {
            socket: admitted.socket,
            media: std::sync::Mutex::new(Some(admitted.media)),
            depacketiser: RtpStreamDepacketiser::new(tracks),
            pt_to_track,
            clock_rate_by_track,
            captured,
            buffered: Vec::new(),
            announced: false,
            pending: std::collections::VecDeque::new(),
        }
    }

    /// A cheap `Arc` clone of the media socket — [`run_whip`]'s own
    /// per-session read task drives it concurrently with every other
    /// session's, entirely outside this sans-IO [`Stage`] impl.
    fn socket_handle(&self) -> Arc<UdpSocket> {
        Arc::clone(&self.socket)
    }

    /// Hands the owned [`MediaTransport`] to the read loop (SP6.3): the
    /// session task OWNS it outright, so the loop takes it by value and holds
    /// no lock across `send_to().await`. The `std::sync::Mutex` is held only
    /// for the synchronous `take`, never across an await.
    fn take_transport(&self) -> MediaTransport {
        self.media
            .lock()
            .expect("media transport mutex")
            .take()
            .expect("media transport already taken")
    }

    /// Scans `sample`'s length-prefixed NAL data (see [`NAL_LENGTH_PREFIX`])
    /// for a real SPS/PPS pair and, if found, captures the real `avcC` for
    /// `track_id` — see the module doc.
    fn try_capture_config(&mut self, track_id: u32, sample: &Sample) {
        if self.captured.get(&track_id).cloned().flatten().is_some() {
            return;
        }
        let mut sps = Vec::new();
        let mut pps = Vec::new();
        let data = sample.data.as_ref();
        let mut off = 0usize;
        while off + NAL_LENGTH_PREFIX <= data.len() {
            let len = u32::from_be_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]])
                as usize;
            off += NAL_LENGTH_PREFIX;
            if off + len > data.len() {
                break;
            }
            let nal = &data[off..off + len];
            off += len;
            if nal.is_empty() {
                continue;
            }
            match nal[0] & NAL_TYPE_MASK {
                NAL_TYPE_SPS => sps.push(AvcSps(nal.to_vec())),
                NAL_TYPE_PPS => pps.push(AvcPps(nal.to_vec())),
                _ => {}
            }
        }
        if sps.is_empty() || pps.is_empty() {
            return;
        }
        if let Ok(config) = avc_config_from_sps_pps(sps, pps) {
            self.captured.insert(track_id, Some(config));
        }
    }

    /// True once every track this session was constructed with has a
    /// captured real config.
    fn all_captured(&self) -> bool {
        self.captured.values().all(Option::is_some)
    }

    /// Fires the establishment gate: builds real [`TrackSpec`]s from the
    /// captured configs, announces `NewProgram`/`Established`, then replays
    /// every buffered sample — see the module doc.
    fn announce(&mut self) {
        self.announced = true;
        let mut track_ids: Vec<u32> = self.clock_rate_by_track.keys().copied().collect();
        track_ids.sort_unstable();
        let specs: Vec<TrackSpec> = track_ids
            .iter()
            .filter_map(|id| {
                let config = self.captured.get(id)?.clone()?;
                let clock_rate = *self.clock_rate_by_track.get(id)?;
                Some(TrackSpec::new(
                    *id,
                    clock_rate,
                    CodecConfig::Avc {
                        config,
                        width: UNKNOWN_DIMENSION,
                        height: UNKNOWN_DIMENSION,
                    },
                ))
            })
            .collect();
        self.pending.push_back(SessionEvent::NewProgram {
            program: ProgramId(0),
            tracks: specs,
        });
        self.pending.push_back(SessionEvent::Established);
        for (track_id, sample) in std::mem::take(&mut self.buffered) {
            self.pending.push_back(SessionEvent::Sample {
                program: ProgramId(0),
                track_id,
                retention: RetentionClass::Timed,
                sample,
            });
        }
    }

    fn observe_sample(&mut self, track_id: u32, sample: Sample) {
        if !self.announced {
            self.try_capture_config(track_id, &sample);
            self.buffered.push((track_id, sample));
            if self.all_captured() {
                self.announce();
            }
            return;
        }
        self.pending.push_back(SessionEvent::Sample {
            program: ProgramId(0),
            track_id,
            retention: RetentionClass::Timed,
            sample,
        });
    }
}

impl Stage for WhipIngestSession {
    type In<'a> = &'a [u8];
    type Out = SessionEvent;
    type Error = MultimuxError;

    /// `input` is one already-decrypted RTP packet's reconstructed wire
    /// bytes (see this module's private `rebuild_rtp_wire`) — [`run_whip`]'s
    /// read loop performs the ICE/DTLS/SRTP decrypt before ever calling
    /// this.
    fn feed(&mut self, input: &[u8], _now: Timestamp) -> Result<()> {
        let Some(track_id) =
            payload_type_of(input).and_then(|pt| self.pt_to_track.get(&pt).copied())
        else {
            return Ok(());
        };
        let samples =
            self.depacketiser
                .push(track_id, input)
                .map_err(|e| MultimuxError::Depay {
                    reason: e.to_string(),
                })?;
        for sample in samples {
            self.observe_sample(track_id, sample);
        }
        Ok(())
    }

    fn poll(&mut self) -> Option<SessionEvent> {
        self.pending.pop_front()
    }

    fn finish(&mut self) -> Result<()> {
        Ok(())
    }

    fn next_deadline(&self) -> Option<Timestamp> {
        None
    }

    fn on_deadline(&mut self, _now: Timestamp) {}

    fn demand(&self) -> Demand {
        Demand::new(MAX_UDP_DATAGRAM)
    }
}

impl IngestSession for WhipIngestSession {
    /// Uninhabited: every outbound datagram this session needs sent
    /// (STUN/DTLS/SRTP) is written directly by [`run_whip`]'s read loop via
    /// [`MediaTransport::poll_transmit`] — never through this trait, exactly
    /// like `crate::source::rtmp::RtmpIngestSession`'s identical rationale.
    type Request = std::convert::Infallible;
}

/// Extracts the RTP payload-type field (RFC 3550 §5.1, header byte 1 bits
/// `[6:0]`) — see `crate::source::rtp_udp::payload_type_of`'s identical
/// helper.
fn payload_type_of(packet: &[u8]) -> Option<u8> {
    if packet.len() < RTP_MIN_HEADER_LEN {
        return None;
    }
    Some(packet[1] & RTP_PT_MASK)
}

/// Rebuilds a wire-format RTP packet (RFC 3550 §5.1 fixed header, CSRC list,
/// and payload, concatenated) from a [`webrtc_runtime::media::DecryptedRtp`]
/// — the type [`MediaTransport`] hands back has already promoted the header
/// fields to typed values and decrypted the payload, but
/// `RtpStreamDepacketiser::push` (like every RTP depacketiser in this
/// workspace) takes the raw wire packet, so this reconstructs it rather than
/// growing a second, parsed-header entry point on that shared type just for
/// this one caller. A §5.3.1 header extension reported by the transport is
/// written back (X bit set, `defined by profile`, length in 32-bit words,
/// data), so RFC 8285 `mid`/`rid`/CVO survive to the depacketiser. The CSRC
/// count always fits: `csrc` was parsed from the 4-bit CC field.
fn rebuild_rtp_wire(pkt: &webrtc_runtime::media::DecryptedRtp) -> Vec<u8> {
    let csrc_count = pkt.csrc.len().min(0x0F) as u8;
    let mut out = Vec::with_capacity(12 + 4 * csrc_count as usize + pkt.payload.len());
    let extension_bit = if pkt.extension.is_some() {
        RTP_EXTENSION_BIT
    } else {
        0
    };
    out.push(RTP_VERSION_BYTE | extension_bit | csrc_count);
    out.push(if pkt.marker {
        0x80 | (pkt.payload_type & RTP_PT_MASK)
    } else {
        pkt.payload_type & RTP_PT_MASK
    });
    out.extend_from_slice(&pkt.sequence_number.to_be_bytes());
    out.extend_from_slice(&pkt.timestamp.to_be_bytes());
    out.extend_from_slice(&pkt.ssrc.to_be_bytes());
    for csrc in pkt.csrc.iter().take(csrc_count as usize) {
        out.extend_from_slice(&csrc.to_be_bytes());
    }
    if let Some(ext) = &pkt.extension {
        // RFC 3550 §5.3.1: length counts 32-bit words of extension data.
        let words = u16::try_from(ext.data.len() / RTP_EXTENSION_WORD_LEN)
            .expect("extension data was parsed from a 16-bit word-count field");
        out.extend_from_slice(&ext.profile_id.to_be_bytes());
        out.extend_from_slice(&words.to_be_bytes());
        out.extend_from_slice(&ext.data);
    }
    out.extend_from_slice(&pkt.payload);
    out
}

/// What one [`read_one`] call observed — mirrors
/// `crate::source::rtmp::ReadOutcome`'s shape.
#[derive(Debug)]
enum ReadOutcome {
    /// Zero or more decrypted RTP packets, each ready to
    /// [`ListenDriver::feed`] in order. A datagram arrived (even if it
    /// produced no RTP), so the idle clock resets.
    Events(Vec<Vec<u8>>),
    /// The transport's own timer fired with no inbound datagram (defect 1):
    /// `handle_timeout` ran and the session stays live. Does NOT reset the
    /// idle clock.
    Timer,
    /// A [`MediaEvent::TimerError`] surfaced from `handle_timeout` (finding
    /// N3): the ICE/DTLS timer drive failed, so the session is ended rather
    /// than silently stuck.
    TimerError(String),
    /// No datagram within [`IngestTimeouts::read`] — treated as ending this
    /// session, same as `crate::source::rtmp`'s own "timeouts still bound
    /// reads" policy.
    TimedOut,
    /// The underlying socket read failed.
    TransportError(String),
}

type BoxedRead = Pin<Box<dyn Future<Output = (SessionId, MediaTransport, ReadOutcome)> + Send>>;

/// Awaits the next UDP datagram for session `id`, bounded by `idle_deadline`
/// (an absolute instant when the session may end for lack of inbound
/// datagrams — set from the last datagram so a transport-timer wake never
/// re-arms it): decrypts through `media` (ICE/DTLS/SRTP), drains and sends
/// every outbound datagram [`MediaTransport::poll_transmit`] now wants sent,
/// and reconstructs wire bytes for every [`MediaEvent::Rtp`] observed. The
/// [`MediaTransport`] is owned outright and returned in the outcome so the
/// next [`read_one`] takes it again (SP6.3). Boxed so [`run_whip`] can hold
/// many of these, one per admitted session, in one [`FuturesUnordered`] —
/// mirrors `crate::source::rtmp::read_one`.
fn read_one(
    id: SessionId,
    socket: Arc<UdpSocket>,
    mut media: MediaTransport,
    idle_deadline: Instant,
    timer_fires: Arc<AtomicU64>,
    timer_notify: Arc<Notify>,
) -> BoxedRead {
    Box::pin(async move {
        let mut buf = [0u8; MAX_UDP_DATAGRAM];
        // The transport's own next timer, taken BEFORE the select so a quiet
        // session still gets its ICE/DTLS retransmits (defect 1) — the timer
        // arm fires `handle_timeout` WITHOUT ending the session.
        let deadline = media.poll_timeout();
        let outcome = tokio::select! {
            received = socket.recv_from(&mut buf) => match received {
                Ok((n, peer)) => {
                    match media.handle_datagram(Instant::now(), peer, &buf[..n]) {
                        Ok(events) => {
                            let outbound = drain_transmit(&mut media);
                            for Datagram { peer, bytes } in outbound {
                                let _ = socket.send_to(&bytes, peer).await;
                            }
                            let wire: Vec<Vec<u8>> = events
                                .into_iter()
                                .filter_map(|e| match e {
                                    MediaEvent::Rtp(pkt) => Some(rebuild_rtp_wire(&pkt)),
                                    _ => None,
                                })
                                .collect();
                            ReadOutcome::Events(wire)
                        }
                        // A per-datagram failure is NOT session-fatal; an
                        // unauthenticated datagram is logged and skipped.
                        Err(e) => {
                            tracing::debug!(error = %e, %peer, "whip: datagram rejected, skipping");
                            let outbound = drain_transmit(&mut media);
                            for Datagram { peer, bytes } in outbound {
                                let _ = socket.send_to(&bytes, peer).await;
                            }
                            ReadOutcome::Events(Vec::new())
                        }
                    }
                }
                // A socket-level read error IS fatal — the session has no
                // transport left to recover onto.
                Err(e) => ReadOutcome::TransportError(e.to_string()),
            },
            // The read-timeout bound: a session with no datagram since
            // `idle_deadline` ends. This is the transport-timer-independent
            // "vanished publisher" reap.
            () = sleep_until_opt(Some(idle_deadline)) => {
                media.handle_timeout(Instant::now());
                let outbound = drain_transmit(&mut media);
                for Datagram { peer, bytes } in outbound {
                    let _ = socket.send_to(&bytes, peer).await;
                }
                ReadOutcome::TimedOut
            },
            // Defect 1: the transport's own timer fired. Run `handle_timeout`
            // while the session stays live (a timer wake is NOT a session end,
            // and does NOT reset the idle clock). Finding N3: the sleep is
            // floored to MIN_TIMER_INTERVAL so a stuck (always-overdue)
            // deadline cannot fire `handle_timeout` back-to-back and spin a
            // core, and a fatal TimerError ends the session.
            () = crate::webrtc_session::sleep_until_timer(deadline, Instant::now()) => {
                let events = media.handle_timeout(Instant::now());
                timer_fires.fetch_add(1, Ordering::Relaxed);
                timer_notify.notify_waiters();
                let outbound = drain_transmit(&mut media);
                for Datagram { peer, bytes } in outbound {
                    let _ = socket.send_to(&bytes, peer).await;
                }
                match first_timer_error(&events) {
                    Some(err) => {
                        tracing::warn!(error = %err, "whip: transport timer failed; ending session");
                        ReadOutcome::TimerError(err)
                    }
                    None => ReadOutcome::Timer,
                }
            }
        };
        (id, media, outcome)
    })
}

/// Drain everything the transport currently wants to send, returned as a
/// `Vec` so no `&mut MediaTransport` is held across any `send_to().await` —
/// the session owns the transport here (SP6.3): lock, drain, drop the lock,
/// then send.
fn drain_transmit(media: &mut MediaTransport) -> Vec<Datagram> {
    let mut out = Vec::new();
    while let Some(d) = media.poll_transmit() {
        out.push(d);
    }
    out
}

/// The first [`MediaEvent::TimerError`] among `handle_timeout`'s returned
/// events, if any (finding N3): a fatal ICE/DTLS timer-drive failure ends the
/// session rather than being silently discarded.
fn first_timer_error(events: &[MediaEvent]) -> Option<String> {
    events.iter().find_map(|e| match e {
        MediaEvent::TimerError(err) => Some(err.clone()),
        _ => None,
    })
}

/// The `last_datagram` idle-clock map with an optional live-size gauge (minor
/// review finding): the driver's `last_datagram` must drop a reaped session's
/// entry, else the map leaks one entry per reaped session. Wrapping the
/// `HashMap` here (rather than three bare mutation sites) keeps the gauge and
/// the map in lock-step so a test can observe the leak.
struct IdleClockMap {
    map: HashMap<SessionId, Instant>,
    live: Option<Arc<AtomicUsize>>,
}

impl IdleClockMap {
    fn new(live: Option<Arc<AtomicUsize>>) -> Self {
        Self {
            map: HashMap::new(),
            live,
        }
    }

    /// Set the idle clock for `id` (inserting it if new), returning the
    /// resulting value; increments the gauge only on a NEW insertion.
    fn set(&mut self, id: SessionId, t: Instant) -> Instant {
        use std::collections::hash_map::Entry;
        match self.map.entry(id) {
            Entry::Occupied(mut e) => {
                *e.get_mut() = t;
                t
            }
            Entry::Vacant(e) => {
                e.insert(t);
                if let Some(g) = &self.live {
                    g.fetch_add(1, Ordering::Relaxed);
                }
                t
            }
        }
    }

    fn get(&self, id: &SessionId) -> Option<Instant> {
        self.map.get(id).copied()
    }

    fn remove(&mut self, id: &SessionId) {
        if self.map.remove(id).is_some()
            && let Some(g) = &self.live
        {
            g.fetch_sub(1, Ordering::Relaxed);
        }
    }
}

/// Sleep until an optional wall-clock deadline (`std::time::Instant`, as
/// `MediaTransport::poll_timeout` returns), or never fire when `None`. An
/// already-overdue deadline fires IMMEDIATELY (it maps to a completed future,
/// never `pending()`): the deadline is sampled before the `select!`, so by the
/// time the timer arm is polled the deadline may already be in the past and
/// must be treated as ready, or the transport's timer would be delayed until
/// the read timeout.
fn sleep_until_opt(deadline: Option<Instant>) -> Pin<Box<dyn Future<Output = ()> + Send>> {
    let now = Instant::now();
    match deadline {
        None => Box::pin(std::future::pending()),
        Some(d) if d <= now => Box::pin(std::future::ready(())),
        Some(d) => Box::pin(tokio::time::sleep(d.saturating_duration_since(now))),
    }
}

/// Per-session [`DriverProgress`] bookkeeping — mirrors
/// `crate::source::rtmp::ProgressBySession`.
type ProgressBySession = HashMap<SessionId, DriverProgress>;

/// Publishes session `id`'s progress and, if that left it terminal, releases
/// whatever it published (`crate::source::release_route`) and reaps it —
/// mirrors `crate::source::rtmp::report_and_maybe_reap` exactly (the feed
/// call in [`run_whip`]'s loop goes through `driver_mut`/`IngestDriver::feed`
/// rather than `ListenDriver::feed`'s own `&[u8]`-pinned convenience wrapper,
/// specifically so a terminal session is never removed before this helper's
/// release-then-reap sequence runs for it).
async fn report_and_maybe_reap(
    driver: &mut ListenDriver<WhipListener>,
    id: SessionId,
    route_handle: &Arc<RouteHandle>,
    progress: &mut ProgressBySession,
    active_sessions: &Arc<AtomicUsize>,
) -> bool {
    if let Some(d) = driver.driver(id) {
        crate::source::advance_route(d, route_handle, progress.entry(id).or_default()).await;
        if !d.health().is_running() {
            crate::source::release_route(d, route_handle);
        }
    }
    let reaped = driver.reap_if_terminal(id).is_some();
    if reaped {
        progress.remove(&id);
        // Frees the capacity slot `handle_whip_connection` reserved before
        // this session's UDP bind/ICE setup (issue r07-C11) — without this,
        // every session that ever completes leaves that slot permanently
        // occupied.
        active_sessions.fetch_sub(1, Ordering::SeqCst);
    }
    reaped
}

/// Binds `route` (once ever), then admits and drives up to
/// [`Listener::max_sessions`] WHIP publishers **concurrently** until a
/// listen-socket-level failure occurs — mirrors
/// `crate::source::rtmp::run_rtmp`'s identical shape and "never returns in
/// ordinary operation" contract.
pub async fn run_whip(
    route: &WhipRoute,
    trunk_config: TrunkConfig,
    handshake: HandshakePolicy,
    route_handle: &Arc<RouteHandle>,
) -> MultimuxError {
    run_whip_with_clock(route, trunk_config, handshake, route_handle, &Instant::now).await
}

/// [`run_whip`] with an injectable wall clock: every instant the loop reads —
/// the route's start and each session's admission and feed — comes from
/// `clock`, so a test can move time deterministically instead of sleeping
/// (see the `SessionClocks` tests).
pub(crate) async fn run_whip_with_clock(
    route: &WhipRoute,
    trunk_config: TrunkConfig,
    handshake: HandshakePolicy,
    route_handle: &Arc<RouteHandle>,
    clock: &(dyn Fn() -> Instant + Sync),
) -> MultimuxError {
    let infra = match route.ensure_infra().await {
        Ok(v) => v,
        Err(e) => return e,
    };
    let accept_rx = Arc::clone(&infra.accept_rx);
    let active_sessions = Arc::clone(&infra.active_sessions);
    let listener = WhipListener {
        accept_rx,
        max_sessions: route.max_sessions,
    };
    let mut driver: ListenDriver<WhipListener> = ListenDriver::new(
        listener,
        trunk_config,
        handshake,
        media_plane::DEFAULT_MAX_PROGRAMS,
    );
    let start = clock();
    let read_timeout = route.timeouts.read;

    let mut progress: ProgressBySession = HashMap::new();
    // Each session's own clock origin (see `SessionClocks`).
    let mut clocks = SessionClocks::new(start);
    // The last time each session received a datagram, driving the idle reap
    // bound (a transport-timer wake must NOT reset it).
    let mut last_datagram = IdleClockMap::new({
        let g = Arc::clone(&infra.last_datagram_entries);
        Some(g)
    });
    let mut reads: FuturesUnordered<BoxedRead> = FuturesUnordered::new();

    loop {
        tokio::select! {
            // I2: admits on a wake from the signalling accept pump, instead of
            // a fixed poll interval a steady publish load can starve.
            () = infra.admit_notify.notified() => {
                clocks.retain(|id| progress.contains_key(id));
            }
            Some((id, media, outcome)) = reads.next(), if !reads.is_empty() => {
                let now = clocks.now(id, clock());
                match outcome {
                    ReadOutcome::Events(events) => {
                        // A datagram arrived: reset the idle clock.
                        last_datagram.set(id, clock());
                        // Fed via `driver_mut`/`IngestDriver::feed`, not
                        // `ListenDriver::feed` — that convenience wrapper
                        // removes a session that goes terminal as a *result*
                        // of this feed before `report_and_maybe_reap` below
                        // ever runs, which would skip its release-then-reap
                        // sequence for this session.
                        if let Some(d) = driver.driver_mut(id) {
                            for wire in &events {
                                d.feed(wire, now);
                            }
                        }
                        let reaped =
                            report_and_maybe_reap(&mut driver, id, route_handle, &mut progress, &active_sessions).await;
                        if reaped {
                            // The session is gone: drop its idle-clock entry, or
                            // the map leaks one entry per reaped session.
                            last_datagram.remove(&id);
                        } else if let Some(d) = driver.driver(id) {
                            let socket = d.session().socket_handle();
                            let idle_deadline = last_datagram.get(&id).unwrap_or_else(clock) + read_timeout;
                            reads.push(read_one(id, socket, media, idle_deadline, Arc::clone(&infra.timer_fires), Arc::clone(&infra.timer_notify)));
                        }
                    }
                    // A transport-timer wake (defect 1): the session stays live
                    // and its idle clock is NOT reset.
                    ReadOutcome::Timer => {
                        if progress.contains_key(&id)
                            && let Some(d) = driver.driver(id)
                        {
                            let socket = d.session().socket_handle();
                            let idle_deadline = last_datagram.get(&id).unwrap_or_else(clock) + read_timeout;
                            reads.push(read_one(id, socket, media, idle_deadline, Arc::clone(&infra.timer_fires), Arc::clone(&infra.timer_notify)));
                        }
                    }
                    ReadOutcome::TimedOut => {
                        tracing::warn!("whip: session idle past read timeout");
                        if let Some(d) = driver.driver_mut(id) {
                            d.finish();
                        }
                        last_datagram.remove(&id);
                        report_and_maybe_reap(&mut driver, id, route_handle, &mut progress, &active_sessions).await;
                    }
                    ReadOutcome::TransportError(reason) => {
                        tracing::warn!(error = %reason, "whip: session read failed");
                        if let Some(d) = driver.driver_mut(id) {
                            d.finish();
                        }
                        last_datagram.remove(&id);
                        report_and_maybe_reap(&mut driver, id, route_handle, &mut progress, &active_sessions).await;
                    }
                    // A fatal ICE/DTLS timer error (finding N3): log and end
                    // the session (a stuck transport can never recover).
                    ReadOutcome::TimerError(reason) => {
                        tracing::warn!(error = %reason, "whip: transport timer error; ending session");
                        if let Some(d) = driver.driver_mut(id) {
                            d.finish();
                        }
                        last_datagram.remove(&id);
                        report_and_maybe_reap(&mut driver, id, route_handle, &mut progress, &active_sessions).await;
                    }
                }
            }
        }

        // Drain every admitted session, on every loop iteration (whichever arm
        // won): a continuously-ready read can no longer starve an admission.
        loop {
            match driver.poll_accept() {
                AcceptOutcome::Idle => break,
                AcceptOutcome::Refused => {
                    tracing::warn!("whip: connection refused, max_sessions reached");
                }
                AcceptOutcome::Error(e) => return e,
                AcceptOutcome::Admitted(id) => {
                    #[cfg_attr(not(feature = "test-hooks"), allow(unused_mut))]
                    let (socket, mut media) = {
                        let s = driver
                            .driver(id)
                            .expect("just admitted by poll_accept")
                            .session();
                        (s.socket_handle(), s.take_transport())
                    };
                    // Test seam: a staged fatal timer error is applied to the
                    // freshly-admitted transport so its next `handle_timeout`
                    // ends the session (finding N3).
                    #[cfg(feature = "test-hooks")]
                    {
                        if let Some(err) = infra
                            .force_timer_error
                            .lock()
                            .expect("force_timer_error")
                            .take()
                        {
                            media.force_next_timer_error(err);
                        }
                    }
                    infra.admitted_total.fetch_add(1, Ordering::SeqCst);
                    infra.admit_count_notify.notify_waiters();
                    progress.insert(id, DriverProgress::new());
                    clocks.admit(id, clock());
                    let idle_deadline = last_datagram.set(id, clock());
                    reads.push(read_one(
                        id,
                        socket,
                        media,
                        idle_deadline + read_timeout,
                        Arc::clone(&infra.timer_fires),
                        Arc::clone(&infra.timer_notify),
                    ));
                }
                _ => break,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpStream;

    /// A well-formed SHA-256 SDP fingerprint (RFC 8122 §5) — the shape
    /// `MediaTransport::new` validates and pins to its DTLS verify callback.
    /// These unit tests never run a handshake, so no real certificate's
    /// digest is at stake; anything shorter/malformed would be rejected by
    /// `MediaTransport::new` itself (see `webrtc-runtime`'s
    /// `dtls_fingerprint` integration test for the real handshakes).
    const OFFER_FINGERPRINT: &str = "sha-256 \
00:11:22:33:44:55:66:77:88:99:aa:bb:cc:dd:ee:ff:00:11:22:33:44:55:66:77:88:\
99:aa:bb:cc:dd:ee:ff";

    const OFFER: &str = "v=0\r\n\
o=- 0 0 IN IP4 127.0.0.1\r\n\
s=-\r\n\
t=0 0\r\n\
m=video 9 UDP/TLS/RTP/SAVPF 96\r\n\
c=IN IP4 0.0.0.0\r\n\
a=ice-ufrag:abcd\r\n\
a=ice-pwd:abcdefghijklmnopqrstuvwx\r\n\
a=fingerprint:sha-256 00:11:22:33:44:55:66:77:88:99:aa:bb:cc:dd:ee:ff:\
00:11:22:33:44:55:66:77:88:99:aa:bb:cc:dd:ee:ff\r\n\
a=setup:actpass\r\n\
a=mid:0\r\n\
a=rtcp-mux\r\n\
a=rtpmap:96 H264/90000\r\n\
a=fmtp:96 level-asymmetry-allowed=1;packetization-mode=1;profile-level-id=42e01f\r\n\
a=candidate:1 1 udp 2130706431 10.0.0.5 54321 typ host\r\n";

    #[test]
    fn local_bind_ip_uses_the_signalling_local_address_not_the_peer() {
        // A non-loopback publisher (peer 192.0.2.7) reaching a server whose
        // own side of that connection is 10.0.0.1 must bind 10.0.0.1, not the
        // peer address (which is the client's, and `EADDRNOTAVAIL` on this
        // host).
        let peer: std::net::SocketAddr = "192.0.2.7:12345".parse().unwrap();
        let local = Some("10.0.0.1:8080".parse::<std::net::SocketAddr>().unwrap());
        assert_eq!(
            local_bind_ip(local, peer),
            "10.0.0.1".parse::<std::net::IpAddr>().unwrap(),
            "must bind the signalling connection's LOCAL address"
        );
        // Without a known local address, fall back to the peer's family.
        let peer_v4: std::net::SocketAddr = "192.0.2.7:12345".parse().unwrap();
        assert_eq!(
            local_bind_ip(None, peer_v4),
            std::net::IpAddr::V4(std::net::Ipv4Addr::UNSPECIFIED)
        );
        let peer_v6: std::net::SocketAddr = "[::1]:12345".parse().unwrap();
        assert_eq!(
            local_bind_ip(None, peer_v6),
            std::net::IpAddr::V6(std::net::Ipv6Addr::UNSPECIFIED)
        );
    }

    /// A poisoned pre-bound lock must NOT abort WHIP startup: `ensure_infra`
    /// recovers the inner slot and serves on the route's own bound listener.
    /// PRE-FIX the production `.expect("prebound lock")` panicked on the
    /// poisoned lock (the poisoning thread's panic propagated through the join
    /// here, so `ensure_infra` panicked rather than returning `Ok`).
    #[tokio::test]
    async fn ensure_infra_recovers_from_a_poisoned_prebound_lock() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        let route = WhipRoute::with_listener("cam", listener, DEFAULT_WHIP_MAX_SESSIONS);

        // Poison `route.prebound`: hold the lock across a panic (caught so the
        // test itself survives to observe the recovery).
        // The caught panic still prints via the default hook; harmless, and
        // avoiding a global `set_hook` keeps this test safe under the parallel
        // test runner.
        let poisoned = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = route.prebound.lock().expect("lock for poisoning");
            panic!("poison the whip prebound lock");
        }));
        assert!(poisoned.is_err(), "the poisoning panic must be caught");

        let infra = route
            .ensure_infra()
            .await
            .expect("ensure_infra must recover from a poisoned pre-bound lock");
        let token = infra.cancel.clone();
        // The caller-bound address must actually accept a connection.
        tokio::net::TcpStream::connect(addr)
            .await
            .expect("the route must still serve on its own address after a poisoned lock");
        token.cancel();
    }

    /// Defect 1 (structural): a transport with a pending timer (here a STUN
    /// gatherer, armed with `stun_server`) fires the timer arm in `read_one`
    /// with NO inbound datagram — `timer_fires` increments and the outcome is
    /// `Timer`, never a reap. If `poll_timeout` were not consulted this test
    /// could not observe any fire.
    #[tokio::test]
    async fn a_transport_timer_fires_with_no_inbound_datagram() {
        let socket = {
            let s = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
            s.set_nonblocking(true).unwrap();
            Arc::new(UdpSocket::from_std(s).unwrap())
        };
        let mut media = MediaTransport::new(
            MediaTransportConfig {
                local_addr: "127.0.0.1:0".parse().unwrap(),
                local_ice_ufrag: rand_token(8),
                local_ice_pwd: rand_token(24),
                remote_ice_ufrag: rand_token(8),
                remote_ice_pwd: rand_token(24),
                remote_fingerprint: OFFER_FINGERPRINT.into(),
                is_controlling: false,
                local_setup: SetupRole::Passive,
                // A STUN server arms the gatherer, which schedules a Binding
                // retransmit — so `poll_timeout` returns `Some` with nothing
                // inbound, the exact "vanished publisher" case under test.
                stun_server: Some("127.0.0.1:9".parse().unwrap()),
                max_remote_candidates: MAX_REMOTE_CANDIDATES,
            },
            Instant::now(),
        )
        .unwrap();
        assert!(
            media.poll_timeout().is_some(),
            "a transport with a STUN gatherer must have a pending timer"
        );

        let timer_fires = Arc::new(AtomicU64::new(0));
        let timer_notify = Arc::new(Notify::new());
        // Idle deadline far in the future so the transport-timer arm, not the
        // read-timeout arm, is what completes. No datagram is sent, so only
        // the timer arm can fire.
        let idle_deadline = Instant::now() + Duration::from_secs(3600);
        let fut = read_one(
            SessionId(0),
            socket,
            media,
            idle_deadline,
            Arc::clone(&timer_fires),
            Arc::clone(&timer_notify),
        );

        // Bound the wait generously; the STUN gatherer's first retransmit is
        // the arm that fires (real time, sub-second).
        let (_id, _media, outcome) = tokio::time::timeout(Duration::from_secs(10), fut)
            .await
            .expect("the timer arm must fire (no inbound datagram)");
        assert!(
            matches!(outcome, ReadOutcome::Timer),
            "a transport timer must fire as `Timer`, not reap: {outcome:?}"
        );
        assert!(
            timer_fires.load(Ordering::Relaxed) > 0,
            "the timer counter must observe the fire"
        );
    }

    /// `sleep_until_opt` treats an already-overdue deadline as READY
    /// immediately (it is sampled before the `select!`, so by the time the arm
    /// is polled it may already be past); `pending()` would delay a transport
    /// timer until the read timeout.
    #[tokio::test]
    async fn an_overdue_deadline_is_ready_immediately() {
        let past = Instant::now() - Duration::from_secs(1);
        tokio::time::timeout(Duration::from_secs(1), sleep_until_opt(Some(past)))
            .await
            .expect("an overdue deadline must resolve immediately, not hang");
    }

    /// Finding N3: a stuck deadline (one that keeps returning an instant at or
    /// before now) can never drive `handle_timeout` faster than the
    /// `MIN_TIMER_INTERVAL` floor. Under a paused clock, two consecutive
    /// `sleep_until_timer(Some(now), now)` waits advance elapsed time by at
    /// least the floor each — if the floor were missing the first would be
    /// `ready` and the loop would spin.
    #[tokio::test(start_paused = true)]
    async fn a_stuck_deadline_can_not_spin_faster_than_the_timer_floor() {
        // Drive many consecutive "overdue" deadlines and confirm each wait
        // advances the paused clock by at least MIN_TIMER_INTERVAL.
        for _ in 0..128 {
            let before = tokio::time::Instant::now();
            {
                let now = tokio::time::Instant::now().into_std();
                crate::webrtc_session::sleep_until_timer(Some(now), now).await;
            }
            let elapsed = before.elapsed();
            assert!(
                elapsed >= crate::webrtc_session::MIN_TIMER_INTERVAL,
                "an overdue deadline must still be floored to MIN_TIMER_INTERVAL, \
                 but only {elapsed:?} elapsed"
            );
        }
    }

    /// Finding N3: a `MediaEvent::TimerError` is surfaced (not silently
    /// dropped) and maps to an end-of-session outcome.
    #[test]
    fn a_timer_error_event_is_surfaced() {
        assert_eq!(
            first_timer_error(&[MediaEvent::TimerError("boom".into())]).as_deref(),
            Some("boom")
        );
        assert_eq!(
            first_timer_error(&[
                MediaEvent::DtlsHandshakeComplete,
                MediaEvent::IceStateChanged("connected".into()),
            ]),
            None,
            "a non-error event must not be mistaken for a timer failure"
        );
    }

    #[test]
    fn parses_video_only_offer() {
        let parsed = parse_whip_offer(OFFER).expect("parse");
        assert_eq!(parsed.payload_type, 96);
        assert_eq!(parsed.clock_rate, 90_000);
        assert_eq!(parsed.remote_ufrag, "abcd");
        assert_eq!(parsed.remote_pwd, "abcdefghijklmnopqrstuvwx");
        // `parse_remote_fingerprint` returns the typed (`sdp-types`) normalised
        // text: lower-case hash token, upper-case colon-hex digest. A fingerprint
        // is case-insensitive (RFC 8122 §5), so compare it that way.
        assert!(
            parsed
                .remote_fingerprint
                .eq_ignore_ascii_case(OFFER_FINGERPRINT),
            "{} vs {OFFER_FINGERPRINT}",
            parsed.remote_fingerprint
        );
        assert_eq!(parsed.mid, "0");
        assert_eq!(parsed.candidates.len(), 1);
    }

    /// RFC 5764 §5 / RFC 8122 §5: an offer with no `a=fingerprint` gives
    /// nothing to authenticate the peer's DTLS certificate against —
    /// rejected like any other malformed offer, never admitted unverified.
    #[test]
    fn rejects_offer_with_no_fingerprint() {
        let offer = OFFER.replace(
            "a=fingerprint:sha-256 00:11:22:33:44:55:66:77:88:99:aa:bb:cc:dd:ee:ff:\
00:11:22:33:44:55:66:77:88:99:aa:bb:cc:dd:ee:ff\r\n",
            "",
        );
        assert!(parse_whip_offer(&offer).is_err());
    }

    #[test]
    fn rejects_offer_with_no_video() {
        let offer = "v=0\r\no=- 0 0 IN IP4 127.0.0.1\r\ns=-\r\nt=0 0\r\n\
m=audio 9 UDP/TLS/RTP/SAVPF 111\r\na=ice-ufrag:x\r\na=ice-pwd:xxxxxxxxxxxxxxxxxxxxxxxx\r\n";
        assert!(parse_whip_offer(offer).is_err());
    }

    #[test]
    fn rejects_offer_with_audio_and_video() {
        // Adds an `m=audio` section ahead of the existing `m=video` one;
        // minimally well-formed is enough — the point under test is the
        // section-count rejection, not full audio semantics.
        let offer = OFFER.replace("m=video 9", "m=audio 9 UDP/TLS/RTP/SAVPF 111\r\nm=video 9");
        assert!(parse_whip_offer(&offer).is_err());
    }

    #[test]
    fn payload_type_of_extracts_pt() {
        let mut pkt = vec![0x80u8, 96];
        pkt.extend_from_slice(&[0u8; 10]);
        assert_eq!(payload_type_of(&pkt), Some(96));
        assert_eq!(payload_type_of(&[0x80]), None);
    }

    #[test]
    fn rebuild_rtp_wire_keeps_the_header_extension() {
        let pkt = webrtc_runtime::media::DecryptedRtp {
            marker: false,
            payload_type: 96,
            sequence_number: 7,
            timestamp: 3000,
            ssrc: 0x0102_0304,
            csrc: Vec::new(),
            extension: Some(webrtc_runtime::media::DecryptedRtpExtension {
                profile_id: 0xBEDE,
                data: vec![0x10, 0xAA, 0x00, 0x00],
            }),
            payload: vec![0x55],
        };
        let wire = rebuild_rtp_wire(&pkt);
        assert_eq!(wire[0], 0x80 | 0x10, "version 2 with the X bit, no CSRC");
        assert_eq!(&wire[12..14], &[0xBE, 0xDE], "defined by profile");
        assert_eq!(&wire[14..16], &[0x00, 0x01], "length = one 32-bit word");
        assert_eq!(&wire[16..20], &[0x10, 0xAA, 0x00, 0x00]);
        assert_eq!(&wire[20..], &[0x55]);
    }

    #[test]
    fn rebuild_rtp_wire_round_trips_header_fields() {
        let pkt = webrtc_runtime::media::DecryptedRtp {
            marker: true,
            payload_type: 96,
            sequence_number: 1234,
            timestamp: 90_000,
            ssrc: 0xDEAD_BEEF,
            csrc: vec![0x1111_2222],
            extension: None,
            payload: vec![0xAA, 0xBB, 0xCC],
        };
        let wire = rebuild_rtp_wire(&pkt);
        // Version=2, one CSRC.
        assert_eq!(wire[0], 0x80 | 1);
        assert_eq!(wire[1], 0x80 | 96, "marker bit set, PT 96");
        assert_eq!(u16::from_be_bytes([wire[2], wire[3]]), 1234);
        assert_eq!(
            u32::from_be_bytes([wire[4], wire[5], wire[6], wire[7]]),
            90_000
        );
        assert_eq!(
            u32::from_be_bytes([wire[8], wire[9], wire[10], wire[11]]),
            0xDEAD_BEEF
        );
        assert_eq!(
            u32::from_be_bytes([wire[12], wire[13], wire[14], wire[15]]),
            0x1111_2222
        );
        assert_eq!(&wire[16..], &[0xAA, 0xBB, 0xCC]);
    }

    /// MUTATION-CHECKED: change `try_capture_config`'s NAL-type match to
    /// require only `NAL_TYPE_SPS` (drop the PPS requirement) and this test
    /// starts capturing a config from an SPS-only sample — fails because
    /// `avc_config_from_sps_pps` is never even reached (the early-return
    /// guard `sps.is_empty() || pps.is_empty()` is the thing under test);
    /// restoring the `&&`-equivalent guard makes it pass again.
    #[tokio::test]
    async fn try_capture_config_requires_both_sps_and_pps() {
        let admitted = AdmittedWhip {
            socket: {
                let s = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
                s.set_nonblocking(true).unwrap();
                Arc::new(UdpSocket::from_std(s).unwrap())
            },
            media: MediaTransport::new(
                MediaTransportConfig {
                    local_addr: "127.0.0.1:0".parse().unwrap(),
                    local_ice_ufrag: rand_token(8),
                    local_ice_pwd: rand_token(24),
                    remote_ice_ufrag: rand_token(8),
                    remote_ice_pwd: rand_token(24),
                    is_controlling: false,
                    local_setup: SetupRole::Passive,
                    stun_server: None,
                    // This test never handshakes (it exercises the deferred
                    // `avcC` capture gate); it only needs a transport that
                    // `MediaTransport::new` accepts — i.e. a well-formed
                    // fingerprint. See `OFFER_FINGERPRINT`.
                    remote_fingerprint: OFFER_FINGERPRINT.into(),
                    max_remote_candidates: MAX_REMOTE_CANDIDATES,
                },
                Instant::now(),
            )
            .unwrap(),
            tracks: vec![WhipTrack {
                track_id: 1,
                payload_type: 96,
                clock_rate: 90_000,
            }],
        };
        let mut session = WhipIngestSession::new(admitted);

        // SPS only (type 7, 4 bytes -- the minimum `avc_config_from_sps_pps`
        // reads profile/compat/level from), no PPS: real (small,
        // hand-built-for-the-test) NAL header + profile/level bytes --
        // `try_capture_config` only reads the type nibble, never decodes the
        // RBSP any further than that.
        let sps_only = [0u8, 0, 0, 4, 0x67, 0x42, 0x00, 0x1F];
        let sample = Sample::new(
            bytes::Bytes::copy_from_slice(&sps_only),
            None,
            None,
            None,
            true,
        );
        session.try_capture_config(1, &sample);
        assert!(
            session.captured.get(&1).cloned().flatten().is_none(),
            "SPS alone must not capture a config"
        );

        // Now a real SPS+PPS pair (types 7 and 8).
        let mut both = Vec::new();
        both.extend_from_slice(&4u32.to_be_bytes());
        both.extend_from_slice(&[0x67, 0x42, 0x00, 0x1F]);
        both.extend_from_slice(&2u32.to_be_bytes());
        both.extend_from_slice(&[0x68, 0xCE]);
        let sample = Sample::new(bytes::Bytes::copy_from_slice(&both), None, None, None, true);
        session.try_capture_config(1, &sample);
        assert!(
            session.captured.get(&1).cloned().flatten().is_some(),
            "SPS+PPS must capture a config"
        );
    }

    /// `max_sessions: 0` means the very first connection is already "at
    /// capacity" — `handle_whip_connection` must answer `503` right after
    /// parsing the offer, releasing the slot it reserved to check, and
    /// never reach the `UdpSocket::bind`/`MediaTransport::new` work below
    /// it (issue r07-C11): nothing is sent to `tx`, so a driver on the
    /// A WHIP signalling server bound to an ephemeral port, plus the address
    /// to reach it and the admit-channel receiver. Kept alive by the caller.
    struct TestServer {
        addr: std::net::SocketAddr,
        rx: mpsc::Receiver<AdmittedWhip>,
        active_sessions: Arc<AtomicUsize>,
        _cancel: tokio_util::sync::CancellationToken,
    }

    async fn test_server(max_sessions: usize) -> TestServer {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = mpsc::channel::<AdmittedWhip>(1);
        let active_sessions = Arc::new(AtomicUsize::new(0));
        let state = Arc::new(WhipServeState {
            tx,
            active_sessions: Arc::clone(&active_sessions),
            max_sessions,
            admit_notify: Arc::new(Notify::new()),
        });
        let cancel = tokio_util::sync::CancellationToken::new();
        let serve_cancel = cancel.clone();
        tokio::spawn(async move {
            let _ =
                crate::origin::serve_hyper_util(listener, whip_router(state), serve_cancel).await;
        });
        TestServer {
            addr,
            rx,
            active_sessions,
            _cancel: cancel,
        }
    }

    /// Send one raw request and read the whole response (the caller closes).
    async fn exchange(addr: std::net::SocketAddr, request: &str) -> String {
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
        let mut client = TcpStream::connect(addr).await.unwrap();
        client.write_all(request.as_bytes()).await.unwrap();
        let mut buf = Vec::new();
        let _ = tokio::time::timeout(Duration::from_secs(5), client.read_to_end(&mut buf)).await;
        String::from_utf8_lossy(&buf).into_owned()
    }

    fn post_offer_request(offer: &str) -> String {
        format!(
            "POST /whip HTTP/1.1\r\nContent-Type: application/sdp\r\n\
             Content-Length: {}\r\nConnection: close\r\n\r\n{offer}",
            offer.len()
        )
    }

    /// `max_sessions: 0` means the very first connection is already "at
    /// capacity" — the router must answer `503` right after parsing the
    /// offer, releasing the slot it reserved to check, and never reach the
    /// `UdpSocket::bind`/`MediaTransport::new` work below it (issue
    /// r07-C11): nothing is sent to `tx`, so a driver on the other end never
    /// sees an admitted session at all.
    #[tokio::test]
    async fn whip_at_capacity_answers_503_without_admitting_a_session() {
        let mut server = test_server(0).await;
        let resp = exchange(server.addr, &post_offer_request(OFFER)).await;
        assert!(
            resp.starts_with("HTTP/1.1 503"),
            "expected a 503 response, got: {resp}"
        );
        assert_eq!(
            server.active_sessions.load(Ordering::SeqCst),
            0,
            "the capacity slot reserved to check must be released, not leaked"
        );
        assert!(
            matches!(server.rx.try_recv(), Err(mpsc::error::TryRecvError::Empty)),
            "a route at capacity must never admit a session"
        );
    }

    /// Audit W18: a WHIP preflight (`OPTIONS`) with
    /// `Access-Control-Request-Headers: authorization` must be answered with a
    /// CORS response that names `Authorization` (a Bearer-auth publisher needs
    /// it) and the WHIP methods.
    #[tokio::test]
    async fn whip_preflight_allows_authorization_and_methods() {
        let server = test_server(4).await;
        let request = "OPTIONS /whip HTTP/1.1\r\nOrigin: https://publisher.example\r\n\
             Access-Control-Request-Method: POST\r\n\
             Access-Control-Request-Headers: authorization\r\n\
             Connection: close\r\n\r\n";
        let resp = exchange(server.addr, request).await.to_ascii_lowercase();
        assert!(resp.starts_with("http/1.1 204"), "preflight got: {resp}");
        assert!(
            resp.contains("access-control-allow-headers: authorization"),
            "the preflight must name Authorization: {resp}"
        );
        for m in ["post", "patch", "delete"] {
            assert!(
                resp.contains(m),
                "Access-Control-Allow-Methods must include {m}: {resp}"
            );
        }
    }

    /// PRE-FIX FAILURE OBSERVED: before the `SessionSlot` RAII guard, the
    /// connection handler only decremented `active_sessions` on the
    /// *specific* `MediaTransport::new` failure via a hand-written
    /// `map_err` closure — reverting that closure back to a plain
    /// `.map_err(|e| MultimuxError::Connect { .. })?` (no decrement) leaves
    /// `active_sessions.load() == 1` after this test's request fails
    /// instead of returning to `0`; every *other* `?` on the way there
    /// (`local_addr`, `UdpSocket::bind`, the answer write) had no decrement
    /// at all even before that. The guard fixes all of them at once because
    /// it doesn't matter which `?` fires.
    #[tokio::test]
    async fn capacity_slot_is_released_when_media_transport_build_fails() {
        // A syntactically-present but too-short digest: `parse_whip_offer`
        // doesn't validate the fingerprint's shape (only that one exists),
        // so this reaches `MediaTransport::new`, which does validate it and
        // fails — after the capacity slot has already been reserved.
        let offer = OFFER.replace(
            "a=fingerprint:sha-256 00:11:22:33:44:55:66:77:88:99:aa:bb:cc:dd:ee:ff:\
00:11:22:33:44:55:66:77:88:99:aa:bb:cc:dd:ee:ff\r\n",
            "a=fingerprint:sha-256 00:11\r\n",
        );
        let server = test_server(4).await;
        let resp = exchange(server.addr, &post_offer_request(&offer)).await;
        assert!(
            !resp.starts_with("HTTP/1.1 201"),
            "a malformed fingerprint must not produce a 201: {resp}"
        );
        assert_eq!(
            server.active_sessions.load(Ordering::SeqCst),
            0,
            "the capacity slot must be released when setup fails after it was reserved"
        );
    }

    /// A session that is admitted successfully hands its slot off to the
    /// admitted-session channel's receiver, which decrements the counter
    /// exactly once when it later reaps that session — never twice (the
    /// guard must not *also* decrement on drop after a successful
    /// `disarm()`).
    #[tokio::test]
    async fn capacity_slot_is_decremented_exactly_once_after_a_normal_session() {
        let mut server = test_server(4).await;
        let resp = exchange(server.addr, &post_offer_request(OFFER)).await;
        assert!(resp.starts_with("HTTP/1.1 201"), "admitted: {resp}");
        assert_eq!(
            server.active_sessions.load(Ordering::SeqCst),
            1,
            "the slot stays reserved for the live session"
        );

        // Stands in for `report_and_maybe_reap`'s own decrement once
        // `run_whip`'s driver reaps this session — the one place that owns
        // the slot from here on.
        let _admitted = server.rx.recv().await.expect("the session was admitted");
        server.active_sessions.fetch_sub(1, Ordering::SeqCst);
        assert_eq!(
            server.active_sessions.load(Ordering::SeqCst),
            0,
            "exactly one decrement must bring the counter back to 0 — no leak, no double-release"
        );
    }

    /// Byte-for-byte golden of the WHIP signalling RESPONSE HEADERS (W2a
    /// Task 1 Step 1): the `201 Created` (POST offer), the `204 No Content`
    /// (OPTIONS preflight) and the `413 Payload Too Large` (oversized body).
    /// Taken from `main` BEFORE the axum 0.8 / tower-http 0.7 bump and the
    /// WP2.1 move onto an axum router. Only the status line and the header
    /// lines are pinned; the SDP body (and its `Content-Length`) is not.
    /// `GOLDEN_BLESS=<dir>` writes instead of comparing.
    #[tokio::test]
    async fn whip_response_headers_match_golden() {
        async fn response_headers(request: &str) -> String {
            let server = test_server(4).await;
            let text = exchange(server.addr, request).await;
            let head = text.split("\r\n\r\n").next().unwrap_or(&text);
            let mut lines = head.lines();
            let status = lines.next().unwrap_or("").to_string();
            let mut headers: Vec<String> = lines
                .filter(|l| !l.to_ascii_lowercase().starts_with("content-length:"))
                // `date` is the wall clock and `etag` a fresh random token —
                // neither is stable, so normalise both to a placeholder. The
                // golden pins the header SET, not one instant.
                .map(|l| {
                    let lower = l.to_ascii_lowercase();
                    if lower.starts_with("date:") {
                        "date: {date}".to_string()
                    } else if lower.starts_with("etag:") {
                        "etag: {etag}".to_string()
                    } else {
                        l.to_string()
                    }
                })
                .collect();
            headers.sort();
            let mut out = status;
            out.push('\n');
            for h in headers {
                out.push_str(&h);
                out.push('\n');
            }
            out
        }

        let post_offer = post_offer_request(OFFER);
        let preflight = "OPTIONS /whip HTTP/1.1\r\nOrigin: https://p.example\r\n\
             Access-Control-Request-Method: POST\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
        let over_limit = "POST /whip HTTP/1.1\r\nContent-Type: application/sdp\r\n\
             Content-Length: 131073\r\nConnection: close\r\n\r\n";

        let mut actual = String::new();
        actual.push_str("POST /whip 201\n");
        actual.push_str(&response_headers(post_offer.as_str()).await);
        actual.push_str("OPTIONS /whip 204\n");
        actual.push_str(&response_headers(preflight).await);
        actual.push_str("POST /whip 413\n");
        actual.push_str(&response_headers(over_limit).await);

        if let Ok(dir) = std::env::var("GOLDEN_BLESS") {
            std::fs::create_dir_all(&dir).expect("create golden dir");
            std::fs::write(
                std::path::Path::new(&dir).join("whip_response_headers.golden"),
                &actual,
            )
            .expect("write golden");
            return;
        }
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/golden/whip_response_headers.golden");
        let expected = std::fs::read_to_string(&path).expect("read whip header golden");
        assert_eq!(
            actual, expected,
            "WHIP response headers differ from the golden; every wire-visible \
             change (e.g. the WP2.1 router move) must be listed in the \
             multimux CHANGELOG"
        );
    }
}
