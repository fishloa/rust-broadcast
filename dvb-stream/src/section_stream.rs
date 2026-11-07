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

use bytes::Bytes;
use dvb_si::demux::{SectionEvent, SiDemux, SiDemuxBuilder};
use futures_core::Stream;
use tokio::io::AsyncRead;
use tokio_util::codec::FramedRead;

use crate::ResyncStats;
use crate::ts_codec::TsDecoder;
#[cfg(feature = "udp")]
use crate::udp::{MulticastConfig, UdpPackets};

/// Shared by every `SectionStream` flavour: drains a stream of aligned packets
/// into the demux.
pub(crate) struct SectionPump<S> {
    pub(crate) frames: S,
    demux: SiDemux,
    queue: VecDeque<SectionEvent>,
    io_error: Option<std::io::Error>,
    done: bool,
}

impl<S: Stream<Item = std::io::Result<Bytes>> + Unpin> SectionPump<S> {
    pub(crate) fn new(frames: S, demux: SiDemux) -> Self {
        Self {
            frames,
            demux,
            queue: VecDeque::new(),
            io_error: None,
            done: false,
        }
    }

    pub(crate) fn poll_next(&mut self, cx: &mut Context<'_>) -> Poll<Option<SectionEvent>> {
        loop {
            if let Some(ev) = self.queue.pop_front() {
                return Poll::Ready(Some(ev));
            }
            if self.done {
                return Poll::Ready(None);
            }
            match Pin::new(&mut self.frames).poll_next(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(None) => self.done = true,
                Poll::Ready(Some(Err(e))) => {
                    // Keep the error so a caller can tell a failed source from a
                    // clean EOF (#1036 / W-DS-1).
                    self.io_error = Some(e);
                    self.done = true;
                }
                Poll::Ready(Some(Ok(pkt))) => self.queue.extend(self.demux.feed(&pkt)),
            }
        }
    }

    pub(crate) fn stats(&self) -> dvb_si::demux::Stats {
        self.demux.stats()
    }

    pub(crate) fn take_io_error(&mut self) -> Option<std::io::Error> {
        self.io_error.take()
    }
}

/// Async [`Stream`] of [`SectionEvent`]s from a raw TS byte source.
///
/// Feed any [`tokio::io::AsyncRead`] byte source (file, TCP socket) and
/// receive one `SectionEvent` per changed complete SI section.
///
/// Internally the adapter:
/// 1. Frames `reader` into aligned 188-byte packets with a
///    [`FramedRead`] over [`TsDecoder`] (sync-byte resync via
///    [`crate::resync::resync`]).
/// 2. Feeds each packet into the owned [`SiDemux`].
/// 3. Yields events from the demux's output queue before reading more.
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
    pump: SectionPump<FramedRead<R, TsDecoder>>,
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
            pump: SectionPump::new(FramedRead::new(reader, TsDecoder::new()), demux),
        }
    }

    /// Access the underlying demux statistics.
    #[must_use]
    pub fn stats(&self) -> dvb_si::demux::Stats {
        self.pump.stats()
    }

    /// Access the resync statistics.
    #[must_use]
    pub fn resync_stats(&self) -> ResyncStats {
        self.pump.frames.decoder().resync_stats()
    }

    /// Take the I/O error that ended the stream, if `poll_next` yielded
    /// `None` because the reader errored rather than reaching a clean EOF.
    /// A caller (e.g. a reconnect supervisor) uses this to distinguish
    /// "source finished" from "source failed" (#1036 / W-DS-1).
    pub fn take_io_error(&mut self) -> Option<std::io::Error> {
        self.pump.take_io_error()
    }
}

impl<R: AsyncRead + Unpin> Stream for SectionStream<R> {
    type Item = SectionEvent;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.get_mut().pump.poll_next(cx)
    }
}

/// Async [`Stream`] of [`SectionEvent`]s from a UDP/multicast socket — enabled
/// by the `udp` feature.
///
/// Each datagram is an independent framing unit (framed by
/// [`TsDecoder::datagram`] over `tokio_util::udp::UdpFramed`): it may carry any
/// number of 188-byte packets (more than 7 for RTP-encapsulated payloads), and a
/// stray tail is dropped rather than stitched onto the next datagram (#1036 /
/// W-DS-2).
#[cfg(feature = "udp")]
pub struct UdpSectionStream {
    pump: SectionPump<UdpPackets>,
}

#[cfg(feature = "udp")]
impl UdpSectionStream {
    /// Wrap an already-bound socket (tests bind port 0 and read the port back).
    #[must_use]
    pub fn from_socket(socket: tokio::net::UdpSocket) -> Self {
        Self::from_socket_with_demux(socket, SiDemux::builder().build())
    }

    /// Like [`from_socket`](Self::from_socket) with an already-constructed [`SiDemux`].
    #[must_use]
    pub fn from_socket_with_demux(socket: tokio::net::UdpSocket, demux: SiDemux) -> Self {
        Self {
            pump: SectionPump::new(UdpPackets::new(socket), demux),
        }
    }

    /// Bind a UDP socket to `bind_addr` and join `multicast_addr`.
    ///
    /// Typical DVB multicast delivery uses addresses like `239.0.0.1:5004`.
    /// Equivalent to [`bind`](Self::bind) with [`MulticastConfig::new`].
    ///
    /// # Errors
    ///
    /// Returns a [`std::io::Error`] if binding or joining the multicast group
    /// fails.
    pub async fn bind_multicast(
        bind_addr: std::net::SocketAddrV4,
        multicast_addr: std::net::Ipv4Addr,
    ) -> std::io::Result<Self> {
        Self::bind(&MulticastConfig::new(bind_addr, multicast_addr)).await
    }

    /// Bind and join as described by `config` (`SO_RCVBUF`, reuse, interface).
    ///
    /// # Errors
    ///
    /// Returns a [`std::io::Error`] if binding or joining the multicast group
    /// fails.
    pub async fn bind(config: &MulticastConfig) -> std::io::Result<Self> {
        let std_socket = config.bind()?;
        Ok(Self::from_socket(tokio::net::UdpSocket::from_std(
            std_socket,
        )?))
    }

    /// Access the underlying demux statistics.
    #[must_use]
    pub fn stats(&self) -> dvb_si::demux::Stats {
        self.pump.stats()
    }

    /// Access the resync statistics.
    #[must_use]
    pub fn resync_stats(&self) -> ResyncStats {
        self.pump.frames.resync_stats()
    }

    /// Take the I/O error that ended the stream, if any (#1036 / W-DS-1).
    pub fn take_io_error(&mut self) -> Option<std::io::Error> {
        self.pump.take_io_error()
    }
}

#[cfg(feature = "udp")]
impl Stream for UdpSectionStream {
    type Item = SectionEvent;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.get_mut().pump.poll_next(cx)
    }
}
