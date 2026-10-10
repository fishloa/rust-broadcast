//! Zeroing for control-word–derived cipher state.
//!
//! Every type that holds state derived from a [`crate::key::ControlWord`]
//! (round-key schedules, LFSR registers, expanded stream seeds, ...) must
//! clear that state on drop rather than leaving it to linger in freed
//! memory. A plain assignment on drop is not enough: the optimizer is free
//! to prove the write dead and elide it, so every zeroing `Drop` impl in this
//! crate goes through [`zeroize`], which delegates to the vetted `zeroize`
//! crate (volatile writes + compiler fence) instead of a hand-rolled
//! `unsafe` implementation.
use ::zeroize::Zeroize;

/// Overwrite every element of `buf` with zero, in a way the optimizer cannot
/// prove is dead and drop.
pub(crate) fn zeroize<T: ::zeroize::DefaultIsZeroes>(buf: &mut [T]) {
    buf.zeroize();
}

/// Wraps a control-word–derived byte array for just long enough to hand it
/// to the type that owns the cipher state built from it (e.g.
/// [`crate::block::BlockCipher`]), zeroing *this* stack copy when it goes
/// out of scope. The receiving type separately zeroizes the copy it stores
/// on its own drop — this covers the local/argument copy that evaluating
/// `cw.expand_block()`/`cw.expand_stream()` leaves behind in the caller.
pub(crate) struct Zeroizing<T: AsMut<[u8]>>(pub(crate) T);

impl<T: AsMut<[u8]>> Drop for Zeroizing<T> {
    fn drop(&mut self) {
        zeroize(self.0.as_mut());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zeroize_clears_every_byte() {
        let mut buf = [0x11u8, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88];
        zeroize(&mut buf);
        assert_eq!(buf, [0u8; 8]);
    }

    #[test]
    fn zeroize_clears_every_word() {
        let mut buf = [0xdead_beefu32, 0xcafe_f00d, 1, 2];
        zeroize(&mut buf);
        assert_eq!(buf, [0u32; 4]);
    }

    /// `Zeroizing`'s storage cannot be read back through the wrapper after
    /// drop (that would be reading through a value whose destructor already
    /// ran), so this instead takes a raw pointer to the wrapped array before
    /// the wrapper drops and reads through *that* immediately after — the
    /// same "prove it zeroed" technique the `zeroize` crate's own tests use.
    /// `u8` has no validity invariant, so reading plain bytes back from
    /// still-live (not yet reused) stack storage is sound.
    #[test]
    fn zeroizing_wrapper_clears_its_storage_on_drop() {
        // Run only `Drop` while keeping the storage owned, so reading it afterwards is sound.
        let mut slot = core::mem::ManuallyDrop::new(Zeroizing([
            0x11u8, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88,
        ]));
        unsafe { core::ptr::drop_in_place(&mut *slot) };
        assert_eq!(slot.0, [0u8; 8]);
    }
}
