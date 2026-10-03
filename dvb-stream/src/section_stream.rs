//! [`SectionStream`] — async [`futures_core::Stream`] of owned SI section events.
//!
//! Wraps [`dvb_si::demux::SiDemux`] over any [`tokio::io::AsyncRead`] source,
//! yielding one [`dvb_si::demux::SectionEvent`] per changed complete section.
//! Events are already owned (`bytes::Bytes` internally) and therefore `'static`,
//! `Clone`, and `Send + Sync` — no yoke wrapping is required.
//!
//! # Usage
//!
//! ```no_run
//! use futures_core::Stream;
//! use std::pin::Pin;
//!
//! // Stream from a file:
//! // let f = tokio::fs::File::open("stream.ts").await?;
//! // let mut s = dvb_stream::SectionStream::new(f);
//! // while let Some(event) = futures_util::StreamExt::next(&mut s).await { ... }
//! ```
//!
//! # Cancellation
//!
//! Dropping the `SectionStream` cancels cleanly — no internal tasks are
//! spawned. Any pending I/O is abandoned; partially reassembled sections are
//! discarded.

use std::collections::VecDeque;
use std::pin::Pin;
use std::task::{Context, Poll};

use dvb_si::demux::{SectionEvent, SiDemux, SiDemuxBuilder};
use futures_core::Stream;
use tokio::io::AsyncRead;
#[cfg(feature = "udp")]
use tokio::io::ReadBuf;

use crate::ResyncStats;
use crate::framer::TsFramer;

/// Async [`Stream`] of [`SectionEvent`]s from a raw TS byte source.
///
/// Feed any [`tokio::io::AsyncRead`] byte source (file, TCP socket, UDP
/// socket) and receive one `SectionEvent` per changed complete SI section.
///
/// Internally the adapter:
/// 1. Reads bytes from `reader` into a fixed-size buffer.
/// 2. Resyncs on the first `0x47` sync byte (via [`crate::resync::resync`]).
/// 3. Feeds each aligned 188-byte packet into the owned [`SiDemux`].
/// 4. Yields events from the demux's output queue before reading more.
///
/// # Owned events
///
/// [`SectionEvent`] already owns its section bytes via `bytes::Bytes` and is
/// `'static`, `Clone`, and `Send + Sync`. No additional wrapping is needed.
///
/// # Cancellation
///
/// Drop the stream. No internal tasks are spawned.
pub struct SectionStream<R> {
    /// Read + resync + packet-alignment state, shared with `T2miEventStream`.
    framer: TsFramer<R>,
    demux: SiDemux,
    queue: VecDeque<SectionEvent>,
}

impl<R: AsyncRead + Unpin> SectionStream<R> {
    /// Create a `SectionStream` with the default [`SiDemux`] configuration
    /// (all standard DVB/SI PIDs, PAT-follow enabled, version gating).
    #[must_use]
    pub fn new(reader: R) -> Self {
        Self::with_demux(reader, SiDemux::builder().build())
    }

    /// Create a `SectionStream` with a custom [`SiDemuxBuilder`].
    #[must_use]
    pub fn with_builder(reader: R, builder: SiDemuxBuilder) -> Self {
        Self::with_demux(reader, builder.build())
    }

    /// Create a `SectionStream` with an already-constructed [`SiDemux`].
    #[must_use]
    pub fn with_demux(reader: R, demux: SiDemux) -> Self {
        Self {
            framer: TsFramer::new(reader),
            demux,
            queue: VecDeque::new(),
        }
    }

    /// Access the underlying demux statistics.
    #[must_use]
    pub fn stats(&self) -> dvb_si::demux::Stats {
        self.demux.stats()
    }

    /// Access the resync statistics.
    #[must_use]
    pub fn resync_stats(&self) -> ResyncStats {
        self.framer.resync_stats()
    }

    /// Take the I/O error that ended the stream, if `poll_next` yielded
    /// `None` because the reader errored rather than reaching a clean EOF.
    /// A caller (e.g. a reconnect supervisor) uses this to distinguish
    /// "source finished" from "source failed" (#1036 / W-DS-1).
    pub fn take_io_error(&mut self) -> Option<std::io::Error> {
        self.framer.take_io_error()
    }
}

impl<R: AsyncRead + Unpin> Stream for SectionStream<R> {
    type Item = SectionEvent;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();

        loop {
            // Drain the event queue first.
            if let Some(event) = this.queue.pop_front() {
                return Poll::Ready(Some(event));
            }

            // Read more bytes; a finished source (EOF or I/O error) with an
            // empty queue ends the stream.
            let SectionStream {
                framer,
                demux,
                queue,
            } = &mut *this;
            match framer.poll_feed(cx, &mut |pkt| queue.extend(demux.feed(pkt))) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(false) => return Poll::Ready(None),
                Poll::Ready(true) => {}
            }
        }
    }
}

/// A thin [`AsyncRead`] adapter over a [`tokio::net::UdpSocket`].
///
/// Each `poll_read` call attempts one `recv` from the socket, writing the
/// received datagram bytes into the provided buffer. This is sufficient for
/// DVB multicast delivery where each UDP datagram carries exactly 7 aligned
/// 188-byte TS packets (1316 bytes).
///
/// Only constructed by [`SectionStream::bind_multicast`] and
/// [`crate::T2miEventStream::bind_multicast`].
#[cfg(feature = "udp")]
pub struct UdpReader {
    pub(crate) socket: tokio::net::UdpSocket,
}

#[cfg(feature = "udp")]
impl AsyncRead for UdpReader {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        self.get_mut().socket.poll_recv(cx, buf)
    }
}

/// UDP/multicast convenience constructor — enabled by the `udp` feature.
#[cfg(feature = "udp")]
impl SectionStream<UdpReader> {
    /// Bind a UDP socket to `bind_addr` and join `multicast_addr`.
    ///
    /// Typical DVB multicast delivery uses addresses like `239.0.0.1:5004`.
    /// The returned `SectionStream` reads one UDP datagram per `poll_next`
    /// cycle from the socket (treated as a raw TS byte source).
    ///
    /// # Errors
    ///
    /// Returns a [`std::io::Error`] if binding or joining the multicast group
    /// fails.
    pub async fn bind_multicast(
        bind_addr: std::net::SocketAddrV4,
        multicast_addr: std::net::Ipv4Addr,
    ) -> std::io::Result<Self> {
        use tokio::net::UdpSocket;
        let socket = UdpSocket::bind(bind_addr).await?;
        socket.join_multicast_v4(multicast_addr, *bind_addr.ip())?;
        let mut stream = Self::new(UdpReader { socket });
        // A UDP datagram is an independent framing unit and can be larger
        // than 7×188 bytes (RTP-encapsulated payloads) — see
        // `framer::TsFramer::set_datagram_framed` (#1036 / W-DS-2).
        stream.framer.set_datagram_framed();
        Ok(stream)
    }
}
