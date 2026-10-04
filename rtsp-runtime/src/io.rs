//! Async real-socket IO adapter over the sans-IO engine — RFC 2326 transport.
//!
//! The sans-IO [`ClientSession`] / [`ServerSession`] engines never touch a
//! socket: they turn method calls into request/response *bytes* and consume
//! inbound *bytes* into typed events. This module is the thin layer that
//! actually moves those bytes over a [`tokio`] socket — it owns the stream,
//! writes what the session produces, reads the peer's reply (buffering partial
//! reads until a full RTSP message or interleaved `$`-frame parses, per §10.12),
//! feeds it back through [`ClientSession::handle_data`] /
//! [`ServerSession::handle_request`], and returns the resulting events. The
//! engine stays pure; the adapter is pure plumbing.
//!
//! Both the client and server are generic over the stream type
//! (`S: AsyncRead + AsyncWrite + Unpin`), so the identical driver logic runs over
//! a plain [`tokio::net::TcpStream`] and over a TLS stream.
//!
//! # `rtsp://` vs `rtsps://`
//!
//! Plain RTSP (`rtsp://`) is carried over TCP on default port **554**
//! ([`RTSP_DEFAULT_PORT`]). RTSP-over-TLS (`rtsps://`) wraps the TCP stream in a
//! TLS session *before* any RTSP is exchanged and uses default port **322**
//! ([`RTSPS_DEFAULT_PORT`], per the IANA `rtsps` assignment). The TLS entry
//! points ([`AsyncRtspClient::connect_tls`], [`AsyncRtspServer::accept_tls`]) are
//! gated behind the `tls` feature; everything else is behind `tokio`.

use std::collections::VecDeque;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;
use tokio_util::codec::Framed;

use crate::client::{ClientEvent, ClientSession};
use crate::codec::{ClientCodec, ServerCodec, ServerFrame, bounded};
use crate::error::{Error, Result};
use crate::server::{ServerEvent, ServerSession};
use crate::transport::Transport;

/// Default TCP port for `rtsp://` (RFC 2326 §1 / IANA `rtsp`).
pub const RTSP_DEFAULT_PORT: u16 = 554;

/// Default TCP port for `rtsps://` (RTSP over TLS; IANA `rtsps`).
pub const RTSPS_DEFAULT_PORT: u16 = 322;

/// Explicit bounds on every awaited IO of the RTSP adapters (W1 SP1.3): no wait
/// is unbounded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct RtspTimeouts {
    /// TCP connect. Default 10 s.
    pub connect: Duration,
    /// TLS handshake (rtsps) / first complete request on a server connection. Default 10 s.
    pub handshake: Duration,
    /// Longest wait for the next complete frame (a response, a request, an interleaved frame).
    /// It bounds the whole frame, not each byte, so a peer dripping one byte at a time still
    /// times out. Default 30 s.
    pub read_idle: Duration,
    /// Longest wait for a write to be accepted by the socket. Default 10 s.
    pub write: Duration,
}

impl Default for RtspTimeouts {
    fn default() -> Self {
        Self {
            connect: Duration::from_secs(10),
            handshake: Duration::from_secs(10),
            read_idle: Duration::from_secs(30),
            write: Duration::from_secs(10),
        }
    }
}

impl RtspTimeouts {
    /// Sets [`connect`](Self::connect).
    #[must_use]
    pub fn with_connect(mut self, d: Duration) -> Self {
        self.connect = d;
        self
    }
    /// Sets [`handshake`](Self::handshake).
    #[must_use]
    pub fn with_handshake(mut self, d: Duration) -> Self {
        self.handshake = d;
        self
    }
    /// Sets [`read_idle`](Self::read_idle).
    #[must_use]
    pub fn with_read_idle(mut self, d: Duration) -> Self {
        self.read_idle = d;
        self
    }
    /// Sets [`write`](Self::write).
    #[must_use]
    pub fn with_write(mut self, d: Duration) -> Self {
        self.write = d;
        self
    }
}

/// Maximum interleaved [`ClientEvent::MediaData`] frames held in
/// `pending_media` awaiting [`AsyncRtspClient::recv_interleaved`]. A camera
/// that never answers a `GET_PARAMETER` (or any other control request) while
/// still streaming media would otherwise grow this queue without bound while
/// [`AsyncRtspClient::exchange`] keeps reading and waiting for the response
/// (audit run-09 W7). Once full, the oldest frame is dropped — for a live
/// stream a fresher sample is worth more than one still queued.
const MAX_PENDING_MEDIA_FRAMES: usize = 1024;

/// The current instant as a `std` one, virtual-time aware under tokio's paused clock.
fn now() -> std::time::Instant {
    tokio::time::Instant::now().into_std()
}

/// Maps a tokio IO error into the crate error type.
fn io_err(context: &str, e: std::io::Error) -> Error {
    Error::Io(format!("{context}: {e}"))
}

// ===========================================================================
// Client
// ===========================================================================

/// An async RTSP client that owns a socket and drives a [`ClientSession`].
///
/// Each request method (`options`/`describe`/`setup`/`play`/`pause`/`teardown`)
/// writes the request bytes the session produces, reads the response off the
/// socket, feeds it through the sans-IO engine, and returns the resulting
/// [`ClientEvent`]s. A Digest `401` is answered transparently: when the engine
/// emits [`ClientEvent::AuthRetry`], the adapter writes the retried request and
/// reads its response before returning, so the caller only sees the final
/// [`ClientEvent::Response`].
///
/// Interleaved media (`$`-framed RTP/RTCP, §10.12) is pulled with
/// [`recv_interleaved`](Self::recv_interleaved).
#[derive(Debug)]
pub struct AsyncRtspClient<S> {
    framed: Framed<S, ClientCodec>,
    /// Media events surfaced while awaiting a response (e.g. interleaved frames
    /// arriving between control messages), buffered for `recv_interleaved`.
    pending_media: VecDeque<ClientEvent>,
    timeouts: RtspTimeouts,
}

impl AsyncRtspClient<TcpStream> {
    /// Connects a plain-TCP (`rtsp://`) client to `addr`.
    ///
    /// `addr` is any [`tokio::net::ToSocketAddrs`]; for the RTSP default port use
    /// `(host, RTSP_DEFAULT_PORT)`.
    pub async fn connect<A: tokio::net::ToSocketAddrs>(addr: A) -> Result<Self> {
        Self::connect_with_timeouts(addr, ClientSession::new(), RtspTimeouts::default()).await
    }

    /// Connects a plain-TCP client to `addr` using a pre-configured session
    /// (e.g. one carrying [`Credentials`](crate::Credentials)).
    pub async fn connect_with<A: tokio::net::ToSocketAddrs>(
        addr: A,
        session: ClientSession,
    ) -> Result<Self> {
        Self::connect_with_timeouts(addr, session, RtspTimeouts::default()).await
    }

    /// Like [`connect_with`](Self::connect_with), with explicit [`RtspTimeouts`].
    pub async fn connect_with_timeouts<A: tokio::net::ToSocketAddrs>(
        addr: A,
        session: ClientSession,
        timeouts: RtspTimeouts,
    ) -> Result<Self> {
        let stream = bounded(timeouts.connect, "connect", async {
            TcpStream::connect(addr)
                .await
                .map_err(|e| io_err("connect", e))
        })
        .await?;
        Ok(Self::with_stream_timeouts(stream, session, timeouts))
    }
}

impl<S> AsyncRtspClient<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    /// Wraps an already-connected stream (plain or TLS) and a session, with the
    /// default [`RtspTimeouts`].
    pub fn with_stream(stream: S, session: ClientSession) -> Self {
        Self::with_stream_timeouts(stream, session, RtspTimeouts::default())
    }

    /// Wraps an already-connected stream with explicit [`RtspTimeouts`].
    pub fn with_stream_timeouts(stream: S, session: ClientSession, timeouts: RtspTimeouts) -> Self {
        AsyncRtspClient {
            framed: Framed::new(stream, ClientCodec::new(session)),
            pending_media: VecDeque::new(),
            timeouts,
        }
    }

    /// The timeouts this adapter applies.
    pub fn timeouts(&self) -> RtspTimeouts {
        self.timeouts
    }

    /// The current session state.
    pub fn state(&self) -> crate::SessionState {
        self.framed.codec().session.state()
    }

    /// The negotiated session id, once a SETUP response has been processed.
    pub fn session_id(&self) -> Option<&str> {
        self.framed.codec().session.session_id()
    }

    /// Borrows the underlying sans-IO session (read-only inspection).
    pub fn session(&self) -> &ClientSession {
        &self.framed.codec().session
    }

    async fn write(&mut self, bytes: Vec<u8>) -> Result<()> {
        bounded(self.timeouts.write, "write", self.framed.send(bytes)).await?;
        self.framed.codec_mut().session.mark_activity(now());
        Ok(())
    }

    /// The next decoded event; `Ok(None)` = clean EOF. One deadline covers the WHOLE
    /// frame (see [`RtspTimeouts::read_idle`]).
    async fn next_event(&mut self) -> Result<Option<ClientEvent>> {
        // Inbound events (media, responses) deliberately do NOT count as keepalive
        // activity: the server's session timeout is refreshed by RTSP *requests*
        // (RFC 2326 §12.37), so a busy interleaved stream must not postpone the
        // GET_PARAMETER keepalive.
        match tokio::time::timeout(self.timeouts.read_idle, self.framed.next()).await {
            Err(_) => Err(Error::Timeout { what: "read" }),
            Ok(None) => Ok(None),
            Ok(Some(r)) => r.map(Some),
        }
    }

    /// Sends `OPTIONS` and awaits the response.
    pub async fn options(&mut self, uri: &str) -> Result<ClientEvent> {
        let cseq = self.framed.codec().session.peek_next_cseq();
        let bytes = self.framed.codec_mut().session.options(uri)?;
        self.exchange(cseq, bytes).await
    }

    /// Sends `DESCRIBE` (with `Accept: application/sdp`) and awaits the response.
    pub async fn describe(&mut self, uri: &str) -> Result<ClientEvent> {
        let cseq = self.framed.codec().session.peek_next_cseq();
        let bytes = self.framed.codec_mut().session.describe(uri)?;
        self.exchange(cseq, bytes).await
    }

    /// Sends `SETUP` carrying `transport` and awaits the response.
    pub async fn setup(&mut self, uri: &str, transport: &Transport) -> Result<ClientEvent> {
        let cseq = self.framed.codec().session.peek_next_cseq();
        let bytes = self.framed.codec_mut().session.setup(uri, transport)?;
        self.exchange(cseq, bytes).await
    }

    /// Sends `PLAY` and awaits the response.
    pub async fn play(&mut self, uri: &str) -> Result<ClientEvent> {
        let cseq = self.framed.codec().session.peek_next_cseq();
        let bytes = self.framed.codec_mut().session.play(uri)?;
        self.exchange(cseq, bytes).await
    }

    /// Sends `PAUSE` and awaits the response.
    pub async fn pause(&mut self, uri: &str) -> Result<ClientEvent> {
        let cseq = self.framed.codec().session.peek_next_cseq();
        let bytes = self.framed.codec_mut().session.pause(uri)?;
        self.exchange(cseq, bytes).await
    }

    /// Sends `TEARDOWN` and awaits the response.
    pub async fn teardown(&mut self, uri: &str) -> Result<ClientEvent> {
        let cseq = self.framed.codec().session.peek_next_cseq();
        let bytes = self.framed.codec_mut().session.teardown(uri)?;
        self.exchange(cseq, bytes).await
    }

    /// Sends `GET_PARAMETER` (empty body = liveness ping) and awaits the response.
    pub async fn get_parameter(&mut self, uri: &str, body: &[u8]) -> Result<ClientEvent> {
        let cseq = self.framed.codec().session.peek_next_cseq();
        let bytes = self.framed.codec_mut().session.get_parameter(uri, body)?;
        self.exchange(cseq, bytes).await
    }

    /// Sends `ANNOUNCE` carrying `sdp` and awaits the response (RFC 2326
    /// §10.3) — the first request of an RTSP push (`ANNOUNCE` -> `SETUP` ->
    /// [`record`](Self::record)).
    pub async fn announce(&mut self, uri: &str, sdp: &str) -> Result<ClientEvent> {
        let cseq = self.framed.codec().session.peek_next_cseq();
        let bytes = self.framed.codec_mut().session.announce(uri, sdp)?;
        self.exchange(cseq, bytes).await
    }

    /// Sends `RECORD` and awaits the response (RFC 2326 §10.11) — starts the
    /// media push negotiated by a preceding [`announce`](Self::announce) +
    /// [`setup`](Self::setup).
    pub async fn record(&mut self, uri: &str) -> Result<ClientEvent> {
        let cseq = self.framed.codec().session.peek_next_cseq();
        let bytes = self.framed.codec_mut().session.record(uri)?;
        self.exchange(cseq, bytes).await
    }

    /// Sends `payload` as an interleaved (`$`-framed) media packet on `channel`
    /// (§10.12) — the mirror of the server's
    /// [`AsyncRtspServer::send_interleaved`], for an RTSP push that delivers
    /// media interleaved over the control connection. The frame is built by
    /// [`crate::interleaved::InterleavedFrame::new`] and the write is bounded
    /// by [`RtspTimeouts::write`].
    pub async fn send_interleaved(&mut self, channel: u8, payload: &[u8]) -> Result<()> {
        let frame = crate::interleaved::InterleavedFrame::new(channel, payload.to_vec());
        let bytes = frame.to_bytes()?;
        bounded(self.timeouts.write, "write", self.framed.send(bytes)).await
    }

    /// Writes an outbound request and reads until the response correlated to
    /// `cseq` (the `CSeq` just assigned to `request`) arrives, transparently
    /// completing any Digest `AuthRetry` round-trip.
    ///
    /// `cseq` is required (rather than reading it back off the `Response`
    /// event) because a stray response can otherwise be mistaken for this
    /// one: if a *previous* `exchange` call's future was dropped (e.g. by a
    /// caller-side `tokio::time::timeout`) after it had already written its
    /// request, that request's `Pending` entry stays in the session, and its
    /// late-arriving response is returned here later unless it's filtered by
    /// `CSeq` (audit run-09 W7). Such a stray response is simply dropped: no
    /// one is waiting for it any more.
    ///
    /// Every awaited IO is bounded by [`RtspTimeouts`]: a write that the peer
    /// does not accept is [`Error::Timeout`] `{ what: "write" }`, a response that
    /// does not complete in `read_idle` is `{ what: "read" }`. Bytes already read
    /// stay in the framed buffer if this future is dropped, but a caller relying
    /// on getting *this* response back should not drop and retry: build a fresh
    /// request instead.
    ///
    /// Interleaved media frames that arrive before the response are buffered and
    /// later returned by [`recv_interleaved`](Self::recv_interleaved).
    async fn exchange(&mut self, cseq: u32, request: Vec<u8>) -> Result<ClientEvent> {
        // Tracks whichever CSeq we're currently waiting an answer for: a 401
        // retry re-signs the SAME logical request under a NEW CSeq (a fresh
        // `Pending` entry), so once we see our request's `AuthRetry`, we must
        // switch to waiting for ITS CSeq instead.
        let mut cseq = cseq;
        self.write(request).await?;
        loop {
            let Some(event) = self.next_event().await? else {
                return Err(Error::Io("peer closed connection before response".into()));
            };
            match event {
                // Only the response whose CSeq matches the request THIS call
                // sent answers it; anything else (a duplicate, or a response
                // for a request a previous, abandoned `exchange` call sent) is
                // a stray and is dropped. Interleaved media that the codec
                // decoded in the same read stays queued in the codec for the
                // next call, so it is not lost (§10.12).
                ClientEvent::Response { cseq: rcseq, .. } if rcseq == cseq => return Ok(event),
                ClientEvent::Response { .. } => {}
                ClientEvent::AuthRetry {
                    cseq: retry_cseq,
                    ref request,
                    ..
                } => {
                    // Write the retried (now-authenticated) request and wait
                    // for ITS CSeq from here on.
                    let retry = request.clone();
                    self.write(retry).await?;
                    cseq = retry_cseq;
                }
                ClientEvent::MediaData { .. } => {
                    if self.pending_media.len() >= MAX_PENDING_MEDIA_FRAMES {
                        self.pending_media.pop_front();
                    }
                    self.pending_media.push_back(event);
                }
            }
        }
    }

    /// Receives the next interleaved media frame ([`ClientEvent::MediaData`]),
    /// driving the socket until one is available (§10.12).
    ///
    /// Returns `Ok(None)` if the peer closes the connection cleanly before a
    /// frame arrives. Any control responses interleaved with media are consumed
    /// and their state transitions applied, but not returned here.
    ///
    /// While waiting, a `GET_PARAMETER` keepalive is sent at half the session
    /// timeout ([`ClientSession::poll_timeout`]), so an idle-but-healthy
    /// stream does not lose its session.
    pub async fn recv_interleaved(&mut self) -> Result<Option<ClientEvent>> {
        enum Woke {
            Frame(Result<Option<ClientEvent>>),
            Keepalive,
        }
        loop {
            if let Some(event) = self.pending_media.pop_front() {
                return Ok(Some(event));
            }
            let deadline = self
                .framed
                .codec()
                .session
                .poll_timeout()
                .map(tokio::time::Instant::from_std);
            let woke = {
                let next = self.next_event();
                tokio::pin!(next);
                match deadline {
                    Some(d) => tokio::select! {
                        r = &mut next => Woke::Frame(r),
                        _ = tokio::time::sleep_until(d) => Woke::Keepalive,
                    },
                    None => Woke::Frame(next.await),
                }
            };
            match woke {
                Woke::Frame(r) => match r? {
                    None if self.framed.codec().session.has_buffered_input() => {
                        return Err(Error::Io("peer closed connection mid-message".into()));
                    }
                    None => return Ok(None),
                    Some(ev @ ClientEvent::MediaData { .. }) => return Ok(Some(ev)),
                    Some(_) => {} // control responses are applied by the session; not surfaced here
                },
                Woke::Keepalive => {
                    if let Some(req) = self.framed.codec_mut().session.handle_timeout(now())? {
                        self.write(req).await?;
                    }
                }
            }
        }
    }
}

#[cfg(feature = "tls")]
impl AsyncRtspClient<tokio_rustls::client::TlsStream<TcpStream>> {
    /// Connects an `rtsps://` (TLS) client to `addr`, verifying the server
    /// against the given `config` and presenting `server_name` for SNI/cert
    /// validation.
    ///
    /// For the public-CA default trust store, build `config` with
    /// [`default_tls_client_config`]. For a self-signed camera cert, build a
    /// [`rustls::ClientConfig`] whose root store contains that cert. For the
    /// `rtsps` default port use `(host, RTSPS_DEFAULT_PORT)`.
    pub async fn connect_tls<A: tokio::net::ToSocketAddrs>(
        addr: A,
        server_name: &str,
        config: rustls::ClientConfig,
    ) -> Result<Self> {
        Self::connect_tls_with(addr, server_name, config, ClientSession::new()).await
    }

    /// Connects an `rtsps://` (TLS) client to `addr` using a pre-configured
    /// session (e.g. one carrying [`Credentials`](crate::Credentials) via
    /// [`ClientSession::with_credentials`]), otherwise identical to
    /// [`connect_tls`](Self::connect_tls).
    pub async fn connect_tls_with<A: tokio::net::ToSocketAddrs>(
        addr: A,
        server_name: &str,
        config: rustls::ClientConfig,
        session: ClientSession,
    ) -> Result<Self> {
        Self::connect_tls_with_timeouts(addr, server_name, config, session, RtspTimeouts::default())
            .await
    }

    /// Like [`connect_tls_with`](Self::connect_tls_with), with explicit
    /// [`RtspTimeouts`]: `connect` bounds the TCP connect, `handshake` the TLS
    /// handshake.
    pub async fn connect_tls_with_timeouts<A: tokio::net::ToSocketAddrs>(
        addr: A,
        server_name: &str,
        config: rustls::ClientConfig,
        session: ClientSession,
        timeouts: RtspTimeouts,
    ) -> Result<Self> {
        let addr = addr;
        connect_tls_via(
            async move {
                TcpStream::connect(addr)
                    .await
                    .map_err(|e| io_err("connect", e))
            },
            server_name,
            config,
            session,
            timeouts,
        )
        .await
    }
}

/// TLS client setup over any dialled transport: `connect` bounds the dial,
/// `handshake` the TLS handshake. Generic so the bounds are testable without a network.
#[cfg(feature = "tls")]
async fn connect_tls_via<T>(
    dial: impl std::future::Future<Output = Result<T>>,
    server_name: &str,
    config: rustls::ClientConfig,
    session: ClientSession,
    timeouts: RtspTimeouts,
) -> Result<AsyncRtspClient<tokio_rustls::client::TlsStream<T>>>
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    use std::sync::Arc;
    use tokio_rustls::TlsConnector;

    let tcp = bounded(timeouts.connect, "connect", dial).await?;
    let connector = TlsConnector::from(Arc::new(config));
    let dns = rustls::pki_types::ServerName::try_from(server_name.to_string())
        .map_err(|e| Error::Tls(format!("invalid server name {server_name:?}: {e}")))?;
    let stream = bounded(timeouts.handshake, "handshake", async {
        connector
            .connect(dns, tcp)
            .await
            .map_err(|e| io_err("TLS handshake", e))
    })
    .await?;
    Ok(AsyncRtspClient::with_stream_timeouts(
        stream, session, timeouts,
    ))
}

/// Builds a [`rustls::ClientConfig`] trusting the `webpki-roots` public-CA
/// bundle (the default trust store for `rtsps://` to a well-known server).
///
/// For a self-signed camera cert, construct the config directly with a root
/// store containing that cert and pass it to
/// [`AsyncRtspClient::connect_tls`].
#[cfg(feature = "tls")]
pub fn default_tls_client_config() -> rustls::ClientConfig {
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    // Select the aws-lc-rs provider explicitly rather than via
    // `ClientConfig::builder()`'s process-global default: another crate in the
    // same build (e.g. a `reqwest` that pulls `aws-lc-rs`) can put a *second*
    // `CryptoProvider` in the tree, leaving no unambiguous default and making
    // the plain builder panic. Choosing the provider here keeps `rtsp-runtime`
    // working regardless of what else is linked.
    rustls::ClientConfig::builder_with_provider(std::sync::Arc::new(
        rustls::crypto::aws_lc_rs::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .expect("aws-lc-rs provider supports the safe default protocol versions")
    .with_root_certificates(roots)
    .with_no_client_auth()
}

// ===========================================================================
// Server
// ===========================================================================

/// An async RTSP server connection that owns a socket and drives a
/// [`ServerSession`].
///
/// Reads requests off the socket (framed by a `tokio_util` codec over
/// `rtsp_types`), calls [`ServerSession::handle_request`], writes the response
/// bytes back, and returns the [`ServerEvent`]s. Every awaited IO is bounded by
/// [`RtspTimeouts`].
#[derive(Debug)]
pub struct AsyncRtspServer<S> {
    framed: Framed<S, ServerCodec>,
    session: ServerSession,
    timeouts: RtspTimeouts,
    first_request_seen: bool,
    /// Events of a request whose response is still being written (cancel safety).
    ready: Option<Vec<ServerEvent>>,
}

impl AsyncRtspServer<TcpStream> {
    /// Wraps an accepted plain-TCP connection with a fresh [`ServerSession`]
    /// whose `Session` ids come from the OS random source.
    ///
    /// # Panics
    ///
    /// When the OS random source is unavailable.
    pub fn accept(stream: TcpStream) -> Self {
        Self::with_stream(stream, ServerSession::new(os_session_id))
    }

    /// Wraps an accepted plain-TCP connection with a pre-configured session.
    pub fn accept_with(stream: TcpStream, session: ServerSession) -> Self {
        Self::with_stream(stream, session)
    }

    /// Like [`accept_with`](Self::accept_with), with explicit [`RtspTimeouts`].
    pub fn accept_with_timeouts(
        stream: TcpStream,
        session: ServerSession,
        timeouts: RtspTimeouts,
    ) -> Self {
        Self::with_stream_timeouts(stream, session, timeouts)
    }
}

#[cfg(feature = "tls")]
impl AsyncRtspServer<tokio_rustls::server::TlsStream<TcpStream>> {
    /// Performs the TLS handshake over an accepted TCP connection (an
    /// `rtsps://` server), then wraps the TLS stream with a fresh session.
    pub async fn accept_tls(stream: TcpStream, config: rustls::ServerConfig) -> Result<Self> {
        Self::accept_tls_with_timeouts(stream, config, RtspTimeouts::default()).await
    }

    /// Like [`accept_tls`](Self::accept_tls), with explicit [`RtspTimeouts`]:
    /// `handshake` bounds the TLS handshake.
    pub async fn accept_tls_with_timeouts(
        stream: TcpStream,
        config: rustls::ServerConfig,
        timeouts: RtspTimeouts,
    ) -> Result<Self> {
        accept_tls_via(stream, config, timeouts).await
    }
}

/// TLS server setup over any accepted transport; `handshake` bounds the handshake.
#[cfg(feature = "tls")]
async fn accept_tls_via<T>(
    stream: T,
    config: rustls::ServerConfig,
    timeouts: RtspTimeouts,
) -> Result<AsyncRtspServer<tokio_rustls::server::TlsStream<T>>>
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    use std::sync::Arc;
    use tokio_rustls::TlsAcceptor;

    let acceptor = TlsAcceptor::from(Arc::new(config));
    let tls = bounded(timeouts.handshake, "handshake", async {
        acceptor
            .accept(stream)
            .await
            .map_err(|e| io_err("TLS handshake", e))
    })
    .await?;
    Ok(AsyncRtspServer::with_stream_timeouts(
        tls,
        ServerSession::new(os_session_id),
        timeouts,
    ))
}

/// A `Session` id value from the OS random source (RFC 2326 §3.4).
fn os_session_id() -> u64 {
    let mut bytes = [0u8; 8];
    getrandom::getrandom(&mut bytes).expect("OS random source unavailable");
    u64::from_ne_bytes(bytes)
}

impl<S> AsyncRtspServer<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    /// Wraps an already-connected stream (plain or TLS) and a session, with the
    /// default [`RtspTimeouts`].
    pub fn with_stream(stream: S, session: ServerSession) -> Self {
        Self::with_stream_timeouts(stream, session, RtspTimeouts::default())
    }

    /// Wraps an already-connected stream with explicit [`RtspTimeouts`].
    pub fn with_stream_timeouts(stream: S, session: ServerSession, timeouts: RtspTimeouts) -> Self {
        AsyncRtspServer {
            framed: Framed::new(stream, ServerCodec::default()),
            session,
            timeouts,
            first_request_seen: false,
            ready: None,
        }
    }

    /// The timeouts this adapter applies.
    pub fn timeouts(&self) -> RtspTimeouts {
        self.timeouts
    }

    /// The current session state.
    pub fn state(&self) -> crate::SessionState {
        self.session.state()
    }

    /// The allocated session id, once a SETUP has been handled.
    pub fn session_id(&self) -> Option<&str> {
        self.session.session_id()
    }

    /// Mutable access to the underlying stream, for writing raw bytes (e.g.
    /// deliberately fragmenting an interleaved frame, or sending several frames
    /// back-to-back) alongside the framed [`send_interleaved`](Self::send_interleaved)
    /// helper.
    pub fn stream_mut(&mut self) -> &mut S {
        self.framed.get_mut()
    }

    /// Reads the next complete request or interleaved frame, handling a
    /// request (writing the response back) and returning the produced
    /// events, or surfacing an interleaved `$`-frame as
    /// [`ServerEvent::MediaData`].
    ///
    /// The FIRST call on a connection waits at most [`RtspTimeouts::handshake`]
    /// for a complete request; later calls wait at most
    /// [`RtspTimeouts::read_idle`]. A frame must complete inside that single
    /// deadline, so a client dripping bytes cannot hold the connection forever.
    ///
    /// Cancel-safe: dropping the future while the response is being written
    /// loses neither the response bytes nor the events; the next call finishes
    /// the write and returns them.
    ///
    /// Returns `Ok(None)` when the peer closes the connection cleanly before a
    /// full request or frame arrives.
    pub async fn next_request(&mut self) -> Result<Option<Vec<ServerEvent>>> {
        let limit = if self.first_request_seen {
            self.timeouts.read_idle
        } else {
            self.timeouts.handshake
        };
        loop {
            // 1. Finish any response write a cancelled earlier call left in the
            //    framed write buffer (never re-sent: `flush` drains it once).
            bounded(self.timeouts.write, "write", self.framed.flush()).await?;
            // 2. Hand out the events of a request whose response is now fully written.
            if let Some(events) = self.ready.take() {
                return Ok(Some(events));
            }
            // 3. Read the next frame under ONE deadline.
            let frame = match tokio::time::timeout(limit, self.framed.next()).await {
                Err(_) => return Err(Error::Timeout { what: "read" }),
                Ok(None) => return Ok(None),
                Ok(Some(frame)) => frame?,
            };
            self.first_request_seen = true;
            match frame {
                ServerFrame::Media { channel, data } => {
                    return Ok(Some(vec![ServerEvent::MediaData { channel, data }]));
                }
                ServerFrame::Request(bytes) => {
                    // Synchronous from here to the next await: the session has
                    // advanced, so its response and events are recorded together;
                    // a drop during the flush loses neither.
                    let (response, events) = self.session.handle_request(&bytes)?;
                    self.framed.write_buffer_mut().extend_from_slice(&response);
                    self.ready = Some(events);
                }
            }
        }
    }

    /// Sends an interleaved (`$`-framed) media frame to the client on `channel`
    /// (§10.12), e.g. an RTP or RTCP packet during PLAY.
    pub async fn send_interleaved(&mut self, channel: u8, payload: &[u8]) -> Result<()> {
        let frame = crate::interleaved::InterleavedFrame::new(channel, payload.to_vec());
        let bytes = frame.to_bytes()?;
        bounded(self.timeouts.write, "write", self.framed.send(bytes)).await
    }
}

#[cfg(all(test, feature = "tls"))]
mod tls_timeout_tests {
    //! The rtsps connect / handshake bounds (review: untested). Paused virtual time over
    //! in-memory pipes; the peer is alive but silent.
    use super::*;
    use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
    use tokio::io::{DuplexStream, duplex};

    fn timeouts() -> RtspTimeouts {
        RtspTimeouts::default()
            .with_connect(Duration::from_secs(3))
            .with_handshake(Duration::from_secs(4))
    }

    /// Virtual-time hang guard: a missing bound fails instead of hanging the run.
    async fn guard<F: std::future::Future>(f: F) -> F::Output {
        tokio::time::timeout(Duration::from_secs(60), f)
            .await
            .expect("the connect/handshake bound did not fire")
    }

    fn server_config() -> rustls::ServerConfig {
        let cert = include_bytes!("../tests/fixtures/localhost-cert.der").to_vec();
        let key = include_bytes!("../tests/fixtures/localhost-key.der").to_vec();
        rustls::ServerConfig::builder_with_provider(std::sync::Arc::new(
            rustls::crypto::aws_lc_rs::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(
            vec![CertificateDer::from(cert)],
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key)),
        )
        .unwrap()
    }

    #[tokio::test(start_paused = true)]
    async fn a_dial_that_never_completes_times_out_at_connect() {
        let r = guard(connect_tls_via::<DuplexStream>(
            std::future::pending(),
            "localhost",
            default_tls_client_config(),
            ClientSession::new(),
            timeouts(),
        ))
        .await;
        assert!(
            matches!(r, Err(Error::Timeout { what: "connect" })),
            "{:?}",
            r.err()
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_silent_server_times_out_the_client_tls_handshake() {
        let (client_io, _server_io) = duplex(4096);
        let r = guard(connect_tls_via(
            async { Ok(client_io) },
            "localhost",
            default_tls_client_config(),
            ClientSession::new(),
            timeouts(),
        ))
        .await;
        assert!(
            matches!(r, Err(Error::Timeout { what: "handshake" })),
            "{:?}",
            r.err()
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_silent_client_times_out_the_server_tls_handshake() {
        let (_client_io, server_io) = duplex(4096);
        let r = guard(accept_tls_via(server_io, server_config(), timeouts())).await;
        assert!(
            matches!(r, Err(Error::Timeout { what: "handshake" })),
            "{:?}",
            r.err()
        );
    }
}
