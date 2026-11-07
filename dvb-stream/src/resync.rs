//! TS byte-stream resynchronisation helpers for `dvb-stream`.
//!
//! The stateless 188-byte alignment scan `TsDecoder` uses
//! per datagram, plus the size/sync constants. It is deliberately not
//! [`mpeg_ts::resync::TsResync`]: that resynchroniser needs
//! `LOCK_CONFIRMATIONS` consecutive packets (and also locks 204-byte
//! strides), so a short UDP datagram holding one or two packets would never
//! lock, while this scan accepts a lone sync byte when the buffer is too
//! small to confirm.

pub use mpeg_ts::ts::{TS_PACKET_SIZE, TS_SYNC_BYTE};

/// Find the byte offset of the first confirmed 0x47 sync byte in `buf`.
///
/// "Confirmed" means either `buf[offset + 188] == 0x47` (two-packet
/// confirmation), or the buffer is too small for two packets (best-effort
/// single-sync-byte alignment).
///
/// Returns `None` when `buf` contains no `0x47` byte at all.
///
/// This is a stateless best-effort helper.  For a full stateful resynchroniser
/// with 204-byte detection and per-call carry-over buffering, use
/// [`mpeg_ts::resync::TsResync`].
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
/// Requires that `buf` is already aligned (i.e. `buf[0] == 0x47`).  Each
/// yielded slice is exactly [`TS_PACKET_SIZE`] bytes.  Trailing bytes that do
/// not form a complete packet are ignored.
pub fn aligned_packets(buf: &[u8]) -> impl Iterator<Item = &[u8]> {
    buf.chunks_exact(TS_PACKET_SIZE)
        .filter(|pkt| pkt[0] == TS_SYNC_BYTE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aligned_packets_yields_whole_sync_prefixed_packets_only() {
        let mut buf = vec![0u8; TS_PACKET_SIZE * 2 + 10];
        buf[0] = TS_SYNC_BYTE;
        buf[TS_PACKET_SIZE] = TS_SYNC_BYTE;
        let pkts: Vec<&[u8]> = aligned_packets(&buf).collect();
        assert_eq!(pkts.len(), 2);
        assert!(pkts.iter().all(|p| p.len() == TS_PACKET_SIZE));
        buf[TS_PACKET_SIZE] = 0x00;
        assert_eq!(aligned_packets(&buf).count(), 1);
    }
}
