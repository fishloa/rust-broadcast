//! 188-byte TS packet alignment and resync over a byte buffer.
//!
//! The DVB transport stream is a sequence of 188-byte packets, each beginning
//! with the sync byte `0x47` (ISO/IEC 13818-1 §2.4.3.2). A raw byte stream
//! (e.g. from a UDP payload or a file) may be misaligned or contain leading
//! garbage; this module provides [`resync`] to locate the first `0x47` sync
//! byte and [`aligned_packets`] to yield aligned 188-byte slices from a buffer.

/// MPEG-TS packet size in bytes (ISO/IEC 13818-1 §2.4.3.2).
pub const TS_PACKET_SIZE: usize = 188;

/// MPEG-TS sync byte value.
pub const TS_SYNC_BYTE: u8 = 0x47;

/// Find the byte offset of the first `0x47` sync byte in `buf` that is a valid
/// packet boundary: `buf[offset] == 0x47` and (when a second packet header is
/// visible) `buf[offset + 188] == 0x47`.
///
/// If no confirmed boundary is found (e.g. the buffer is too small for a
/// two-packet confirmation), the offset of the first `0x47` alone is returned
/// as a best-effort alignment.
///
/// Returns `None` when `buf` contains no `0x47` byte at all.
#[must_use]
pub fn resync(buf: &[u8]) -> Option<usize> {
    let mut i = 0;
    while i < buf.len() {
        if buf[i] == TS_SYNC_BYTE {
            // Try to confirm with the next packet header.
            let next = i + TS_PACKET_SIZE;
            if next < buf.len() {
                if buf[next] == TS_SYNC_BYTE {
                    return Some(i);
                }
                // False sync: keep scanning.
                i += 1;
                continue;
            }
            // Buffer too small for two-packet confirmation; best-effort.
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Return an iterator over aligned 188-byte TS packet slices in `buf`.
///
/// Requires that `buf` is already aligned (i.e. `buf[0] == 0x47`). Each
/// yielded slice is exactly [`TS_PACKET_SIZE`] bytes. Trailing bytes that do
/// not form a complete packet are ignored.
pub fn aligned_packets(buf: &[u8]) -> impl Iterator<Item = &[u8]> {
    buf.chunks_exact(TS_PACKET_SIZE)
        .filter(|pkt| pkt[0] == TS_SYNC_BYTE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resync_finds_first_confirmed_sync() {
        // Two garbage bytes, then two back-to-back 0x47-aligned packets.
        let mut buf = vec![0x00u8, 0x01];
        let pkt_a = [TS_SYNC_BYTE; TS_PACKET_SIZE];
        let pkt_b = [TS_SYNC_BYTE; TS_PACKET_SIZE];
        buf.extend_from_slice(&pkt_a);
        buf.extend_from_slice(&pkt_b);
        assert_eq!(resync(&buf), Some(2));
    }

    #[test]
    fn resync_skips_false_sync_byte() {
        // A 0x47 at offset 0 but no confirming 0x47 at offset 188.
        // Build: [0x47, <187 non-sync bytes>, <real 0x47 packet>, <real 0x47 packet>].
        // buf[0] = 0x47 but buf[188] = 0x00 (not a sync), so offset 0 is a false match.
        // The real boundary is at offset 188 (buf[188]=0x47, buf[376]=0x47).
        let mut buf = vec![TS_SYNC_BYTE]; // false 0x47 at 0
        buf.extend(std::iter::repeat(0x00u8).take(TS_PACKET_SIZE - 1)); // buf[1..188] = 0x00
                                                                        // buf[188]: start of real packet pair
        buf.extend_from_slice(&[TS_SYNC_BYTE; TS_PACKET_SIZE]); // buf[188..376]
        buf.extend_from_slice(&[TS_SYNC_BYTE; TS_PACKET_SIZE]); // buf[376..564]
                                                                // buf[0]=0x47, buf[188]=0x47 → wait, that's actually a valid double-confirm!
                                                                // We need buf[188] != 0x47 for offset 0 to be skipped.
                                                                // Rebuild: offset 0 = 0x47, offsets 1-188 non-sync, offset 189 = 0x47, offset 377 = 0x47.
        let mut buf2 = vec![TS_SYNC_BYTE]; // false sync at 0
        buf2.extend(std::iter::repeat(0x00u8).take(TS_PACKET_SIZE)); // buf2[1..189] = 0x00, so buf2[188]=0x00
                                                                     // Real pair at offset 189.
        buf2.extend_from_slice(&[TS_SYNC_BYTE; TS_PACKET_SIZE]); // buf2[189..377]
        buf2.extend_from_slice(&[TS_SYNC_BYTE; TS_PACKET_SIZE]); // buf2[377..565]
        let off = resync(&buf2).unwrap();
        // buf2[0]=0x47, buf2[188]=0x00 → false; next 0x47 is at 189.
        // buf2[189]=0x47, buf2[189+188=377]=0x47 → confirmed.
        assert_eq!(off, 189);
    }

    #[test]
    fn resync_none_when_no_sync() {
        let buf = vec![0x00u8; 400];
        assert_eq!(resync(&buf), None);
    }

    #[test]
    fn aligned_packets_yields_full_packets() {
        let data: Vec<u8> = (0..3)
            .flat_map(|_| std::iter::repeat(TS_SYNC_BYTE).take(TS_PACKET_SIZE))
            .collect();
        let pkts: Vec<_> = aligned_packets(&data).collect();
        assert_eq!(pkts.len(), 3);
        assert!(pkts.iter().all(|p| p.len() == TS_PACKET_SIZE));
    }

    #[test]
    fn aligned_packets_drops_trailing_partial() {
        // 2 full packets + 10 extra bytes.
        let mut data: Vec<u8> = vec![TS_SYNC_BYTE; TS_PACKET_SIZE * 2];
        data.extend_from_slice(&[TS_SYNC_BYTE; 10]);
        let pkts: Vec<_> = aligned_packets(&data).collect();
        assert_eq!(pkts.len(), 2);
    }
}
