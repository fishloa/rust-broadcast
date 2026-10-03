//! Shared read -> resync -> 188-byte-packet framing for [`SectionStream`] and
//! [`T2miEventStream`].
//!
//! Both streams carried a byte-for-byte copy of this logic (`feed_buf` plus the
//! read loop) and the copies drifted: #1036 fixed the partial-packet carry-over
//! in one and had to be re-applied to the other (audit #1141). [`TsFramer`] is
//! the one implementation; a stream owns one and supplies only the per-packet
//! sink (`SiDemux::feed` / `T2miPump::feed_ts`).
//!
//! [`SectionStream`]: crate::SectionStream
//! [`T2miEventStream`]: crate::T2miEventStream

use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, ReadBuf};

use crate::ResyncStats;
use crate::resync::{TS_PACKET_SIZE, TS_SYNC_BYTE, resync};

/// Read buffer size: 7 × 188 bytes = 1316 bytes (one UDP/RTP payload
/// as used in DVB multicast delivery per ETSI TR 101 290 §B).
const READ_BUF_SIZE: usize = TS_PACKET_SIZE * 7;

/// Read buffer size for datagram-framed sources (UDP): the maximum possible
/// UDP payload, so a real-world datagram larger than 7×188 bytes (e.g. an
/// RTP-encapsulated 1328-byte payload) is never silently truncated by the OS
/// (#1036 / W-DS-2).
#[cfg(feature = "udp")]
const UDP_READ_BUF_SIZE: usize = 65_535;

/// Reader + resync/alignment state shared by the TS-over-`AsyncRead` streams.
pub(crate) struct TsFramer<R> {
    reader: R,
    buf: Vec<u8>,
    /// Carry-over bytes at the start of `buf` (a partial TS packet from the
    /// previous read); the next read appends after them.
    filled: usize,
    /// Whether the reader has reached EOF (or failed).
    eof: bool,
    /// True once a sync byte has been found and the leading garbage trimmed.
    synced: bool,
    resync_stats: ResyncStats,
    /// The I/O error that ended the stream, if it ended on one rather than a
    /// clean EOF (#1036 / W-DS-1).
    last_io_error: Option<std::io::Error>,
    /// Set for datagram-framed readers (UDP): each read is one independent
    /// datagram, so a trailing partial packet is never stitched onto the
    /// *next* (unrelated) datagram, and every read starts at buffer offset 0
    /// (#1036 / W-DS-2).
    datagram_framed: bool,
}

impl<R: AsyncRead + Unpin> TsFramer<R> {
    pub(crate) fn new(reader: R) -> Self {
        Self {
            reader,
            buf: vec![0u8; READ_BUF_SIZE],
            filled: 0,
            eof: false,
            synced: false,
            resync_stats: ResyncStats::default(),
            last_io_error: None,
            datagram_framed: false,
        }
    }

    /// Switch to datagram framing with a max-UDP-payload read buffer.
    #[cfg(feature = "udp")]
    pub(crate) fn set_datagram_framed(&mut self) {
        self.buf = vec![0u8; UDP_READ_BUF_SIZE];
        self.datagram_framed = true;
    }

    pub(crate) fn resync_stats(&self) -> ResyncStats {
        self.resync_stats
    }

    pub(crate) fn take_io_error(&mut self) -> Option<std::io::Error> {
        self.last_io_error.take()
    }

    /// Perform one read and hand every aligned 188-byte packet it completes
    /// to `sink`.
    ///
    /// `Ready(true)`: a read was processed (the sink may have been fed);
    /// the caller drains its queue and calls again. `Ready(false)`: the
    /// source is finished (EOF or I/O error — see [`Self::take_io_error`]).
    pub(crate) fn poll_feed(
        &mut self,
        cx: &mut Context<'_>,
        sink: &mut impl FnMut(&[u8]),
    ) -> Poll<bool> {
        if self.eof {
            return Poll::Ready(false);
        }
        let read_from = self.filled;
        let buf_len = self.buf.len();
        let mut read_buf = ReadBuf::new(&mut self.buf[read_from..buf_len]);
        match Pin::new(&mut self.reader).poll_read(cx, &mut read_buf) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(Err(e)) => {
                // Keep the error so a caller can tell a failed source from a
                // clean EOF (#1036 / W-DS-1).
                self.last_io_error = Some(e);
                self.eof = true;
                Poll::Ready(false)
            }
            Poll::Ready(Ok(())) => {
                let n = read_buf.filled().len();
                if n == 0 {
                    self.eof = true;
                    return Poll::Ready(false);
                }
                // Feed the entire accumulated data (carry-over + new). The
                // buffer is moved out so `feed` may rewrite the carry-over
                // into it without copying the whole read first.
                let mut buf = std::mem::take(&mut self.buf);
                self.feed(&mut buf, read_from + n, sink);
                self.buf = buf;
                Poll::Ready(true)
            }
        }
    }

    /// Resync `buf[..total]`, feed whole packets to `sink`, and keep a partial
    /// trailing packet at the front of `buf` for the next read.
    fn feed(&mut self, buf: &mut [u8], total: usize, sink: &mut impl FnMut(&[u8])) {
        let data = &buf[..total];
        // Each datagram is an independent framing unit (no relationship to
        // the next), so resync fresh every time instead of trusting a sync
        // state carried from an unrelated earlier datagram (#1036 / W-DS-2).
        if self.datagram_framed {
            self.synced = false;
        }
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
                    // No sync byte yet — discard this chunk.
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
                self.resync_stats.bytes_discarded += (aligned.len() - pkt_start) as u64;
                self.synced = false;
                self.filled = 0;
                return;
            }
            sink(pkt);
        }

        // If the tail was not a full packet, preserve the partial bytes —
        // unless this source is datagram-framed, where the trailing bytes
        // belong to *this* datagram only and stitching them onto the next,
        // unrelated datagram would misalign and corrupt both (#1036 / W-DS-2).
        let aligned_end = start + n_packets * TS_PACKET_SIZE;
        let remainder_len = total - aligned_end;
        if remainder_len != 0 && !self.datagram_framed {
            buf.copy_within(aligned_end..total, 0);
            self.filled = remainder_len;
        } else {
            self.resync_stats.bytes_discarded += remainder_len as u64;
            self.filled = 0;
        }
    }
}
