//! RFC 3711 (SRTP) Appendix B "Test Vectors" — official IETF test vectors,
//! transcribed at `fixtures/webrtc/rfc3711-appendix-b.md` (see
//! `fixtures/webrtc/PROVENANCE.md` for source/licence).
//!
//! Three layers of check, each documented at its test:
//!
//! 1. [`aes_cm_keystream_matches_rfc_appendix_b2`] — the AES-CM keystream
//!    vector (B.2), checked against the plain `aes` crate with no SRTP
//!    involved at all (this is the primitive AES-CM keystream generation
//!    bottoms out in).
//! 2. [`key_derivation_matches_rfc_appendix_b3`] — the key-derivation
//!    vector (B.3): cipher key, cipher salt, and (the profile's leading 20
//!    bytes of the RFC's 94-byte illustrative output) auth key, computed
//!    by an independent from-scratch implementation of the KDF formula
//!    (`rfc3711_kdf` below, transcribed directly from the RFC §4.3.1/
//!    Appendix B.3 text) against the RFC's own published numbers.
//! 3. [`srtp_context_reproduces_appendix_b3_ciphertext`] /
//!    [`srtcp_round_trip_and_index_behaviour`] — **our SRTP path**:
//!    `rtc_srtp::context::Context`, the exact type
//!    `webrtc_runtime::media::transport::MediaTransport` builds from DTLS-
//!    exported keying material, keyed with the Appendix B.3 master
//!    key/salt, cross-checked byte-for-byte against an independent
//!    AES-CTR + HMAC-SHA1 computation built from the values verified in
//!    step 2. SRTCP additionally exercises the index-increment/E-bit
//!    behaviour explicitly called out as under-tested (RFC 3711 §3.4,
//!    §9.1) — the browser-only interop test that motivated this fixture
//!    never touched RTCP at all.
//!
//! Fixture-only test code: no production source in this crate is touched.
#![cfg(feature = "media")]

use aes::Aes128;
use broadcast_common::{Parse, Serialize};
use cipher::{BlockEncrypt, KeyInit, KeyIvInit, StreamCipher, generic_array::GenericArray};
use ctr::Ctr128BE;
use hmac::{Hmac, Mac};
use rtc_srtp::context::Context as SrtpContext;
use rtc_srtp::protection_profile::ProtectionProfile;
use rtcp_packet::{CompoundPacket, RtcpPacket, SenderReport};
use rtp_packet::RtpPacket;
use sha1::Sha1;

// ---------------------------------------------------------------------------
// B.2 — AES-CM Test Vectors (session key given directly, no KDF involved).
// ---------------------------------------------------------------------------

const B2_SESSION_KEY: [u8; 16] = [
    0x2B, 0x7E, 0x15, 0x16, 0x28, 0xAE, 0xD2, 0xA6, 0xAB, 0xF7, 0x15, 0x88, 0x09, 0xCF, 0x4F, 0x3C,
];

#[test]
fn aes_cm_keystream_matches_rfc_appendix_b2() {
    let cipher = Aes128::new(GenericArray::from_slice(&B2_SESSION_KEY));

    let cases: &[([u8; 16], [u8; 16])] = &[
        (
            [
                0xF0, 0xF1, 0xF2, 0xF3, 0xF4, 0xF5, 0xF6, 0xF7, 0xF8, 0xF9, 0xFA, 0xFB, 0xFC, 0xFD,
                0x00, 0x00,
            ],
            [
                0xE0, 0x3E, 0xAD, 0x09, 0x35, 0xC9, 0x5E, 0x80, 0xE1, 0x66, 0xB1, 0x6D, 0xD9, 0x2B,
                0x4E, 0xB4,
            ],
        ),
        (
            [
                0xF0, 0xF1, 0xF2, 0xF3, 0xF4, 0xF5, 0xF6, 0xF7, 0xF8, 0xF9, 0xFA, 0xFB, 0xFC, 0xFD,
                0x00, 0x01,
            ],
            [
                0xD2, 0x35, 0x13, 0x16, 0x2B, 0x02, 0xD0, 0xF7, 0x2A, 0x43, 0xA2, 0xFE, 0x4A, 0x5F,
                0x97, 0xAB,
            ],
        ),
        (
            [
                0xF0, 0xF1, 0xF2, 0xF3, 0xF4, 0xF5, 0xF6, 0xF7, 0xF8, 0xF9, 0xFA, 0xFB, 0xFC, 0xFD,
                0x00, 0x02,
            ],
            [
                0x41, 0xE9, 0x5B, 0x3B, 0xB0, 0xA2, 0xE8, 0xDD, 0x47, 0x79, 0x01, 0xE4, 0xFC, 0xA8,
                0x94, 0xC0,
            ],
        ),
        (
            [
                0xF0, 0xF1, 0xF2, 0xF3, 0xF4, 0xF5, 0xF6, 0xF7, 0xF8, 0xF9, 0xFA, 0xFB, 0xFC, 0xFD,
                0xFE, 0xFF,
            ],
            [
                0xEC, 0x8C, 0xDF, 0x73, 0x98, 0x60, 0x7C, 0xB0, 0xF2, 0xD2, 0x16, 0x75, 0xEA, 0x9E,
                0xA1, 0xE4,
            ],
        ),
        (
            [
                0xF0, 0xF1, 0xF2, 0xF3, 0xF4, 0xF5, 0xF6, 0xF7, 0xF8, 0xF9, 0xFA, 0xFB, 0xFC, 0xFD,
                0xFF, 0x00,
            ],
            [
                0x36, 0x2B, 0x7C, 0x3C, 0x67, 0x73, 0x51, 0x63, 0x18, 0xA0, 0x77, 0xD7, 0xFC, 0x50,
                0x73, 0xAE,
            ],
        ),
        (
            [
                0xF0, 0xF1, 0xF2, 0xF3, 0xF4, 0xF5, 0xF6, 0xF7, 0xF8, 0xF9, 0xFA, 0xFB, 0xFC, 0xFD,
                0xFF, 0x01,
            ],
            [
                0x6A, 0x2C, 0xC3, 0x78, 0x78, 0x89, 0x37, 0x4F, 0xBE, 0xB4, 0xC8, 0x1B, 0x17, 0xBA,
                0x6C, 0x44,
            ],
        ),
    ];

    for (counter, expected_keystream) in cases {
        let mut block = GenericArray::clone_from_slice(counter.as_slice());
        cipher.encrypt_block(&mut block);
        assert_eq!(
            block.as_slice(),
            expected_keystream,
            "AES-ECB(session key, counter {counter:02X?}) must equal the RFC 3711 §B.2 keystream block"
        );
    }
}

// ---------------------------------------------------------------------------
// B.3 — Key Derivation Test Vectors.
// ---------------------------------------------------------------------------

const B3_MASTER_KEY: [u8; 16] = [
    0xE1, 0xF9, 0x7A, 0x0D, 0x3E, 0x01, 0x8B, 0xE0, 0xD6, 0x4F, 0xA3, 0x2C, 0x06, 0xDE, 0x41, 0x39,
];
const B3_MASTER_SALT: [u8; 14] = [
    0x0E, 0xC6, 0x75, 0xAD, 0x49, 0x8A, 0xFE, 0xEB, 0xB6, 0x96, 0x0B, 0x3A, 0xAB, 0xE6,
];

const LABEL_SRTP_ENCRYPTION: u8 = 0x00;
const LABEL_SRTP_AUTHENTICATION_TAG: u8 = 0x01;
const LABEL_SRTP_SALT: u8 = 0x02;
// The SRTCP-side labels (0x03/0x04/0x05, same KDF, RFC 3711 §4.3.2) are not
// used here: Appendix B.3 gives no published numeric output for them (see
// this file's module doc and fixtures/webrtc/rfc3711-appendix-b.md), so the
// SRTCP section below verifies via round-trip + index behaviour through
// `rtc_srtp::context::Context` itself rather than an independent byte
// oracle that would just be asserting two homemade implementations agree.

/// RFC 3711 §4.3.1 / Appendix B.3's key derivation function, from scratch:
/// XOR the (zero-padded-to-16-byte) master salt with the label at byte
/// offset 7, then AES-ECB-encrypt with the master key once per 16-byte
/// output block (the last two bytes of the input carry a big-endian block
/// counter for outputs longer than one block). Independent of
/// `rtc_srtp::key_derivation` (a private module of that crate) — this is a
/// second, from-spec-text implementation used purely to cross-check the
/// RFC's published numbers, not a copy of the crate's internals.
fn rfc3711_kdf(label: u8, master_key: [u8; 16], master_salt: [u8; 14], out_len: usize) -> Vec<u8> {
    let mut prf_in = [0u8; 16];
    prf_in[..14].copy_from_slice(&master_salt);
    prf_in[7] ^= label;

    let cipher = Aes128::new(GenericArray::from_slice(&master_key));
    let mut out = Vec::with_capacity(out_len);
    let mut block_index: u16 = 0;
    while out.len() < out_len {
        let mut block = prf_in;
        block[14] = (block_index >> 8) as u8;
        block[15] = (block_index & 0xFF) as u8;
        let mut arr = GenericArray::clone_from_slice(&block);
        cipher.encrypt_block(&mut arr);
        out.extend_from_slice(&arr);
        block_index += 1;
    }
    out.truncate(out_len);
    out
}

#[test]
fn key_derivation_matches_rfc_appendix_b3() {
    let cipher_key = rfc3711_kdf(LABEL_SRTP_ENCRYPTION, B3_MASTER_KEY, B3_MASTER_SALT, 16);
    assert_eq!(
        cipher_key.as_slice(),
        &[
            0xC6, 0x1E, 0x7A, 0x93, 0x74, 0x4F, 0x39, 0xEE, 0x10, 0x73, 0x4A, 0xFE, 0x3F, 0xF7,
            0xA0, 0x87,
        ],
        "cipher key must match RFC 3711 Appendix B.3"
    );

    let cipher_salt = rfc3711_kdf(LABEL_SRTP_SALT, B3_MASTER_KEY, B3_MASTER_SALT, 14);
    assert_eq!(
        cipher_salt.as_slice(),
        &[
            0x30, 0xCB, 0xBC, 0x08, 0x86, 0x3D, 0x8C, 0x85, 0xD4, 0x9D, 0xB3, 0x4A, 0x9A, 0xE1,
        ],
        "cipher salt must match RFC 3711 Appendix B.3"
    );

    // The profile's HMAC-SHA1-80 auth key is 20 bytes — the leading 20
    // bytes of the RFC's own 94-byte illustrative output (see
    // fixtures/webrtc/rfc3711-appendix-b.md for why the RFC's worked
    // example is longer than any real profile consumes).
    let auth_key = rfc3711_kdf(
        LABEL_SRTP_AUTHENTICATION_TAG,
        B3_MASTER_KEY,
        B3_MASTER_SALT,
        20,
    );
    assert_eq!(
        auth_key.as_slice(),
        &[
            0xCE, 0xBE, 0x32, 0x1F, 0x6F, 0xF7, 0x71, 0x6B, 0x6F, 0xD4, 0xAB, 0x49, 0xAF, 0x25,
            0x6A, 0x15, 0x6D, 0x38, 0xBA, 0xA4,
        ],
        "auth key (first 20 bytes) must match RFC 3711 Appendix B.3"
    );
}

// ---------------------------------------------------------------------------
// "Our SRTP path": rtc_srtp::context::Context keyed with the Appendix B.3
// master key/salt, cross-checked against an independent implementation of
// RFC 3711 §4.1.1 (AES-CM keystream) + §4.2 (HMAC-SHA1 auth tag).
// ---------------------------------------------------------------------------

/// RFC 3711 §4.1.1's IV/counter construction: the 14-byte session salt
/// zero-padded to 16 bytes, XORed with SSRC (bytes 4..8), ROC (bytes 8..12)
/// and `SEQ << 16` (bytes 12..16).
fn generate_counter(sequence_number: u16, roc: u32, ssrc: u32, session_salt: &[u8]) -> [u8; 16] {
    let mut counter = [0u8; 16];
    counter[4..8].copy_from_slice(&ssrc.to_be_bytes());
    counter[8..12].copy_from_slice(&roc.to_be_bytes());
    counter[12..16].copy_from_slice(&((sequence_number as u32) << 16).to_be_bytes());
    for (c, s) in counter.iter_mut().zip(session_salt) {
        *c ^= s;
    }
    counter
}

/// Independent re-implementation of `CipherAesCmHmacSha1::encrypt_rtp`
/// (RFC 3711 §4.1.1 encrypt + §4.2 HMAC-SHA1-80 authenticate), built only
/// from the Appendix B.3 master key/salt and the already-verified KDF
/// above — used purely as an oracle to cross-check
/// `rtc_srtp::context::Context::encrypt_rtp`'s byte-exact output.
fn independent_srtp_encrypt(plaintext_rtp: &[u8], roc: u32) -> Vec<u8> {
    let header = RtpPacket::parse(plaintext_rtp).expect("valid RTP header");
    let header_len = plaintext_rtp.len() - header.payload.len();

    let cipher_key = rfc3711_kdf(LABEL_SRTP_ENCRYPTION, B3_MASTER_KEY, B3_MASTER_SALT, 16);
    let cipher_salt = rfc3711_kdf(LABEL_SRTP_SALT, B3_MASTER_KEY, B3_MASTER_SALT, 14);
    let auth_key = rfc3711_kdf(
        LABEL_SRTP_AUTHENTICATION_TAG,
        B3_MASTER_KEY,
        B3_MASTER_SALT,
        20,
    );

    let counter = generate_counter(header.sequence_number, roc, header.ssrc, &cipher_salt);
    let mut out = plaintext_rtp.to_vec();
    let mut stream = Ctr128BE::<Aes128>::new(
        GenericArray::from_slice(&cipher_key),
        GenericArray::from_slice(&counter),
    );
    stream.apply_keystream(&mut out[header_len..]);

    // `Hmac::<Sha1>::new_from_slice` is ambiguous once `cipher::KeyInit` (used
    // above for `Aes128::new`) is also in scope, since `KeyInit` has its own
    // `new_from_slice` too — disambiguate via `Mac::new_from_slice`.
    let mut mac: Hmac<Sha1> = Mac::new_from_slice(&auth_key).expect("valid HMAC key length");
    mac.update(&out);
    mac.update(&roc.to_be_bytes());
    let tag = mac.finalize().into_bytes();
    out.extend_from_slice(&tag[..10]); // Aes128CmHmacSha1_80: 80-bit (10-byte) tag.
    out
}

fn build_rtp_packet(payload: &[u8]) -> Vec<u8> {
    RtpPacket {
        marker: false,
        payload_type: 96,
        sequence_number: 0,
        timestamp: 0,
        ssrc: 0,
        csrc: Vec::new(),
        extension: None,
        padding: None,
        payload,
    }
    .to_bytes()
}

#[test]
fn srtp_context_reproduces_appendix_b3_ciphertext() {
    let payload = [0xAAu8; 32];
    let plaintext_rtp = build_rtp_packet(&payload);

    let mut enc_ctx = SrtpContext::new(
        &B3_MASTER_KEY,
        &B3_MASTER_SALT,
        ProtectionProfile::Aes128CmHmacSha1_80,
        None,
        None,
    )
    .expect("build encrypt context");
    let protected = enc_ctx.encrypt_rtp(&plaintext_rtp).expect("encrypt_rtp");

    let expected = independent_srtp_encrypt(&plaintext_rtp, 0);
    assert_eq!(
        protected.to_vec(),
        expected,
        "rtc_srtp::context::Context::encrypt_rtp must reproduce the independent \
         RFC 3711 §4.1.1/§4.2 computation keyed from Appendix B.3's master key/salt"
    );

    // Round trip: a second context (the peer's decrypt direction) recovers
    // the exact original plaintext.
    let mut dec_ctx = SrtpContext::new(
        &B3_MASTER_KEY,
        &B3_MASTER_SALT,
        ProtectionProfile::Aes128CmHmacSha1_80,
        None,
        None,
    )
    .expect("build decrypt context");
    let recovered = dec_ctx.decrypt_rtp(&protected).expect("decrypt_rtp");
    assert_eq!(recovered.to_vec(), plaintext_rtp);
}

/// Bite test: corrupt one ciphertext byte, prove decryption fails
/// (authentication, not just garbled payload), restore, prove it passes.
#[test]
fn srtp_corrupted_ciphertext_byte_fails_auth() {
    let payload = [0x11u8; 20];
    let plaintext_rtp = build_rtp_packet(&payload);
    let mut enc_ctx = SrtpContext::new(
        &B3_MASTER_KEY,
        &B3_MASTER_SALT,
        ProtectionProfile::Aes128CmHmacSha1_80,
        None,
        None,
    )
    .unwrap();
    let mut protected = enc_ctx.encrypt_rtp(&plaintext_rtp).unwrap().to_vec();

    // Flip a bit in the middle of the ciphertext payload (well after the
    // 12-byte RTP header, well before the 10-byte trailing auth tag).
    let idx = protected.len() - 15;
    protected[idx] ^= 0x01;

    let mut dec_ctx = SrtpContext::new(
        &B3_MASTER_KEY,
        &B3_MASTER_SALT,
        ProtectionProfile::Aes128CmHmacSha1_80,
        None,
        None,
    )
    .unwrap();
    assert!(
        dec_ctx.decrypt_rtp(&protected).is_err(),
        "corrupted ciphertext byte must fail SRTP auth-tag verification"
    );

    // Restore and prove the same context (replay protection is per-SSRC
    // sequence-number window, not per-attempt, so a fresh context is used
    // for the clean restore check) verifies the untouched packet.
    protected[idx] ^= 0x01;
    let mut dec_ctx2 = SrtpContext::new(
        &B3_MASTER_KEY,
        &B3_MASTER_SALT,
        ProtectionProfile::Aes128CmHmacSha1_80,
        None,
        None,
    )
    .unwrap();
    let recovered = dec_ctx2
        .decrypt_rtp(&protected)
        .expect("restored packet must verify");
    assert_eq!(recovered.to_vec(), plaintext_rtp);
}

// ---------------------------------------------------------------------------
// SRTCP — the RFC gives no numeric worked example for the SRTCP-labelled
// (0x03/0x04/0x05) session keys, so this section round-trips
// `rtc_srtp::context::Context::encrypt_rtcp`/`decrypt_rtcp` (keyed from the
// same Appendix B.3 master key/salt) and separately exercises the E-bit +
// 31-bit SRTCP index behaviour (RFC 3711 §3.4, §9.1) that had zero test
// coverage before this fixture (the prior browser-interop test never
// touched RTCP).
// ---------------------------------------------------------------------------

fn build_sender_report(ssrc: u32) -> Vec<u8> {
    CompoundPacket::new(vec![RtcpPacket::SenderReport(SenderReport {
        ssrc,
        ntp_msw: 0,
        ntp_lsw: 0,
        rtp_timestamp: 0,
        packet_count: 0,
        octet_count: 0,
        report_blocks: Vec::new(),
    })])
    .expect("compound packet starting with SR")
    .to_bytes()
}

#[test]
fn srtcp_round_trip_and_index_behaviour() {
    let rtcp_plaintext = build_sender_report(0);
    let mut ctx = SrtpContext::new(
        &B3_MASTER_KEY,
        &B3_MASTER_SALT,
        ProtectionProfile::Aes128CmHmacSha1_80,
        None,
        None,
    )
    .expect("build context");

    let first = ctx.encrypt_rtcp(&rtcp_plaintext).expect("encrypt_rtcp #1");
    let second = ctx.encrypt_rtcp(&rtcp_plaintext).expect("encrypt_rtcp #2");

    // The 4-byte (E-bit | 31-bit index) trailer sits right before the
    // 10-byte HMAC-SHA1-80 auth tag.
    let trailer = |packet: &[u8]| -> u32 {
        let n = packet.len();
        u32::from_be_bytes(packet[n - 14..n - 10].try_into().unwrap())
    };
    let first_trailer = trailer(&first);
    let second_trailer = trailer(&second);

    assert_eq!(
        first_trailer >> 31,
        1,
        "E-bit must be set (packet is encrypted)"
    );
    assert_eq!(
        second_trailer >> 31,
        1,
        "E-bit must be set (packet is encrypted)"
    );
    assert_eq!(
        first_trailer & 0x7FFF_FFFF,
        1,
        "first SRTCP index must be 1"
    );
    assert_eq!(
        second_trailer & 0x7FFF_FFFF,
        2,
        "SRTCP index must increment on every encrypt_rtcp call"
    );

    // Round trip both packets through a fresh decrypt-direction context.
    let mut dec_ctx = SrtpContext::new(
        &B3_MASTER_KEY,
        &B3_MASTER_SALT,
        ProtectionProfile::Aes128CmHmacSha1_80,
        None,
        None,
    )
    .expect("build decrypt context");
    assert_eq!(
        dec_ctx.decrypt_rtcp(&first).unwrap().to_vec(),
        rtcp_plaintext
    );
    assert_eq!(
        dec_ctx.decrypt_rtcp(&second).unwrap().to_vec(),
        rtcp_plaintext
    );
}

/// Bite test for SRTCP: corrupt one auth-tag byte, prove decryption fails,
/// restore, prove it passes.
#[test]
fn srtcp_corrupted_auth_tag_byte_fails() {
    let rtcp_plaintext = build_sender_report(0xAABB_CC00);
    let mut ctx = SrtpContext::new(
        &B3_MASTER_KEY,
        &B3_MASTER_SALT,
        ProtectionProfile::Aes128CmHmacSha1_80,
        None,
        None,
    )
    .unwrap();
    let mut protected = ctx.encrypt_rtcp(&rtcp_plaintext).unwrap().to_vec();

    let last = protected.len() - 1;
    protected[last] ^= 0x01;

    let mut dec_ctx = SrtpContext::new(
        &B3_MASTER_KEY,
        &B3_MASTER_SALT,
        ProtectionProfile::Aes128CmHmacSha1_80,
        None,
        None,
    )
    .unwrap();
    assert!(
        dec_ctx.decrypt_rtcp(&protected).is_err(),
        "corrupted SRTCP auth-tag byte must fail verification"
    );

    protected[last] ^= 0x01;
    let mut dec_ctx2 = SrtpContext::new(
        &B3_MASTER_KEY,
        &B3_MASTER_SALT,
        ProtectionProfile::Aes128CmHmacSha1_80,
        None,
        None,
    )
    .unwrap();
    assert_eq!(
        dec_ctx2.decrypt_rtcp(&protected).unwrap().to_vec(),
        rtcp_plaintext,
        "restored SRTCP packet must verify and decrypt back to the original"
    );
}
