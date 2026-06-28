//! TS byte-stream resynchronisation helpers for `dvb-stream`.
//!
//! Thin wrappers over [`mpeg_ts::resync::TsResync`] — the shared stateful
//! resynchroniser lives in `mpeg-ts` (feature `ts`).  This module keeps
//! the public surface that `section_stream` and `t2mi_stream` use (the
//! stateless `resync` + `aligned_packets` pair and the size/sync constants).

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
