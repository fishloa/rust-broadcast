//! [`T2miEventStream`] — async [`futures_core::Stream`] of owned T2-MI events.
//!
//! Wraps [`dvb_t2mi::pump::T2miPump`] over any [`tokio::io::AsyncRead`] source,
//! yielding one [`dvb_t2mi::pump::T2miEvent`] per complete, CRC-valid T2-MI packet.
//! Events own their bytes via `bytes::Bytes` and are `'static`, `Clone`, and
//! `Send + Sync`.
//!
//! # Cancellation
//!
//! Dropping the `T2miEventStream` cancels cleanly — no internal tasks are
//! spawned.

use mpeg_ts::pid::Pid;
use std::pin::Pin;
use std::task::{Context, Poll};

use dvb_t2mi::pump::{T2miEvent, T2miPump};
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

/// Async [`Stream`] of [`T2miEvent`]s from a raw TS byte source.
///
/// Feed any [`tokio::io::AsyncRead`] byte source and receive one `T2miEvent`
/// per complete, CRC-valid T2-MI packet.
///
/// The adapter performs 188-byte TS packet alignment with the same
/// [`TsDecoder`] as [`SectionStream`](crate::SectionStream) (see
/// [`crate::resync`]).
///
/// # Cancellation
///
/// Drop the stream. No internal tasks are spawned.
pub struct T2miEventStream<R> {
    pump: PacketPump<FramedRead<R, TsDecoder>, T2miPump>,
}

impl<R: AsyncRead + Unpin> T2miEventStream<R> {
    /// Create a `T2miEventStream` from a TS-encapsulated source on `pid`.
    ///
    /// `pid` is the 13-bit T2-MI PID from the PMT (e.g. `0x0006`).
    #[must_use]
    pub fn new(reader: R, pid: Pid) -> Self {
        Self::with_pump(reader, T2miPump::new(pid))
    }

    /// Create a `T2miEventStream` with an already-constructed [`T2miPump`].
    #[must_use]
    pub fn with_pump(reader: R, pump: T2miPump) -> Self {
        Self {
            pump: PacketPump::new(FramedRead::new(reader, TsDecoder::new()), pump),
        }
    }

    /// Access the underlying pump statistics.
    #[must_use]
    pub fn stats(&self) -> dvb_t2mi::pump::Stats {
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

impl<R: AsyncRead + Unpin> Stream for T2miEventStream<R> {
    type Item = T2miEvent;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.get_mut().pump.poll_next(cx)
    }
}

/// Async [`Stream`] of [`T2miEvent`]s from a UDP/multicast socket — enabled by
/// the `udp` feature. Framing is as for [`crate::section_stream::UdpSectionStream`].
#[cfg(feature = "udp")]
pub struct UdpT2miStream {
    pump: PacketPump<UdpPackets, T2miPump>,
}

#[cfg(feature = "udp")]
impl UdpT2miStream {
    /// Wrap an already-bound socket; `pid` is the 13-bit T2-MI PID from the PMT.
    #[must_use]
    pub fn from_socket(socket: tokio::net::UdpSocket, pid: Pid) -> Self {
        Self {
            pump: PacketPump::new(UdpPackets::new(socket), T2miPump::new(pid)),
        }
    }

    /// Bind a UDP socket to `bind_addr` and join `multicast_addr`.
    ///
    /// `pid` is the 13-bit T2-MI PID from the PMT.
    ///
    /// # Errors
    ///
    /// Returns a [`std::io::Error`] if binding or joining the multicast group
    /// fails.
    pub async fn bind_multicast(
        bind_addr: std::net::SocketAddrV4,
        multicast_addr: std::net::Ipv4Addr,
        pid: Pid,
    ) -> std::io::Result<Self> {
        Self::bind(
            std::net::SocketAddr::V4(bind_addr),
            &MulticastJoin::new(std::net::IpAddr::V4(multicast_addr))
                .with_interface(MulticastInterface::V4(*bind_addr.ip())),
            pid,
        )
        .await
    }

    /// Bind `bind_addr` and join `join`'s multicast group, on the OS-default
    /// socket options; the socket itself is [`crate::udp::bind_udp`], whose
    /// refusals apply.
    ///
    /// `pid` is the 13-bit T2-MI PID from the PMT.
    ///
    /// # Errors
    ///
    /// Returns a [`std::io::Error`] if `join`'s group is not a multicast
    /// address, if its interface is of the other address family, or if binding
    /// or joining fails.
    pub async fn bind(
        bind_addr: std::net::SocketAddr,
        join: &MulticastJoin,
        pid: Pid,
    ) -> std::io::Result<Self> {
        let std_socket =
            crate::udp::bind_udp(bind_addr, &UdpOptions::new().with_join(join.clone()))?;
        Ok(Self::from_socket(
            tokio::net::UdpSocket::from_std(std_socket.into())?,
            pid,
        ))
    }

    /// Access the underlying pump statistics.
    #[must_use]
    pub fn stats(&self) -> dvb_t2mi::pump::Stats {
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
impl Stream for UdpT2miStream {
    type Item = T2miEvent;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.get_mut().pump.poll_next(cx)
    }
}
