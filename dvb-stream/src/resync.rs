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
