//! Block cipher — 56-round substitution/permutation network on 8-byte blocks.
//!
//! The DVB-CSA2 block cipher operates on 64-bit blocks using:
//! - `SBOX[256]` — the substitution box
//! - `PERM[256]` — the permutation table
//! - `sch[56]` — round keys (derived from the control word via `key::expand_block`)
use super::tables::{PERM, SBOX};

/// The DVB-CSA2 block cipher, initialized with 56 round-key bytes.
pub(crate) struct BlockCipher {
    sch: [u8; 56],
}

impl BlockCipher {
    /// Create a new block cipher with the given round keys.
    pub(crate) fn new(sch: [u8; 56]) -> Self {
        Self { sch }
    }

    /// Encrypt one 8-byte block in-place (rounds 0..55 forward).
    pub(crate) fn encrypt_block(&self, w: &mut [u8]) {
        debug_assert_eq!(w.len(), 8);
        let mut b: [u8; 8] = w.try_into().unwrap();
        for round in 0..56 {
            let s = SBOX[(self.sch[round] ^ b[7]) as usize];
            let save = b[1];
            b[1] = b[2] ^ b[0];
            b[2] = b[3] ^ b[0];
            b[3] = b[4] ^ b[0];
            b[4] = b[5];
            b[5] = b[6] ^ PERM[s as usize];
            b[6] = b[7];
            b[7] = b[0] ^ s;
            b[0] = save;
        }
        w.copy_from_slice(&b);
    }

    /// Decrypt one 8-byte block in-place (rounds 55..0 reverse).
    pub(crate) fn decrypt_block(&self, w: &mut [u8]) {
        debug_assert_eq!(w.len(), 8);
        let mut b: [u8; 8] = w.try_into().unwrap();
        for round in (0..56).rev() {
            let s = SBOX[(self.sch[round] ^ b[6]) as usize];
            let out = b[7] ^ s;
            b[7] = b[6];
            b[6] = b[5] ^ PERM[s as usize];
            b[5] = b[4];
            b[4] = b[3] ^ out;
            b[3] = b[2] ^ out;
            b[2] = b[1] ^ out;
            b[1] = b[0];
            b[0] = out;
        }
        w.copy_from_slice(&b);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::key::ControlWord;

    #[test]
    fn encrypt_decrypt_roundtrip() {
        let cw = ControlWord::from_bytes([0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08]);
        let sch = cw.expand_block();
        let bc = BlockCipher::new(sch);

        let mut block = [0x00u8, 0x08, 0x10, 0x18, 0x20, 0x28, 0x30, 0x38];
        let plaintext = block;
        bc.encrypt_block(&mut block);
        assert_ne!(block, plaintext);
        bc.decrypt_block(&mut block);
        assert_eq!(block, plaintext);
    }

    #[test]
    fn all_golden_cws_block_cipher() {
        // Test every CW from the golden vectors by ECB-encrypting a known block
        // and checking that encrypt(decrypt(x)) == x
        let all_cws: [([u8; 8], &str); 18] = [
            ([0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08], "vec1"),
            ([0x71, 0x47, 0x1d, 0x94, 0xec, 0x89, 0x93, 0xc7], "vec2"),
            ([0x54, 0xe9, 0xd2, 0x73, 0x2d, 0xb7, 0x4d, 0xad], "vec3"),
            ([0xd1, 0x82, 0x18, 0x08, 0xe4, 0x87, 0xd2, 0xd5], "vec4"),
            ([0x27, 0x52, 0x2d, 0x91, 0x50, 0x3a, 0x63, 0x7d], "vec5"),
            ([0x97, 0x99, 0x51, 0x4f, 0xb2, 0x10, 0x40, 0xe5], "vec6"),
            ([0x61, 0x96, 0xc6, 0x81, 0x4a, 0x48, 0xa8, 0x4f], "vec7"),
            ([0xc5, 0x8a, 0xca, 0x69, 0x57, 0x24, 0xdc, 0xf9], "vec8"),
            ([0x02, 0xb5, 0x9e, 0x45, 0x1b, 0xe2, 0x1c, 0x24], "vec9"),
            ([0x5a, 0x56, 0x82, 0x55, 0xd4, 0xc2, 0xa8, 0x10], "vec10"),
            ([0x0b, 0xae, 0xb5, 0xdb, 0xc2, 0x06, 0xbf, 0xfc], "vec11"),
            ([0x55, 0xfd, 0x78, 0x15, 0x27, 0xec, 0xa2, 0x29], "vec12"),
            ([0xf0, 0x70, 0x94, 0xf2, 0xca, 0x22, 0x74, 0x32], "vec13"),
            ([0x1e, 0x27, 0x9a, 0xdd, 0xca, 0x1c, 0xf5, 0x32], "vec14"),
            ([0xc6, 0xe6, 0x2a, 0x81, 0xff, 0xd6, 0x18, 0xea], "vec15"),
            ([0xe7, 0x55, 0x82, 0x8c, 0xe2, 0x8f, 0xee, 0xf9], "vec16"),
            ([0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00], "vec17"),
            ([0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff], "vec18"),
        ];

        let block = [0x00u8, 0x08, 0x10, 0x18, 0x20, 0x28, 0x30, 0x38];
        for (cw_bytes, name) in all_cws {
            let cw = ControlWord::from_bytes(cw_bytes);
            let sch = cw.expand_block();
            let bc = BlockCipher::new(sch);
            let mut b = block;
            bc.encrypt_block(&mut b);
            // Just check roundtrip
            bc.decrypt_block(&mut b);
            assert_eq!(b, block, "{name}: encrypt+decrypt roundtrip failed");
        }
    }

    #[test]
    fn vector_12_8byte() {
        // Vector 12: CW 55fd781527eca229, plain = f1ee4b395007d425
        // Expected scrambled: 69d17832b1734b58
        let cw = ControlWord::from_bytes([0x55, 0xfd, 0x78, 0x15, 0x27, 0xec, 0xa2, 0x29]);
        let sch = cw.expand_block();
        let bc = BlockCipher::new(sch);

        let mut block = [0xf1, 0xee, 0x4b, 0x39, 0x50, 0x07, 0xd4, 0x25];
        bc.encrypt_block(&mut block);
        assert_eq!(
            block,
            [0x69, 0xd1, 0x78, 0x32, 0xb1, 0x73, 0x4b, 0x58],
            "Vector 12 block encrypt mismatch"
        );

        // Roundtrip
        bc.decrypt_block(&mut block);
        assert_eq!(
            block,
            [0xf1, 0xee, 0x4b, 0x39, 0x50, 0x07, 0xd4, 0x25],
            "Vector 12 block decrypt mismatch"
        );
    }
}
