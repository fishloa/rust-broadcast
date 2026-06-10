//! TS Byte-Stream Resynchroniser — ISO/IEC 13818-1 §2.4.3.2.
//!
//! Recovers 188-byte MPEG-TS packet alignment from an arbitrary byte stream
//! (file reads, UDP payloads) that may start mid-packet or contain junk.
//! Also detects 204-byte packets (188-byte TS + 16 Reed-Solomon parity)
//! and strips the parity bytes.

/// Sync byte value (ISO/IEC 13818-1 §2.4.3.2).
pub const SYNC_BYTE: u8 = 0x47;

/// Transport Stream packet size (ISO/IEC 13818-1 §2.4.3.2).
pub const TS_PACKET_SIZE: usize = 188;

/// TS packet with 16-byte Reed-Solomon outer code (DVB).
pub const RS_PACKET_SIZE: usize = 204;

/// Consecutive sync bytes at the candidate stride required to declare lock.
const LOCK_CONFIRMATIONS: usize = 5;

/// Detected packet size after locking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum PacketStride {
    /// Standard 188-byte TS packets.
    Ts188,
    /// 204-byte packets (188-byte TS + 16 Reed-Solomon parity bytes).
    Rs204,
}

/// Counters accumulated during resynchronisation.
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

/// TS byte-stream resynchroniser.
///
/// Recovers 188-byte MPEG-TS packet alignment from an arbitrary byte stream
/// that may start mid-packet or contain junk, and detects 204-byte
/// Reed-Solomon-coded packets (stripping the 16 parity bytes).
///
/// ISO/IEC 13818-1 §2.4.3.2.
#[derive(Debug, Default)]
pub struct TsResync {
    buf: Vec<u8>,
    stride: Option<PacketStride>,
    stats: ResyncStats,
}

impl TsResync {
    /// Create a new resynchroniser with an empty buffer.
    pub fn new() -> Self {
        Self::default()
    }

    /// Append `data`, emit every newly-aligned 188-byte TS packet.
    ///
    /// When the stream is detected as 204-byte, the 16 Reed-Solomon parity
    /// bytes are stripped and only the 188-byte TS payload is emitted.
    /// Bytes are buffered across calls.
    pub fn feed(&mut self, data: &[u8]) -> Vec<[u8; TS_PACKET_SIZE]> {
        self.buf.extend_from_slice(data);
        let mut emitted = Vec::new();

        loop {
            match self.stride {
                None => {
                    if let Some((offset, s)) = find_sync(&self.buf) {
                        self.stats.dropped_bytes += offset as u64;
                        self.buf.drain(..offset);
                        self.stride = Some(s);
                    } else {
                        let keep = LOCK_CONFIRMATIONS * RS_PACKET_SIZE;
                        if self.buf.len() > keep {
                            let excess = self.buf.len() - keep;
                            self.stats.dropped_bytes += excess as u64;
                            self.buf.drain(..excess);
                        }
                        return emitted;
                    }
                }
                Some(stride) => {
                    let s = match stride {
                        PacketStride::Ts188 => TS_PACKET_SIZE,
                        PacketStride::Rs204 => RS_PACKET_SIZE,
                    };
                    if self.buf.len() < s {
                        return emitted;
                    }
                    if self.buf[0] == SYNC_BYTE {
                        let mut packet = [0u8; TS_PACKET_SIZE];
                        packet.copy_from_slice(&self.buf[..TS_PACKET_SIZE]);
                        emitted.push(packet);
                        self.buf.drain(..s);
                        self.stats.packets += 1;
                    } else {
                        self.stats.resyncs += 1;
                        self.stats.dropped_bytes += 1;
                        self.buf.drain(..1);
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
}

/// Scan the buffer for the smallest offset `o` such that a stride S
/// (tried 188 first, then 204) yields [`LOCK_CONFIRMATIONS`] consecutive
/// sync bytes at positions `o + k*S` for `k = 0 .. LOCK_CONFIRMATIONS`.
///
/// Returns `(offset, stride)` on success, or [`None`] if no lock is found.
fn find_sync(buf: &[u8]) -> Option<(usize, PacketStride)> {
    for o in 0..buf.len() {
        if buf[o] != SYNC_BYTE {
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

/// Check that `LOCK_CONFIRMATIONS` consecutive sync bytes exist at stride `s`
/// starting from `offset` (inclusive).  The first byte (`buf[offset]`) is
/// already known to be [`SYNC_BYTE`]; this checks the next
/// `LOCK_CONFIRMATIONS - 1` positions.
fn try_stride(buf: &[u8], offset: usize, s: usize) -> bool {
    for k in 1..LOCK_CONFIRMATIONS {
        let pos = offset + k * s;
        if pos >= buf.len() || buf[pos] != SYNC_BYTE {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a 188-byte TS packet starting with 0x47 followed by a distinct
    /// payload byte `tag` (repeated).  All non-sync bytes are kept away from
    /// 0x47 so that no false lock can occur.
    fn ts_packet(tag: u8) -> [u8; TS_PACKET_SIZE] {
        assert_ne!(tag, SYNC_BYTE, "tag must not equal sync byte");
        let mut pkt = [tag; TS_PACKET_SIZE];
        pkt[0] = SYNC_BYTE;
        pkt
    }

    /// Build a 204-byte RS-coded packet: 188-byte TS payload (with tag) +
    /// 16 parity bytes filled with `parity` (must not be 0x47).
    fn rs_packet(ts_tag: u8, parity: u8) -> [u8; RS_PACKET_SIZE] {
        assert_ne!(ts_tag, SYNC_BYTE);
        assert_ne!(parity, SYNC_BYTE);
        let mut pkt = [parity; RS_PACKET_SIZE];
        pkt[0] = SYNC_BYTE;
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

    /// Feed `data` into a fresh [`TsResync`] and return everything emitted.
    fn feed_once(data: &[u8]) -> Vec<[u8; TS_PACKET_SIZE]> {
        TsResync::new().feed(data)
    }

    // ------------------------------------------------------------------
    // Test 1 — aligned 188-byte passthrough
    // ------------------------------------------------------------------
    #[test]
    fn aligned_188_passthrough() {
        let p0 = ts_packet(0x01);
        let p1 = ts_packet(0x02);
        let p2 = ts_packet(0x03);
        let p3 = ts_packet(0x04);
        let p4 = ts_packet(0x05);
        let data = concat_ts(&[p0, p1, p2, p3, p4]);

        let mut r = TsResync::new();
        let emitted = r.feed(&data);

        assert_eq!(emitted.len(), 5);
        assert_eq!(emitted[0], p0);
        assert_eq!(emitted[1], p1);
        assert_eq!(emitted[2], p2);
        assert_eq!(emitted[3], p3);
        assert_eq!(emitted[4], p4);
        assert_eq!(r.stride(), Some(PacketStride::Ts188));
        let s = r.stats();
        assert_eq!(s.packets, 5);
        assert_eq!(s.resyncs, 0);
        assert_eq!(s.dropped_bytes, 0);
    }

    // ------------------------------------------------------------------
    // Test 2 — junk prefix, then lock and recover all packets
    // ------------------------------------------------------------------
    #[test]
    fn junk_prefix_locks() {
        let pkts: Vec<_> = (0..6).map(|i| ts_packet(i + 1)).collect();
        let stream = concat_ts(&pkts);

        let junk: Vec<u8> = vec![0x00; 7];
        let mut data = junk.clone();
        data.extend_from_slice(&stream);

        let mut r = TsResync::new();
        let emitted = r.feed(&data);

        assert_eq!(emitted.len(), 6);
        for (i, p) in emitted.iter().enumerate() {
            assert_eq!(*p, pkts[i], "packet {i} mismatch");
        }
        assert_eq!(r.stride(), Some(PacketStride::Ts188));
        let s = r.stats();
        assert_eq!(s.packets, 6);
        assert_eq!(s.resyncs, 0);
        assert_eq!(s.dropped_bytes, 7);
    }

    // ------------------------------------------------------------------
    // Test 3 — chunked feed equivalence
    // ------------------------------------------------------------------
    #[test]
    fn chunked_feed_equivalence() {
        let pkts: Vec<_> = (0..6).map(|i| ts_packet(i + 1)).collect();
        let stream = concat_ts(&pkts);

        // Whole feed
        let whole = feed_once(&stream);

        // Chunked feed — 100 bytes at a time
        let mut r = TsResync::new();
        let mut chunked = Vec::new();
        for chunk in stream.chunks(100) {
            chunked.extend(r.feed(chunk));
        }

        assert_eq!(whole.len(), chunked.len());
        for (i, (w, c)) in whole.iter().zip(chunked.iter()).enumerate() {
            assert_eq!(w, c, "packet {i} mismatch");
        }
    }

    // ------------------------------------------------------------------
    // Test 4 — mid-stream sync loss and reacquisition
    // ------------------------------------------------------------------
    #[test]
    fn midstream_loss_resync() {
        // Need >= LOCK_CONFIRMATIONS clean packets BEFORE the corruption (so
        // the stream actually locks first, making the disruption a genuine
        // resync rather than a delayed initial lock) and >= LOCK_CONFIRMATIONS
        // after (so it can re-lock and recover). 14 packets, corruption inside
        // packet 7, satisfies both.
        let pkts: Vec<_> = (0..14).map(|i| ts_packet(i + 1)).collect();
        let clean = concat_ts(&pkts);

        // Insert a single stray byte 12 bytes into packet 7.
        let insert_at = 7 * TS_PACKET_SIZE + 12;
        let stray: u8 = 0x00;
        let mut data = clean[..insert_at].to_vec();
        data.push(stray);
        data.extend_from_slice(&clean[insert_at..]);

        let mut r = TsResync::new();
        let emitted = r.feed(&data);

        let s = r.stats();
        // A locked stream losing alignment mid-way must record a resync.
        assert!(
            s.resyncs >= 1,
            "mid-stream corruption must trigger a resync, got {}",
            s.resyncs
        );
        // Locks on the clean prefix: the first emitted packet is P0.
        assert_eq!(emitted[0], pkts[0], "first emitted packet is P0");
        // Re-locks after the corruption and recovers most of the stream.
        assert!(
            emitted.len() >= 10,
            "should recover and emit most packets, got {}",
            emitted.len()
        );
    }

    // ------------------------------------------------------------------
    // Test 5 — 204-byte RS-coded packets detected and stripped
    // ------------------------------------------------------------------
    #[test]
    fn rs204_detected_and_stripped() {
        let mut stream = Vec::new();
        let mut expected_payloads = Vec::new();
        for i in 0u8..6 {
            let tag = i + 1;
            let rs = rs_packet(tag, 0xFF);
            stream.extend_from_slice(&rs);
            expected_payloads.push(ts_packet(tag));
        }

        let mut r = TsResync::new();
        let emitted = r.feed(&stream);

        assert_eq!(emitted.len(), 6);
        assert_eq!(r.stride(), Some(PacketStride::Rs204));
        for (i, (e, p)) in emitted.iter().zip(expected_payloads.iter()).enumerate() {
            assert_eq!(e, p, "packet {i} mismatch");
        }
        let s = r.stats();
        assert_eq!(s.packets, 6);
        assert_eq!(s.resyncs, 0);
        assert_eq!(s.dropped_bytes, 0);
    }
}
