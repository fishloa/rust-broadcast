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

/// `emulation_prevention_three_byte` (H.264 §7.4.1).
pub(crate) const EMULATION_PREVENTION_BYTE: u8 = 0x03;
/// Zero bytes that must precede an emulation prevention byte.
pub(crate) const EMULATION_BYTE_RUN_LEN: usize = 2;

/// Unescape an EBSP NAL into RBSP: remove every `00 00 03` → `00 00`.
pub(crate) fn unescape(escaped: &[u8]) -> Vec<u8> {
    let n = escaped.len();
    let mut out = Vec::with_capacity(n);
    let mut i = 0;
    while i < n {
        if i + EMULATION_BYTE_RUN_LEN < n
            && escaped[i] == 0
            && escaped[i + 1] == 0
            && escaped[i + EMULATION_BYTE_RUN_LEN] == EMULATION_PREVENTION_BYTE
        {
            out.push(0);
            out.push(0);
            i += EMULATION_BYTE_RUN_LEN + 1;
        } else {
            out.push(escaped[i]);
            i += 1;
        }
    }
    // trailing triplet where i+2 == n-1: handled above? i+2 < n for i=n-3 gives n-1 < n true.
    out
}

/// Escape RBSP into EBSP: insert `03` before `00..=03` after `00 00`, and after
/// a trailing `00 00`.
pub(crate) fn escape(unescaped: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(unescaped.len());
    let mut zero_run = 0usize;
    for &b in unescaped {
        if zero_run >= EMULATION_BYTE_RUN_LEN && b <= EMULATION_PREVENTION_BYTE {
            out.push(EMULATION_PREVENTION_BYTE);
            zero_run = 0;
        }
        out.push(b);
        if b == 0 {
            zero_run += 1;
        } else {
            zero_run = 0;
        }
    }
    if zero_run >= EMULATION_BYTE_RUN_LEN {
        out.push(EMULATION_PREVENTION_BYTE);
    }
    out
}
#[cfg(test)]
mod tests {
    use super::unescape;

    #[test]
    fn unescape_only_drops_a_three_byte_after_two_zero_bytes() {
        assert_eq!(
            unescape(&[0x00, 0x05, 0x03]).as_slice(),
            &[0x00, 0x05, 0x03]
        );
        assert_eq!(
            unescape(&[0x00, 0x00, 0x03, 0x01]).as_slice(),
            &[0x00, 0x00, 0x01]
        );
        assert_eq!(unescape(&[0x00, 0x00, 0x03]).as_slice(), &[0x00, 0x00]);
        assert_eq!(
            unescape(&[0x01, 0x00, 0x03]).as_slice(),
            &[0x01, 0x00, 0x03]
        );
    }
}
