//! Async `tokio` socket adapter driving the sans-IO ingest server session
//! over a real `tokio::net::TcpStream` — the Layer-2 adapter (see
//! `private/specs-verified/rtmp/rtmp-runtime/rtmp.md` § Layer-2 adapter; mirrors the
//! `rtsp_runtime::io` tokio adapter shape in this same workspace).
//!
//! [`ServerSession`] never touches a socket: it turns inbound bytes into
//! `(reply bytes, [`ServerEvent`]s)`. This module owns the stream as a
//! [`tokio_util::codec::Framed`] whose decoder feeds every inbound chunk to
//! [`ServerSession::handle_data`], writes the reply bytes back, and returns
//! the events — no business logic beyond that plumbing.
//!
//! [`ServerSession::handle_data`] buffers partial handshake/chunk input
//! internally (see its doc comment), so unlike an RTSP or HTTP adapter this
//! one does not need to detect message boundaries itself: any chunk size,
//! split anywhere, is fine to feed straight through.
//!
//! Every awaited IO is bounded by [`RtmpTimeouts`] (W1 SP1.3).

use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::{TcpListener, TcpStream, ToSocketAddrs};
use tokio::time::Instant;
use tokio_util::codec::Framed;

use crate::amf0::Amf0Value;
use crate::client::{ClientConfig, ClientEvent, ClientSession};
use crate::codec::{ClientCodec, RtmpSession, ServerCodec, SessionCodec, io_err};
use crate::server::{ServerConfig, ServerEvent, ServerSession};
use crate::target::RtmpTarget;

/// Explicit bounds on every awaited IO of the RTMP adapters (W1 SP1.3): no wait
/// is unbounded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct RtmpTimeouts {
    /// DNS + TCP connect (client). Default 10 s.
    pub connect: Duration,
    /// From connection start until the `connect` command is accepted
    /// ([`ServerEvent::Connected`]). Default 10 s.
    pub handshake: Duration,
    /// Longest wait for the next non-empty batch of events. One deadline covers the
    /// whole wait: chunks that yield no events (a partial chunk, a handshake step)
    /// do not restart it, so a peer dripping bytes still times out. Default 30 s.
    pub read_idle: Duration,
    /// Longest wait for the socket to accept pending writes. Default 10 s.
    pub write: Duration,
}

impl Default for RtmpTimeouts {
    fn default() -> Self {
        Self {
            connect: Duration::from_secs(10),
            handshake: Duration::from_secs(10),
            read_idle: Duration::from_secs(30),
            write: Duration::from_secs(10),
        }
    }
}

impl RtmpTimeouts {
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

fn timed_out(what: &str) -> io::Error {
    io::Error::new(io::ErrorKind::TimedOut, format!("RTMP {what} timed out"))
}

/// A `tokio::net::TcpListener` that accepts inbound RTMP publishers and hands
/// back a driven [`RtmpConnection`] per connection.
#[derive(Debug)]
pub struct AsyncRtmpServer {
    listener: TcpListener,
    config: ServerConfig,
    timeouts: RtmpTimeouts,
}

impl AsyncRtmpServer {
    /// Binds a listen address (e.g. `"0.0.0.0:1935"`, the IANA-assigned RTMP
    /// port). `config` is cloned into a fresh [`ServerSession`] for each
    /// accepted connection.
    pub async fn bind<A: ToSocketAddrs>(addr: A, config: ServerConfig) -> io::Result<Self> {
        let listener = TcpListener::bind(addr).await?;
        Ok(Self {
            listener,
            config,
            timeouts: RtmpTimeouts::default(),
        })
    }

    /// Replaces the [`RtmpTimeouts`] given to every accepted connection.
    #[must_use]
    pub fn with_timeouts(mut self, timeouts: RtmpTimeouts) -> Self {
        self.timeouts = timeouts;
        self
    }

    /// Accepts the next inbound connection and wraps it with a fresh
    /// [`ServerSession`] built from this server's [`ServerConfig`].
    pub async fn accept(&self) -> io::Result<AcceptedRtmpConnection> {
        let (stream, peer) = self.listener.accept().await?;
        Ok(AcceptedRtmpConnection::new(
            RtmpConnection::from_stream_erased(
                stream,
                ServerSession::new(self.config.clone()),
                self.timeouts,
            ),
            peer,
        ))
    }

    /// The address this server is actually bound to (useful for `":0"`
    /// ephemeral-port binds).
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.listener.local_addr()
    }
}

#[derive(Debug)]
struct Driver<S, T: RtmpSession> {
    framed: Framed<S, SessionCodec<T>>,
    /// Events decoded but not yet handed to the caller. Set synchronously
    /// together with the reply bytes, so a cancelled flush leaves both in place
    /// for the next call.
    ready: Option<Vec<<T as RtmpSession>::Event>>,
    /// Once set, the connection is done (clean EOF, an end-of-stream event, a
    /// prior `RtmpError` or a timeout) and further calls return `None` without
    /// touching the socket again.
    closed: bool,
    handshake_deadline: Option<Instant>,
    /// Armed when a wait starts and cleared only when a batch with events is handed
    /// out, so `read_idle` bounds a whole frame/batch, not each (possibly empty) chunk
    /// a peer drips.
    idle_deadline: Option<Instant>,
    timeouts: RtmpTimeouts,
}

impl<S: AsyncRead + AsyncWrite + Unpin, T: RtmpSession> Driver<S, T> {
    fn new(framed: Framed<S, SessionCodec<T>>, timeouts: RtmpTimeouts) -> Self {
        Self {
            framed,
            ready: None,
            closed: false,
            handshake_deadline: Some(Instant::now() + timeouts.handshake),
            idle_deadline: None,
            timeouts,
        }
    }

    async fn next_events(&mut self) -> io::Result<Option<Vec<T::Event>>> {
        if self.closed {
            return Ok(None);
        }
        loop {
            match tokio::time::timeout(self.timeouts.write, self.framed.flush()).await {
                Err(_) => {
                    self.closed = true;
                    return Err(timed_out("write"));
                }
                Ok(Err(e)) => {
                    self.closed = true;
                    return Err(e);
                }
                Ok(Ok(())) => {}
            }
            if let Some(events) = self.ready.take() {
                if events.iter().any(T::opens_stream) {
                    self.handshake_deadline = None;
                }
                if events.iter().any(T::is_eof) {
                    self.closed = true;
                }
                if !events.is_empty() {
                    self.idle_deadline = None;
                }
                return Ok(Some(events));
            }
            let read_idle = self.timeouts.read_idle;
            let idle = *self
                .idle_deadline
                .get_or_insert_with(|| Instant::now() + read_idle);
            let (deadline, what) = match self.handshake_deadline {
                Some(h) if h < idle => (h, "handshake"),
                _ => (idle, "read"),
            };
            match tokio::time::timeout_at(deadline, self.framed.next()).await {
                Err(_) => {
                    self.closed = true;
                    return Err(timed_out(what));
                }
                Ok(None) => {
                    self.closed = true;
                    return Ok(None);
                }
                Ok(Some(Err(e))) => {
                    self.closed = true;
                    return Err(e);
                }
                Ok(Some(Ok(events))) => {
                    let reply = self.framed.codec_mut().take_reply();
                    self.framed.write_buffer_mut().extend_from_slice(&reply);
                    self.ready = Some(events);
                }
            }
        }
    }
}

/// An erased, transport-agnostic inbound byte stream: an accepted TCP socket,
/// a TLS stream wrapping one, or a test's in-memory pipe.
pub trait RtmpStream: AsyncRead + AsyncWrite + Unpin + Send {}

impl<T: AsyncRead + AsyncWrite + Unpin + Send> RtmpStream for T {}

/// `RtmpConnection`'s default stream type — the erased form — so a caller
/// naming one `RtmpConnection` does not name its transport.
pub type BoxStream = Pin<Box<dyn RtmpStream>>;

/// A connection [`AsyncRtmpServer::accept`] just took off its listen socket:
/// the publisher's address (which its erased stream cannot report) and the
/// connection itself.
pub struct AcceptedRtmpConnection {
    peer: SocketAddr,
    conn: RtmpConnection,
}

impl core::fmt::Debug for AcceptedRtmpConnection {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("AcceptedRtmpConnection")
            .field("peer", &self.peer)
            .finish_non_exhaustive()
    }
}

impl AcceptedRtmpConnection {
    /// Wrap an already-accepted connection with the peer it came from.
    pub fn new(conn: RtmpConnection, peer: SocketAddr) -> Self {
        Self { peer, conn }
    }

    /// The remote address of the publisher.
    pub fn peer_addr(&self) -> SocketAddr {
        self.peer
    }

    /// The connection, ready for [`RtmpConnection::next_events`].
    pub fn into_connection(self) -> RtmpConnection {
        self.conn
    }
}

/// One accepted RTMP connection: a stream (an erased [`BoxStream`] by
/// default) driving a [`ServerSession`].
///
/// [`next_events`](Self::next_events) is the whole surface: read a chunk,
/// drive the session, write the reply, return the events.
#[derive(Debug)]
pub struct RtmpConnection<S = BoxStream> {
    driver: Driver<S, ServerSession>,
}

impl RtmpConnection<BoxStream> {
    /// Wrap any connected stream, erasing its type — the constructor
    /// [`AsyncRtmpServer::accept`] uses, so one `RtmpConnection` names the
    /// connection whatever the transport.
    pub fn from_stream_erased<S>(stream: S, session: ServerSession, timeouts: RtmpTimeouts) -> Self
    where
        S: RtmpStream + 'static,
    {
        let erased: BoxStream = Box::pin(stream);
        Self::from_stream(erased, session, timeouts)
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin> RtmpConnection<S> {
    /// Wraps an already-connected stream with a session and explicit
    /// [`RtmpTimeouts`].
    pub fn from_stream(stream: S, session: ServerSession, timeouts: RtmpTimeouts) -> Self {
        Self {
            driver: Driver::new(Framed::new(stream, ServerCodec::new(session)), timeouts),
        }
    }

    /// Reads one chunk from the socket, drives [`ServerSession::handle_data`],
    /// writes the reply bytes back (all of them), and returns the resulting
    /// events.
    ///
    /// Returns `Ok(None)` when the connection is finished: the peer closed
    /// the socket (clean EOF), or the most recent batch of events included
    /// [`ServerEvent::Eof`] (so the caller sees that final batch once, then
    /// `None` on the next call). Once a call returns `None`, every
    /// subsequent call also returns `None` without reading the socket again.
    ///
    /// Cancel-safe: dropping the future at any await point loses neither a reply
    /// byte nor an event (the reply sits in the framed write buffer, the events
    /// in `self`, until a later call delivers them).
    ///
    /// # Errors
    /// An [`io::Error`] from the underlying socket read/write, or a mapped
    /// [`RtmpError`](crate::RtmpError) (kind [`io::ErrorKind::InvalidData`])
    /// from [`ServerSession::handle_data`]. On an `RtmpError` the session is
    /// unrecoverable (per `handle_data`'s own doc): this connection is torn
    /// down immediately — marked closed and not driven further, even if the
    /// caller keeps calling `next_events`. A deadline expiry
    /// ([`RtmpTimeouts`]) is [`io::ErrorKind::TimedOut`] and closes the
    /// connection too.
    pub async fn next_events(&mut self) -> io::Result<Option<Vec<ServerEvent>>> {
        self.driver.next_events().await
    }
}

/// A publishing RTMP client over a stream (a [`TcpStream`] by default): the
/// adapter for [`ClientSession`], mirroring
/// [`RtmpConnection`]. Every awaited IO is bounded by [`RtmpTimeouts`].
#[derive(Debug)]
pub struct AsyncRtmpClient<S = TcpStream> {
    driver: Driver<S, ClientSession>,
}

impl AsyncRtmpClient<TcpStream> {
    /// DNS + TCP connect bounded by `timeouts.connect`; `publish` is NOT started.
    pub async fn connect(target: &RtmpTarget, timeouts: RtmpTimeouts) -> io::Result<Self> {
        let stream = tokio::time::timeout(timeouts.connect, async {
            let addrs = target.resolve().await?;
            TcpStream::connect(&addrs[..]).await
        })
        .await
        .map_err(|_| timed_out("connect"))??;
        Ok(Self::from_stream(stream, target, timeouts))
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin> AsyncRtmpClient<S> {
    /// Wraps an already-connected stream; the handshake bytes are queued and go
    /// out with the first call that flushes.
    pub fn from_stream(stream: S, target: &RtmpTarget, timeouts: RtmpTimeouts) -> Self {
        let cfg = ClientConfig {
            app: target.app.clone(),
            stream_key: target.stream_key.clone(),
            tc_url: Some(target.tc_url.clone()),
            ..ClientConfig::default()
        };
        let mut codec = ClientCodec::new(ClientSession::new(cfg));
        let c0c1 = codec.session_mut().start();
        let mut framed = Framed::new(stream, codec);
        framed.write_buffer_mut().extend_from_slice(&c0c1);
        Self {
            driver: Driver::new(framed, timeouts),
        }
    }

    /// Runs handshake -> connect -> createStream -> publish; returns once the
    /// server accepted the publish. Bounded by [`RtmpTimeouts::handshake`] as a
    /// whole. A server error (`ClientEvent::Error`) is
    /// [`io::ErrorKind::ConnectionRefused`].
    pub async fn publish(&mut self) -> io::Result<()> {
        loop {
            let Some(events) = self.next_events().await? else {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "peer closed during publish",
                ));
            };
            for e in &events {
                match e {
                    ClientEvent::Publishing => return Ok(()),
                    ClientEvent::Error { code, description } => {
                        return Err(io::Error::new(
                            io::ErrorKind::ConnectionRefused,
                            format!("{code}: {description}"),
                        ));
                    }
                    _ => {}
                }
            }
        }
    }

    /// Queues one already-framed message and flushes it under `timeouts.write`. A
    /// cancel during the flush leaves the message queued (sent by the next call),
    /// never half-framed.
    async fn queue(&mut self, bytes: Vec<u8>) -> io::Result<()> {
        self.driver
            .framed
            .write_buffer_mut()
            .extend_from_slice(&bytes);
        match tokio::time::timeout(self.driver.timeouts.write, self.driver.framed.flush()).await {
            Err(_) => Err(timed_out("write")),
            Ok(r) => r,
        }
    }

    /// Sends one audio message (FLV audio tag body) at `timestamp` ms.
    ///
    /// The adapter reads inbound traffic (server acks, pings, errors) only inside
    /// [`next_events`](Self::next_events); a caller that only sends must also drive
    /// `next_events` (e.g. in a `select!`) or the server's acknowledgement window
    /// is never serviced.
    pub async fn send_audio(&mut self, timestamp: u32, data: &[u8]) -> io::Result<()> {
        let bytes = self.encode_audio(timestamp, data)?;
        self.queue(bytes).await
    }

    /// Sends one video message (FLV video tag body) at `timestamp` ms.
    pub async fn send_video(&mut self, timestamp: u32, data: &[u8]) -> io::Result<()> {
        let bytes = self.encode_video(timestamp, data)?;
        self.queue(bytes).await
    }

    /// Sends `@setDataFrame`/`onMetaData` stream metadata.
    pub async fn send_metadata(&mut self, metadata: &[(String, Amf0Value)]) -> io::Result<()> {
        let bytes = self.encode_metadata(metadata)?;
        self.queue(bytes).await
    }

    /// Sans-IO half of [`send_audio`](Self::send_audio): the chunk-stream-framed
    /// message bytes for one audio tag, **without writing**. Lets a caller
    /// driving its own egress (e.g. `multimux`'s `PushTransportEgress`) frame a
    /// message now and write it verbatim later via
    /// [`write_frame`](Self::write_frame).
    pub fn encode_audio(&mut self, timestamp: u32, data: &[u8]) -> io::Result<Vec<u8>> {
        self.driver
            .framed
            .codec_mut()
            .session_mut()
            .send_audio(timestamp, data)
            .map_err(io_err)
    }

    /// Sans-IO half of [`send_video`](Self::send_video) — see
    /// [`encode_audio`](Self::encode_audio).
    pub fn encode_video(&mut self, timestamp: u32, data: &[u8]) -> io::Result<Vec<u8>> {
        self.driver
            .framed
            .codec_mut()
            .session_mut()
            .send_video(timestamp, data)
            .map_err(io_err)
    }

    /// Sans-IO half of [`send_metadata`](Self::send_metadata) — see
    /// [`encode_audio`](Self::encode_audio).
    pub fn encode_metadata(&mut self, metadata: &[(String, Amf0Value)]) -> io::Result<Vec<u8>> {
        self.driver
            .framed
            .codec_mut()
            .session_mut()
            .send_metadata(metadata)
            .map_err(io_err)
    }

    /// Writes one already-framed message verbatim (the async half of the
    /// `encode_*` methods above), bounded by [`RtmpTimeouts::write`].
    pub async fn write_frame(&mut self, bytes: Vec<u8>) -> io::Result<()> {
        self.queue(bytes).await
    }

    /// Next batch of server events (acks, errors, close); `Ok(None)` at EOF.
    /// Same deadline rules and cancel-safety as [`RtmpConnection::next_events`].
    pub async fn next_events(&mut self) -> io::Result<Option<Vec<ClientEvent>>> {
        self.driver.next_events().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    const FIXTURE: &str = "rtmp/obs-publish.bin";

    /// Replays the real captured `ffmpeg` publish (T8's `obs-publish.bin`)
    /// over an actual loopback TCP socket, driving `AsyncRtmpServer`/
    /// `RtmpConnection` end to end: a spawned client task writes the fixture
    /// bytes and drains the server's replies, while the server side accepts
    /// the connection and loops `next_events` until the connection closes.
    ///
    /// This is the online counterpart of `transmux/tests/it/rtmp_ingest_fixture.rs`'s offline
    /// replay — it proves the tokio adapter (not just the sans-IO session)
    /// actually drives a real publish to `Connected` -> `Publish` -> `Media`.
    #[tokio::test]
    async fn loopback_replay_of_real_publish_reaches_connected_publish_media() {
        let fixture = std::fs::read(test_fixtures::path(FIXTURE))
            .expect("read ../fixtures/rtmp/obs-publish.bin");

        let server = AsyncRtmpServer::bind("127.0.0.1:0", ServerConfig::default())
            .await
            .expect("bind ephemeral loopback port");
        let addr = server.local_addr().expect("local_addr");

        // Client task: play back the captured publisher's raw bytes, and
        // drain whatever the server writes back (so the server's writes
        // never block on an unread socket buffer), counting the bytes
        // received — this is what proves `next_events` actually wrote the
        // session's reply bytes to the socket, not just decoded events.
        let client = tokio::spawn(async move {
            let mut stream = TcpStream::connect(addr).await.expect("connect loopback");
            stream
                .write_all(&fixture)
                .await
                .expect("write fixture bytes");
            let mut sink = [0u8; 8192];
            let mut replied_bytes = 0usize;
            loop {
                match stream.read(&mut sink).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => replied_bytes += n,
                }
            }
            replied_bytes
        });

        let mut conn = server
            .accept()
            .await
            .expect("accept the client connection")
            .into_connection();
        let mut events = Vec::new();
        while let Some(batch) = conn
            .next_events()
            .await
            .expect("next_events must not error")
        {
            events.extend(batch);
        }
        // Close the server-side socket so the client's drain loop observes
        // EOF and the spawned task actually finishes.
        drop(conn);
        let replied_bytes = client.await.expect("client task must not panic");

        // The handshake alone (S0 + S1 + S2, §5.2) is 1 + 1536 + 1536 = 3073
        // bytes; `connect`/`createStream`/`publish` each add a reply on top.
        // If `next_events` failed to write the reply bytes back, the client
        // would observe a bare EOF and this would be 0.
        const HANDSHAKE_REPLY_LEN: usize = 1 + 1536 + 1536;
        assert!(
            replied_bytes >= HANDSHAKE_REPLY_LEN,
            "next_events must write the session's reply bytes back to the socket \
             (expected at least the {HANDSHAKE_REPLY_LEN}-byte S0+S1+S2 handshake reply), \
             got {replied_bytes} bytes"
        );

        assert!(
            events
                .iter()
                .any(|e| matches!(e, ServerEvent::Connected { app } if app == "live")),
            "must emit Connected{{app: \"live\"}} over the real socket; got {events:?}"
        );
        assert!(
            events.iter().any(
                |e| matches!(e, ServerEvent::Publish { stream_key, .. } if stream_key == "testkey")
            ),
            "must emit Publish{{stream_key: \"testkey\", ..}} over the real socket; got {events:?}"
        );
        let media_count = events
            .iter()
            .filter(|e| matches!(e, ServerEvent::Media { .. }))
            .count();
        assert!(
            media_count >= 1,
            "must emit at least one Media event over the real socket, got {media_count}"
        );
    }

    /// I2: the sans-IO `encode_video`/`encode_audio`/`encode_metadata` +
    /// `write_frame` split round-trips through the server's chunk parser, and
    /// two video frames exercise chunk-header compression (the second message
    /// reuses the first's chunk-stream basic header via a type-2/3 header).
    #[tokio::test]
    async fn encode_then_write_frames_decode_on_the_server_with_header_compression() {
        let server = AsyncRtmpServer::bind("127.0.0.1:0", ServerConfig::default())
            .await
            .expect("bind");
        let addr = server.local_addr().expect("addr");
        let target = RtmpTarget::from_parts(&format!("rtmp://{addr}/live"), "live", "testkey")
            .expect("target");

        // Drive the server concurrently: `publish` needs the server to answer
        // the handshake/connect/createStream/publish exchange as the client
        // sends.
        let server_task = tokio::spawn(async move {
            let mut conn = server.accept().await.expect("accept").into_connection();
            // Collect the raw FLV bodies in arrival order, so the test can
            // check both the count AND the payload bytes/order (a
            // header-compression bug that still yields 2 events would
            // otherwise pass).
            let mut medias: Vec<Vec<u8>> = Vec::new();
            let is_video = |m: &Vec<u8>| m.contains(&0xAA) || m.contains(&0xBB);
            while medias.iter().filter(|m| is_video(m)).count() < 2 {
                match conn.next_events().await {
                    Ok(Some(batch)) => {
                        for e in batch {
                            if let ServerEvent::Media { flv } = e {
                                medias.push(flv);
                            }
                        }
                    }
                    Ok(None) | Err(_) => break,
                }
            }
            medias
        });

        let mut client = AsyncRtmpClient::connect(&target, RtmpTimeouts::default())
            .await
            .expect("connect");
        client.publish().await.expect("publish");

        // Two video messages (FLV AVCVIDEOPACKET bodies) via the sans-IO
        // encode half, then the write half — the shape `multimux`'s egress
        // uses.
        let frame_a = client
            .encode_video(0, &[0x17, 0x01, 0, 0, 0, 0xAA])
            .expect("encode a");
        let frame_b = client
            .encode_video(33, &[0x17, 0x01, 0, 0, 0, 0xBB])
            .expect("encode b");
        // A metadata message too.
        let meta = client
            .encode_metadata(&[("width".to_string(), crate::amf0::Amf0Value::Number(1280.0))])
            .expect("encode meta");
        for bytes in [meta, frame_a, frame_b] {
            client.write_frame(bytes).await.expect("write_frame");
        }

        let medias = server_task.await.expect("server join");
        // Filter to the two VIDEO frames we sent (the metadata message is also
        // a media event on some paths), preserving arrival order.
        let video: Vec<&Vec<u8>> = medias
            .iter()
            .filter(|m| m.contains(&0xAA) || m.contains(&0xBB))
            .collect();
        assert_eq!(
            video.len(),
            2,
            "both encoded video frames must decode on the server (chunk-header compression exercised): {medias:?}"
        );
        // Order + payload bytes: 0xAA first, then 0xBB — a reordered or
        // corrupt stream fails this.
        assert!(
            video[0].contains(&0xAA) && video[1].contains(&0xBB),
            "frames must arrive in order with their own payloads: {medias:?}"
        );
    }
}
