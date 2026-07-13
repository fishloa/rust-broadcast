//! CENC cipher core (ISO/IEC 23001-7 §10) — shared by decrypt and encrypt.
//!
//! Factors the AES sample-cipher logic out of [`crate::cenc_decrypt`] so an
//! encrypt path can reuse it verbatim: AES-128-CTR (`cenc`, ISO/IEC 23001-7
//! §10.1) is symmetric (the same keystream walk both encrypts and decrypts),
//! so [`apply_ctr`] is called by both directions unchanged. AES-128-CBC
//! pattern mode (`cbcs`, ISO/IEC 23001-7 §10.2) is *not* symmetric — CBC
//! chaining reads the ciphertext, so encrypt and decrypt need mirrored block
//! loops that differ only in which side of the block cipher call produces the
//! next chain IV — so [`cbcs_pattern`] and [`cbcs_sample`] take a [`CbcsOp`]
//! to select the direction.
//!
//! # The CBC continuous-chain rule (both directions)
//!
//! Across a sample's protected bytes, `crypt_byte_block` 16-byte blocks are
//! CBC-en/decrypted, then `skip_byte_block` 16-byte blocks are passed through
//! clear, repeating for the whole sample — across every pattern-skip run
//! **and** every subsample boundary, mirroring how `cenc`'s CTR counter also
//! runs continuously across a whole sample regardless of subsample
//! boundaries. Only the very first encrypted block of the sample is seeded
//! from the resolved IV; every subsequent encrypted block's CBC input is the
//! immediately *preceding encrypted* block's ciphertext — skipped (pattern)
//! and clear (subsample) bytes are excluded from the chain entirely, never
//! entering the cipher and never updating the chain state. A trailing
//! partial block (fewer than 16 bytes remaining in a crypt run) is left
//! clear. When both `crypt_byte_block` and `skip_byte_block` are `0` (no
//! pattern configured), the whole range is treated as one `1`:`0` run
//! (ISO/IEC 23001-7 §10.2 note).
//!
//! For **decrypt**, the next chain IV is the run's last *ciphertext* block,
//! captured *before* in-place decryption overwrites it. For **encrypt**, the
//! next chain IV is the run's last *ciphertext* block too — but ciphertext is
//! what encryption *produces*, so it is read *after* the in-place encryption
//! writes it. Both directions therefore chain on ciphertext; only the timing
//! of the read (before vs. after the block-cipher pass) differs, since
//! encrypt doesn't have the ciphertext until it computes it.
//!
//! This was verified against a real Bento4-produced `cbcs` fixture (see
//! `tests/cenc_fragmented_fixture.rs`): resetting the IV at every crypt run,
//! rather than chaining across skip runs, reproduces only each run's first
//! block correctly and diverges thereafter.
//!
//! No AES is rolled by hand: the [`aes`] + [`ctr`] + [`cbc`] RustCrypto crates
//! do the block cipher and mode work. This module is gated on the `cenc`
//! feature.

use aes::cipher::generic_array::GenericArray;
use aes::cipher::{BlockDecryptMut, BlockEncryptMut, KeyIvInit, StreamCipher};

use crate::cenc::{SampleEncryptionEntry, SubSampleEntry, TrackEncryptionBox};
use crate::error::{Error, Result};

/// AES-128 in big-endian counter mode (CENC `cenc` cipher, ISO/IEC 23001-7
/// §10.1). Symmetric: the same keystream apply-in-place both encrypts and
/// decrypts.
type Aes128Ctr = ctr::Ctr128BE<aes::Aes128>;
/// AES-128-CBC decryptor (CENC `cbcs` cipher, ISO/IEC 23001-7 §10.2).
type Aes128CbcDec = cbc::Decryptor<aes::Aes128>;
/// AES-128-CBC encryptor (CENC `cbcs` cipher, ISO/IEC 23001-7 §10.2).
type Aes128CbcEnc = cbc::Encryptor<aes::Aes128>;

/// Size of a KID / content key / AES-128 key **or block**, in bytes (AES-128's
/// key length and block length coincide).
const KEY_LEN: usize = 16;

/// Which direction [`cbcs_pattern`] / [`cbcs_sample`] runs the CBC cipher.
///
/// CTR mode ([`apply_ctr`]) needs no such parameter — XOR-with-keystream is
/// its own inverse — but CBC chaining reads ciphertext, so the two directions
/// need mirrored (not identical) block loops.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) enum CbcsOp {
    /// Turn plaintext into ciphertext. Used by the encrypt path (Task 2) and
    /// the round-trip unit tests; not yet reached by the decrypt-only callers,
    /// hence the `dead_code` allowance below until the encryptor lands.
    Encrypt,
    /// Turn ciphertext back into plaintext.
    Decrypt,
}

/// Apply the `cenc` AES-CTR cipher (ISO/IEC 23001-7 §10.1) to one sample's
/// bytes in place. Symmetric: the same call both encrypts and decrypts.
///
/// The 16-byte AES-CTR counter block is `iv`, left-justified and zero-padded
/// to 16 bytes; the low 64 bits act as the AES block counter, incrementing
/// once per 16-byte cipher block across the concatenated *protected* bytes of
/// the sample (clear subsample ranges are skipped, not counted). When
/// `subsamples` is empty the entire sample is protected.
pub(crate) fn apply_ctr(
    iv: &[u8],
    key: &[u8; KEY_LEN],
    subsamples: &[SubSampleEntry],
    data: &mut [u8],
) -> Result<()> {
    if iv.len() > KEY_LEN {
        return Err(Error::InvalidInput(
            "CENC per-sample IV longer than 16 bytes",
        ));
    }
    let mut counter = [0u8; KEY_LEN];
    counter[..iv.len()].copy_from_slice(iv);

    let mut cipher = Aes128Ctr::new(key.into(), (&counter).into());

    if subsamples.is_empty() {
        // Whole-sample encryption: the entire sample is one protected range.
        cipher.apply_keystream(data);
        return Ok(());
    }

    // Walk the subsample map, keystreaming only the protected ranges. The
    // CTR counter advances continuously across the protected bytes (the
    // clear bytes are skipped, never counted), so a single cipher instance
    // spans the whole sample.
    let mut offset = 0usize;
    for sub in subsamples {
        let clear = sub.bytes_of_clear_data as usize;
        let protected = sub.bytes_of_protected_data as usize;
        offset = offset
            .checked_add(clear)
            .ok_or(Error::InvalidInput("CENC subsample clear length overflow"))?;
        let end = offset.checked_add(protected).ok_or(Error::InvalidInput(
            "CENC subsample protected length overflow",
        ))?;
        if end > data.len() {
            return Err(Error::BufferTooShort {
                need: end,
                have: data.len(),
                what: "CENC subsample range exceeds sample",
            });
        }
        cipher.apply_keystream(&mut data[offset..end]);
        offset = end;
    }
    Ok(())
}

/// Resolve the 16-byte CBC IV for one sample's `cbcs` en/decryption
/// (ISO/IEC 23001-7 §10.2): the per-sample IV from `senc` when the track
/// carries one (`default_Per_Sample_IV_Size != 0`, or an encoder that still
/// emits per-sample IVs under `cbcs`), otherwise the track's
/// `tenc.default_constant_IV` (mandatory when `default_Per_Sample_IV_Size ==
/// 0` — ISO/IEC 23001-7 §12.2). Either form is left-justified and
/// zero-padded to 16 bytes, mirroring the `cenc` CTR counter convention.
fn resolve_cbcs_iv(
    entry: &SampleEncryptionEntry,
    tenc: &TrackEncryptionBox,
) -> Result<[u8; KEY_LEN]> {
    let src: &[u8] = if !entry.initialization_vector.is_empty() {
        &entry.initialization_vector
    } else if let Some(civ) = tenc.default_constant_iv.as_deref() {
        civ
    } else {
        return Err(Error::InvalidInput(
            "cbcs sample has no per-sample IV and tenc carries no default_constant_IV",
        ));
    };
    if src.len() > KEY_LEN {
        return Err(Error::InvalidInput("CBCS IV longer than 16 bytes"));
    }
    let mut iv = [0u8; KEY_LEN];
    iv[..src.len()].copy_from_slice(src);
    Ok(iv)
}

/// Apply the `cbcs` pattern cipher (ISO/IEC 23001-7 §10.2) to one protected
/// byte range in place, in the direction selected by `op`: en/decrypt
/// `crypt_byte_block` 16-byte blocks, then leave `skip_byte_block` 16-byte
/// blocks untouched, repeating across `range`. A trailing partial block
/// (fewer than 16 bytes remaining) is left clear.
///
/// `chain_iv` in/out: the CBC chain input for the range's first en/decrypted
/// block (typically the sample's resolved IV, or the previous range's last
/// ciphertext block when the caller threads the same `chain_iv` across
/// several ranges of one sample — see [`cbcs_sample`]). Skipped (pattern) and
/// clear (subsample) bytes are excluded from the chain entirely — they are
/// never fed to the cipher and never update `chain_iv` — so the *next*
/// en/decrypted block's CBC input is always the immediately *preceding*
/// block's ciphertext, not the fresh `chain_iv` a naive per-pattern-run
/// reading of ISO/IEC 23001-7 §10.2 might suggest (see the module docs for
/// why encrypt and decrypt both chain on ciphertext).
///
/// When both `crypt_byte_block` and `skip_byte_block` are `0` (no pattern
/// configured — full-sample protection, e.g. some `cbcs` audio tracks) the
/// entire range is treated as one `1`:`0` run (ISO/IEC 23001-7 §10.2 note).
pub(crate) fn cbcs_pattern(
    key: &[u8; KEY_LEN],
    chain_iv: &mut [u8; KEY_LEN],
    crypt_byte_block: u8,
    skip_byte_block: u8,
    range: &mut [u8],
    op: CbcsOp,
) {
    let (crypt_blocks, skip_blocks) = if crypt_byte_block == 0 && skip_byte_block == 0 {
        (1usize, 0usize)
    } else {
        (crypt_byte_block as usize, skip_byte_block as usize)
    };

    let mut offset = 0usize;
    while offset < range.len() {
        let remaining = range.len() - offset;
        let want = crypt_blocks * KEY_LEN;
        let run_len = (want.min(remaining) / KEY_LEN) * KEY_LEN;
        if run_len == 0 {
            // Fewer than one whole block remains in this crypt run: the
            // trailing partial block is left clear (CBCS pattern rule).
            break;
        }

        match op {
            CbcsOp::Decrypt => {
                // Capture this run's last ciphertext block (before it is
                // overwritten in place) to seed the chain for whatever
                // encrypted block follows — possibly across an intervening
                // skip run or subsample boundary.
                let mut next_chain = [0u8; KEY_LEN];
                next_chain.copy_from_slice(&range[offset + run_len - KEY_LEN..offset + run_len]);

                let mut dec = Aes128CbcDec::new(key.into(), (&*chain_iv).into());
                for chunk in range[offset..offset + run_len].chunks_exact_mut(KEY_LEN) {
                    let block = GenericArray::from_mut_slice(chunk);
                    dec.decrypt_block_mut(block);
                }
                *chain_iv = next_chain;
            }
            CbcsOp::Encrypt => {
                // The next chain IV is this run's last block's ciphertext —
                // but ciphertext is what encryption *produces*, so it can
                // only be read *after* the in-place encryption pass writes
                // it (unlike decrypt, which already holds the ciphertext
                // before touching the buffer).
                let mut enc = Aes128CbcEnc::new(key.into(), (&*chain_iv).into());
                for chunk in range[offset..offset + run_len].chunks_exact_mut(KEY_LEN) {
                    let block = GenericArray::from_mut_slice(chunk);
                    enc.encrypt_block_mut(block);
                }
                chain_iv.copy_from_slice(&range[offset + run_len - KEY_LEN..offset + run_len]);
            }
        }

        offset += run_len;
        if run_len < want {
            // The crypt run itself was truncated by end-of-range: nothing
            // left to skip.
            break;
        }
        offset += (skip_blocks * KEY_LEN).min(range.len() - offset);
    }
}

/// En/decrypt one sample's bytes in place, given its crypto entry + content
/// key (`cbcs` — AES-CBC pattern cipher, ISO/IEC 23001-7 §10.2), in the
/// direction selected by `op`.
///
/// Resolves the sample's chain-seed IV ([`resolve_cbcs_iv`]) then walks the
/// subsample map (or the whole sample, if unset), threading one continuous
/// [`cbcs_pattern`] chain across every subsample boundary — see the module
/// docs for the continuous-chain rule.
pub(crate) fn cbcs_sample(
    tenc: &TrackEncryptionBox,
    entry: &SampleEncryptionEntry,
    key: &[u8; KEY_LEN],
    data: &mut [u8],
    op: CbcsOp,
) -> Result<()> {
    let mut chain_iv = resolve_cbcs_iv(entry, tenc)?;
    let crypt_blocks = tenc.default_crypt_byte_block;
    let skip_blocks = tenc.default_skip_byte_block;

    if entry.subsamples.is_empty() {
        cbcs_pattern(key, &mut chain_iv, crypt_blocks, skip_blocks, data, op);
        return Ok(());
    }

    let mut offset = 0usize;
    for sub in &entry.subsamples {
        let clear = sub.bytes_of_clear_data as usize;
        let protected = sub.bytes_of_protected_data as usize;
        offset = offset
            .checked_add(clear)
            .ok_or(Error::InvalidInput("CBCS subsample clear length overflow"))?;
        let end = offset.checked_add(protected).ok_or(Error::InvalidInput(
            "CBCS subsample protected length overflow",
        ))?;
        if end > data.len() {
            return Err(Error::BufferTooShort {
                need: end,
                have: data.len(),
                what: "CBCS subsample range exceeds sample",
            });
        }
        cbcs_pattern(
            key,
            &mut chain_iv,
            crypt_blocks,
            skip_blocks,
            &mut data[offset..end],
            op,
        );
        offset = end;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: [u8; KEY_LEN] = [
        0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F,
        0x10,
    ];
    const IV8: [u8; 8] = [0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88];

    /// CTR: encrypt then decrypt with the same iv/key/subsamples returns the
    /// original plaintext (CTR is its own inverse).
    #[test]
    fn ctr_encrypt_then_decrypt_round_trips() {
        let plaintext: Vec<u8> = (0u8..97).collect(); // spans several 16B blocks + a partial one
        let subsamples = alloc::vec![
            SubSampleEntry {
                bytes_of_clear_data: 5,
                bytes_of_protected_data: 32,
            },
            SubSampleEntry {
                bytes_of_clear_data: 3,
                bytes_of_protected_data: 57,
            },
        ];

        let mut buf = plaintext.clone();
        apply_ctr(&IV8, &KEY, &subsamples, &mut buf).unwrap();
        assert_ne!(
            buf, plaintext,
            "encryption should change the protected bytes"
        );

        // Same call decrypts (CTR keystream XOR is its own inverse).
        apply_ctr(&IV8, &KEY, &subsamples, &mut buf).unwrap();
        assert_eq!(buf, plaintext);
    }

    /// CBCS: encrypt a multi-block range with a non-trivial 1:9 pattern, then
    /// decrypt with the same key/iv/pattern, recovering the original bytes.
    /// Length exercises: several full crypt/skip runs plus a trailing partial
    /// (<16B) crypt block, which must be left clear by both directions.
    #[test]
    fn cbcs_encrypt_then_decrypt_round_trips_with_pattern_and_trailing_partial() {
        // Pattern 1:9 -> each run is 1 crypt block (16B) + 9 skip blocks (144B) = 160B.
        // Use 2 full runs (320B) plus a partial 40-byte tail (< 1 crypt block skip
        // territory) so the trailing bytes exercise the "final block only if
        // whole" leftover-clear rule inside a crypt run boundary.
        const CRYPT_BLOCKS: u8 = 1;
        const SKIP_BLOCKS: u8 = 9;
        let plaintext: Vec<u8> = (0u8..=255).cycle().take(320 + 10).collect();

        let tenc = TrackEncryptionBox {
            version: 1,
            default_crypt_byte_block: CRYPT_BLOCKS,
            default_skip_byte_block: SKIP_BLOCKS,
            default_is_protected: 1,
            default_per_sample_iv_size: 16,
            default_kid: [0u8; KEY_LEN],
            default_constant_iv: None,
        };
        let entry = SampleEncryptionEntry {
            initialization_vector: IV8.to_vec(),
            subsamples: Vec::new(),
        };

        let mut buf = plaintext.clone();
        cbcs_sample(&tenc, &entry, &KEY, &mut buf, CbcsOp::Encrypt).unwrap();
        assert_ne!(
            buf, plaintext,
            "encryption should change the protected blocks"
        );

        cbcs_sample(&tenc, &entry, &KEY, &mut buf, CbcsOp::Decrypt).unwrap();
        assert_eq!(buf, plaintext);
    }
}
