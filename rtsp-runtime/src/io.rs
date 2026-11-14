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
//! ([`crate::RTSP_DEFAULT_PORT`]). RTSP-over-TLS (`rtsps://`) wraps the TCP stream in a
//! TLS session *before* any RTSP is exchanged and uses default port **322**
//! ([`crate::RTSPS_DEFAULT_PORT`], per the IANA `rtsps` assignment). The TLS entry
//! points ([`AsyncRtspClient::connect_tls`], [`AsyncRtspServer::accept_tls`]) are
//! gated behind the `tls` feature; everything else is behind `tokio`.

use std::collections::VecDeque;
use std::time::Duration;

use futures_util::{FutureExt, SinkExt, StreamExt};
use tokio::io::{AsyncRead, AsyncWrite};

use crate::PlaySession;
use broadcast_common::stage::Timestamp;
use broadcast_common::timeouts::Timeouts;
use tokio::net::TcpStream;
use tokio_util::codec::Framed;

use crate::Method;
use crate::client::{ClientEvent, ClientSession};
use crate::codec::{ClientCodec, ServerCodec, ServerFrame, bounded};
use crate::config::RtspClientIoConfig;
use crate::error::{Error, IoContext, Result};
use crate::server::{ServerEvent, ServerSession};
use crate::server_config::RtspServerConfig;
use crate::transport::{Transport, TransportSpec};

/// The epoch [`now_us`] measures from: this process's first use of the clock.
/// (An epoch is needed because tokio's paused-clock `Instant` has an arbitrary
/// zero; the value itself carries no wall-clock meaning.)
static NOW_EPOCH: std::sync::LazyLock<tokio::time::Instant> =
    std::sync::LazyLock::new(tokio::time::Instant::now);

/// Maximum interleaved [`ClientEvent::MediaData`] frames held in
/// `pending_media` awaiting [`AsyncRtspClient::recv_interleaved`]. A camera
/// that never answers a `GET_PARAMETER` (or any other control request) while
/// still streaming media would otherwise grow this queue without bound while
/// [`AsyncRtspClient::exchange`] keeps reading and waiting for the response
/// (audit run-09 W7). Once full, the oldest frame is dropped — for a live
/// stream a fresher sample is worth more than one still queued.
const MAX_PENDING_MEDIA_FRAMES: usize = 1024;

/// Maximum items [`AsyncRtspClient::drain_inbound`] will consume from the
/// socket in a single call. A single poll of `framed.next()` returns
/// `Ready` for every already-buffered or readable item, so without this cap a
/// peer streaming interleaved frames faster than we parse (or a hostile
/// server answering a RECORD session) would keep `drain_inbound` looping
/// forever, starving the pusher's writes and the keepalive. Whatever is left
/// after this many items stays in the socket buffer for the next read.
const MAX_DRAIN_ITEMS: usize = 64;

/// The current time in whole microseconds on this adapter's monotonic clock —
/// the caller-supplied `now_us` the sans-IO session's keepalive schedule is
/// driven with, on the same [`NOW_EPOCH`] as [`deadline_instant`].
fn now_us() -> u64 {
    u64::try_from(NOW_EPOCH.elapsed().as_micros()).unwrap_or(u64::MAX)
}

/// The tokio timer instant for a session deadline read back from
/// [`now_us`], so the adapter's own `sleep_until` waits exactly as long as the
/// session says.
fn deadline_instant(due_us: u64) -> tokio::time::Instant {
    *NOW_EPOCH + Duration::from_micros(due_us)
}

/// Maps a tokio IO error into the crate error type.
fn io_err(context: IoContext, source: std::io::Error) -> Error {
    Error::Io { context, source }
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
    deferred_error: Option<Error>,
    timeouts: Timeouts,
}

impl AsyncRtspClient<TcpStream> {
    /// Connects a plain-TCP (`rtsp://`) client to `addr`.
    ///
    /// `addr` is any [`tokio::net::ToSocketAddrs`]; for the RTSP default port use
    /// `(host, RTSP_DEFAULT_PORT)`.
    ///
    /// `config` is an [`RtspClientIoConfig`], e.g. one carrying
    /// [`Credentials`](crate::Credentials).
    pub async fn connect<A: tokio::net::ToSocketAddrs>(
        addr: A,
        config: RtspClientIoConfig,
    ) -> Result<Self> {
        let stream = bounded(config.timeouts.connect, "connect", async {
            TcpStream::connect(addr)
                .await
                .map_err(|e| io_err(IoContext::Connect, e))
        })
        .await?;
        Ok(Self::from_stream(stream, config))
    }
}

impl<S> AsyncRtspClient<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    /// Wraps an already-connected stream (plain or TLS) with `config`.
    pub fn from_stream(stream: S, config: RtspClientIoConfig) -> Self {
        AsyncRtspClient {
            framed: Framed::new(stream, ClientCodec::new(config.session)),
            pending_media: VecDeque::new(),
            deferred_error: None,
            timeouts: config.timeouts,
        }
    }

    /// The timeouts this adapter applies.
    pub fn timeouts(&self) -> Timeouts {
        self.timeouts
    }

    /// The current session state.
    pub fn state(&self) -> crate::SessionState {
        self.framed.codec().session.state()
    }

    /// The negotiated session id, once a SETUP response has been processed.
    pub fn session_id(&self) -> Option<crate::SessionId> {
        self.framed.codec().session.session_id()
    }

    /// Borrows the underlying sans-IO session (read-only inspection).
    pub fn session(&self) -> &ClientSession {
        &self.framed.codec().session
    }

    async fn write(&mut self, bytes: Vec<u8>) -> Result<()> {
        bounded(self.timeouts.write, "write", self.framed.send(bytes)).await?;
        self.framed
            .codec_mut()
            .session
            .mark_activity(Timestamp::from_micros(now_us()));
        Ok(())
    }

    /// The next decoded event; `Ok(None)` = clean EOF. One deadline covers the WHOLE
    /// frame (see [`Timeouts::read_idle`]).
    async fn next_event(&mut self) -> Result<Option<ClientEvent>> {
        // Inbound events (media, responses) deliberately do NOT count as keepalive
        // activity: the server's session timeout is refreshed by RTSP *requests*
        // (RFC 2326 §12.37), so a busy interleaved stream must not postpone the
        // keepalive.
        if let Some(e) = self.deferred_error.take() {
            return Err(e);
        }
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

    /// Publishes a stream in one call: `ANNOUNCE` carrying `sdp` (RFC 2326
    /// §10.3), `SETUP` on the track's control URL (`trackID=0` appended to
    /// `url`, Appendix C.1.1) with an interleaved `mode=record` transport
    /// (§12.39), then `RECORD` (§10.11).
    ///
    /// `interleaved_channel` is the RTP channel; RTCP takes the next one, as
    /// [`TransportSpec::rtp_avp_tcp_interleaved`] pairs them. Every request is
    /// sent in that order with `url` as the presentation URI and the control
    /// URL only on `SETUP`, so the sequence a server sees is fixed.
    ///
    /// Fails with [`Error::UnexpectedResponse`] on the first request that is
    /// answered with anything but a 2xx, naming the method, and with
    /// [`Error::InvalidUrl`] for a `url` with no authority to derive the
    /// control URL on.
    pub async fn start_record(
        &mut self,
        url: &url::Url,
        sdp: &str,
        interleaved_channel: u8,
    ) -> Result<()> {
        let clean = broadcast_auth::strip_userinfo(url).map_err(Error::from)?;
        let uri = clean.as_str();
        let control = crate::sdp::control_url(&clean)?;
        let rtcp = interleaved_channel
            .checked_add(1)
            .ok_or(Error::InterleavedChannelExhausted)?;
        let mut spec = TransportSpec::rtp_avp_tcp_interleaved(interleaved_channel, rtcp);
        spec.mode = vec![crate::TransportMode::Record];
        let transport = crate::Transport::single(spec);

        expect_success(Method::Announce, self.announce(uri, sdp).await?)?;
        expect_success(Method::Setup, self.setup(&control, &transport).await?)?;
        expect_success(Method::Record, self.record(uri).await?)?;
        Ok(())
    }

    /// Pulls a stream in one call: `DESCRIBE` (RFC 2326 §10.2), `SETUP` for
    /// each media section with a usable `a=control` (§10.4), then `PLAY` (§10.5).
    ///
    /// Relative `a=control` values resolve against the `Content-Base` of the
    /// DESCRIBE response, else its `Content-Location`, else `url` (Appendix
    /// C.1.1), kept with a trailing slash so a value appends rather than
    /// replaces the last path segment. A media control that crosses
    /// `url`'s authority is not set up and is listed in
    /// [`PlaySession::skipped`]. `PLAY` goes to the session-level `a=control`
    /// (`*` is the base); an absent session-level control, or one on another
    /// authority, leaves `PLAY` on `url`.
    ///
    /// Each track is offered the interleaved pair `first_channel + 2n` /
    /// `first_channel + 2n + 1`; the channel the server answers with is the one
    /// recorded in [`crate::PlayTrack::channel`].
    ///
    /// # Errors
    ///
    /// [`Error::UnexpectedResponse`] for the first non-2xx answer,
    /// [`Error::InvalidSdp`] when the body is not UTF-8 or not an SDP,
    /// [`Error::InterleavedChannelExhausted`] when the pairs no longer fit in a
    /// `u8`, and [`Error::InterleavedChannelCollision`] when the server
    /// assigns one channel to two tracks.
    pub async fn start_play(&mut self, url: &url::Url, first_channel: u8) -> Result<PlaySession> {
        let clean = broadcast_auth::strip_userinfo(url).map_err(Error::from)?;
        let (status, body) = match self.describe(clean.as_str()).await? {
            crate::client::ClientEvent::Response { status, body, .. } => (status, body),
            other => return Err(Error::UnexpectedEvent { event: other }),
        };
        if !status.is_success() {
            return Err(Error::UnexpectedResponse {
                method: Method::Describe,
                status,
            });
        }
        let sdp = String::from_utf8(body)
            .map_err(|e| Error::InvalidSdp(crate::error::SdpParseError::Utf8(e.to_string())))?;
        let mut plan = crate::PlayPlan::from_describe(
            &clean,
            self.framed.codec_mut().session.content_base(),
            None,
            &sdp,
            first_channel,
        )?;

        for index in 0..plan.setups().len() {
            let Some(setup) = plan.setups().get(index) else {
                break;
            };
            let (control, transport) = (setup.control.clone(), setup.transport.clone());
            match self.setup(&control, &transport).await? {
                crate::client::ClientEvent::Response { status, .. } if status.is_success() => {}
                crate::client::ClientEvent::Response { status, .. } => {
                    return Err(Error::UnexpectedResponse {
                        method: Method::Setup,
                        status,
                    });
                }
                other => return Err(Error::UnexpectedEvent { event: other }),
            }
            let negotiated = self.framed.codec_mut().session.take_negotiated_transport();
            plan.accept_setup(index, negotiated.as_ref())?;
        }

        match self.play(plan.play_uri()).await? {
            crate::client::ClientEvent::Response { status, .. } if status.is_success() => {}
            crate::client::ClientEvent::Response { status, .. } => {
                return Err(Error::UnexpectedResponse {
                    method: Method::Play,
                    status,
                });
            }
            other => return Err(Error::UnexpectedEvent { event: other }),
        }

        Ok(plan.into_session(sdp))
    }

    /// Sends `payload` as an interleaved (`$`-framed) media packet on `channel`
    /// (§10.12) — the mirror of the server's
    /// [`AsyncRtspServer::send_interleaved`], for an RTSP push that delivers
    /// media interleaved over the control connection. The frame is built by
    /// [`crate::interleaved::InterleavedFrame::new`] and the write is bounded
    /// by [`Timeouts::write`].
    ///
    /// Before the write this drives the push keepalive (see
    /// [`poll_keepalive`](Self::poll_keepalive)) and drains any server→client
    /// bytes already buffered, so a SEND-ONLY pusher that only ever calls this
    /// method still refreshes its session and does not let the peer's
    /// responses/notifications fill the socket buffer (RFC 2326 §12.37).
    ///
    /// **A `Timeout`/cancelled write leaves a partial `$` frame in the codec's
    /// write buffer and the connection is dead** — an interleaved frame cannot
    /// be resynchronised mid-flight, so the caller must reconnect rather than
    /// retry. Bytes already read stay buffered.
    pub async fn send_interleaved(&mut self, channel: u8, payload: &[u8]) -> Result<()> {
        self.poll_keepalive().await?;
        self.drain_inbound().await?;
        let bytes = crate::interleaved::InterleavedFrame::slice_to_bytes(channel, payload)?;
        bounded(self.timeouts.write, "write", self.framed.send(bytes)).await
    }

    /// Drive the RTSP keepalive for a SEND-ONLY pusher: if the session's
    /// keepalive deadline ([`ClientSession::poll_timeout`]) has passed, write
    /// the liveness ping now — a `GET_PARAMETER` (RFC 2326 §10.6). A no-op
    /// otherwise. Honours the
    /// crate's rule that **only requests we send** count as keepalive activity
    /// (inbound media/responses never do — see `next_event`), and the same
    /// half-timeout floor inside `poll_timeout`.
    ///
    /// [`send_interleaved`](Self::send_interleaved) calls this automatically;
    /// call it directly from a pusher loop that sends media by any other path
    /// (e.g. a separate RTCP/`send` split), or when the media flow is idle.
    pub async fn poll_keepalive(&mut self) -> Result<()> {
        if let Some(req) = self
            .framed
            .codec_mut()
            .session
            .handle_timeout(Timestamp::from_micros(now_us()))?
        {
            self.write(req).await?;
        }
        Ok(())
    }

    /// Non-blockingly read whatever the peer has already sent (control
    /// responses and interleaved media), applying session state and queueing
    /// media frames — so a send-only pusher keeps the socket drained without
    /// parking on a read.
    ///
    /// One `now_or_never()` poll of `self.framed.next()` per readiness probe
    /// (NOT `next_event`, whose own `read_idle` timeout would nest a second
    /// timer around every packet, and NOT `timeout(Duration::ZERO, ..)`, which
    /// waits for tokio's next 1 ms timer tick): if no bytes are ready it
    /// returns immediately.
    ///
    /// A control response that indicates the session is gone — `454 Session
    /// Not Found` (RFC 2326 §11.3.2), the usual answer to a `GET_PARAMETER`
    /// keepalive once the server dropped the session — is surfaced as
    /// [`Error::SessionNotFound`] rather than discarded, so the send-only
    /// pusher learns of the loss on its next call instead of only via a later
    /// write error. Any other response is applied by the codec and dropped
    /// (a pusher has no outstanding request of its own to correlate). A read
    /// or decode error (other than a 454) is kept, not reported here: the next
    /// `next_event`/`recv_interleaved` call returns it as itself.
    ///
    /// At most [`MAX_DRAIN_ITEMS`] items are consumed per call, so a peer that
    /// floods interleaved frames cannot make this loop unbounded — the excess
    /// stays buffered for the next call.
    pub(crate) async fn drain_inbound(&mut self) -> Result<()> {
        for _ in 0..MAX_DRAIN_ITEMS {
            // A single poll, not `timeout(Duration::ZERO, ..)`: tokio rounds a
            // timer deadline up to its next 1 ms tick, so the zero timeout
            // parked the task ~1.5 ms on EVERY call when nothing was ready —
            // capping a pusher at ~650 frames/s (~0.85 MB/s).
            match self.framed.next().now_or_never() {
                None => return Ok(()),       // nothing ready — non-blocking
                Some(None) => return Ok(()), // EOF
                // A read/decode error is not the push path's to report, and
                // `Framed` yields it once: keep it for the next real read.
                Some(Some(Err(e))) => {
                    self.deferred_error = Some(e);
                    return Ok(());
                }
                Some(Some(Ok(ClientEvent::Response { method, status, .. }))) => {
                    if status == rtsp_types::StatusCode::SessionNotFound {
                        return Err(Error::SessionNotFound { method });
                    }
                }
                Some(Some(Ok(event @ ClientEvent::MediaData { .. }))) => {
                    if self.pending_media.len() >= MAX_PENDING_MEDIA_FRAMES {
                        self.pending_media.pop_front();
                    }
                    self.pending_media.push_back(event);
                }
                // `AuthRetry`/any future variant: nothing to do for a
                // send-only pusher.
                Some(Some(Ok(_))) => {}
            }
        }
        // Hit the per-call cap: leave the remainder buffered, do not spin.
        Ok(())
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
    /// Every awaited IO is bounded by [`Timeouts`]: a write that the peer
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
                return Err(io_err(
                    IoContext::Read,
                    std::io::Error::new(
                        std::io::ErrorKind::UnexpectedEof,
                        "peer closed connection before response",
                    ),
                ));
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
                    // From here on, wait for the retried request's own CSeq.
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
    /// While waiting, a keepalive is sent at half the session
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
                .map(|due| deadline_instant(due.as_micros()));
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
                        return Err(io_err(
                            IoContext::Read,
                            std::io::Error::new(
                                std::io::ErrorKind::UnexpectedEof,
                                "peer closed connection mid-message",
                            ),
                        ));
                    }
                    None => return Ok(None),
                    Some(ev @ ClientEvent::MediaData { .. }) => return Ok(Some(ev)),
                    Some(_) => {} // control responses are applied by the session; not surfaced here
                },
                Woke::Keepalive => {
                    if let Some(req) = self
                        .framed
                        .codec_mut()
                        .session
                        .handle_timeout(Timestamp::from_micros(now_us()))?
                    {
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
        tls: rustls::ClientConfig,
        config: RtspClientIoConfig,
    ) -> Result<Self> {
        let addr = addr;
        connect_tls_via(
            async move {
                TcpStream::connect(addr)
                    .await
                    .map_err(|e| io_err(IoContext::Connect, e))
            },
            server_name,
            tls,
            config,
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
    tls: rustls::ClientConfig,
    config: RtspClientIoConfig,
) -> Result<AsyncRtspClient<tokio_rustls::client::TlsStream<T>>>
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    use std::sync::Arc;
    use tokio_rustls::TlsConnector;

    let timeouts = config.timeouts;
    let tcp = bounded(timeouts.connect, "connect", dial).await?;
    let connector = TlsConnector::from(Arc::new(tls));
    let dns = rustls::pki_types::ServerName::try_from(server_name.to_string()).map_err(|e| {
        Error::Tls {
            server_name: server_name.to_string(),
            source: Box::new(e),
        }
    })?;
    let stream = bounded(timeouts.handshake, "handshake", async {
        connector
            .connect(dns, tcp)
            .await
            .map_err(|e| io_err(IoContext::TlsHandshake, e))
    })
    .await?;
    Ok(AsyncRtspClient::from_stream(stream, config))
}

/// Builds a [`rustls::ClientConfig`] trusting the `webpki-roots` public-CA
/// bundle (the default trust store for `rtsps://` to a well-known server).
///
/// For a self-signed camera cert, construct the config directly with a root
/// store containing that cert and pass it to
/// [`AsyncRtspClient::connect_tls`].
#[cfg(feature = "tls")]
pub fn default_tls_client_config() -> crate::Result<rustls::ClientConfig> {
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    // Select the aws-lc-rs provider explicitly rather than via
    // `ClientConfig::builder()`'s process-global default: another crate in the
    // same build (e.g. a `reqwest` that pulls `aws-lc-rs`) can put a *second*
    // `CryptoProvider` in the tree, leaving no unambiguous default and making
    // the plain builder panic. Choosing the provider here keeps `rtsp-runtime`
    // working regardless of what else is linked.
    Ok(
        rustls::ClientConfig::builder_with_provider(std::sync::Arc::new(
            rustls::crypto::aws_lc_rs::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .map_err(|e| Error::TlsConfig(Box::new(e)))?
        .with_root_certificates(roots)
        .with_no_client_auth(),
    )
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
/// [`Timeouts`].
///
/// Both constructors, [`from_stream`](Self::from_stream) (an accepted plain
/// or TLS stream; [`from_stream` (plain TCP)](AsyncRtspServer::<TcpStream>::from_stream)
/// takes it without one) and [`accept_tls`](Self::accept_tls) (an `rtsps://`
/// server), take an [`RtspServerConfig`]. A default config's session takes its
/// ids from the OS random source; when that is unavailable the `SETUP` that
/// needs an id fails with [`Error::RandomSource`].
#[derive(Debug)]
pub struct AsyncRtspServer<S> {
    framed: Framed<S, ServerCodec>,
    session: ServerSession,
    timeouts: Timeouts,
    first_request_seen: bool,
    /// Events of a request whose response is still being written (cancel safety).
    ready: Option<Vec<ServerEvent>>,
}

#[cfg(feature = "tls")]
impl AsyncRtspServer<tokio_rustls::server::TlsStream<TcpStream>> {
    /// Performs the TLS handshake over an accepted TCP connection (an
    /// `rtsps://` server), then wraps the TLS stream with a fresh session.
    ///
    /// `handshake` of the config's [`Timeouts`] bounds the TLS handshake.
    pub async fn accept_tls(
        stream: TcpStream,
        tls: rustls::ServerConfig,
        config: RtspServerConfig,
    ) -> Result<Self> {
        accept_tls_via(stream, tls, config).await
    }
}

/// TLS server setup over any accepted transport; `handshake` bounds the handshake.
#[cfg(feature = "tls")]
async fn accept_tls_via<T>(
    stream: T,
    tls: rustls::ServerConfig,
    config: RtspServerConfig,
) -> Result<AsyncRtspServer<tokio_rustls::server::TlsStream<T>>>
where
    T: AsyncRead + AsyncWrite + Unpin,
{
    use std::sync::Arc;
    use tokio_rustls::TlsAcceptor;

    let acceptor = TlsAcceptor::from(Arc::new(tls));
    let tls = bounded(config.timeouts.handshake, "handshake", async {
        acceptor
            .accept(stream)
            .await
            .map_err(|e| io_err(IoContext::TlsHandshake, e))
    })
    .await?;
    AsyncRtspServer::from_stream(tls, config)
}

impl<S> AsyncRtspServer<S>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    /// Wraps an already-connected stream (plain or TLS) with `config`.
    pub fn from_stream(stream: S, config: RtspServerConfig) -> Result<Self> {
        Ok(AsyncRtspServer {
            framed: Framed::new(stream, ServerCodec::default()),
            session: ServerSession::new(config.clone())?,
            timeouts: config.timeouts,
            first_request_seen: false,
            ready: None,
        })
    }

    /// The timeouts this adapter applies.
    pub fn timeouts(&self) -> Timeouts {
        self.timeouts
    }

    /// The current session state.
    pub fn state(&self) -> crate::SessionState {
        self.session.state()
    }

    /// The allocated session id, once a SETUP has been handled.
    pub fn session_id(&self) -> Option<crate::SessionId> {
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
    /// The FIRST call on a connection waits at most [`Timeouts::handshake`]
    /// for a complete request; later calls wait at most
    /// [`Timeouts::read_idle`]. A frame must complete inside that single
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
            bounded(self.timeouts.write, "write", self.framed.flush()).await?;
            if let Some(events) = self.ready.take() {
                return Ok(Some(events));
            }
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
        let frame = crate::interleaved::InterleavedFrame::new(
            channel,
            bytes::Bytes::copy_from_slice(payload),
        );
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

    fn timeouts() -> Timeouts {
        Timeouts::default()
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
        let cert = test_fixtures::read("tls/localhost-cert.der");
        let key = test_fixtures::read("tls/localhost-key.der");
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
            default_tls_client_config().expect("TLS client config"),
            RtspClientIoConfig::default().with_timeouts(timeouts()),
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
            default_tls_client_config().expect("TLS client config"),
            RtspClientIoConfig::default().with_timeouts(timeouts()),
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
        let r = guard(accept_tls_via(
            server_io,
            server_config(),
            RtspServerConfig::default().with_timeouts(timeouts()),
        ))
        .await;
        assert!(
            matches!(r, Err(Error::Timeout { what: "handshake" })),
            "{:?}",
            r.err()
        );
    }
}

fn expect_success(method: Method, event: ClientEvent) -> Result<()> {
    match event {
        ClientEvent::Response { status, .. } if status.is_success() => Ok(()),
        ClientEvent::Response { status, .. } => Err(Error::UnexpectedResponse { method, status }),
        other => Err(Error::UnexpectedEvent { event: other }),
    }
}

#[cfg(test)]
mod tests {
    use crate::SessionIdSource;

    #[test]
    fn the_os_random_id_source_varies() {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..100 {
            seen.insert(
                SessionIdSource::default()
                    .next_id(0)
                    .expect("OS random source")
                    .value(),
            );
        }
        assert!(
            seen.len() > 90,
            "the OS id source did not vary: {}",
            seen.len()
        );
    }
}
