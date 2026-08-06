//! CSA combination — block cipher + stream cipher.
//!
//! DVB-CSA2 encrypt/decrypt consists of two phases applied in a specific order:
//!
//! **Encrypt** (scramble):
//! 1. Block-cipher CBC: encrypt the last block first, then work backward,
//!    XORing each plaintext block into the next before encrypting.
//! 2. Stream-cipher XOR: seed from the (now-encrypted) first block, XOR bytes 8..end.
//!
//! **Decrypt** (descramble):
//! 1. Stream-cipher XOR: seed from the (still-encrypted) first block, XOR bytes 8..end.
//! 2. Block-cipher CBC undo: decrypt first block, then work forward,
//!    XORing each decrypted block into the next before decrypting.
//!
//! Payloads shorter than 8 bytes are passed through unchanged.
use crate::block::BlockCipher;
use crate::key::ControlWord;
use crate::stream::StreamCipher;

/// Scramble (encrypt) `data` in-place with the given control word.
///
/// Payloads shorter than 8 bytes are not scrambled.
pub fn scramble(cw: &ControlWord, data: &mut [u8]) {
    let len = data.len();
    if len < 8 {
        return;
    }

    let sch = cw.expand_block();
    let cws = cw.expand_stream();
    let bc = BlockCipher::new(sch);

    let nblocks = len / 8;

    // Phase 1: Block cipher, reverse CBC
    // Encrypt the last block first (ECB)
    bc.encrypt_block(&mut data[(nblocks - 1) * 8..nblocks * 8]);

    // Work backward: XOR (already-encrypted) block i+1 INTO block i, then encrypt block i
    for i in (0..nblocks - 1).rev() {
        for j in 0..8 {
            data[i * 8 + j] ^= data[(i + 1) * 8 + j];
        }
        bc.encrypt_block(&mut data[i * 8..(i + 1) * 8]);
    }

    // Phase 2: Stream cipher XOR bytes 8..len
    let iv: [u8; 8] = data[0..8].try_into().unwrap();
    let mut sc = StreamCipher::new(&cws, &iv);
    sc.xor_stream(&mut data[8..]);
}

/// Descramble (decrypt) `data` in-place with the given control word.
///
/// Payloads shorter than 8 bytes are not descrambled.
pub fn descramble(cw: &ControlWord, data: &mut [u8]) {
    let len = data.len();
    if len < 8 {
        return;
    }

    let sch = cw.expand_block();
    let cws = cw.expand_stream();
    let bc = BlockCipher::new(sch);

    // Phase 1: Stream cipher XOR bytes 8..len (using encrypted first block as IV)
    let iv: [u8; 8] = data[0..8].try_into().unwrap();
    let mut sc = StreamCipher::new(&cws, &iv);
    sc.xor_stream(&mut data[8..]);

    // Phase 2: Block cipher, forward CBC undo
    let nblocks = len / 8;

    // Decrypt first block (ECB)
    bc.decrypt_block(&mut data[0..8]);

    // Work forward: XOR current (still encrypted) block INTO previous (decrypted), then decrypt current
    for i in 1..nblocks {
        for j in 0..8 {
            data[(i - 1) * 8 + j] ^= data[i * 8 + j];
        }
        bc.decrypt_block(&mut data[i * 8..(i + 1) * 8]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_payload_unchanged() {
        let cw = ControlWord::from_bytes([0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08]);
        let mut data = [0xaa, 0xbb, 0xcc, 0xdd];
        let orig = data;
        scramble(&cw, &mut data);
        assert_eq!(data, orig);
        descramble(&cw, &mut data);
        assert_eq!(data, orig);
    }

    #[test]
    fn roundtrip() {
        let cw = ControlWord::from_bytes([0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08]);
        // Use first 16 bytes from golden vector 1 plaintext
        let plaintext: [u8; 16] = [
            0x00, 0x08, 0x10, 0x18, 0x20, 0x28, 0x30, 0x38, 0x40, 0x48, 0x50, 0x58, 0x60, 0x5b,
            0x63, 0x6b,
        ];
        let mut data = plaintext;
        scramble(&cw, &mut data);
        assert_ne!(data, plaintext);
        descramble(&cw, &mut data);
        assert_eq!(data, plaintext);
    }

    #[test]
    fn vector_13_16byte() {
        let cw = ControlWord::from_bytes([0xf0, 0x70, 0x94, 0xf2, 0xca, 0x22, 0x74, 0x32]);
        let plaintext: [u8; 16] = [
            0x1e, 0x25, 0x49, 0x31, 0x30, 0x96, 0xf4, 0xe4, 0xc2, 0x79, 0x3c, 0x92, 0x54, 0x44,
            0x66, 0x40,
        ];
        let expected: [u8; 16] = [
            0x3d, 0xf6, 0x9d, 0xf1, 0x4e, 0x97, 0x82, 0x31, 0x1e, 0x31, 0xe6, 0x0f, 0xe0, 0x70,
            0x17, 0x4e,
        ];

        let mut data = plaintext;
        scramble(&cw, &mut data);
        assert_eq!(data, expected, "Vector 13 scramble mismatch");

        let mut data2 = expected;
        descramble(&cw, &mut data2);
        assert_eq!(data2, plaintext, "Vector 13 descramble mismatch");
    }

    #[test]
    fn vector_15_64byte() {
        let cw = ControlWord::from_bytes([0xc6, 0xe6, 0x2a, 0x81, 0xff, 0xd6, 0x18, 0xea]);
        let plaintext: [u8; 64] = [
            0x11, 0x80, 0xf1, 0x5b, 0x59, 0xf0, 0x0e, 0x95, 0x78, 0xd5, 0x74, 0x6a, 0x89, 0x10,
            0x64, 0x2c, 0x1f, 0xfe, 0xc1, 0xd7, 0x01, 0x86, 0x26, 0xfe, 0xa5, 0xe1, 0xc3, 0x76,
            0x6d, 0x2c, 0xfc, 0xc4, 0xa3, 0xaf, 0xc0, 0x47, 0x36, 0xeb, 0xa9, 0x24, 0x32, 0x66,
            0xd8, 0xf6, 0x01, 0x35, 0x8e, 0x33, 0x63, 0x4e, 0x8b, 0x5c, 0x2e, 0x8b, 0x27, 0xec,
            0xc7, 0x7d, 0x31, 0xfe, 0x5c, 0xf6, 0x8b, 0xbc,
        ];
        let expected: [u8; 64] = [
            0x45, 0xd2, 0x49, 0x70, 0x85, 0x52, 0x6f, 0x02, 0x12, 0x94, 0x39, 0x41, 0x07, 0x2e,
            0x59, 0x3a, 0x28, 0x93, 0x92, 0xf1, 0x99, 0x7e, 0x2c, 0x0e, 0x5d, 0x3d, 0x72, 0xd5,
            0xe3, 0x9a, 0xc6, 0x70, 0x39, 0x81, 0x31, 0x43, 0x83, 0x2c, 0xd5, 0xc8, 0xf5, 0xd2,
            0x7a, 0x28, 0x2b, 0xff, 0x5f, 0x4d, 0xb3, 0xc7, 0x2e, 0xce, 0xa8, 0xb1, 0xff, 0xbf,
            0x7d, 0x12, 0x4d, 0x53, 0xe2, 0xa7, 0x91, 0x4b,
        ];

        let mut data = plaintext;
        scramble(&cw, &mut data);
        assert_eq!(data, expected, "Vector 15 scramble mismatch");

        let mut data2 = expected;
        descramble(&cw, &mut data2);
        assert_eq!(data2, plaintext, "Vector 15 descramble mismatch");
    }

    #[test]
    fn vector_14_32byte() {
        // Vector 14: CW=1e279addca1cf532, 32 bytes
        let cw = ControlWord::from_bytes([0x1e, 0x27, 0x9a, 0xdd, 0xca, 0x1c, 0xf5, 0x32]);
        let plaintext: [u8; 32] = [
            0xf1, 0xb3, 0x6d, 0x88, 0x5c, 0x9b, 0x68, 0x16, 0xf7, 0xef, 0x1c, 0x31, 0x94, 0x46,
            0xa5, 0x32, 0x66, 0x79, 0xe7, 0x26, 0x38, 0x32, 0x29, 0x39, 0x70, 0x39, 0x6e, 0xdf,
            0xc7, 0x7e, 0x93, 0xc8,
        ];
        let expected: [u8; 32] = [
            0xb9, 0x59, 0x3e, 0xbd, 0xe4, 0xae, 0x0a, 0x30, 0xc7, 0x57, 0xe8, 0x6f, 0xf7, 0x6e,
            0x7a, 0x42, 0xb1, 0x17, 0x03, 0x16, 0xca, 0x69, 0x5d, 0x8b, 0x0f, 0x73, 0x7e, 0x1b,
            0x62, 0x4e, 0x55, 0x54,
        ];

        let mut data = plaintext;
        scramble(&cw, &mut data);
        assert_eq!(data, expected, "Vector 14 scramble mismatch");

        let mut data2 = expected;
        descramble(&cw, &mut data2);
        assert_eq!(data2, plaintext, "Vector 14 descramble mismatch");
    }
}
