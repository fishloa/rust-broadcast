//! External-standard test vectors for `srt_runtime::crypto` (issue #608).
//!
//! `draft-sharabayko-srt-01` §6 has **no worked test vectors of its own**
//! (`specs/rules/srt-crypto.md`, "No test vectors" note) — so the ground
//! truth here is the two external algorithms §6 names but does not restate:
//!
//! - **AES Key Wrap** — RFC 3394 §4.1, "Wrap 128 bits of Key Data with a
//!   128-bit KEK" (<https://datatracker.ietf.org/doc/html/rfc3394#section-4.1>),
//!   exercised through this crate's own [`srt_runtime::crypto::wrap_sek`] /
//!   [`srt_runtime::crypto::unwrap_sek`].
//! - **AES-CTR** — NIST SP 800-38A Appendix F.5.1, "CTR-AES128 (Encrypt)"
//!   (<https://nvlpubs.nist.gov/nistpubs/Legacy/SP/nistspecialpublication800-38a.pdf>),
//!   fetched and transcribed directly from the NIST publication during
//!   authoring. This vector has no Salt/PktSeqNo (that framing is
//!   SRT-specific, §6.2.2) — it grounds-truths the underlying `aes`+`ctr`
//!   RustCrypto dependency versions this crate pins, run directly the same
//!   way [`srt_runtime::crypto::aes_ctr_apply`] uses them internally.
//!
//! Both are byte-exact. The third block below exercises the full §6 SRT
//! payload path (SEK+Salt+PktSeqNo) end-to-end through the public
//! [`srt_runtime::crypto::aes_ctr_apply`] entry point, including the
//! negative "wrong SEK" case (no spec vector exists for this — SRT-specific
//! framing on top of the vector-checked AES-CTR primitive above).

#![cfg(feature = "crypto")]

use aes::Aes128;
use aes::cipher::{KeyIvInit, StreamCipher};
use ctr::Ctr128BE;
use srt_runtime::crypto::{aes_ctr_apply, unwrap_sek, wrap_sek};

// ---------------------------------------------------------------------------
// RFC 3394 §4.1 — AES Key Wrap, 128-bit KEK / 128-bit key data.
// ---------------------------------------------------------------------------

const RFC3394_KEK: [u8; 16] = [
    0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F,
];
const RFC3394_KEY_DATA: [u8; 16] = [
    0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF,
];
/// RFC 3394 §4.1's full 24-byte ciphertext: byte `[0..8]` is the ICV
/// (RFC 3394 §2.2.3.1's encrypted default IV), `[8..24]` is the wrapped key.
const RFC3394_CIPHERTEXT: [u8; 24] = [
    0x1F, 0xA6, 0x8B, 0x0A, 0x81, 0x12, 0xB4, 0x47, 0xAE, 0xF3, 0x4B, 0xD8, 0xFB, 0x5A, 0x7B, 0x82,
    0x9D, 0x3E, 0x86, 0x23, 0x71, 0xD2, 0xCF, 0xE5,
];

#[test]
fn rfc3394_wrap_is_byte_exact() {
    let (icv, wrapped) = wrap_sek(&RFC3394_KEK, &RFC3394_KEY_DATA).expect("wrap");
    let mut got = Vec::with_capacity(24);
    got.extend_from_slice(&icv);
    got.extend_from_slice(&wrapped);
    assert_eq!(
        got, RFC3394_CIPHERTEXT,
        "RFC 3394 §4.1 wrap output mismatch"
    );
}

#[test]
fn rfc3394_unwrap_round_trips_byte_exact() {
    let mut icv = [0u8; 8];
    icv.copy_from_slice(&RFC3394_CIPHERTEXT[..8]);
    let wrapped = &RFC3394_CIPHERTEXT[8..];

    let recovered = unwrap_sek(&RFC3394_KEK, &icv, wrapped).expect("unwrap");
    assert_eq!(
        recovered.as_slice(),
        &RFC3394_KEY_DATA[..],
        "RFC 3394 §4.1 unwrap did not recover the original key data"
    );
}

#[test]
fn rfc3394_unwrap_wrong_kek_fails_integrity_check() {
    let mut icv = [0u8; 8];
    icv.copy_from_slice(&RFC3394_CIPHERTEXT[..8]);
    let wrapped = &RFC3394_CIPHERTEXT[8..];

    let mut wrong_kek = RFC3394_KEK;
    wrong_kek[15] ^= 0x01;
    assert!(
        unwrap_sek(&wrong_kek, &icv, wrapped).is_err(),
        "unwrap with the wrong KEK must fail its integrity check, not silently succeed"
    );
}

// ---------------------------------------------------------------------------
// NIST SP 800-38A Appendix F.5.1 — CTR-AES128 (Encrypt).
// ---------------------------------------------------------------------------

const NIST_KEY: [u8; 16] = [
    0x2B, 0x7E, 0x15, 0x16, 0x28, 0xAE, 0xD2, 0xA6, 0xAB, 0xF7, 0x15, 0x88, 0x09, 0xCF, 0x4F, 0x3C,
];
/// F.5.1's "Initial Counter" — a raw 128-bit CTR seed (not this crate's
/// Salt+PktSeqNo `packet_counter` construction, which is SRT-specific,
/// §6.2.2 — NIST's own vector has no such framing).
const NIST_INIT_COUNTER: [u8; 16] = [
    0xF0, 0xF1, 0xF2, 0xF3, 0xF4, 0xF5, 0xF6, 0xF7, 0xF8, 0xF9, 0xFA, 0xFB, 0xFC, 0xFD, 0xFE, 0xFF,
];
const NIST_PLAINTEXT: [u8; 64] = [
    0x6B, 0xC1, 0xBE, 0xE2, 0x2E, 0x40, 0x9F, 0x96, 0xE9, 0x3D, 0x7E, 0x11, 0x73, 0x93, 0x17, 0x2A,
    0xAE, 0x2D, 0x8A, 0x57, 0x1E, 0x03, 0xAC, 0x9C, 0x9E, 0xB7, 0x6F, 0xAC, 0x45, 0xAF, 0x8E, 0x51,
    0x30, 0xC8, 0x1C, 0x46, 0xA3, 0x5C, 0xE4, 0x11, 0xE5, 0xFB, 0xC1, 0x19, 0x1A, 0x0A, 0x52, 0xEF,
    0xF6, 0x9F, 0x24, 0x45, 0xDF, 0x4F, 0x9B, 0x17, 0xAD, 0x2B, 0x41, 0x7B, 0xE6, 0x6C, 0x37, 0x10,
];
const NIST_CIPHERTEXT: [u8; 64] = [
    0x87, 0x4D, 0x61, 0x91, 0xB6, 0x20, 0xE3, 0x26, 0x1B, 0xEF, 0x68, 0x64, 0x99, 0x0D, 0xB6, 0xCE,
    0x98, 0x06, 0xF6, 0x6B, 0x79, 0x70, 0xFD, 0xFF, 0x86, 0x17, 0x18, 0x7B, 0xB9, 0xFF, 0xFD, 0xFF,
    0x5A, 0xE4, 0xDF, 0x3E, 0xDB, 0xD5, 0xD3, 0x5E, 0x5B, 0x4F, 0x09, 0x02, 0x0D, 0xB0, 0x3E, 0xAB,
    0x1E, 0x03, 0x1D, 0xDA, 0x2F, 0xBE, 0x03, 0xD1, 0x79, 0x21, 0x70, 0xA0, 0xF3, 0x00, 0x9C, 0xEE,
];

/// Byte-exact against the NIST vector, run directly through the `aes`+`ctr`
/// RustCrypto dependency this crate pins for [`aes_ctr_apply`] — the same
/// primitive, seeded with the vector's own raw counter rather than one
/// derived from an SRT Salt/PktSeqNo (NIST's format has no such framing).
#[test]
fn nist_sp800_38a_f5_1_ctr_aes128_encrypt_is_byte_exact() {
    let mut buf = NIST_PLAINTEXT;
    let mut cipher =
        Ctr128BE::<Aes128>::new_from_slices(&NIST_KEY, &NIST_INIT_COUNTER).expect("16-byte key/IV");
    cipher.apply_keystream(&mut buf);
    assert_eq!(
        buf, NIST_CIPHERTEXT,
        "NIST SP 800-38A F.5.1 ciphertext mismatch"
    );
}

#[test]
fn nist_sp800_38a_f5_1_ctr_aes128_decrypt_is_byte_exact() {
    // CTR is self-inverse: re-applying the keystream to the ciphertext
    // recovers the plaintext (this is exactly what §6.3.2's
    // `DecryptedPayload = AES_CTR_Encrypt(SEK, IV, EncryptedPayload)` reuses).
    let mut buf = NIST_CIPHERTEXT;
    let mut cipher =
        Ctr128BE::<Aes128>::new_from_slices(&NIST_KEY, &NIST_INIT_COUNTER).expect("16-byte key/IV");
    cipher.apply_keystream(&mut buf);
    assert_eq!(
        buf, NIST_PLAINTEXT,
        "NIST SP 800-38A F.5.1 plaintext mismatch"
    );
}

// ---------------------------------------------------------------------------
// SRT §6 payload round-trip (SEK + Salt + PktSeqNo) — no spec vector exists
// for this (srt-crypto.md's "No test vectors" note), so this validates the
// crate's own encrypt/decrypt agree, and that a wrong SEK does not.
// ---------------------------------------------------------------------------

#[test]
fn srt_payload_encrypt_decrypt_round_trips() {
    let sek = [0x5Au8; 16];
    let salt = [0xC3u8; 16];
    let pkt_seq_no = 0x0000_2A2Au32;
    let plaintext = b"the quick brown fox jumps over the lazy dog";

    let mut buf = plaintext.to_vec();
    aes_ctr_apply(&sek, &salt, pkt_seq_no, &mut buf).expect("encrypt");
    assert_ne!(buf.as_slice(), &plaintext[..]);

    aes_ctr_apply(&sek, &salt, pkt_seq_no, &mut buf).expect("decrypt");
    assert_eq!(buf.as_slice(), &plaintext[..]);
}

#[test]
fn srt_payload_wrong_sek_does_not_recover_plaintext() {
    let sek = [0x5Au8; 16];
    let wrong_sek = [0x5Bu8; 16];
    let salt = [0xC3u8; 16];
    let pkt_seq_no = 0x0000_2A2Au32;
    let plaintext = b"the quick brown fox jumps over the lazy dog";

    let mut encrypted = plaintext.to_vec();
    aes_ctr_apply(&sek, &salt, pkt_seq_no, &mut encrypted).expect("encrypt");

    let mut decrypted_with_wrong_sek = encrypted.clone();
    aes_ctr_apply(&wrong_sek, &salt, pkt_seq_no, &mut decrypted_with_wrong_sek)
        .expect("decrypt (wrong SEK still runs — AES-CTR has no integrity check)");
    assert_ne!(
        decrypted_with_wrong_sek.as_slice(),
        &plaintext[..],
        "decrypting with the wrong SEK must not recover the plaintext"
    );
}
