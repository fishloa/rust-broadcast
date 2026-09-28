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

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;

use rtsp_types::Message;

use crate::client::{ClientEvent, ClientSession};
use crate::error::{Error, Result};
use crate::interleaved::MAGIC;
use crate::server::{ServerEvent, ServerSession};
use crate::transport::Transport;

/// A message body type: owned bytes.
type Body = Vec<u8>;

/// Default TCP port for `rtsp://` (RFC 2326 §1 / IANA `rtsp`).
pub const RTSP_DEFAULT_PORT: u16 = 554;

/// Default TCP port for `rtsps://` (RTSP over TLS; IANA `rtsps`).
pub const RTSPS_DEFAULT_PORT: u16 = 322;

/// Size of one socket read chunk. Reads accumulate into an internal buffer, so
/// this only bounds a single `read` syscall, not a message.
const READ_CHUNK: usize = 8192;

/// Maximum interleaved [`ClientEvent::MediaData`] frames held in
/// `pending_media` awaiting [`AsyncRtspClient::recv_interleaved`]. A camera
/// that never answers a `GET_PARAMETER` (or any other control request) while
/// still streaming media would otherwise grow this queue without bound while
/// [`AsyncRtspClient::exchange`] keeps reading and waiting for the response
/// (audit run-09 W7). Once full, the oldest frame is dropped — for a live
/// stream a fresher sample is worth more than one still queued.
const MAX_PENDING_MEDIA_FRAMES: usize = 1024;

/// Maximum bytes retained in [`AsyncRtspServer::read_buf`](AsyncRtspServer)
/// awaiting a complete request. Without a cap, a `Content-Length` promising
/// more than the peer ever sends (or one that just never terminates its
/// header) grows this buffer without bound (audit run-09 W8). 2 MiB covers a
/// large `ANNOUNCE` SDP body with headroom.
const MAX_SERVER_READ_BUFFER: usize = 2 * 1024 * 1024;

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
    stream: S,
    session: ClientSession,
    /// Bytes read from the socket but not yet fully parsed (partial message or
    /// `$`-frame tail), plus any already-decoded events not yet drained.
    read_buf: Vec<u8>,
    /// Media events surfaced while awaiting a response (e.g. interleaved frames
    /// arriving between control messages), buffered for `recv_interleaved`.
    pending_media: std::collections::VecDeque<ClientEvent>,
}

impl AsyncRtspClient<TcpStream> {
    /// Connects a plain-TCP (`rtsp://`) client to `addr`.
    ///
    /// `addr` is any [`tokio::net::ToSocketAddrs`]; for the RTSP default port use
    /// `(host, RTSP_DEFAULT_PORT)`.
    pub async fn connect<A: tokio::net::ToSocketAddrs>(addr: A) -> Result<Self> {
        let stream = TcpStream::connect(addr)
            .await
            .map_err(|e| io_err("connect", e))?;
        Ok(Self::with_stream(stream, ClientSession::new()))
    }

    /// Connects a plain-TCP client to `addr` using a pre-configured session
    /// (e.g. one carrying [`Credentials`](crate::Credentials)).
    pub async fn connect_with<A: tokio::net::ToSocketAddrs>(
        addr: A,
        session: ClientSession,
    ) -> Result<Self> {
        let stream = TcpStream::connect(addr)
            .await
            .map_err(|e| io_err("connect", e))?;
        Ok(Self::with_stream(stream, session))
    }
}

impl<S> AsyncRtspClient<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    /// Wraps an already-connected stream (plain or TLS) and a session.
    pub fn with_stream(stream: S, session: ClientSession) -> Self {
        AsyncRtspClient {
            stream,
            session,
            read_buf: Vec::new(),
            pending_media: std::collections::VecDeque::new(),
        }
    }

    /// The current session state.
    pub fn state(&self) -> crate::SessionState {
        self.session.state()
    }

    /// The negotiated session id, once a SETUP response has been processed.
    pub fn session_id(&self) -> Option<&str> {
        self.session.session_id()
    }

    /// Borrows the underlying sans-IO session (read-only inspection).
    pub fn session(&self) -> &ClientSession {
        &self.session
    }

    /// Sends `OPTIONS` and awaits the response.
    pub async fn options(&mut self, uri: &str) -> Result<ClientEvent> {
        let cseq = self.session.peek_next_cseq();
        let bytes = self.session.options(uri)?;
        self.exchange(cseq, bytes).await
    }

    /// Sends `DESCRIBE` (with `Accept: application/sdp`) and awaits the response.
    pub async fn describe(&mut self, uri: &str) -> Result<ClientEvent> {
        let cseq = self.session.peek_next_cseq();
        let bytes = self.session.describe(uri)?;
        self.exchange(cseq, bytes).await
    }

    /// Sends `SETUP` carrying `transport` and awaits the response.
    pub async fn setup(&mut self, uri: &str, transport: &Transport) -> Result<ClientEvent> {
        let cseq = self.session.peek_next_cseq();
        let bytes = self.session.setup(uri, transport)?;
        self.exchange(cseq, bytes).await
    }

    /// Sends `PLAY` and awaits the response.
    pub async fn play(&mut self, uri: &str) -> Result<ClientEvent> {
        let cseq = self.session.peek_next_cseq();
        let bytes = self.session.play(uri)?;
        self.exchange(cseq, bytes).await
    }

    /// Sends `PAUSE` and awaits the response.
    pub async fn pause(&mut self, uri: &str) -> Result<ClientEvent> {
        let cseq = self.session.peek_next_cseq();
        let bytes = self.session.pause(uri)?;
        self.exchange(cseq, bytes).await
    }

    /// Sends `TEARDOWN` and awaits the response.
    pub async fn teardown(&mut self, uri: &str) -> Result<ClientEvent> {
        let cseq = self.session.peek_next_cseq();
        let bytes = self.session.teardown(uri)?;
        self.exchange(cseq, bytes).await
    }

    /// Sends `GET_PARAMETER` (empty body = liveness ping) and awaits the response.
    pub async fn get_parameter(&mut self, uri: &str, body: &[u8]) -> Result<ClientEvent> {
        let cseq = self.session.peek_next_cseq();
        let bytes = self.session.get_parameter(uri, body)?;
        self.exchange(cseq, bytes).await
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
    /// Not cancel-safe: if this future is dropped before it resolves, bytes
    /// already read (and any response already decoded but not yet returned)
    /// are retained in `self` for the next call, but a caller relying on
    /// getting *this* response back should not drop and retry — build a fresh
    /// request instead. This method has no built-in deadline; wrap the call
    /// in a timeout if one is needed.
    ///
    /// Interleaved media frames that arrive before the response are buffered and
    /// later returned by [`recv_interleaved`](Self::recv_interleaved).
    async fn exchange(&mut self, cseq: u32, request: Vec<u8>) -> Result<ClientEvent> {
        // Tracks whichever CSeq we're currently waiting an answer for: a 401
        // retry re-signs the SAME logical request under a NEW CSeq (a fresh
        // `Pending` entry), so once we see our request's `AuthRetry`, we must
        // switch to waiting for ITS CSeq instead.
        let mut cseq = cseq;
        self.stream
            .write_all(&request)
            .await
            .map_err(|e| io_err("write request", e))?;
        self.stream.flush().await.map_err(|e| io_err("flush", e))?;

        loop {
            // Drain any events already decoded from buffered bytes first.
            let events = self
                .session
                .handle_data(&std::mem::take(&mut self.read_buf))?;
            let mut response = None;
            for event in events {
                match event {
                    // Only the response whose CSeq matches the request THIS
                    // call sent answers it; anything else (a duplicate, or a
                    // response for a request a previous, abandoned `exchange`
                    // call sent) is a stray and is dropped. Hold ours until
                    // every event decoded from this same read has been
                    // processed: interleaved media frames can arrive
                    // coalesced *after* the response in one TCP segment, and
                    // must be buffered rather than dropped by an early return
                    // (§10.12).
                    ClientEvent::Response { cseq: rcseq, .. } if rcseq == cseq => {
                        response = Some(event);
                    }
                    ClientEvent::Response { .. } => {}
                    ClientEvent::AuthRetry {
                        cseq: retry_cseq,
                        ref request,
                        ..
                    } => {
                        // Write the retried (now-authenticated) request and
                        // wait for ITS CSeq from here on.
                        let retry = request.clone();
                        self.stream
                            .write_all(&retry)
                            .await
                            .map_err(|e| io_err("write auth retry", e))?;
                        self.stream.flush().await.map_err(|e| io_err("flush", e))?;
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
            if let Some(response) = response {
                return Ok(response);
            }
            // Need more bytes from the socket.
            self.fill_from_socket().await?;
        }
    }

    /// Receives the next interleaved media frame ([`ClientEvent::MediaData`]),
    /// driving the socket until one is available (§10.12).
    ///
    /// Returns `Ok(None)` if the peer closes the connection cleanly before a
    /// frame arrives. Any control responses interleaved with media are consumed
    /// and their state transitions applied, but not returned here.
    pub async fn recv_interleaved(&mut self) -> Result<Option<ClientEvent>> {
        loop {
            if let Some(event) = self.pending_media.pop_front() {
                return Ok(Some(event));
            }
            let events = self
                .session
                .handle_data(&std::mem::take(&mut self.read_buf))?;
            for event in events {
                if matches!(event, ClientEvent::MediaData { .. }) {
                    self.pending_media.push_back(event);
                }
            }
            if let Some(event) = self.pending_media.pop_front() {
                return Ok(Some(event));
            }
            // No frame decoded yet; read more, treating clean EOF as end-of-stream.
            let n = self.read_once().await?;
            if n == 0 {
                return Ok(None);
            }
        }
    }

    /// Reads one chunk from the socket into `read_buf`, erroring on EOF (used
    /// where a response is *required*).
    async fn fill_from_socket(&mut self) -> Result<()> {
        let n = self.read_once().await?;
        if n == 0 {
            return Err(Error::Io("peer closed connection before response".into()));
        }
        Ok(())
    }

    /// Reads one chunk from the socket into `read_buf`; returns the byte count
    /// (`0` = clean EOF).
    async fn read_once(&mut self) -> Result<usize> {
        let mut chunk = [0u8; READ_CHUNK];
        let n = self
            .stream
            .read(&mut chunk)
            .await
            .map_err(|e| io_err("read", e))?;
        self.read_buf.extend_from_slice(&chunk[..n]);
        Ok(n)
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
        use std::sync::Arc;
        use tokio_rustls::TlsConnector;

        let tcp = TcpStream::connect(addr)
            .await
            .map_err(|e| io_err("connect", e))?;
        let connector = TlsConnector::from(Arc::new(config));
        let dns = rustls::pki_types::ServerName::try_from(server_name.to_string())
            .map_err(|e| Error::Tls(format!("invalid server name {server_name:?}: {e}")))?;
        let stream = connector
            .connect(dns, tcp)
            .await
            .map_err(|e| io_err("TLS handshake", e))?;
        Ok(Self::with_stream(stream, session))
    }
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
/// Reads requests off the socket (buffering partial reads until a full RTSP
/// message parses), calls [`ServerSession::handle_request`], writes the response
/// bytes back, and returns the [`ServerEvent`]s.
#[derive(Debug)]
pub struct AsyncRtspServer<S> {
    stream: S,
    session: ServerSession,
    read_buf: Vec<u8>,
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
}

#[cfg(feature = "tls")]
impl AsyncRtspServer<tokio_rustls::server::TlsStream<TcpStream>> {
    /// Performs the TLS handshake over an accepted TCP connection (an
    /// `rtsps://` server), then wraps the TLS stream with a fresh session.
    pub async fn accept_tls(stream: TcpStream, config: rustls::ServerConfig) -> Result<Self> {
        use std::sync::Arc;
        use tokio_rustls::TlsAcceptor;

        let acceptor = TlsAcceptor::from(Arc::new(config));
        let tls = acceptor
            .accept(stream)
            .await
            .map_err(|e| io_err("TLS handshake", e))?;
        Ok(Self::with_stream(tls, ServerSession::new(os_session_id)))
    }
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
    /// Wraps an already-connected stream (plain or TLS) and a session.
    pub fn with_stream(stream: S, session: ServerSession) -> Self {
        AsyncRtspServer {
            stream,
            session,
            read_buf: Vec::new(),
        }
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
        &mut self.stream
    }

    /// Reads the next complete request or interleaved frame, handling a
    /// request (writing the response back) and returning the produced
    /// events, or surfacing an interleaved `$`-frame as
    /// [`ServerEvent::MediaData`].
    ///
    /// Returns `Ok(None)` when the peer closes the connection cleanly before a
    /// full request or frame arrives.
    pub async fn next_request(&mut self) -> Result<Option<Vec<ServerEvent>>> {
        loop {
            // An interleaved `$`-framed block (RTCP receiver reports during
            // PLAY, or media during RECORD, §10.12) takes priority: it is not
            // a request, and every RTSP client that streams over TCP sends
            // these on the same connection (audit run-09 W8).
            if self.read_buf.first() == Some(&MAGIC) {
                if let Some((frame, consumed)) =
                    crate::interleaved::InterleavedFrame::parse(&self.read_buf)?
                {
                    self.read_buf.drain(..consumed);
                    return Ok(Some(vec![ServerEvent::MediaData {
                        channel: frame.channel,
                        data: frame.payload,
                    }]));
                }
                // Incomplete frame: fall through to read more bytes.
            } else if let Some(consumed) = complete_request_len(&self.read_buf)? {
                let request: Vec<u8> = self.read_buf.drain(..consumed).collect();
                let (response, events) = self.session.handle_request(&request)?;
                self.stream
                    .write_all(&response)
                    .await
                    .map_err(|e| io_err("write response", e))?;
                self.stream.flush().await.map_err(|e| io_err("flush", e))?;
                return Ok(Some(events));
            }
            // Need more bytes.
            let mut chunk = [0u8; READ_CHUNK];
            let n = self
                .stream
                .read(&mut chunk)
                .await
                .map_err(|e| io_err("read", e))?;
            if n == 0 {
                if self.read_buf.is_empty() {
                    return Ok(None);
                }
                return Err(Error::Io("peer closed connection mid-request".into()));
            }
            self.read_buf.extend_from_slice(&chunk[..n]);
            if self.read_buf.len() > MAX_SERVER_READ_BUFFER {
                return Err(Error::MessageParse(format!(
                    "request buffer of {} bytes exceeds the {MAX_SERVER_READ_BUFFER}-byte maximum",
                    self.read_buf.len()
                )));
            }
        }
    }

    /// Sends an interleaved (`$`-framed) media frame to the client on `channel`
    /// (§10.12), e.g. an RTP or RTCP packet during PLAY.
    pub async fn send_interleaved(&mut self, channel: u8, payload: &[u8]) -> Result<()> {
        let frame = crate::interleaved::InterleavedFrame::new(channel, payload.to_vec());
        let bytes = frame.to_bytes()?;
        self.stream
            .write_all(&bytes)
            .await
            .map_err(|e| io_err("write interleaved frame", e))?;
        self.stream.flush().await.map_err(|e| io_err("flush", e))?;
        Ok(())
    }
}

/// Returns the byte length of a complete RTSP request at the front of `buf`, or
/// `None` if more bytes are needed. Errors on a malformed message.
///
/// A leading `$` (interleaved frame) is not a request; [`AsyncRtspServer::next_request`]
/// checks for and demultiplexes that case before ever calling this, but this
/// still errors defensively rather than misparsing one as a request.
fn complete_request_len(buf: &[u8]) -> Result<Option<usize>> {
    if buf.is_empty() {
        return Ok(None);
    }
    if buf[0] == MAGIC {
        return Err(Error::MessageParse(
            "interleaved '$' frame received where a request was expected".into(),
        ));
    }
    match Message::<Body>::parse(buf) {
        Ok((_, consumed)) => Ok(Some(consumed)),
        Err(rtsp_types::ParseError::Incomplete(_)) => Ok(None),
        Err(rtsp_types::ParseError::Error) => {
            Err(Error::MessageParse("malformed RTSP request".into()))
        }
    }
}
