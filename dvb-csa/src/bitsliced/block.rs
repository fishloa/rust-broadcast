//! Bitsliced block cipher — the same 56-round SPN as [`crate::block`], with
//! every table replaced by a boolean circuit so one pass covers
//! [`LANES`](super::LANES) blocks.
//!
//! Only the S-box needs real gates. `PERM` is GF(2)-linear and maps each input
//! bit to exactly one output bit, so bitsliced it is free — a rewiring, done
//! here by [`permute`] writing `s[k]` to position `PERM_BIT[k]`. The round key
//! is the same for every lane, so `sch[round] ^ b[7]` is a complement of the
//! selected bit-words rather than a XOR against data.
use super::circuits::{PERM_BIT, block_sbox};
use super::{BITS_PER_BYTE, BLOCK_BITS, BLOCK_BYTES, Word};

/// Rounds in the DVB-CSA2 block cipher; one round key byte each.
const ROUNDS: usize = 56;

/// One byte of block state, sliced: `[bit][lane]`.
type ByteSlice = [Word; BITS_PER_BYTE];
/// A whole block, sliced: `[byte][bit][lane]`.
type BlockSlice = [ByteSlice; BLOCK_BYTES];

/// The bitsliced DVB-CSA2 block cipher, initialised with 56 round-key bytes.
pub(super) struct BitslicedBlock {
    sch: [u8; ROUNDS],
}

/// Apply the block cipher's `PERM` permutation to a sliced byte.
#[inline]
fn permute(s: &ByteSlice) -> ByteSlice {
    let mut p = [0 as Word; BITS_PER_BYTE];
    for (bit, word) in s.iter().enumerate() {
        p[PERM_BIT[bit]] = *word;
    }
    p
}

/// `SBOX[key ^ byte]`, sliced. `key` is a per-round constant shared by every
/// lane, so applying it is a complement of the bit-words it selects.
#[inline]
fn keyed_sbox(byte: &ByteSlice, key: u8) -> ByteSlice {
    let mut input = *byte;
    for (bit, word) in input.iter_mut().enumerate() {
        if (key >> bit) & 1 == 1 {
            *word = !*word;
        }
    }
    block_sbox(&input)
}

#[inline]
fn xor(a: &ByteSlice, b: &ByteSlice) -> ByteSlice {
    let mut o = [0 as Word; BITS_PER_BYTE];
    for (bit, word) in o.iter_mut().enumerate() {
        *word = a[bit] ^ b[bit];
    }
    o
}

/// Split a flat transposed matrix into per-byte, per-bit slices.
#[inline]
fn unpack(m: &[Word; BLOCK_BITS]) -> BlockSlice {
    let mut b = [[0 as Word; BITS_PER_BYTE]; BLOCK_BYTES];
    for (byte, slot) in b.iter_mut().enumerate() {
        for (bit, word) in slot.iter_mut().enumerate() {
            *word = m[byte * BITS_PER_BYTE + bit];
        }
    }
    b
}

/// Inverse of [`unpack`].
#[inline]
fn pack(b: &BlockSlice, m: &mut [Word; BLOCK_BITS]) {
    for (byte, slot) in b.iter().enumerate() {
        for (bit, word) in slot.iter().enumerate() {
            m[byte * BITS_PER_BYTE + bit] = *word;
        }
    }
}

impl BitslicedBlock {
    pub(super) fn new(sch: [u8; ROUNDS]) -> Self {
        Self { sch }
    }

    /// Encrypt [`LANES`](super::LANES) blocks in place, rounds 0..56 forward.
    ///
    /// `m` is the transposed form: `m[byte * 8 + bit]` holds that bit of that
    /// byte for every lane.
    pub(super) fn encrypt(&self, m: &mut [Word; BLOCK_BITS]) {
        let mut b = unpack(m);
        for round in 0..ROUNDS {
            let s = keyed_sbox(&b[7], self.sch[round]);
            let ps = permute(&s);
            // Every right-hand side below reads the pre-round value; b[0] and
            // b[1] are the only ones overwritten before their last use.
            let old0 = b[0];
            let old1 = b[1];
            b[1] = xor(&b[2], &old0);
            b[2] = xor(&b[3], &old0);
            b[3] = xor(&b[4], &old0);
            b[4] = b[5];
            b[5] = xor(&b[6], &ps);
            b[6] = b[7];
            b[7] = xor(&old0, &s);
            b[0] = old1;
        }
        pack(&b, m);
    }

    /// Decrypt [`LANES`](super::LANES) blocks in place, rounds 55..0 in reverse.
    pub(super) fn decrypt(&self, m: &mut [Word; BLOCK_BITS]) {
        let mut b = unpack(m);
        for round in (0..ROUNDS).rev() {
            let s = keyed_sbox(&b[6], self.sch[round]);
            let ps = permute(&s);
            let out = xor(&b[7], &s);
            b[7] = b[6];
            b[6] = xor(&b[5], &ps);
            b[5] = b[4];
            b[4] = xor(&b[3], &out);
            b[3] = xor(&b[2], &out);
            b[2] = xor(&b[1], &out);
            b[1] = b[0];
            b[0] = out;
        }
        pack(&b, m);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bitsliced::{LANES, transpose};
    use crate::block::BlockCipher;
    use crate::key::ControlWord;

    /// Every lane must reproduce the scalar block cipher exactly, and lanes
    /// must not leak into one another — so each lane gets a different block.
    #[test]
    fn matches_the_scalar_block_cipher_in_every_lane() {
        let cw = ControlWord::from_bytes([0x55, 0xfd, 0x78, 0x15, 0x27, 0xec, 0xa2, 0x29]);
        let sch = cw.expand_block();
        let scalar = BlockCipher::new(sch);
        let bs = BitslicedBlock::new(sch);

        let mut blocks = [[0u8; BLOCK_BYTES]; LANES];
        let mut seed: u64 = 0x0123_4567_89ab_cdef;
        for block in blocks.iter_mut() {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            *block = seed.to_le_bytes();
        }

        for encrypt in [true, false] {
            let mut m = [0u64; LANES];
            for (lane, block) in blocks.iter().enumerate() {
                m[lane] = u64::from_le_bytes(*block);
            }
            transpose(&mut m);
            if encrypt {
                bs.encrypt(&mut m);
            } else {
                bs.decrypt(&mut m);
            }
            transpose(&mut m);

            for (lane, block) in blocks.iter().enumerate() {
                let mut want = *block;
                if encrypt {
                    scalar.encrypt_block(&mut want);
                } else {
                    scalar.decrypt_block(&mut want);
                }
                assert_eq!(
                    m[lane].to_le_bytes(),
                    want,
                    "lane {lane} disagrees with the scalar path (encrypt={encrypt})"
                );
            }
        }
    }
}
