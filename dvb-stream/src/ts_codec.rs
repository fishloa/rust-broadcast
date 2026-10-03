//! 188-byte TS packet framing as a `tokio_util::codec::Decoder` (W1 SP1.1).
//! Replaces `framer::TsFramer`'s hand-rolled read buffer: `FramedRead` owns the
//! buffer and the read loop; this type owns only the resync/alignment state.
//!
//! Both [`SectionStream`](crate::SectionStream) and
//! [`T2miEventStream`](crate::T2miEventStream) use it, so the framing is
//! implemented once (audit #1141 found two drifting copies of the old loop).

use std::io;

use bytes::{Buf, Bytes, BytesMut};
use tokio_util::codec::Decoder;

use crate::ResyncStats;
use crate::resync::{TS_PACKET_SIZE, TS_SYNC_BYTE, resync};

/// Most bytes dropped per mid-stream desync in byte-stream mode: 7 x 188 = 1316,
/// the read size of the framer this decoder replaced (so loss on a corrupt sync
/// byte stays bounded however large `FramedRead`'s buffer grows).
const DESYNC_DROP_MAX: usize = TS_PACKET_SIZE * 7;

/// Frames a byte stream (or a stream of datagrams) into aligned 188-byte TS
/// packets. `Item` is exactly one packet.
#[derive(Debug, Clone, Default)]
pub struct TsDecoder {
    synced: bool,
    stats: ResyncStats,
    datagram: bool,
}

impl TsDecoder {
    /// Byte-stream mode (file, TCP): alignment carries across reads.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Each buffer handed to `decode_eof` is one independent datagram (UDP):
    /// resync from scratch every time and never carry a partial tail over
    /// (#1036 / W-DS-2).
    #[must_use]
    pub fn datagram() -> Self {
        Self {
            datagram: true,
            ..Self::default()
        }
    }

    /// Resynchronisation counters accumulated so far.
    #[must_use]
    pub fn resync_stats(&self) -> ResyncStats {
        self.stats
    }
}

impl Decoder for TsDecoder {
    type Item = Bytes;
    type Error = io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> io::Result<Option<Bytes>> {
        loop {
            if !self.synced {
                if src.is_empty() {
                    return Ok(None);
                }
                // On first use (or after a loss of alignment), resync to the nearest 0x47.
                match resync(src) {
                    Some(off) => {
                        src.advance(off);
                        self.synced = true;
                        self.stats.resyncs += 1;
                        self.stats.bytes_discarded += off as u64;
                    }
                    None => {
                        // No sync byte at all: discard the chunk (counted).
                        self.stats.bytes_discarded += src.len() as u64;
                        src.clear();
                        return Ok(None);
                    }
                }
            }
            if src.len() < TS_PACKET_SIZE {
                return Ok(None);
            }
            if src[0] != TS_SYNC_BYTE {
                // Mid-stream desync: drop at most one read's worth (the old `TsFramer` dropped
                // the rest of its 7 x 188 = 1316-byte read), then resync on what remains.
                // A datagram is dropped whole (its remainder is never stitched on).
                self.stats.desyncs += 1;
                let drop = if self.datagram {
                    src.len()
                } else {
                    src.len().min(DESYNC_DROP_MAX)
                };
                self.stats.bytes_discarded += drop as u64;
                src.advance(drop);
                self.synced = false;
                continue; // resync on what remains, in this same call
            }
            return Ok(Some(src.split_to(TS_PACKET_SIZE).freeze()));
        }
    }

    fn decode_eof(&mut self, src: &mut BytesMut) -> io::Result<Option<Bytes>> {
        if let Some(p) = self.decode(src)? {
            return Ok(Some(p));
        }
        // End of input (stream mode) or end of datagram (datagram mode): a trailing
        // partial packet is dropped. Stream mode ignores it silently (clean EOF);
        // datagram mode counts it and forgets the sync state so the next datagram
        // resyncs independently.
        if self.datagram {
            self.stats.bytes_discarded += src.len() as u64;
            self.synced = false;
        }
        src.clear();
        Ok(None)
    }
}
