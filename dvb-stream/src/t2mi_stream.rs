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
use tokio::io::ReadBuf;

use crate::ResyncStats;
use crate::resync::{TS_PACKET_SIZE, TS_SYNC_BYTE, resync};

/// Read buffer size: 7 × 188 bytes (matches typical DVB UDP payload size).
const READ_BUF_SIZE: usize = TS_PACKET_SIZE * 7;

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
    reader: R,
    pump: T2miPump,
    queue: VecDeque<T2miEvent>,
    buf: Vec<u8>,
    /// Carry-over bytes from the previous read (partial TS packet).
    filled: usize,
    /// Whether the reader has reached EOF.
    eof: bool,
    /// True once the stream has found an initial 0x47 sync byte.
    synced: bool,
    /// Resync statistics.
    resync_stats: ResyncStats,
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
            reader,
            pump,
            queue: VecDeque::new(),
            buf: vec![0u8; READ_BUF_SIZE],
            filled: 0,
            eof: false,
            synced: false,
            resync_stats: ResyncStats::default(),
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
        self.resync_stats
    }

    /// Feed a completed read into the pump and push events into `queue`.
    fn feed_buf(&mut self, data: &[u8]) {
        let start = if self.synced {
            0
        } else {
            match resync(data) {
                Some(off) => {
                    self.synced = true;
                    self.resync_stats.resyncs += 1;
                    self.resync_stats.bytes_discarded += off as u64;
                    off
                }
                None => {
                    self.resync_stats.bytes_discarded += data.len() as u64;
                    return;
                }
            }
        };

        // Per-packet loop with mid-stream desync detection.
        let aligned = &data[start..];
        let n_packets = aligned.len() / TS_PACKET_SIZE;
        for i in 0..n_packets {
            let pkt_start = i * TS_PACKET_SIZE;
            let pkt = &aligned[pkt_start..pkt_start + TS_PACKET_SIZE];
            if pkt[0] != TS_SYNC_BYTE {
                // Mid-stream desync: discard rest of this chunk and re-resync.
                self.resync_stats.desyncs += 1;
                let discarded = aligned.len() - pkt_start;
                self.resync_stats.bytes_discarded += discarded as u64;
                self.synced = false;
                self.filled = 0;
                return;
            }
            for event in self.pump.feed_ts(pkt) {
                self.queue.push_back(event);
            }
        }

        self.filled = 0;
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

            if this.eof {
                return Poll::Ready(None);
            }

            let buf_len = this.buf.len();
            let read_from = this.filled;
            let mut read_buf = ReadBuf::new(&mut this.buf[read_from..buf_len]);

            match Pin::new(&mut this.reader).poll_read(cx, &mut read_buf) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Err(_)) => {
                    this.eof = true;
                    return Poll::Ready(None);
                }
                Poll::Ready(Ok(())) => {
                    let n = read_buf.filled().len();
                    if n == 0 {
                        this.eof = true;
                        return Poll::Ready(None);
                    }
                    let total = read_from + n;
                    let data: Vec<u8> = this.buf[..total].to_vec();
                    this.feed_buf(&data);
                }
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
        Ok(Self::new(crate::section_stream::UdpReader { socket }, pid))
    }
}
