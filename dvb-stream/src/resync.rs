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

/// Find the byte offset of the first sync byte in `buf` that is confirmed by a
/// second sync byte exactly one packet later.
///
/// Unlike [`resync`] this never accepts a lone sync byte: a `0x47` with no
/// following packet in `buf` is "not yet known" and yields `None`, so a
/// byte-stream caller can wait for more input instead of locking onto a
/// payload byte that happens to equal the sync byte.
#[must_use]
pub fn resync_confirmed(buf: &[u8]) -> Option<usize> {
    (0..buf.len().saturating_sub(TS_PACKET_SIZE))
        .find(|&i| buf[i] == TS_SYNC_BYTE && buf[i + TS_PACKET_SIZE] == TS_SYNC_BYTE)
}

/// Offset of the first `0x47` in `buf` that has no room for a confirming sync
/// byte one packet later (so it may yet be confirmed by more input).
#[must_use]
pub fn unconfirmed_candidate(buf: &[u8]) -> Option<usize> {
    let from = buf.len().saturating_sub(TS_PACKET_SIZE);
    buf[from..]
        .iter()
        .position(|&b| b == TS_SYNC_BYTE)
        .map(|i| from + i)
}
