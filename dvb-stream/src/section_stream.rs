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

use std::pin::Pin;
use std::task::{Context, Poll};

use dvb_si::demux::{SectionEvent, SiDemux};
use futures_core::Stream;
use tokio::io::AsyncRead;
use tokio_util::codec::FramedRead;

use crate::ResyncStats;
use crate::pump::PacketPump;
use crate::ts_codec::TsDecoder;
#[cfg(feature = "udp")]
use crate::udp::UdpPackets;
#[cfg(feature = "udp")]
use broadcast_common::net::{MulticastInterface, MulticastJoin, UdpOptions};

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
    pump: PacketPump<FramedRead<R, TsDecoder>, SiDemux>,
}

impl<R: AsyncRead + Unpin> SectionStream<R> {
    /// Create a `SectionStream` with the default [`SiDemux`] configuration
    /// (all standard DVB/SI PIDs, PAT-follow enabled, version gating).
    #[must_use]
    pub fn new(reader: R) -> Self {
        Self::with_demux(reader, SiDemux::builder().build())
    }

    /// Create a `SectionStream` with an already-constructed [`SiDemux`].
    #[must_use]
    pub fn with_demux(reader: R, demux: SiDemux) -> Self {
        Self {
            pump: PacketPump::new(FramedRead::new(reader, TsDecoder::new()), demux),
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
    pump: PacketPump<UdpPackets, SiDemux>,
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
            pump: PacketPump::new(UdpPackets::new(socket), demux),
        }
    }

    /// Bind a UDP socket to `bind_addr` and join `multicast_addr`.
    ///
    /// Typical DVB multicast delivery uses addresses like `239.0.0.1:5004`.
    /// Equivalent to [`bind`](Self::bind) with a default [`UdpOptions`] that
    /// joins `multicast_addr` on the bind address's interface.
    ///
    /// # Errors
    ///
    /// Returns a [`std::io::Error`] if binding or joining the multicast group
    /// fails.
    pub async fn bind_multicast(
        bind_addr: std::net::SocketAddrV4,
        multicast_addr: std::net::Ipv4Addr,
    ) -> std::io::Result<Self> {
        Self::bind(
            std::net::SocketAddr::V4(bind_addr),
            &MulticastJoin::new(std::net::IpAddr::V4(multicast_addr))
                .with_interface(MulticastInterface::V4(*bind_addr.ip())),
        )
        .await
    }

    /// Bind `bind_addr` and join `join`'s multicast group, on the OS-default
    /// socket options; the socket itself is [`crate::udp::bind_udp`], whose
    /// refusals apply.
    ///
    /// # Errors
    ///
    /// Returns a [`std::io::Error`] if `join`'s group is not a multicast
    /// address, if its interface is of the other address family, or if binding
    /// or joining fails.
    pub async fn bind(
        bind_addr: std::net::SocketAddr,
        join: &MulticastJoin,
    ) -> std::io::Result<Self> {
        let std_socket =
            crate::udp::bind_udp(bind_addr, &UdpOptions::new().with_join(join.clone()))?;
        Ok(Self::from_socket(tokio::net::UdpSocket::from_std(
            std_socket.into(),
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
