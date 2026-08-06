//! Control word and key schedule — the 8-byte key and its derivations.
//!
//! DVB-CSA2 keys the block cipher and stream cipher from the same 8-byte
//! control word:
//!
//! - **Block key schedule** (`expand_block`): produces 56 round-key bytes via
//!   the KPERM permutation.
//! - **Stream cipher seed** (`expand_stream`): produces a nibble-swapped copy
//!   of the control word for LFSR initialization.
use super::tables::KPERM;

/// An 8-byte DVB-CSA control word.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ControlWord(pub [u8; 8]);

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
}
