//! Async `tokio` socket adapter driving the sans-IO ingest server session
//! over a real `tokio::net::TcpStream` — the Layer-2 adapter (see
//! [`docs/rtmp.md`](../docs/rtmp.md) § Layer-2 adapter; mirrors the
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
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::{TcpListener, TcpStream, ToSocketAddrs};
use tokio::time::Instant;
use tokio_util::codec::Framed;

use crate::amf0::Amf0Value;
use crate::client::{ClientConfig, ClientEvent};
use crate::codec::{ClientCodec, ServerCodec, io_err};
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
    pub async fn accept(&self) -> io::Result<RtmpConnection> {
        let (stream, _peer) = self.listener.accept().await?;
        Ok(RtmpConnection::from_stream(
            stream,
            ServerSession::new(self.config.clone()),
            self.timeouts,
        ))
    }

    /// The address this server is actually bound to (useful for `":0"`
    /// ephemeral-port binds).
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.listener.local_addr()
    }
}

/// One accepted RTMP connection: a stream (a [`TcpStream`] by default) driving a
/// [`ServerSession`].
///
/// [`next_events`](Self::next_events) is the whole surface: read a chunk,
/// drive the session, write the reply, return the events.
#[derive(Debug)]
pub struct RtmpConnection<S = TcpStream> {
    framed: Framed<S, ServerCodec>,
    /// Events decoded but not yet handed to the caller. Set synchronously
    /// together with the reply bytes, so a cancelled flush leaves both in place
    /// for the next call.
    ready: Option<Vec<ServerEvent>>,
    /// Once set, the connection is done (clean EOF, `ServerEvent::Eof`, a prior
    /// `RtmpError` or a timeout) and further calls to `next_events` return
    /// `None` without touching the socket again.
    closed: bool,
    handshake_deadline: Option<Instant>,
    /// Armed when a wait starts and cleared only when a batch with events is handed
    /// out, so `read_idle` bounds a whole frame/batch, not each (possibly empty) chunk
    /// a peer drips.
    idle_deadline: Option<Instant>,
    timeouts: RtmpTimeouts,
}

impl RtmpConnection<TcpStream> {
    /// The remote address of the connected publisher.
    pub fn peer_addr(&self) -> io::Result<SocketAddr> {
        self.framed.get_ref().peer_addr()
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin> RtmpConnection<S> {
    /// Wraps an already-connected stream with a session and explicit
    /// [`RtmpTimeouts`].
    pub fn from_stream(stream: S, session: ServerSession, timeouts: RtmpTimeouts) -> Self {
        Self {
            framed: Framed::new(stream, ServerCodec::new(session)),
            ready: None,
            closed: false,
            handshake_deadline: Some(Instant::now() + timeouts.handshake),
            idle_deadline: None,
            timeouts,
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
        if self.closed {
            return Ok(None);
        }
        loop {
            // 1. Drain whatever a cancelled earlier call left in the write buffer.
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
            // 2. Hand out events whose reply is now fully written.
            if let Some(events) = self.ready.take() {
                if events
                    .iter()
                    .any(|e| matches!(e, ServerEvent::Connected { .. }))
                {
                    self.handshake_deadline = None;
                }
                if events.iter().any(|e| matches!(e, ServerEvent::Eof)) {
                    self.closed = true;
                }
                if !events.is_empty() {
                    self.idle_deadline = None;
                }
                return Ok(Some(events));
            }
            // 3. Wait for the next batch under ONE deadline (handshake or
            //    read-idle, whichever is nearer).
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
                    // Synchronous from here to the next await: reply and events
                    // move together.
                    let reply = self.framed.codec_mut().take_reply();
                    self.framed.write_buffer_mut().extend_from_slice(&reply);
                    self.ready = Some(events);
                }
            }
        }
    }
}

/// A publishing RTMP client over a stream (a [`TcpStream`] by default): the
/// adapter for [`ClientSession`](crate::client::ClientSession), mirroring
/// [`RtmpConnection`]. Every awaited IO is bounded by [`RtmpTimeouts`].
#[derive(Debug)]
pub struct AsyncRtmpClient<S = TcpStream> {
    framed: Framed<S, ClientCodec>,
    ready: Option<Vec<ClientEvent>>,
    closed: bool,
    handshake_deadline: Option<Instant>,
    /// Armed when a wait starts and cleared only when a batch with events is handed
    /// out, so `read_idle` bounds a whole frame/batch, not each (possibly empty) chunk
    /// a peer drips.
    idle_deadline: Option<Instant>,
    timeouts: RtmpTimeouts,
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
        let mut codec = ClientCodec::new(cfg);
        let c0c1 = codec.session_mut().start();
        let mut framed = Framed::new(stream, codec);
        framed.write_buffer_mut().extend_from_slice(&c0c1);
        Self {
            framed,
            ready: None,
            closed: false,
            handshake_deadline: Some(Instant::now() + timeouts.handshake),
            idle_deadline: None,
            timeouts,
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
        self.framed.write_buffer_mut().extend_from_slice(&bytes);
        match tokio::time::timeout(self.timeouts.write, self.framed.flush()).await {
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
        let bytes = self
            .framed
            .codec_mut()
            .session_mut()
            .send_audio(timestamp, data)
            .map_err(io_err)?;
        self.queue(bytes).await
    }

    /// Sends one video message (FLV video tag body) at `timestamp` ms.
    pub async fn send_video(&mut self, timestamp: u32, data: &[u8]) -> io::Result<()> {
        let bytes = self
            .framed
            .codec_mut()
            .session_mut()
            .send_video(timestamp, data)
            .map_err(io_err)?;
        self.queue(bytes).await
    }

    /// Sends `@setDataFrame`/`onMetaData` stream metadata.
    pub async fn send_metadata(&mut self, metadata: &[(String, Amf0Value)]) -> io::Result<()> {
        let bytes = self
            .framed
            .codec_mut()
            .session_mut()
            .send_metadata(metadata)
            .map_err(io_err)?;
        self.queue(bytes).await
    }

    /// Next batch of server events (acks, errors, close); `Ok(None)` at EOF.
    /// Same deadline rules and cancel-safety as [`RtmpConnection::next_events`].
    pub async fn next_events(&mut self) -> io::Result<Option<Vec<ClientEvent>>> {
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
                if events.iter().any(|e| matches!(e, ClientEvent::Publishing)) {
                    self.handshake_deadline = None;
                }
                if events.iter().any(|e| matches!(e, ClientEvent::Closed)) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    const FIXTURE: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/obs-publish.bin"
    );

    /// Replays the real captured `ffmpeg` publish (T8's `obs-publish.bin`)
    /// over an actual loopback TCP socket, driving `AsyncRtmpServer`/
    /// `RtmpConnection` end to end: a spawned client task writes the fixture
    /// bytes and drains the server's replies, while the server side accepts
    /// the connection and loops `next_events` until the connection closes.
    ///
    /// This is the online counterpart of `tests/ingest_fixture.rs`'s offline
    /// replay — it proves the tokio adapter (not just the sans-IO session)
    /// actually drives a real publish to `Connected` -> `Publish` -> `Media`.
    #[tokio::test]
    async fn loopback_replay_of_real_publish_reaches_connected_publish_media() {
        let fixture = std::fs::read(FIXTURE).expect("read tests/fixtures/obs-publish.bin");

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

        let mut conn = server.accept().await.expect("accept the client connection");
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
}
