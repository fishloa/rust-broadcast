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

use dvb_t2mi::pump::{T2miEvent, T2miPump};
use futures_core::Stream;
use tokio::io::AsyncRead;

use crate::ResyncStats;
use crate::framer::TsFramer;

/// Async [`Stream`] of [`T2miEvent`]s from a raw TS byte source.
///
/// Feed any [`tokio::io::AsyncRead`] byte source and receive one `T2miEvent`
/// per complete, CRC-valid T2-MI packet.
///
/// The adapter performs 188-byte TS packet alignment using the same resync
/// logic as [`SectionStream`](crate::SectionStream) (see [`crate::resync`]).
///
/// # Cancellation
///
/// Drop the stream. No internal tasks are spawned.
pub struct T2miEventStream<R> {
    /// Read + resync + packet-alignment state, shared with `SectionStream`.
    framer: TsFramer<R>,
    pump: T2miPump,
    queue: VecDeque<T2miEvent>,
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
            framer: TsFramer::new(reader),
            pump,
            queue: VecDeque::new(),
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

impl<R: AsyncRead + Unpin> Stream for T2miEventStream<R> {
    type Item = T2miEvent;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();

        loop {
            if let Some(event) = this.queue.pop_front() {
                return Poll::Ready(Some(event));
            }

            let T2miEventStream {
                framer,
                pump,
                queue,
            } = &mut *this;
            match framer.poll_feed(cx, &mut |pkt| queue.extend(pump.feed_ts(pkt))) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(false) => return Poll::Ready(None),
                Poll::Ready(true) => {}
            }
        }
    }
}

/// UDP/multicast convenience constructor — enabled by the `udp` feature.
#[cfg(feature = "udp")]
impl T2miEventStream<crate::section_stream::UdpReader> {
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
        use tokio::net::UdpSocket;
        let socket = UdpSocket::bind(bind_addr).await?;
        socket.join_multicast_v4(multicast_addr, *bind_addr.ip())?;
        let mut stream = Self::new(crate::section_stream::UdpReader { socket }, pid);
        // A UDP datagram is an independent framing unit and can be larger
        // than 7×188 bytes (RTP-encapsulated payloads) — see
        // `framer::TsFramer::set_datagram_framed` (#1036 / W-DS-2).
        stream.framer.set_datagram_framed();
        Ok(stream)
    }
}
