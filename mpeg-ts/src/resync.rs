//! Stateful TS byte-stream resynchroniser — ITU-T H.222.0 §2.4.3.2 (= ISO/IEC 13818-1 §2.4.3.2).
//!
//! Recovers 188-byte MPEG-TS packet alignment from an arbitrary byte stream
//! (file reads, UDP payloads) that may start mid-packet or contain leading
//! garbage.  Also detects 204-byte Reed-Solomon-coded packets (DVB RS-coded
//! outer forward-error-correction layer) and strips the 16 parity bytes, and
//! 192-byte M2TS (BDAV) source packets and strips their 4-byte
//! `TP_extra_header`, yielding standard 188-byte TS packets in every case.
//!
//! # Feature gate
//!
//! This module is only compiled when the `ts` feature is enabled (the
//! default), because it depends on the TS constants in [`crate::ts`].
//!
//! # Example
//!
//! ```
//! use mpeg_ts::resync::TsResync;
//!
//! let mut r = TsResync::new();
//! // Feed arbitrary bytes (file chunks, UDP datagrams, etc.).
//! let packets: Vec<[u8; 188]> = r.feed(b"some raw bytes");
//! let stats = r.stats();
//! ```

use alloc::vec::Vec;

use crate::ts::{TS_PACKET_SIZE, TS_SYNC_BYTE};

/// Reed-Solomon-coded TS packet size: 188-byte payload + 16 parity bytes
/// (DVB RS outer FEC, ISO/IEC 13818-1 §2.4.3.2 informative note).
pub const RS_PACKET_SIZE: usize = 204;

/// Number of Reed-Solomon parity bytes appended to a 204-byte packet.
pub const RS_PARITY_LEN: usize = RS_PACKET_SIZE - TS_PACKET_SIZE;

/// M2TS (BDAV) source packet size: a 4-byte `TP_extra_header`
/// (copy-permission indicator + arrival time stamp) followed by a 188-byte TS
/// packet.
pub const M2TS_PACKET_SIZE: usize = 192;

/// Length of the `TP_extra_header` that precedes each TS packet in M2TS.
pub const M2TS_EXTRA_HEADER_LEN: usize = M2TS_PACKET_SIZE - TS_PACKET_SIZE;

/// Consecutive sync bytes at the candidate stride required to declare lock.
pub const LOCK_CONFIRMATIONS: usize = 5;

/// Detected packet size after locking.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum PacketStride {
    /// Standard 188-byte TS packets (ISO/IEC 13818-1 §2.4.3.2).
    Ts188,
    /// 204-byte packets (188-byte TS + 16 Reed-Solomon parity bytes).
    Rs204,
    /// 192-byte M2TS (BDAV) source packets (4-byte `TP_extra_header` +
    /// 188-byte TS).
    M2ts192,
}

impl PacketStride {
    /// Short machine-readable label for this stride.
    pub fn name(&self) -> &'static str {
        match self {
            PacketStride::Ts188 => "ts_188",
            PacketStride::Rs204 => "rs_204",
            PacketStride::M2ts192 => "m2ts_192",
        }
    }

    /// Bytes from one sync byte to the next at this stride.
    pub fn packet_len(&self) -> usize {
        match self {
            PacketStride::Ts188 => TS_PACKET_SIZE,
            PacketStride::Rs204 => RS_PACKET_SIZE,
            PacketStride::M2ts192 => M2TS_PACKET_SIZE,
        }
    }
}

broadcast_common::impl_spec_display!(PacketStride);

/// Counters accumulated during resynchronisation.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ResyncStats {
    /// Total 188-byte TS packets emitted.
    pub packets: u64,
    /// Times sync was lost and reacquired.
    pub resyncs: u64,
    /// Bytes skipped/dropped (junk before lock + sync-loss bytes).
    pub dropped_bytes: u64,
}

/// One span of bytes the resynchroniser discarded: either the bytes before
/// the first lock (a capture that starts mid-packet or on garbage), a
/// mid-stream sync loss, or a truncated trailing packet.
///
/// Drained with [`TsResync::take_losses`]; one entry per discarded span, so a
/// caller can report each skipped unit exactly once, and the spans account for
/// every byte in [`ResyncStats::dropped_bytes`].
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ResyncLoss {
    /// Byte offset, from the first byte ever fed, where the discarded span starts.
    pub offset: u64,
    /// Length of the discarded span in bytes.
    pub dropped_bytes: u64,
    /// Number of whole packets emitted before this loss, i.e. the index the
    /// next intact packet will carry.
    pub packet_index: u64,
    /// `true` when the span is a truncated trailing packet found by
    /// [`TsResync::finish`] rather than a sync loss in mid-stream.
    pub truncated_tail: bool,
}

/// Stateful TS byte-stream resynchroniser (ISO/IEC 13818-1 §2.4.3.2).
///
/// Recovers 188-byte MPEG-TS packet alignment from an arbitrary byte stream
/// that may start mid-packet or contain junk, and detects 204-byte
/// Reed-Solomon-coded packets (stripping the 16 parity bytes) and 192-byte
/// M2TS source packets (stripping the 4-byte `TP_extra_header`).
///
/// Feed raw bytes with [`feed`](Self::feed); each call returns a `Vec` of
/// aligned 188-byte TS packets.  Bytes are buffered across calls so that
/// packet boundaries may span call boundaries freely.
///
/// Lock is declared after [`LOCK_CONFIRMATIONS`] consecutive sync bytes are
/// found at the candidate stride (188, 192 or 204).  On sync loss the resynchroniser
/// re-scans from the byte after the lost position.
///
/// Every discarded byte is reported as a [`ResyncLoss`], including the bytes
/// before the first lock: a caller that never sees a loss event therefore
/// never silently dropped input.
///
/// # Stats
///
/// [`stats`](Self::stats) returns cumulative counters (packets emitted,
/// resyncs, dropped bytes).
#[derive(Debug, Default)]
pub struct TsResync {
    buf: Vec<u8>,
    /// Logical read head into `buf`; compacted periodically.
    head: usize,
    stride: Option<PacketStride>,
    stats: ResyncStats,
    /// Bytes already compacted away (stream offset of `buf[0]`).
    consumed: u64,
    /// Stream offset where the currently open sync loss began.
    open_loss: Option<u64>,
    /// Closed losses not yet drained by [`take_losses`](Self::take_losses).
    losses: Vec<ResyncLoss>,
    /// Aligned packets recovered by [`finish`](Self::finish), not yet drained
    /// by [`take_tail_packets`](Self::take_tail_packets).
    tail: Vec<[u8; TS_PACKET_SIZE]>,
    /// Bytes of the current stride still to skip after a packet emitted
    /// before its stride's trailing bytes arrived (M2TS: the next packet's
    /// `TP_extra_header`).
    skip: usize,
}

impl TsResync {
    /// Create a new resynchroniser with an empty internal buffer.
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed `data` and emit every newly-aligned 188-byte TS packet.
    ///
    /// For a 204-byte stream the 16 Reed-Solomon parity bytes are stripped,
    /// and for a 192-byte M2TS stream the 4-byte `TP_extra_header`; only the
    /// 188-byte TS packet is returned.  Bytes that cannot yet form
    /// a complete packet (or fall before lock) are buffered for the next call.
    ///
    /// **Drain [`take_losses`](Self::take_losses).** Every span discarded after
    /// lock is appended to an internal list (about 32-40 bytes each) that only
    /// `take_losses` empties; a caller that feeds forever and never drains it
    /// grows memory linearly with the number of re-locks on a lossy input.
    pub fn feed(&mut self, data: &[u8]) -> Vec<[u8; TS_PACKET_SIZE]> {
        let mut emitted = Vec::new();
        self.feed_into(data, &mut emitted);
        emitted
    }

    /// [`feed`](Self::feed) appending to a caller-owned `out` (not cleared),
    /// so a steady-state caller that reuses one `Vec` allocates nothing per
    /// call (audit r01-O4). The output capacity is reserved once from the
    /// buffered byte count instead of growing by doubling.
    ///
    /// Returns the number of packets appended.
    pub fn feed_into(&mut self, data: &[u8], emitted: &mut Vec<[u8; TS_PACKET_SIZE]>) -> usize {
        self.buf.extend_from_slice(data);
        let before = emitted.len();
        emitted.reserve((self.buf.len() - self.head) / TS_PACKET_SIZE);

        loop {
            match self.stride {
                None => {
                    if let Some((offset, s)) = find_sync(&self.buf[self.head..]) {
                        self.lock(offset, s);
                    } else {
                        // Keep at most enough bytes to detect a future lock.
                        let keep = LOCK_CONFIRMATIONS * RS_PACKET_SIZE;
                        let tail_len = self.buf.len() - self.head;
                        if tail_len > keep {
                            let excess = tail_len - keep;
                            self.stats.dropped_bytes += excess as u64;
                            self.head += excess;
                        }
                        self.compact();
                        return emitted.len() - before;
                    }
                }
                Some(stride) => {
                    let s = stride.packet_len();
                    let tail_len = self.buf.len() - self.head;
                    if self.skip > 0 {
                        let n = self.skip.min(tail_len);
                        self.head += n;
                        self.skip -= n;
                        if self.skip > 0 {
                            self.compact();
                            return emitted.len() - before;
                        }
                        continue;
                    }
                    if tail_len < emit_len(stride) {
                        self.compact();
                        return emitted.len() - before;
                    }
                    if self.buf[self.head] == TS_SYNC_BYTE {
                        let mut packet = [0u8; TS_PACKET_SIZE];
                        packet.copy_from_slice(&self.buf[self.head..self.head + TS_PACKET_SIZE]);
                        emitted.push(packet);
                        let advance = s.min(tail_len);
                        self.head += advance;
                        self.skip = s - advance;
                        self.stats.packets += 1;
                    } else {
                        // Sync byte missing — record the loss and re-scan.
                        self.stats.resyncs += 1;
                        self.stats.dropped_bytes += 1;
                        self.open_loss
                            .get_or_insert(self.consumed + self.head as u64);
                        self.head += 1;
                        self.stride = None;
                    }
                }
            }
        }
    }

    /// Detected packet stride, or [`None`] before the stream has locked.
    pub fn stride(&self) -> Option<PacketStride> {
        self.stride
    }

    /// Drain the sync losses recorded since the last call (see [`ResyncLoss`]).
    ///
    /// This is the only thing that empties the internal loss list, which is
    /// unbounded by design (dropping a loss would hide data loss): call it
    /// regularly — e.g. after every [`feed`](Self::feed) — or the list grows
    /// with every re-lock on a lossy input. If you do not need the losses,
    /// still call it and discard the result.
    pub fn take_losses(&mut self) -> Vec<ResyncLoss> {
        core::mem::take(&mut self.losses)
    }

    fn lock(&mut self, offset: usize, stride: PacketStride) {
        self.open_loss
            .get_or_insert(self.consumed + self.head as u64);
        self.head += offset;
        self.stride = Some(stride);
        if let Some(start) = self.open_loss.take() {
            let end = self.consumed + self.head as u64;
            if end > start {
                self.stats.dropped_bytes += end - start;
                self.losses.push(ResyncLoss {
                    offset: start,
                    dropped_bytes: end - start,
                    packet_index: self.stats.packets,
                    truncated_tail: false,
                });
            }
        }
    }

    /// Declare end of input. Packets that were still waiting for a full
    /// [`LOCK_CONFIRMATIONS`] window cannot be confirmed by more bytes, so the
    /// aligned packets that fill the rest of the input (a stream shorter than
    /// the window included) are kept; drain them with
    /// [`take_tail_packets`](Self::take_tail_packets), like
    /// [`take_losses`](Self::take_losses) for the losses.
    ///
    /// Also closes a sync loss that never re-locked and records a truncated
    /// trailing packet, both retrievable with
    /// [`take_losses`](Self::take_losses). The buffer is discarded.
    pub fn finish(&mut self) {
        if self.stride.is_none()
            && let Some((offset, s)) = find_tail_sync(&self.buf[self.head..])
        {
            self.lock(offset, s);
            let mut emitted = core::mem::take(&mut self.tail);
            self.feed_into(&[], &mut emitted);
            self.tail = emitted;
        }
        let end = self.consumed + self.buf.len() as u64;
        if let Some(start) = self.open_loss.take() {
            self.losses.push(ResyncLoss {
                offset: start,
                dropped_bytes: end - start,
                packet_index: self.stats.packets,
                truncated_tail: false,
            });
        } else if self.stride.is_some() && self.head < self.buf.len() {
            self.losses.push(ResyncLoss {
                offset: self.consumed + self.head as u64,
                dropped_bytes: (self.buf.len() - self.head) as u64,
                packet_index: self.stats.packets,
                truncated_tail: true,
            });
        }
        self.stats.dropped_bytes += (self.buf.len() - self.head) as u64;
        self.head = self.buf.len();
        self.compact();
    }

    /// Drain the aligned packets [`finish`](Self::finish) recovered at end of
    /// input. Call it after `finish` and process the result like the output
    /// of [`feed`](Self::feed).
    pub fn take_tail_packets(&mut self) -> Vec<[u8; TS_PACKET_SIZE]> {
        core::mem::take(&mut self.tail)
    }

    /// Accumulated statistics.
    pub fn stats(&self) -> ResyncStats {
        self.stats
    }

    /// Compact the internal buffer by discarding consumed bytes.
    fn compact(&mut self) {
        if self.head > 0 {
            self.consumed += self.head as u64;
            self.buf.drain(..self.head);
            self.head = 0;
        }
    }
}

/// Scan `buf` for the smallest offset `o` such that stride `S` (tried 188
/// first, then 192, then 204) yields [`LOCK_CONFIRMATIONS`] consecutive sync bytes at
/// positions `o + k*S` for `k = 0 .. LOCK_CONFIRMATIONS`.
///
/// Returns `(offset, stride)` on success, or `None` if no lock is found.
fn find_sync(buf: &[u8]) -> Option<(usize, PacketStride)> {
    for o in 0..buf.len() {
        if buf[o] != TS_SYNC_BYTE {
            continue;
        }
        if let Some(stride) = STRIDES
            .into_iter()
            .find(|stride| try_stride(buf, o, stride.packet_len()))
        {
            return Some((o, stride));
        }
    }
    None
}

/// The strides [`find_sync`] and [`find_tail_sync`] try, in order.
const STRIDES: [PacketStride; 3] = [
    PacketStride::Ts188,
    PacketStride::M2ts192,
    PacketStride::Rs204,
];

/// End-of-input counterpart of [`find_sync`]: the smallest offset whose
/// stride (188 first, then 192, then 204) holds a sync byte at every packet boundary
/// the buffer still covers, with at least one whole packet.
fn find_tail_sync(buf: &[u8]) -> Option<(usize, PacketStride)> {
    let chain = |o: usize, stride: PacketStride| {
        let need = emit_len(stride);
        o + need <= buf.len()
            && (o..buf.len())
                .step_by(stride.packet_len())
                .take_while(|pos| pos + need <= buf.len())
                .all(|pos| buf[pos] == TS_SYNC_BYTE)
    };
    (0..buf.len())
        .filter(|&o| buf[o] == TS_SYNC_BYTE)
        .find_map(|o| {
            STRIDES
                .into_iter()
                .find(|&stride| chain(o, stride))
                .map(|stride| (o, stride))
        })
}

/// Bytes from a sync byte that complete one packet at `stride`: the 204-byte
/// stride's parity trails its packet, while the 4 bytes after an M2TS packet
/// belong to the next one.
fn emit_len(stride: PacketStride) -> usize {
    match stride {
        PacketStride::Rs204 => RS_PACKET_SIZE,
        PacketStride::Ts188 | PacketStride::M2ts192 => TS_PACKET_SIZE,
    }
}

/// Return `true` if `LOCK_CONFIRMATIONS` consecutive sync bytes exist at
/// stride `s` starting from `offset` (the first is already known to be
/// [`TS_SYNC_BYTE`]).
fn try_stride(buf: &[u8], offset: usize, s: usize) -> bool {
    for k in 1..LOCK_CONFIRMATIONS {
        let pos = offset + k * s;
        if pos >= buf.len() || buf[pos] != TS_SYNC_BYTE {
            return false;
        }
    }
    true
}

// ── Stateless datagram resync (dvb-stream origin) ───────────────────────────

/// Find the byte offset of the first confirmed `0x47` sync byte in `buf`,
/// accepting a lone sync byte when the buffer is too small to hold a
/// confirming packet.
///
/// "Confirmed" means either `buf[offset + TS_PACKET_SIZE] == TS_SYNC_BYTE`
/// (two-packet confirmation), or the buffer is too small for two packets
/// (best-effort single-sync-byte alignment).
///
/// Returns `None` when `buf` contains no `0x47` byte at all.
#[must_use]
pub fn resync(buf: &[u8]) -> Option<usize> {
    let mut i = 0;
    while i < buf.len() {
        if buf[i] == TS_SYNC_BYTE {
            let next = i + TS_PACKET_SIZE;
            if next < buf.len() {
                if buf[next] == TS_SYNC_BYTE {
                    return Some(i);
                }
                i += 1;
                continue;
            }
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Find the byte offset of the first sync byte in `buf` confirmed by a
/// second sync byte exactly one packet later. Never accepts a lone sync byte.
#[must_use]
pub fn resync_confirmed(buf: &[u8]) -> Option<usize> {
    (0..buf.len().saturating_sub(TS_PACKET_SIZE))
        .find(|&i| buf[i] == TS_SYNC_BYTE && buf[i + TS_PACKET_SIZE] == TS_SYNC_BYTE)
}

/// Offset of the first `0x47` in `buf` that has no room for a confirming
/// sync byte one packet later.
#[must_use]
pub fn unconfirmed_candidate(buf: &[u8]) -> Option<usize> {
    let from = buf.len().saturating_sub(TS_PACKET_SIZE);
    buf[from..]
        .iter()
        .position(|&b| b == TS_SYNC_BYTE)
        .map(|i| from + i)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;
    use alloc::vec::Vec;
    extern crate std;

    /// Build a 188-byte TS packet starting with `TS_SYNC_BYTE` followed by
    /// `tag` (repeated).  All non-sync bytes are kept away from `0x47` so
    /// that no false lock occurs in the fixture data.
    fn ts_packet(tag: u8) -> [u8; TS_PACKET_SIZE] {
        assert_ne!(tag, TS_SYNC_BYTE, "tag must not equal sync byte");
        let mut pkt = [tag; TS_PACKET_SIZE];
        pkt[0] = TS_SYNC_BYTE;
        pkt
    }

    /// Build a 204-byte RS-coded packet: 188-byte TS payload (with `ts_tag`)
    /// plus 16 parity bytes filled with `parity` (must not be `0x47`).
    fn rs_packet(ts_tag: u8, parity: u8) -> [u8; RS_PACKET_SIZE] {
        assert_ne!(ts_tag, TS_SYNC_BYTE);
        assert_ne!(parity, TS_SYNC_BYTE);
        let mut pkt = [parity; RS_PACKET_SIZE];
        pkt[0] = TS_SYNC_BYTE;
        pkt[1..TS_PACKET_SIZE].fill(ts_tag);
        pkt
    }

    /// Concatenate several 188-byte packet arrays into a flat `Vec<u8>`.
    fn concat_ts(packets: &[[u8; TS_PACKET_SIZE]]) -> Vec<u8> {
        let mut v = Vec::with_capacity(packets.len() * TS_PACKET_SIZE);
        for p in packets {
            v.extend_from_slice(p);
        }
        v
    }

    // ------------------------------------------------------------------
    // Test 1 — aligned 188-byte passthrough
    // ------------------------------------------------------------------
    #[test]
    fn lone_sync_byte_accepted_by_resync_not_confirmed() {
        let buf = [TS_SYNC_BYTE];
        assert_eq!(resync(&buf), Some(0));
        assert_eq!(resync_confirmed(&buf), None);
    }

    #[test]
    fn aligned_188_passthrough() {
        let packets: Vec<_> = (0u8..5).map(|i| ts_packet(i + 1)).collect();
        let data = concat_ts(&packets);

        let mut r = TsResync::new();
        let emitted = r.feed(&data);

        assert_eq!(emitted.len(), 5);
        for (i, (e, p)) in emitted.iter().zip(packets.iter()).enumerate() {
            assert_eq!(e, p, "packet {i} mismatch");
        }
        assert_eq!(r.stride(), Some(PacketStride::Ts188));
        let s = r.stats();
        assert_eq!(s.packets, 5);
        assert_eq!(s.resyncs, 0);
        assert_eq!(s.dropped_bytes, 0);
    }

    // ------------------------------------------------------------------
    // Test 2 — lock only after LOCK_CONFIRMATIONS sync bytes
    // ------------------------------------------------------------------
    #[test]
    fn requires_n_confirmations_before_lock() {
        // Pin the default lock window. The hardcoded 4-vs-5 boundary below only
        // bites the threshold when this is 5 — assert it explicitly so a change
        // to the default (or to this test's literals) cannot silently pass.
        assert_eq!(
            LOCK_CONFIRMATIONS, 5,
            "this test pins the default lock window of 5"
        );

        // FOUR confirming, stride-aligned packets are one short of the window:
        // only four sync bytes sit at the 188-stride boundaries, so the
        // resynchroniser must NOT lock and must emit nothing (it buffers).
        let four = concat_ts(&(1u8..=4).map(ts_packet).collect::<Vec<_>>());
        let mut r = TsResync::new();
        assert_eq!(
            r.feed(&four).len(),
            0,
            "4 confirmations (< 5) must not lock or emit"
        );
        assert_eq!(r.stride(), None, "stride must remain None below the window");

        // The FIFTH stride-aligned sync byte completes the window → lock.
        let mut out = r.feed(&ts_packet(5));
        out.extend(r.feed(&[]));
        assert!(
            r.stride().is_some(),
            "the 5th confirmation must trigger lock"
        );
        // Once locked, the buffered packets are emitted (5 fed, all aligned).
        assert_eq!(out.len(), 5, "all five aligned packets emit once locked");
    }

    // ------------------------------------------------------------------
    // Test 3 — junk prefix: leading garbage dropped, correct count returned
    // ------------------------------------------------------------------
    #[test]
    fn junk_prefix_correct_dropped_count() {
        let pkts: Vec<_> = (0u8..6).map(|i| ts_packet(i + 1)).collect();
        let stream = concat_ts(&pkts);

        let junk_len = 7usize;
        let mut data: Vec<u8> = vec![0x00; junk_len];
        data.extend_from_slice(&stream);

        let mut r = TsResync::new();
        let emitted = r.feed(&data);

        assert_eq!(emitted.len(), 6);
        for (i, (e, p)) in emitted.iter().zip(pkts.iter()).enumerate() {
            assert_eq!(*e, *p, "packet {i} mismatch after junk prefix");
        }
        let s = r.stats();
        assert_eq!(
            s.dropped_bytes, junk_len as u64,
            "dropped bytes must equal junk prefix"
        );
        assert_eq!(s.resyncs, 0);
        assert_eq!(s.packets, 6);
    }

    // ------------------------------------------------------------------
    // Test 4 — mid-stream sync loss and reacquisition
    // ------------------------------------------------------------------
    #[test]
    fn midstream_loss_resync() {
        // Need > LOCK_CONFIRMATIONS clean packets before and after the stray
        // byte so the stream locks, loses sync, and re-locks.
        let pkts: Vec<_> = (0u8..14).map(|i| ts_packet(i + 1)).collect();
        let clean = concat_ts(&pkts);

        // Insert a single stray byte 12 bytes into packet 7.
        let insert_at = 7 * TS_PACKET_SIZE + 12;
        let mut data = clean[..insert_at].to_vec();
        data.push(0x00);
        data.extend_from_slice(&clean[insert_at..]);

        let mut r = TsResync::new();
        let emitted = r.feed(&data);

        let s = r.stats();
        assert!(
            s.resyncs >= 1,
            "mid-stream corruption must trigger a resync, got {}",
            s.resyncs
        );
        assert_eq!(emitted[0], pkts[0], "first emitted packet is P0");
        assert!(
            emitted.len() >= 10,
            "should recover and emit most packets, got {}",
            emitted.len()
        );
    }

    // ------------------------------------------------------------------
    // Test 5 — 204-byte RS-coded packets detected + RS stripped
    // ------------------------------------------------------------------
    #[test]
    fn rs204_detected_and_stripped() {
        let mut stream = Vec::new();
        let mut expected = Vec::new();
        for i in 0u8..6 {
            let tag = i + 1;
            let rs = rs_packet(tag, 0xFF);
            stream.extend_from_slice(&rs);
            expected.push(ts_packet(tag));
        }

        let mut r = TsResync::new();
        let emitted = r.feed(&stream);

        assert_eq!(emitted.len(), 6, "RS-coded stream must emit 6 packets");
        assert_eq!(
            r.stride(),
            Some(PacketStride::Rs204),
            "stride must be Rs204"
        );
        for (i, (e, p)) in emitted.iter().zip(expected.iter()).enumerate() {
            assert_eq!(e, p, "RS-stripped packet {i} mismatch");
        }
        // Confirm the emitted 188-byte packets are parseable by TsPacket.
        for (i, pkt) in emitted.iter().enumerate() {
            crate::ts::TsPacket::parse(pkt)
                .unwrap_or_else(|e| panic!("RS-stripped packet {i} TsPacket::parse failed: {e}"));
        }
        let s = r.stats();
        assert_eq!(s.packets, 6);
        assert_eq!(s.resyncs, 0);
        assert_eq!(s.dropped_bytes, 0);
    }

    // ------------------------------------------------------------------
    // Test 6 — aligned 188 stream yields same packets as plain chunks(188)
    //   (equivalence: fixture-less variant using synthetic data)
    // ------------------------------------------------------------------
    /// The losses are one per discarded span, in stream order: the leading
    /// garbage (if any) and the mid-stream sync loss or truncated tail.
    fn assert_one_loss_plus_leading(losses: &[ResyncLoss], leading: u64, expected: ResyncLoss) {
        let expected_leading = usize::from(leading > 0);
        assert_eq!(losses.len(), expected_leading + 1, "{losses:?}");
        if leading > 0 {
            assert_eq!(losses[0].offset, 0);
            assert_eq!(losses[0].dropped_bytes, leading);
            assert!(!losses[0].truncated_tail);
        }
        assert_eq!(losses[expected_leading], expected);
    }

    #[test]
    fn broken_sync_byte_after_lock_is_one_loss_with_offset() {
        let mut bytes = Vec::new();
        for t in 1..=12u8 {
            bytes.extend_from_slice(&ts_packet(t));
        }
        bytes[5 * TS_PACKET_SIZE] = 0x00;
        let mut r = TsResync::new();
        let got = r.feed(&bytes);
        assert_eq!(got.len(), 11);
        r.finish();
        let losses = r.take_losses();
        assert_one_loss_plus_leading(
            &losses,
            0,
            ResyncLoss {
                offset: 5 * TS_PACKET_SIZE as u64,
                dropped_bytes: TS_PACKET_SIZE as u64,
                packet_index: 5,
                truncated_tail: false,
            },
        );
    }

    #[test]
    fn leading_junk_at_the_first_lock_is_a_loss() {
        let mut bytes = vec![0x11; 30];
        for t in 1..=8u8 {
            bytes.extend_from_slice(&ts_packet(t));
        }
        let mut r = TsResync::new();
        assert_eq!(r.feed(&bytes).len(), 8);
        let losses = r.take_losses();
        assert_eq!(losses.len(), 1, "{losses:?}");
        assert_eq!(losses[0].offset, 0);
        assert_eq!(losses[0].dropped_bytes, 30);
        assert_eq!(losses[0].packet_index, 0);
        assert!(!losses[0].truncated_tail);
    }

    #[test]
    fn bulk_discarded_junk_before_lock_is_a_loss() {
        let junk_len = LOCK_CONFIRMATIONS * RS_PACKET_SIZE + 7;
        let mut bytes = vec![0x00; junk_len];
        for t in 1..=8u8 {
            bytes.extend_from_slice(&ts_packet(t));
        }
        let mut r = TsResync::new();
        assert_eq!(r.feed(&bytes).len(), 8);
        let losses = r.take_losses();
        assert_eq!(losses.len(), 1, "{losses:?}");
        assert_eq!(losses[0].offset, 0);
        assert_eq!(losses[0].dropped_bytes, junk_len as u64);
        assert_eq!(r.stats().dropped_bytes, junk_len as u64);
    }

    #[test]
    fn leading_junk_is_a_loss_and_so_is_the_truncated_tail() {
        let mut bytes = vec![0x11; 30];
        for t in 1..=8u8 {
            bytes.extend_from_slice(&ts_packet(t));
        }
        bytes.extend_from_slice(&[0x22; 50]);
        let mut r = TsResync::new();
        let _ = r.feed(&bytes);
        r.finish();
        let losses = r.take_losses();
        assert_one_loss_plus_leading(
            &losses,
            30,
            ResyncLoss {
                offset: 30 + 8 * TS_PACKET_SIZE as u64,
                dropped_bytes: 50,
                packet_index: 8,
                truncated_tail: true,
            },
        );
    }

    #[test]
    fn aligned_188_matches_plain_chunks() {
        let pkts: Vec<_> = (0u8..10).map(|i| ts_packet(i + 1)).collect();
        let data = concat_ts(&pkts);

        // Oracle: plain chunks(188) filtered by sync byte.
        let oracle: Vec<[u8; TS_PACKET_SIZE]> = data
            .chunks_exact(TS_PACKET_SIZE)
            .filter(|c| c[0] == TS_SYNC_BYTE)
            .map(|c| c.try_into().unwrap())
            .collect();

        let mut r = TsResync::new();
        let emitted = r.feed(&data);

        assert_eq!(emitted.len(), oracle.len(), "count mismatch");
        for (i, (e, o)) in emitted.iter().zip(oracle.iter()).enumerate() {
            assert_eq!(e, o, "packet {i} differs from chunks-oracle");
        }
    }

    // ------------------------------------------------------------------
    // Test 7 — chunked feed equivalence (same result in small increments)
    // ------------------------------------------------------------------
    #[test]
    fn chunked_feed_equivalence() {
        let pkts: Vec<_> = (0u8..6).map(|i| ts_packet(i + 1)).collect();
        let stream = concat_ts(&pkts);

        let whole = {
            let mut r = TsResync::new();
            r.feed(&stream)
        };

        let chunked = {
            let mut r = TsResync::new();
            let mut out = Vec::new();
            for chunk in stream.chunks(100) {
                out.extend(r.feed(chunk));
            }
            out
        };

        assert_eq!(whole.len(), chunked.len());
        for (i, (w, c)) in whole.iter().zip(chunked.iter()).enumerate() {
            assert_eq!(w, c, "packet {i} mismatch");
        }
    }

    // ------------------------------------------------------------------
    // Test 8 — fixture-based: m6-single.ts differential
    //   feeding the real fixture through TsResync must yield the same
    //   188-byte packets as plain chunks_exact(188) + sync-byte filter.
    // ------------------------------------------------------------------
    #[test]
    fn fixture_m6_matches_plain_chunks() {
        let path = test_fixtures::path("ts/m6-single.ts");
        let data = std::fs::read(path).expect("m6-single.ts fixture not found");

        // Oracle: plain aligned 188-byte reads.
        let oracle: Vec<[u8; TS_PACKET_SIZE]> = data
            .chunks_exact(TS_PACKET_SIZE)
            .filter(|c| c[0] == TS_SYNC_BYTE)
            .map(|c| c.try_into().unwrap())
            .collect();
        assert!(!oracle.is_empty(), "oracle empty — fixture empty?");

        let mut r = TsResync::new();
        let emitted = r.feed(&data);

        assert_eq!(
            emitted.len(),
            oracle.len(),
            "packet count: TsResync={} oracle={}",
            emitted.len(),
            oracle.len()
        );
        for (i, (e, o)) in emitted.iter().zip(oracle.iter()).enumerate() {
            assert_eq!(e, o, "packet {i} mismatch vs oracle");
        }
    }

    #[test]
    fn finish_emits_the_aligned_packets_after_a_corrupt_sync_byte() {
        let mut input = Vec::new();
        for tag in 1u8..=10 {
            input.extend_from_slice(&ts_packet(tag));
        }
        let mut bad = ts_packet(11);
        bad[0] = 0x00;
        input.extend_from_slice(&bad);
        for tag in 12u8..=14 {
            input.extend_from_slice(&ts_packet(tag));
        }

        let mut r = TsResync::new();
        let mut packets = r.feed(&input);
        let tags: Vec<u8> = packets.iter().map(|p| p[1]).collect();
        assert_eq!(tags, (1u8..=10).collect::<Vec<_>>());
        r.finish();
        packets.extend(r.take_tail_packets());
        let tags: Vec<u8> = packets.iter().map(|p| p[1]).collect();
        assert_eq!(tags, [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 12, 13, 14]);
        let stats = r.stats();
        assert_eq!(stats.packets, 13);
        assert_eq!(stats.dropped_bytes, TS_PACKET_SIZE as u64 + 1);
        let losses = r.take_losses();
        assert_one_loss_plus_leading(
            &losses,
            0,
            ResyncLoss {
                offset: 10 * TS_PACKET_SIZE as u64,
                dropped_bytes: TS_PACKET_SIZE as u64,
                packet_index: 10,
                truncated_tail: false,
            },
        );
    }

    #[test]
    fn finish_emits_a_stream_shorter_than_the_lock_window() {
        let packets: Vec<_> = (1u8..=4).map(ts_packet).collect();
        let mut r = TsResync::new();
        assert!(r.feed(&concat_ts(&packets)).is_empty());
        r.finish();
        assert_eq!(r.take_tail_packets(), packets);
        assert!(r.take_tail_packets().is_empty());
        assert_eq!(r.stride(), Some(PacketStride::Ts188));
        assert_eq!(r.stats().dropped_bytes, 0);
    }

    #[test]
    fn finish_emits_a_short_rs204_stream_stripped() {
        let mut stream = Vec::new();
        for tag in 1u8..=3 {
            stream.extend_from_slice(&rs_packet(tag, 0xFF));
        }
        let mut r = TsResync::new();
        assert!(r.feed(&stream).is_empty());
        r.finish();
        let packets = r.take_tail_packets();
        assert_eq!(packets, [ts_packet(1), ts_packet(2), ts_packet(3)]);
        assert_eq!(r.stride(), Some(PacketStride::Rs204));
    }

    #[test]
    fn finish_does_not_lock_onto_a_stray_sync_byte_without_a_whole_packet() {
        let mut junk = vec![0x00u8; 100];
        junk[40] = TS_SYNC_BYTE;
        let mut r = TsResync::new();
        assert!(r.feed(&junk).is_empty());
        r.finish();
        assert!(r.take_tail_packets().is_empty());
        assert_eq!(r.stats().dropped_bytes, 100);
    }

    #[test]
    fn finish_does_not_emit_a_trailing_partial_packet_starting_with_sync_byte() {
        let mut input = Vec::new();
        for tag in 1u8..=10 {
            input.extend_from_slice(&ts_packet(tag));
        }
        input.push(TS_SYNC_BYTE);
        input.extend_from_slice(&[0x11, 0x22, 0x33, 0x44]);

        let mut r = TsResync::new();
        let mut packets = r.feed(&input);
        r.finish();
        packets.extend(r.take_tail_packets());
        let tags: Vec<u8> = packets.iter().map(|p| p[1]).collect();
        assert_eq!(tags, (1u8..=10).collect::<Vec<_>>());
        assert!(r.take_tail_packets().is_empty());
        let losses = r.take_losses();
        assert_one_loss_plus_leading(
            &losses,
            0,
            ResyncLoss {
                offset: 10 * TS_PACKET_SIZE as u64,
                dropped_bytes: 5,
                packet_index: 10,
                truncated_tail: true,
            },
        );
    }
}
