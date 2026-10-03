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

use std::collections::VecDeque;
use std::pin::Pin;
use std::task::{Context, Poll};

use bytes::Bytes;
use dvb_t2mi::pump::{T2miEvent, T2miPump};
use futures_core::Stream;
use tokio::io::AsyncRead;
use tokio_util::codec::FramedRead;

use crate::ResyncStats;
use crate::ts_codec::TsDecoder;
#[cfg(feature = "udp")]
use crate::udp::{MulticastConfig, UdpPackets};

/// Drains a stream of aligned packets into the T2-MI pump.
pub(crate) struct T2miPumpDriver<S> {
    pub(crate) frames: S,
    pump: T2miPump,
    queue: VecDeque<T2miEvent>,
    io_error: Option<std::io::Error>,
    done: bool,
}

impl<S: Stream<Item = std::io::Result<Bytes>> + Unpin> T2miPumpDriver<S> {
    pub(crate) fn new(frames: S, pump: T2miPump) -> Self {
        Self {
            frames,
            pump,
            queue: VecDeque::new(),
            io_error: None,
            done: false,
        }
    }

    pub(crate) fn poll_next(&mut self, cx: &mut Context<'_>) -> Poll<Option<T2miEvent>> {
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
                    self.io_error = Some(e);
                    self.done = true;
                }
                Poll::Ready(Some(Ok(pkt))) => self.queue.extend(self.pump.feed_ts(&pkt)),
            }
        }
    }

    pub(crate) fn stats(&self) -> dvb_t2mi::pump::Stats {
        self.pump.stats()
    }

    pub(crate) fn take_io_error(&mut self) -> Option<std::io::Error> {
        self.io_error.take()
    }
}

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
    pump: T2miPumpDriver<FramedRead<R, TsDecoder>>,
}

impl<R: AsyncRead + Unpin> T2miEventStream<R> {
    /// Create a `T2miEventStream` from a TS-encapsulated source on `pid`.
    ///
    /// `pid` is the 13-bit T2-MI PID from the PMT (e.g. `0x0006`).
    #[must_use]
    pub fn new(reader: R, pid: u16) -> Self {
        Self::with_pump(reader, T2miPump::new(pid))
    }

    /// Create a `T2miEventStream` with an already-constructed [`T2miPump`].
    #[must_use]
    pub fn with_pump(reader: R, pump: T2miPump) -> Self {
        Self {
            pump: T2miPumpDriver::new(FramedRead::new(reader, TsDecoder::new()), pump),
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
    pump: T2miPumpDriver<UdpPackets>,
}

#[cfg(feature = "udp")]
impl UdpT2miStream {
    /// Wrap an already-bound socket; `pid` is the 13-bit T2-MI PID from the PMT.
    #[must_use]
    pub fn from_socket(socket: tokio::net::UdpSocket, pid: u16) -> Self {
        Self {
            pump: T2miPumpDriver::new(UdpPackets::new(socket), T2miPump::new(pid)),
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
        pid: u16,
    ) -> std::io::Result<Self> {
        Self::bind(&MulticastConfig::new(bind_addr, multicast_addr), pid).await
    }

    /// Bind and join as described by `config` (`SO_RCVBUF`, reuse, interface).
    ///
    /// # Errors
    ///
    /// Returns a [`std::io::Error`] if binding or joining the multicast group
    /// fails.
    pub async fn bind(config: &MulticastConfig, pid: u16) -> std::io::Result<Self> {
        let std_socket = config.bind()?;
        Ok(Self::from_socket(
            tokio::net::UdpSocket::from_std(std_socket)?,
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
