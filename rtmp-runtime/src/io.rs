//! Async `tokio` socket adapter driving the sans-IO ingest server session
//! over a real `tokio::net::TcpStream` — the Layer-2 adapter (see
//! [`docs/rtmp.md`](../docs/rtmp.md) § Layer-2 adapter; mirrors the
//! `rtsp_runtime::io` tokio adapter shape in this same workspace).
//!
//! [`ServerSession`] never touches a socket: it turns inbound bytes into
//! `(reply bytes, [`ServerEvent`]s)`. This module is the thin layer that
//! actually owns a [`TcpStream`], reads whatever bytes are available, feeds
//! them to [`ServerSession::handle_data`], writes the reply bytes back, and
//! returns the events — no business logic beyond that plumbing.
//!
//! [`ServerSession::handle_data`] buffers partial handshake/chunk input
//! internally (see its doc comment), so unlike an RTSP or HTTP adapter this
//! one does not need to detect message boundaries itself: any chunk size,
//! split anywhere, is fine to feed straight through.

use std::io;
use std::net::SocketAddr;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, ToSocketAddrs};

use crate::server::{ServerConfig, ServerEvent, ServerSession};

/// Size of one socket read chunk. Reads are handed to
/// [`ServerSession::handle_data`] as soon as they arrive, so this only bounds
/// a single `read` syscall, never a message.
const READ_CHUNK: usize = 8192;

/// Maps an [`RtmpError`](crate::RtmpError) from the sans-IO session into an
/// [`io::Error`] so callers only deal with one error type at this layer.
fn io_err(e: crate::RtmpError) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, e)
}

/// A `tokio::net::TcpListener` that accepts inbound RTMP publishers and hands
/// back a driven [`RtmpConnection`] per connection.
#[derive(Debug)]
pub struct AsyncRtmpServer {
    listener: TcpListener,
    config: ServerConfig,
}

impl AsyncRtmpServer {
    /// Binds a listen address (e.g. `"0.0.0.0:1935"`, the IANA-assigned RTMP
    /// port). `config` is cloned into a fresh [`ServerSession`] for each
    /// accepted connection.
    pub async fn bind<A: ToSocketAddrs>(addr: A, config: ServerConfig) -> io::Result<Self> {
        let listener = TcpListener::bind(addr).await?;
        Ok(Self { listener, config })
    }

    /// Accepts the next inbound connection and wraps it with a fresh
    /// [`ServerSession`] built from this server's [`ServerConfig`].
    pub async fn accept(&self) -> io::Result<RtmpConnection> {
        let (stream, _peer) = self.listener.accept().await?;
        Ok(RtmpConnection::new(
            stream,
            ServerSession::new(self.config.clone()),
        ))
    }

    /// The address this server is actually bound to (useful for `":0"`
    /// ephemeral-port binds).
    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.listener.local_addr()
    }
}

/// One accepted RTMP connection: a [`TcpStream`] driving a [`ServerSession`].
///
/// [`next_events`](Self::next_events) is the whole surface: read a chunk,
/// drive the session, write the reply, return the events.
#[derive(Debug)]
pub struct RtmpConnection {
    stream: TcpStream,
    session: ServerSession,
    /// Once set, the connection is done (clean EOF, `ServerEvent::Eof`, or a
    /// prior `RtmpError`) and further calls to `next_events` return `None`
    /// without touching the socket again.
    closed: bool,
}

impl RtmpConnection {
    fn new(stream: TcpStream, session: ServerSession) -> Self {
        Self {
            stream,
            session,
            closed: false,
        }
    }

    /// The remote address of the connected publisher.
    pub fn peer_addr(&self) -> io::Result<SocketAddr> {
        self.stream.peer_addr()
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
    /// # Errors
    /// An [`io::Error`] from the underlying socket read/write, or a mapped
    /// [`RtmpError`](crate::RtmpError) (kind [`io::ErrorKind::InvalidData`])
    /// from [`ServerSession::handle_data`]. On an `RtmpError` the session is
    /// unrecoverable (per `handle_data`'s own doc): this connection is torn
    /// down immediately — marked closed and not driven further, even if the
    /// caller keeps calling `next_events`.
    pub async fn next_events(&mut self) -> io::Result<Option<Vec<ServerEvent>>> {
        if self.closed {
            return Ok(None);
        }

        let mut chunk = [0u8; READ_CHUNK];
        let n = self.stream.read(&mut chunk).await?;
        if n == 0 {
            self.closed = true;
            return Ok(None);
        }

        let (reply, events) = match self.session.handle_data(&chunk[..n]) {
            Ok(v) => v,
            Err(e) => {
                // Unrecoverable: tear down rather than keep driving a session
                // whose internal state may have partially advanced.
                self.closed = true;
                return Err(io_err(e));
            }
        };

        if !reply.is_empty() {
            self.stream.write_all(&reply).await?;
        }

        if events.iter().any(|e| matches!(e, ServerEvent::Eof)) {
            self.closed = true;
        }

        Ok(Some(events))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
            let mut sink = [0u8; READ_CHUNK];
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
