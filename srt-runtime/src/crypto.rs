//! SRT payload encryption — `draft-sharabayko-srt-01` §6 ("Encryption").
//!
//! Spec grounding: `specs/rules/srt-crypto.md` (curated §6, external-algorithm
//! references grep-verified against RFC 3394 / RFC 8018 / RFC 2104 / FIPS 197
//! / NIST SP 800-38A). Cross-refs: `specs/rules/srt-rules.md` §"Key Material
//! message — §3.2.2" (wire layout of `Salt`/`ICV`/`xSEK`/`oSEK`/`KLen`, all
//! carried opaquely by [`crate::packet::KeyMaterial`]) and §"Data packet —
//! §3.1" (`KK` field, [`crate::packet::EncryptionKeyField`]).
//!
//! This module adds the crypto *primitives* §6 names but does not restate:
//!
//! - **AES-CTR** payload encrypt/decrypt ([`aes_ctr_apply`]) — §6.2.2/§6.3.2.
//! - **RFC 3394 AES key wrap/unwrap** of the SEK ([`wrap_sek`]/[`unwrap_sek`])
//!   — §6.1.5/§6.2.1/§6.3.1.
//! - **PBKDF2 (HMAC-SHA1) KEK derivation** from a passphrase
//!   ([`derive_kek`]) — §6.1.4/§6.2.1/§6.3.1.
//!
//! Gated behind the `crypto` feature; the default/no_std packet-codec core
//! (this crate's `--no-default-features` build) pulls none of these
//! dependencies.
//!
//! # ⚠ The draft's two IV formulas (srt-crypto.md, "Conflicting IV formula")
//!
//! `draft-sharabayko-srt-01` gives **two different, unreconciled** formulas
//! for the AES-CTR IV:
//!
//! - §6.1.2 (Overview): build a 128-bit `{80 zero bits | 32-bit packet index
//!   | 16-bit block counter}` word and XOR its upper 112 bits with `IV =
//!   MSB(112, Salt)`.
//! - §6.2.2 (Encryption Process) / §6.3.2 (Decryption Process), verbatim and
//!   identical in both places: `IV = (MSB(112, Salt) << 2) XOR (PktSeqNo)`.
//!
//! These are not algebraically the same construction, and the draft never
//! reconciles them. This module implements the **§6.2.2/§6.3.2 form** (the
//! one attached to the actual Encryption/Decryption Process pseudocode,
//! rather than the more abstract §6.1.2 overview) — see [`packet_counter`].
//! No sample ciphertext exists anywhere in the draft to disambiguate by
//! reproduction (`srt-crypto.md`, "No test vectors" note); this crate picks
//! deliberately rather than average/guess, and flags the ambiguity here
//! rather than silently picking one.

use aes::cipher::{KeyIvInit, StreamCipher};
use aes::{Aes128, Aes192, Aes256};
use aes_kw::{KekAes128, KekAes192, KekAes256};
use alloc::vec;
use alloc::vec::Vec;
use ctr::Ctr128BE;
use hmac::Hmac;
use pbkdf2::pbkdf2;
use sha1::Sha1;

use crate::error::{Error, Result};
use crate::packet::EncryptionKeyField;

/// PBKDF2 iteration count mandated by the draft (`Iter = 2048`,
/// §6.1.4/§6.2.1/§6.3.1 — identical value at every citation).
pub const PBKDF2_ITERATIONS: u32 = 2048;

/// Length of the 128-bit Key Material `Salt` field in bytes (`SLen/4 = 4`
/// words — the only salt length the draft defines, `srt-rules.md` §3.2.2).
pub const SALT_LEN: usize = 16;

/// Number of least-significant bytes of the 128-bit `Salt` fed to PBKDF2 as
/// its salt argument (`LSB(64,Salt)`, §6.2.1/§6.3.1: 64 bits = 8 bytes). The
/// remaining (most-significant) bytes of `Salt` feed the AES-CTR IV instead
/// (§6.1.2/§6.2.2/§6.3.2).
const PBKDF2_SALT_LEN: usize = 8;

/// Number of most-significant bytes of the 128-bit `Salt` used to build the
/// AES-CTR IV (`MSB(112, Salt)`, §6.1.2/§6.2.2/§6.3.2: 112 bits = 14 bytes).
const IV_SALT_LEN: usize = 14;

/// AES block size in bytes (FIPS 197) — also the AES-CTR counter width
/// (§6.1.2: "The counter for AES-CTR is the size of the cipher's block, i.e.
/// 128 bits").
const AES_BLOCK_LEN: usize = 16;

/// A key length not one of AES-128/192/256's 16/24/32 bytes (`KLen/4` ∈
/// `{4,6,8}`, `srt-rules.md` §3.2.2).
fn invalid_key_length(what: &'static str) -> Error {
    Error::InvalidField {
        what,
        reason: "length must be 16, 24, or 32 bytes (AES-128/192/256)",
    }
}

// ---------------------------------------------------------------------------
// §6.1.4/§6.2.1/§6.3.1 — KEK derivation (passphrase path).
// ---------------------------------------------------------------------------

/// Derive the Key Encrypting Key (KEK) from the pre-shared passphrase
/// (`draft-sharabayko-srt-01` §6.1.4, §6.2.1 sender / §6.3.1 receiver —
/// identical formula both sides):
///
/// ```text
/// KEK = PBKDF2(passphrase, LSB(64,Salt), Iter=2048, KLen)
/// ```
///
/// `salt` is the Key Material message's 128-bit `Salt` field; `klen` is the
/// desired KEK length in bytes (16/24/32, matching the handshake's
/// Encryption Field / the Key Material message's `KLen/4` — "the KEK has to
/// be at least as long as the SEK", §6.1.4).
///
/// # Errors
/// [`Error::InvalidField`] if `klen` is not 16, 24, or 32.
pub fn derive_kek(passphrase: &[u8], salt: &[u8; SALT_LEN], klen: usize) -> Result<Vec<u8>> {
    if !matches!(klen, 16 | 24 | 32) {
        return Err(invalid_key_length("KLen"));
    }
    // LSB(64, Salt): the low/least-significant 8 bytes of the 128-bit,
    // big-endian-wire Salt.
    let pbkdf2_salt = &salt[SALT_LEN - PBKDF2_SALT_LEN..];
    let mut kek = vec![0u8; klen];
    pbkdf2::<Hmac<Sha1>>(passphrase, pbkdf2_salt, PBKDF2_ITERATIONS, &mut kek)
        .expect("HMAC-SHA1 accepts any key length, so PBKDF2 cannot fail here");
    Ok(kek)
}

// ---------------------------------------------------------------------------
// §6.1.5/§6.2.1/§6.3.1 — RFC 3394 AES key wrap/unwrap of the SEK.
// ---------------------------------------------------------------------------

/// Wrap one or two SEKs with the KEK (RFC 3394 AES key wrap, external
/// algorithm — `draft-sharabayko-srt-01` §6.1.5/§6.2.1: `Wrap = AESkw(KEK,
/// SEK)`).
///
/// `plaintext_keys` is the concatenation of the SEK(s) being wrapped — one
/// SEK's worth of bytes (Key Material `KK` = even/odd) or two concatenated
/// SEKs (`KK` = both), per the Wrap-field length formula `n*KLen + 8` in
/// `srt-rules.md` §"Key Material message — §3.2.2". Returns `(icv, wrapped)`
/// to match [`crate::packet::KeyMaterial`]'s `icv`/`x_sek`/`o_sek` fields —
/// RFC 3394's first 8-byte output block *is* the wrap's Integrity Check
/// Vector (the encrypted default IV, RFC 3394 §2.2.3.1), and the remaining
/// bytes are the wrapped key material, the same length as the input.
///
/// # Errors
/// [`Error::InvalidField`] if `kek.len()` is not 16, 24, or 32, or if
/// `plaintext_keys.len()` is not a multiple of 8 bytes (RFC 3394 operates on
/// 64-bit semiblocks).
pub fn wrap_sek(kek: &[u8], plaintext_keys: &[u8]) -> Result<([u8; 8], Vec<u8>)> {
    let mut out = vec![0u8; plaintext_keys.len() + 8];
    match kek.len() {
        16 => KekAes128::try_from(kek)
            .map_err(|_| invalid_key_length("KEK"))?
            .wrap(plaintext_keys, &mut out),
        24 => KekAes192::try_from(kek)
            .map_err(|_| invalid_key_length("KEK"))?
            .wrap(plaintext_keys, &mut out),
        32 => KekAes256::try_from(kek)
            .map_err(|_| invalid_key_length("KEK"))?
            .wrap(plaintext_keys, &mut out),
        _ => return Err(invalid_key_length("KEK")),
    }
    .map_err(|_| Error::InvalidField {
        what: "SEK",
        reason: "length must be a multiple of 8 bytes (RFC 3394 semiblocks)",
    })?;
    let mut icv = [0u8; 8];
    icv.copy_from_slice(&out[..8]);
    Ok((icv, out[8..].to_vec()))
}

/// Unwrap the SEK(s) with the KEK (inverse RFC 3394 AES key wrap —
/// `draft-sharabayko-srt-01` §6.1.5/§6.3.1: `SEK = AESkuw(KEK, Wrap)`).
///
/// `icv`/`wrapped` are [`crate::packet::KeyMaterial`]'s `icv`/`x_sek` (or
/// `o_sek`) fields. A wrap-integrity failure — wrong KEK (wrong passphrase)
/// or corrupt wire data — is the spec's "it does not have the SEK" case
/// (§6.1.5, L3799-3803/L3820-3823): a structured error, never a panic or
/// silently-wrong plaintext.
///
/// # Errors
/// [`Error::InvalidField`] if `kek.len()` is not 16, 24, or 32, if
/// `wrapped.len()` is not a multiple of 8 bytes, or if the RFC 3394
/// integrity check fails.
pub fn unwrap_sek(kek: &[u8], icv: &[u8; 8], wrapped: &[u8]) -> Result<Vec<u8>> {
    let mut input = Vec::with_capacity(8 + wrapped.len());
    input.extend_from_slice(icv);
    input.extend_from_slice(wrapped);
    let mut out = vec![0u8; wrapped.len()];
    let bad_wrap = || Error::InvalidField {
        what: "AES key wrap",
        reason: "integrity check failed (wrong KEK / passphrase, or corrupt wire data)",
    };
    match kek.len() {
        16 => KekAes128::try_from(kek)
            .map_err(|_| invalid_key_length("KEK"))?
            .unwrap(&input, &mut out),
        24 => KekAes192::try_from(kek)
            .map_err(|_| invalid_key_length("KEK"))?
            .unwrap(&input, &mut out),
        32 => KekAes256::try_from(kek)
            .map_err(|_| invalid_key_length("KEK"))?
            .unwrap(&input, &mut out),
        _ => return Err(invalid_key_length("KEK")),
    }
    .map_err(|_| bad_wrap())?;
    Ok(out)
}

// ---------------------------------------------------------------------------
// §6.1.2/§6.2.2/§6.3.2 — AES-CTR payload encrypt/decrypt.
// ---------------------------------------------------------------------------

/// Compute the 128-bit AES-CTR initial counter for one data packet, per the
/// §6.2.2/§6.3.2 (Encryption/Decryption Process) formula — see the module
/// doc's "conflicting IV formula" note for why this form (and not §6.1.2's)
/// is the one implemented:
///
/// ```text
/// IV = (MSB(112, Salt) << 2) XOR (PktSeqNo)
/// ```
///
/// The resulting 112-bit `IV` occupies the counter's upper 112 bits
/// (`counter[0..14]`); the low 16 bits (`counter[14..16]`) are the
/// per-packet AES-block counter (§6.1.2), left at `0` here — a standard
/// 128-bit big-endian CTR implementation increments the *whole* counter per
/// block, which only touches these low 16 bits as long as a single packet's
/// payload stays under `2^16` AES blocks (1 MiB); every SRT payload (bounded
/// by a UDP datagram) is far smaller.
///
/// `pkt_seq_no` is the data packet's 31-bit Packet Sequence Number
/// (`draft-sharabayko-srt-01` §3.1); bit 31 (the `F` header bit) is never
/// part of it.
pub fn packet_counter(salt: &[u8; SALT_LEN], pkt_seq_no: u32) -> [u8; AES_BLOCK_LEN] {
    // MSB(112, Salt): the most-significant 14 bytes of the 128-bit Salt.
    let msb112 = &salt[..IV_SALT_LEN];

    // `<< 2` over the 112-bit big-endian value: for each byte (MSB-first),
    // the shifted-in low 2 bits come from the *next* byte's top 2 bits; the
    // top 2 bits of the whole 112-bit value are dropped (the sequence stays
    // 112 bits wide, per the draft).
    let mut iv = [0u8; IV_SALT_LEN];
    for i in 0..IV_SALT_LEN {
        let hi = msb112[i] << 2;
        let lo = if i + 1 < IV_SALT_LEN {
            msb112[i + 1] >> 6
        } else {
            0
        };
        iv[i] = hi | lo;
    }

    // XOR PktSeqNo into the low 32 bits of the 112-bit IV (PktSeqNo is a
    // 31-bit field, so this only ever touches bits [30:0] of that word).
    let low = u32::from_be_bytes([iv[10], iv[11], iv[12], iv[13]]) ^ pkt_seq_no;
    iv[10..14].copy_from_slice(&low.to_be_bytes());

    let mut counter = [0u8; AES_BLOCK_LEN];
    counter[..IV_SALT_LEN].copy_from_slice(&iv);
    // counter[14..16] stays 0 — the block counter starts at 0 for this packet.
    counter
}

/// AES-CTR encrypt/decrypt one data packet's payload **in place**
/// (`draft-sharabayko-srt-01` §6.2.2/§6.3.2). CTR mode is its own inverse —
/// `EncryptedPayload = AES_CTR_Encrypt(SEK, IV, UnencryptedPayload)` and
/// `DecryptedPayload = AES_CTR_Encrypt(SEK, IV, EncryptedPayload)` are the
/// same operation (XOR with the same keystream), so this one function serves
/// both directions.
///
/// `sek` selects AES-128/192/256 by its length (16/24/32 bytes); `salt` and
/// `pkt_seq_no` feed [`packet_counter`]. No padding is applied or expected —
/// CTR is a stream cipher (§6.1.1).
///
/// # Errors
/// [`Error::InvalidField`] if `sek.len()` is not 16, 24, or 32.
pub fn aes_ctr_apply(
    sek: &[u8],
    salt: &[u8; SALT_LEN],
    pkt_seq_no: u32,
    data: &mut [u8],
) -> Result<()> {
    let counter = packet_counter(salt, pkt_seq_no);
    match sek.len() {
        16 => {
            let mut cipher =
                Ctr128BE::<Aes128>::new_from_slices(sek, &counter).map_err(|_| bad_sek())?;
            cipher.apply_keystream(data);
        }
        24 => {
            let mut cipher =
                Ctr128BE::<Aes192>::new_from_slices(sek, &counter).map_err(|_| bad_sek())?;
            cipher.apply_keystream(data);
        }
        32 => {
            let mut cipher =
                Ctr128BE::<Aes256>::new_from_slices(sek, &counter).map_err(|_| bad_sek())?;
            cipher.apply_keystream(data);
        }
        _ => return Err(bad_sek()),
    }
    Ok(())
}

fn bad_sek() -> Error {
    invalid_key_length("SEK")
}

// ---------------------------------------------------------------------------
// §3.1 `KK` — select the active SEK by odd/even parity.
// ---------------------------------------------------------------------------

/// Select the SEK to use for a data packet from its `KK` field
/// (`draft-sharabayko-srt-01` §3.1/§6.1.6): `even`/`odd` are the two SEKs
/// currently held (both may be live during the `±`KM-Pre-Announcement-Period`
/// rekey transition window, §6.1.6).
///
/// # Errors
/// [`Error::InvalidField`] if the packet is unencrypted (`KK = NotEncrypted`)
/// or carries the control-packet-only reserved value, or if the selected
/// parity's SEK is not currently held (e.g. not yet unwrapped, or already
/// decommissioned — §6.3, step 11: such packets "must be dropped").
pub fn select_sek<'a>(
    key_flag: EncryptionKeyField,
    even: Option<&'a [u8]>,
    odd: Option<&'a [u8]>,
) -> Result<&'a [u8]> {
    let no_sek = |parity: &'static str| Error::InvalidField {
        what: "SEK",
        reason: match parity {
            "even" => "even key not currently held",
            _ => "odd key not currently held",
        },
    };
    match key_flag {
        EncryptionKeyField::Even => even.ok_or_else(|| no_sek("even")),
        EncryptionKeyField::Odd => odd.ok_or_else(|| no_sek("odd")),
        EncryptionKeyField::NotEncrypted => Err(Error::InvalidField {
            what: "KK",
            reason: "packet is not encrypted (KK=00b)",
        }),
        EncryptionKeyField::Reserved(_) => Err(Error::InvalidField {
            what: "KK",
            reason: "reserved value (11b) is control-packet-only, not valid on a data packet",
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------
    // RFC 3394 §4.1 — "Wrap 128 bits of Key Data with a 128-bit KEK".
    // <https://datatracker.ietf.org/doc/html/rfc3394#section-4.1>
    // -----------------------------------------------------------------
    const RFC3394_KEK_128: [u8; 16] = [
        0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E,
        0x0F,
    ];
    const RFC3394_KEY_DATA_128: [u8; 16] = [
        0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE,
        0xFF,
    ];
    const RFC3394_WRAPPED_128: [u8; 24] = [
        0x1F, 0xA6, 0x8B, 0x0A, 0x81, 0x12, 0xB4, 0x47, 0xAE, 0xF3, 0x4B, 0xD8, 0xFB, 0x5A, 0x7B,
        0x82, 0x9D, 0x3E, 0x86, 0x23, 0x71, 0xD2, 0xCF, 0xE5,
    ];

    #[test]
    fn rfc3394_wrap_matches_worked_vector() {
        let (icv, wrapped) = wrap_sek(&RFC3394_KEK_128, &RFC3394_KEY_DATA_128).unwrap();
        assert_eq!(&icv[..], &RFC3394_WRAPPED_128[..8]);
        assert_eq!(wrapped.as_slice(), &RFC3394_WRAPPED_128[8..]);
    }

    #[test]
    fn rfc3394_unwrap_matches_worked_vector() {
        let mut icv = [0u8; 8];
        icv.copy_from_slice(&RFC3394_WRAPPED_128[..8]);
        let recovered = unwrap_sek(&RFC3394_KEK_128, &icv, &RFC3394_WRAPPED_128[8..]).unwrap();
        assert_eq!(recovered.as_slice(), &RFC3394_KEY_DATA_128[..]);
    }

    #[test]
    fn rfc3394_unwrap_rejects_wrong_kek() {
        let mut icv = [0u8; 8];
        icv.copy_from_slice(&RFC3394_WRAPPED_128[..8]);
        let mut wrong_kek = RFC3394_KEK_128;
        wrong_kek[0] ^= 0xFF;
        assert!(unwrap_sek(&wrong_kek, &icv, &RFC3394_WRAPPED_128[8..]).is_err());
    }

    // -----------------------------------------------------------------
    // NIST SP 800-38A Appendix F.5.1 — "CTR-AES128 (Encrypt)".
    // <https://nvlpubs.nist.gov/nistpubs/Legacy/SP/nistspecialpublication800-38a.pdf>
    // -----------------------------------------------------------------
    const NIST_F5_1_KEY: [u8; 16] = [
        0x2B, 0x7E, 0x15, 0x16, 0x28, 0xAE, 0xD2, 0xA6, 0xAB, 0xF7, 0x15, 0x88, 0x09, 0xCF, 0x4F,
        0x3C,
    ];
    const NIST_F5_1_INIT_COUNTER: [u8; 16] = [
        0xF0, 0xF1, 0xF2, 0xF3, 0xF4, 0xF5, 0xF6, 0xF7, 0xF8, 0xF9, 0xFA, 0xFB, 0xFC, 0xFD, 0xFE,
        0xFF,
    ];
    const NIST_F5_1_PLAINTEXT: [u8; 64] = [
        0x6B, 0xC1, 0xBE, 0xE2, 0x2E, 0x40, 0x9F, 0x96, 0xE9, 0x3D, 0x7E, 0x11, 0x73, 0x93, 0x17,
        0x2A, 0xAE, 0x2D, 0x8A, 0x57, 0x1E, 0x03, 0xAC, 0x9C, 0x9E, 0xB7, 0x6F, 0xAC, 0x45, 0xAF,
        0x8E, 0x51, 0x30, 0xC8, 0x1C, 0x46, 0xA3, 0x5C, 0xE4, 0x11, 0xE5, 0xFB, 0xC1, 0x19, 0x1A,
        0x0A, 0x52, 0xEF, 0xF6, 0x9F, 0x24, 0x45, 0xDF, 0x4F, 0x9B, 0x17, 0xAD, 0x2B, 0x41, 0x7B,
        0xE6, 0x6C, 0x37, 0x10,
    ];
    const NIST_F5_1_CIPHERTEXT: [u8; 64] = [
        0x87, 0x4D, 0x61, 0x91, 0xB6, 0x20, 0xE3, 0x26, 0x1B, 0xEF, 0x68, 0x64, 0x99, 0x0D, 0xB6,
        0xCE, 0x98, 0x06, 0xF6, 0x6B, 0x79, 0x70, 0xFD, 0xFF, 0x86, 0x17, 0x18, 0x7B, 0xB9, 0xFF,
        0xFD, 0xFF, 0x5A, 0xE4, 0xDF, 0x3E, 0xDB, 0xD5, 0xD3, 0x5E, 0x5B, 0x4F, 0x09, 0x02, 0x0D,
        0xB0, 0x3E, 0xAB, 0x1E, 0x03, 0x1D, 0xDA, 0x2F, 0xBE, 0x03, 0xD1, 0x79, 0x21, 0x70, 0xA0,
        0xF3, 0x00, 0x9C, 0xEE,
    ];

    /// The NIST vector's counter is a raw 128-bit CTR seed, not this crate's
    /// packet-counter construction — drive `Ctr128BE` directly to validate
    /// the underlying AES-CTR primitive [`aes_ctr_apply`] wraps.
    #[test]
    fn nist_sp800_38a_f5_1_ctr_aes128_encrypt() {
        let mut buf = NIST_F5_1_PLAINTEXT;
        let mut cipher =
            Ctr128BE::<Aes128>::new_from_slices(&NIST_F5_1_KEY, &NIST_F5_1_INIT_COUNTER).unwrap();
        cipher.apply_keystream(&mut buf);
        assert_eq!(buf, NIST_F5_1_CIPHERTEXT);
    }

    #[test]
    fn nist_sp800_38a_f5_1_ctr_aes128_decrypt() {
        // CTR is self-inverse: re-applying the keystream to the ciphertext
        // recovers the plaintext.
        let mut buf = NIST_F5_1_CIPHERTEXT;
        let mut cipher =
            Ctr128BE::<Aes128>::new_from_slices(&NIST_F5_1_KEY, &NIST_F5_1_INIT_COUNTER).unwrap();
        cipher.apply_keystream(&mut buf);
        assert_eq!(buf, NIST_F5_1_PLAINTEXT);
    }

    // -----------------------------------------------------------------
    // SRT §6 payload round-trip (no spec test vectors exist for this —
    // srt-crypto.md's "No test vectors" note — so this exercises the crate's
    // own packet_counter + aes_ctr_apply against each other, not an
    // external ground truth).
    // -----------------------------------------------------------------

    #[test]
    fn srt_payload_round_trips_and_wrong_sek_does_not_recover() {
        let sek = [0x42u8; 16];
        let salt = [0x99u8; SALT_LEN];
        let pkt_seq_no = 0x0123_4567u32;
        let plaintext = b"SRT payload encryption round trip test vector.".to_vec();

        let mut encrypted = plaintext.clone();
        aes_ctr_apply(&sek, &salt, pkt_seq_no, &mut encrypted).unwrap();
        assert_ne!(encrypted, plaintext, "encryption must change the bytes");

        let mut decrypted = encrypted.clone();
        aes_ctr_apply(&sek, &salt, pkt_seq_no, &mut decrypted).unwrap();
        assert_eq!(
            decrypted, plaintext,
            "correct SEK must recover the plaintext"
        );

        let wrong_sek = [0x43u8; 16];
        let mut wrongly_decrypted = encrypted;
        aes_ctr_apply(&wrong_sek, &salt, pkt_seq_no, &mut wrongly_decrypted).unwrap();
        assert_ne!(
            wrongly_decrypted, plaintext,
            "wrong SEK must not recover the plaintext"
        );
    }

    #[test]
    fn different_seq_no_gives_different_keystream() {
        let sek = [0x11u8; 24];
        let salt = [0x22u8; SALT_LEN];
        let plaintext = [0u8; 32];

        let mut a = plaintext;
        aes_ctr_apply(&sek, &salt, 1, &mut a).unwrap();
        let mut b = plaintext;
        aes_ctr_apply(&sek, &salt, 2, &mut b).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn kek_derivation_all_sizes_and_deterministic() {
        for klen in [16usize, 24, 32] {
            let salt = [0xABu8; SALT_LEN];
            let kek1 = derive_kek(b"correct horse battery staple", &salt, klen).unwrap();
            let kek2 = derive_kek(b"correct horse battery staple", &salt, klen).unwrap();
            assert_eq!(kek1.len(), klen);
            assert_eq!(kek1, kek2, "PBKDF2 is deterministic for the same inputs");

            let different_salt = [0xACu8; SALT_LEN];
            let kek3 = derive_kek(b"correct horse battery staple", &different_salt, klen).unwrap();
            assert_ne!(kek1, kek3, "different salt must give a different KEK");
        }
    }

    #[test]
    fn invalid_klen_errs_without_panic() {
        let salt = [0u8; SALT_LEN];
        assert!(derive_kek(b"pw", &salt, 20).is_err());
        assert!(wrap_sek(&[0u8; 20], &[0u8; 16]).is_err());
        assert!(aes_ctr_apply(&[0u8; 20], &salt, 0, &mut [0u8; 4]).is_err());
    }

    #[test]
    fn select_sek_picks_correct_parity_and_rejects_bad_flags() {
        let even = [1u8; 16];
        let odd = [2u8; 16];
        assert_eq!(
            select_sek(EncryptionKeyField::Even, Some(&even), Some(&odd)).unwrap(),
            &even[..]
        );
        assert_eq!(
            select_sek(EncryptionKeyField::Odd, Some(&even), Some(&odd)).unwrap(),
            &odd[..]
        );
        assert!(select_sek(EncryptionKeyField::Even, None, Some(&odd)).is_err());
        assert!(select_sek(EncryptionKeyField::NotEncrypted, Some(&even), Some(&odd)).is_err());
        assert!(select_sek(EncryptionKeyField::Reserved(0b11), Some(&even), Some(&odd)).is_err());
    }

    #[test]
    fn both_seks_wrap_unwrap_round_trip() {
        // KK=Both: two concatenated SEKs wrapped under one KEK/ICV, matching
        // the Wrap-field length formula `n*KLen + 8` (`srt-rules.md` §3.2.2).
        let kek = [0x77u8; 16];
        let even_sek = [0xAAu8; 16];
        let odd_sek = [0xBBu8; 16];
        let mut plaintext = Vec::new();
        plaintext.extend_from_slice(&even_sek);
        plaintext.extend_from_slice(&odd_sek);

        let (icv, wrapped) = wrap_sek(&kek, &plaintext).unwrap();
        assert_eq!(wrapped.len(), 32);
        let recovered = unwrap_sek(&kek, &icv, &wrapped).unwrap();
        assert_eq!(recovered, plaintext);
        assert_eq!(&recovered[..16], &even_sek[..]);
        assert_eq!(&recovered[16..], &odd_sek[..]);
    }
}
