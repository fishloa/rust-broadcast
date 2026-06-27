//! Stateful TS byte-stream resynchroniser — ITU-T H.222.0 §2.4.3.2 (= ISO/IEC 13818-1 §2.4.3.2).
//!
//! Recovers 188-byte MPEG-TS packet alignment from an arbitrary byte stream
//! (file reads, UDP payloads) that may start mid-packet or contain leading
//! garbage.  Also detects 204-byte Reed-Solomon-coded packets (DVB RS-coded
//! outer forward-error-correction layer) and strips the 16 parity bytes,
//! yielding standard 188-byte TS packets in both cases.
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
}

impl PacketStride {
    /// Short machine-readable label for this stride.
    pub fn name(&self) -> &'static str {
        match self {
            PacketStride::Ts188 => "ts_188",
            PacketStride::Rs204 => "rs_204",
        }
    }
}

dvb_common::impl_spec_display!(PacketStride);

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

/// Stateful TS byte-stream resynchroniser (ISO/IEC 13818-1 §2.4.3.2).
///
/// Recovers 188-byte MPEG-TS packet alignment from an arbitrary byte stream
/// that may start mid-packet or contain junk, and detects 204-byte
/// Reed-Solomon-coded packets (stripping the 16 parity bytes).
///
/// Feed raw bytes with [`feed`](Self::feed); each call returns a `Vec` of
/// aligned 188-byte TS packets.  Bytes are buffered across calls so that
/// packet boundaries may span call boundaries freely.
///
/// Lock is declared after [`LOCK_CONFIRMATIONS`] consecutive sync bytes are
/// found at the candidate stride (188 or 204).  On sync loss the resynchroniser
/// re-scans from the byte after the lost position.
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
}

impl TsResync {
    /// Create a new resynchroniser with an empty internal buffer.
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed `data` and emit every newly-aligned 188-byte TS packet.
    ///
    /// For a 204-byte stream the 16 Reed-Solomon parity bytes are stripped;
    /// only the 188-byte TS payload is returned.  Bytes that cannot yet form
    /// a complete packet (or fall before lock) are buffered for the next call.
    pub fn feed(&mut self, data: &[u8]) -> Vec<[u8; TS_PACKET_SIZE]> {
        self.buf.extend_from_slice(data);
        let mut emitted = Vec::new();

        loop {
            match self.stride {
                None => {
                    if let Some((offset, s)) = find_sync(&self.buf[self.head..]) {
                        self.stats.dropped_bytes += offset as u64;
                        self.head += offset;
                        self.stride = Some(s);
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
                        return emitted;
                    }
                }
                Some(stride) => {
                    let s = match stride {
                        PacketStride::Ts188 => TS_PACKET_SIZE,
                        PacketStride::Rs204 => RS_PACKET_SIZE,
                    };
                    let tail_len = self.buf.len() - self.head;
                    if tail_len < s {
                        self.compact();
                        return emitted;
                    }
                    if self.buf[self.head] == TS_SYNC_BYTE {
                        let mut packet = [0u8; TS_PACKET_SIZE];
                        packet.copy_from_slice(&self.buf[self.head..self.head + TS_PACKET_SIZE]);
                        emitted.push(packet);
                        self.head += s;
                        self.stats.packets += 1;
                    } else {
                        // Sync byte missing — record the loss and re-scan.
                        self.stats.resyncs += 1;
                        self.stats.dropped_bytes += 1;
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

    /// Accumulated statistics.
    pub fn stats(&self) -> ResyncStats {
        self.stats
    }

    /// Compact the internal buffer by discarding consumed bytes.
    fn compact(&mut self) {
        if self.head > 0 {
            self.buf.drain(..self.head);
            self.head = 0;
        }
    }
}

/// Scan `buf` for the smallest offset `o` such that stride `S` (tried 188
/// first, then 204) yields [`LOCK_CONFIRMATIONS`] consecutive sync bytes at
/// positions `o + k*S` for `k = 0 .. LOCK_CONFIRMATIONS`.
///
/// Returns `(offset, stride)` on success, or `None` if no lock is found.
fn find_sync(buf: &[u8]) -> Option<(usize, PacketStride)> {
    for o in 0..buf.len() {
        if buf[o] != TS_SYNC_BYTE {
            continue;
        }
        if try_stride(buf, o, TS_PACKET_SIZE) {
            return Some((o, PacketStride::Ts188));
        }
        if try_stride(buf, o, RS_PACKET_SIZE) {
            return Some((o, PacketStride::Rs204));
        }
    }
    None
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
        let junk: Vec<u8> = vec![0x00; junk_len];
        let mut data = junk;
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
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/m6-single.ts");
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
}
