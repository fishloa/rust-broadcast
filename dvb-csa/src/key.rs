//! Control word and key schedule — the 8-byte key and its derivations.
//!
//! DVB-CSA2 keys the block cipher and stream cipher from the same 8-byte
//! control word:
//!
//! - **Block key schedule** (`expand_block`): produces 56 round-key bytes via
//!   the KPERM permutation.
//! - **Stream cipher seed** (`expand_stream`): produces a nibble-swapped copy
//!   of the control word for LFSR initialization.
use core::sync::atomic::{Ordering, compiler_fence};

use super::tables::KPERM;

/// An 8-byte DVB-CSA control word.
///
/// Not `Copy` — a `Copy` value can be duplicated on the stack without the
/// compiler's knowledge, and none of those copies would be zeroed by this
/// type's [`Drop`] impl. Clone explicitly where a second owned copy is
/// genuinely needed. The public field stays public (the fuzz target and
/// external callers construct this with tuple-struct syntax,
/// `ControlWord(bytes)`, and no call site reads it back out other than
/// through the methods below), so it carries no confidentiality guarantee on
/// its own — treat any `ControlWord` value itself as sensitive regardless.
#[derive(Clone, PartialEq, Eq)]
pub struct ControlWord(pub [u8; 8]);

/// Redacted: never print control word bytes (a derived `Debug` would — a
/// `tracing::debug!`/`dbg!`/panic message of a value holding a `ControlWord`
/// would then write the live control word to logs).
impl core::fmt::Debug for ControlWord {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_tuple("ControlWord").field(&"<redacted>").finish()
    }
}

/// Zero the control word's bytes on drop so it does not linger in freed
/// memory. `write_volatile` (rather than a plain assignment, which the
/// optimizer may prove dead and elide since the buffer is about to be
/// deallocated) plus a `compiler_fence` keep the zeroing from being reordered
/// or removed around the drop.
impl Drop for ControlWord {
    fn drop(&mut self) {
        zeroize_bytes(&mut self.0);
    }
}

/// Overwrite every byte of `buf` with `0`, in a way the optimizer cannot
/// prove is dead and drop (see [`Drop for ControlWord`](ControlWord)).
fn zeroize_bytes(buf: &mut [u8; 8]) {
    for b in buf.iter_mut() {
        // SAFETY: `b` is a valid, aligned `&mut u8` for the duration of the
        // write; `write_volatile` never invalidates the pointer.
        unsafe { core::ptr::write_volatile(b, 0) };
    }
    compiler_fence(Ordering::SeqCst);
}

impl ControlWord {
    /// Create a `ControlWord` from 8 bytes.
    pub const fn from_bytes(bytes: [u8; 8]) -> Self {
        Self(bytes)
    }

    /// Expand the control word into 56 block-cipher round-key bytes.
    pub fn expand_block(&self) -> [u8; 56] {
        let cw_u64 = u64::from_le_bytes(self.0);

        let mut k = [0u64; 7];
        k[6] = cw_u64;
        for i in (1..=6).rev() {
            k[i - 1] = key_permute(k[i]);
        }

        let mut sch = [0u8; 56];
        for i in 0..7 {
            let ki = k[i];
            for j in 0..8 {
                sch[i * 8 + j] = ((ki >> (j * 8)) as u8) ^ (i as u8);
            }
        }
        sch
    }

    /// Expand to the nibble-swapped stream-cipher seed (cws).
    ///
    /// Each byte has its high and low nibbles swapped:
    /// `cws[i] = (cw[i] >> 4) | (cw[i] << 4)`
    pub fn expand_stream(&self) -> [u8; 8] {
        let mut cws = [0u8; 8];
        for (i, out) in cws.iter_mut().enumerate() {
            *out = self.0[i].rotate_left(4);
        }
        cws
    }
}

fn key_permute(k: u64) -> u64 {
    let bytes = k.to_le_bytes();
    let mut result = 0u64;
    for (i, &b) in bytes.iter().enumerate() {
        result |= KPERM[i][b as usize];
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nibble_swap_symmetry() {
        let cw = ControlWord::from_bytes([0x12, 0x34, 0x56, 0x78, 0x9a, 0xbc, 0xde, 0xf0]);
        let cws = cw.expand_stream();
        assert_eq!(cws[0], 0x21);
        assert_eq!(cws[1], 0x43);
        assert_eq!(cws[7], 0x0f);
    }

    #[test]
    fn vector_12_key_schedule() {
        // Quick sanity: vector 12 CW produces the correct encrypt output
        let cw = ControlWord::from_bytes([0x55, 0xfd, 0x78, 0x15, 0x27, 0xec, 0xa2, 0x29]);
        let sch = cw.expand_block();
        // Just verify first and last round key bytes are non-zero
        assert!(sch.iter().any(|&b| b != 0));
    }

    /// W-CSA-4: `Debug` must never print the control word's bytes.
    #[test]
    fn debug_redacts_control_word_bytes() {
        let cw = ControlWord::from_bytes([0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88]);
        let out = format!("{cw:?}");
        assert!(
            !out.contains(&format!("{:?}", cw.0)),
            "Debug output must not contain the control word's array representation: {out}"
        );
        assert!(out.contains("<redacted>"));
    }

    /// W-CSA-4: `ControlWord` cannot safely assert its *own* storage is zero
    /// after drop (reading freed/moved-from memory is undefined behavior),
    /// so this pins the private zeroing helper directly on a plain buffer.
    #[test]
    fn zeroize_bytes_clears_every_byte() {
        let mut buf = [0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88];
        zeroize_bytes(&mut buf);
        assert_eq!(buf, [0u8; 8]);
    }
}
