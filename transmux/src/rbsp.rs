//! RBSP emulation-prevention byte (EPB) handling.
//!
//! One implementation of the H.264 §7.4.1 / H.265 §7.4.2
//! `emulation_prevention_three_byte` rule, shared by every caller in this crate:
//!
//! - **decode** (`unescape`): drop every `03` that follows a `00 00` pair,
//!   including a trailing `00 00 03` at the end of the NAL.
//! - **encode** (`escape`): insert `03` before any byte `<= 0x03` that follows
//!   a `00 00` pair, and append `03` after a trailing `00 00`.

use alloc::vec::Vec;

/// Unescape an EBSP NAL into RBSP: remove every `00 00 03` → `00 00`.
pub(crate) fn unescape(escaped: &[u8]) -> Vec<u8> {
    let n = escaped.len();
    let mut out = Vec::with_capacity(n);
    let mut i = 0;
    while i < n {
        if i + 2 < n && escaped[i] == 0 && escaped[i + 1] == 0 && escaped[i + 2] == 3 {
            out.push(0);
            out.push(0);
            i += 3;
        } else {
            out.push(escaped[i]);
            i += 1;
        }
    }
    // trailing triplet where i+2 == n-1: handled above? i+2 < n for i=n-3 gives n-1 < n true.
    out
}

#[cfg(feature = "sample-aes")]
/// Escape RBSP into EBSP: insert `03` before `00..=03` after `00 00`, and after
/// a trailing `00 00`.
pub(crate) fn escape(unescaped: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(unescaped.len());
    let mut zero_run = 0usize;
    for &b in unescaped {
        if zero_run >= 2 && b <= 0x03 {
            out.push(0x03);
            zero_run = 0;
        }
        out.push(b);
        if b == 0 {
            zero_run += 1;
        } else {
            zero_run = 0;
        }
    }
    if zero_run >= 2 {
        out.push(0x03);
    }
    out
}
