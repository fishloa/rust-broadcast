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
use tokio::io::ReadBuf;

use crate::resync::{resync, TS_PACKET_SIZE, TS_SYNC_BYTE};
use crate::ResyncStats;

/// Read buffer size: 7 × 188 bytes = 1316 bytes (one UDP/RTP payload
/// as used in DVB multicast delivery per ETSI TR 101 290 §B).
const READ_BUF_SIZE: usize = TS_PACKET_SIZE * 7;

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
    reader: R,
    demux: SiDemux,
    queue: VecDeque<SectionEvent>,
    buf: Vec<u8>,
    /// Byte offset within `buf` for the next read.
    filled: usize,
    /// Whether the reader has reached EOF.
    eof: bool,
    /// True once we have found a sync byte and trimmed the leading garbage.
    synced: bool,
    /// Resync statistics.
    resync_stats: ResyncStats,
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
            reader,
            demux,
            queue: VecDeque::new(),
            buf: vec![0u8; READ_BUF_SIZE],
            filled: 0,
            eof: false,
            synced: false,
            resync_stats: ResyncStats::default(),
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
        self.resync_stats
    }

    /// Feed a completed read into the demux and push events into `queue`.
    fn feed_buf(&mut self, data: &[u8]) {
        // On first use (or after a large gap), resync to the nearest 0x47.
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
                    // no sync byte yet — discard this chunk
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
            for event in self.demux.feed(pkt) {
                self.queue.push_back(event);
            }
        }

        // If the tail was not a full packet, preserve the partial bytes.
        let aligned_end = start + (data[start..].len() / TS_PACKET_SIZE) * TS_PACKET_SIZE;
        let remainder = &data[aligned_end..];
        if !remainder.is_empty() {
            // If all bytes were consumed cleanly this will be empty.
            // Non-empty means a partial TS packet at the tail — keep for next read.
            self.buf[..remainder.len()].copy_from_slice(remainder);
            self.filled = remainder.len();
        } else {
            self.filled = 0;
        }
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

            // If EOF and queue empty, the stream is done.
            if this.eof {
                return Poll::Ready(None);
            }

            // Read more bytes from the reader.
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
                    // Feed the entire accumulated data (partial + new).
                    let data: Vec<u8> = this.buf[..total].to_vec();
                    this.feed_buf(&data);
                    // `feed_buf` updates `this.filled`; loop to drain queue.
                }
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
        Ok(Self::new(UdpReader { socket }))
    }
}
