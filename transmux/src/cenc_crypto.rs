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
//! # The CBC chain rule (both directions)
//!
//! *Within* one subsample's protected range (or the whole sample, when there
//! is no subsample map), `crypt_byte_block` 16-byte blocks are CBC-en/decrypted,
//! then `skip_byte_block` 16-byte blocks are passed through clear, repeating
//! across every pattern-skip run in that range — this part mirrors how
//! `cenc`'s CTR counter advances continuously across a whole sample. Only the
//! range's very first encrypted block is seeded from the resolved IV; every
//! subsequent encrypted block's CBC input is the immediately *preceding
//! encrypted* block's ciphertext — skipped (pattern) bytes are excluded from
//! the chain entirely, never entering the cipher and never updating the
//! chain state. A trailing partial block (fewer than 16 bytes remaining in a
//! crypt run) is left clear. When both `crypt_byte_block` and
//! `skip_byte_block` are `0` (no pattern configured), the whole range is
//! treated as one `1`:`0` run (ISO/IEC 23001-7 §10.2 note).
//!
//! **The chain does NOT carry across subsample boundaries**: [`cbcs_sample`]
//! resets the chain to the sample's resolved seed IV at the start of *every*
//! subsample's protected range (each subsample gets its own fresh
//! [`cbcs_pattern`] call seeded from the same IV). This was triangulated
//! against Bento4's `mp4decrypt` reference decryptor (empirically, with an
//! independent AES-CBC re-derivation) and Shaka Packager's `cbcs` behaviour —
//! ISO/IEC 23001-7 itself is not owned by this project, so the reference
//! implementations are the source of truth here. An earlier version of this
//! module carried the chain continuously across subsample boundaries too;
//! that reproduced only the *first* protected subsample of a multi-subsample
//! sample correctly and diverged from Bento4 on every subsequent subsample's
//! first crypt block, while still round-tripping through this crate's own
//! encrypt/decrypt pair (self-consistent, but not spec/interop-correct) — see
//! `tests/cenc_encrypt_e2e.rs`'s `cbcs` case (a real per-NAL subsample map)
//! for the regression coverage. The whole-sample case (an empty subsample
//! map) is unaffected either way: with no subsample boundary to cross, there
//! was never more than one chain in play. This is also consistent with the
//! real fixture in `tests/cenc_fragmented_fixture.rs` (Bento4-produced),
//! whose samples each carry exactly one subsample — a fixture that alone
//! cannot distinguish "resets every subsample" from "never resets", since it
//! never has more than one crypt-bearing subsample per sample to cross a
//! boundary between.
//!
//! For **decrypt**, the next chain IV is the run's last *ciphertext* block,
//! captured *before* in-place decryption overwrites it. For **encrypt**, the
//! next chain IV is the run's last *ciphertext* block too — but ciphertext is
//! what encryption *produces*, so it is read *after* the in-place encryption
//! writes it. Both directions therefore chain on ciphertext; only the timing
//! of the read (before vs. after the block-cipher pass) differs, since
//! encrypt doesn't have the ciphertext until it computes it.
//!
//! No AES is rolled by hand: the [`aes`] + [`ctr`] + [`cbc`] RustCrypto crates
//! do the block cipher and mode work. This module is gated on the `cenc`
//! feature.

use aes::cipher::generic_array::GenericArray;
use aes::cipher::{BlockDecryptMut, BlockEncryptMut, KeyIvInit, StreamCipher};
use bytes::{Bytes, BytesMut};

use crate::cenc::{SampleEncryptionEntry, SubSampleEntry, TrackEncryptionBox};
use crate::error::{Error, Result};

/// Rewrite one sample's [`Bytes`] in place (media plane step 2b, G12 —
/// `docs/superpowers/specs/2026-07-26-media-plane-architecture.md` §4).
///
/// `Bytes` is immutable/shared, so an in-place cipher pass needs a mutable
/// buffer. Takes the zero-copy [`Bytes::try_into_mut`] fast path when the
/// caller holds the only reference to the sample's storage — the common case
/// for a sample encrypted/decrypted before it is fanned out to any other
/// consumer — and falls back to exactly one defensive copy only when the
/// buffer is genuinely shared. Returns whether the fast path was taken, so
/// callers can assert it rather than trust it (never guess-and-hope: see
/// `cenc_encrypt.rs`'s/`cenc_decrypt.rs`'s tests).
///
/// # Contract on `f`: validate, *then* mutate
///
/// Rewriting in place means there is no pristine copy to roll back to, so `f`
/// **must** finish every check that can fail before it writes its first byte.
/// Every cipher entry point in this module honours that: [`apply_ctr`] and
/// [`cbcs_sample`] run [`validate_subsample_map`] (plus their IV checks) to
/// completion up front, after which their block loops are infallible and
/// panic-free — every index has already been bounds-checked. So on `Err` the
/// sample's bytes are returned **unmodified**, which matters because the
/// alternative is committing a half-encrypted payload (e.g. a malformed `senc`
/// that overruns on its *second* subsample, having already keystreamed the
/// first) and presenting it as either plaintext or ciphertext.
///
/// `data` is always restored, `Ok` or `Err` — it is never left empty. A
/// **panic** inside `f` is the one case that does leave `*data` empty (the
/// [`core::mem::take`] below has no unwind guard, and the buffer is dropped
/// during unwind); the cipher core is panic-free by construction as described
/// above, and the crate's fuzz targets cover the parse paths that feed it, so
/// this is documented rather than paid for on every sample.
pub(crate) fn rewrite_in_place(
    data: &mut Bytes,
    f: impl FnOnce(&mut [u8]) -> Result<()>,
) -> Result<bool> {
    let owned = core::mem::take(data);
    let (mut buf, fast_path) = match owned.try_into_mut() {
        Ok(buf) => (buf, true),
        Err(shared) => (BytesMut::from(&shared[..]), false),
    };
    let result = f(&mut buf);
    // Restore the sample's storage either way (never leave `*data` empty); on
    // `Err` the contract above guarantees `buf` still holds the original bytes.
    *data = buf.freeze();
    result.map(|()| fast_path)
}

/// Validate one sample's subsample map against the sample's own length —
/// ISO/IEC 23001-7 §9.3 — **before** any cipher pass mutates a byte.
///
/// Walks the `(clear, protected)` runs exactly as the cipher loops do and
/// rejects three things:
///
/// - an offset/length addition that would overflow `usize`,
/// - a range running past the end of the sample, and
/// - a map that does not cover the sample **exactly**: §9.3 requires the
///   subsample structure to account for every byte of the sample, so a map
///   declaring (say) 100 bytes of a 1000-byte sample must not be accepted —
///   the remaining 900 bytes would be left as ciphertext and handed back as
///   plaintext (or vice versa on encrypt) with an `Ok`.
///
/// Running this to completion first is what lets [`rewrite_in_place`] promise
/// that a failed cipher call leaves the sample's bytes untouched: after this
/// returns `Ok`, the block loops cannot fail (or panic — every index is
/// already proven in range).
fn validate_subsample_map(subsamples: &[SubSampleEntry], data_len: usize) -> Result<()> {
    let mut offset = 0usize;
    for sub in subsamples {
        offset = offset
            .checked_add(sub.bytes_of_clear_data as usize)
            .ok_or(Error::InvalidInput("CENC subsample clear length overflow"))?;
        let end = offset
            .checked_add(sub.bytes_of_protected_data as usize)
            .ok_or(Error::InvalidInput(
                "CENC subsample protected length overflow",
            ))?;
        if end > data_len {
            return Err(Error::BufferTooShort {
                need: end,
                have: data_len,
                what: "CENC subsample range exceeds sample",
            });
        }
        offset = end;
    }
    if offset != data_len {
        return Err(Error::InvalidInput(
            "CENC subsample map does not cover the whole sample (ISO/IEC 23001-7 §9.3): the \
             uncovered bytes would be passed through unprotected",
        ));
    }
    Ok(())
}

/// Valid CENC IV lengths, on either the encrypt or decrypt path — ISO/IEC
/// 23001-7 §9.2 (`cenc` per-sample IV)/§12.2 (`cbcs` per-sample or constant
/// IV) permit exactly 8 or 16 bytes, no other length. Both are left-justified
/// and zero-padded to a 16-byte block before use (see [`apply_ctr`] /
/// [`resolve_cbcs_iv`]), so two different *invalid* lengths that happen to
/// zero-pad to the same block (e.g. a 12-byte and an empty IV both padding
/// toward all but their non-zero prefix) could otherwise collide — tightened
/// to exactly this pair rather than "anything up to 16 bytes" so that cannot
/// happen. Shared by the encrypt and decrypt paths so both enforce the same
/// bound; the decrypt path applies this to untrusted wire input.
const VALID_IV_LENS: [usize; 2] = [8, 16];

/// Reject an IV length that is not exactly 8 or 16 bytes.
fn check_iv_len(len: usize) -> Result<()> {
    if !VALID_IV_LENS.contains(&len) {
        return Err(Error::InvalidValue {
            field: "CENC IV length",
            value: len as u64,
            reason: "ISO/IEC 23001-7 §9.2/§12.2 permit only 8 or 16 bytes",
        });
    }
    Ok(())
}

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
pub(crate) enum CbcsOp {
    /// Turn plaintext into ciphertext. Used by the encrypt path
    /// ([`crate::cenc_encrypt::CencEncryptor`]) and the round-trip unit tests.
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
///
/// # An empty `iv` is rejected, not zero-padded
///
/// `cenc` derives its counter block from the per-sample IV alone — unlike
/// `cbcs`, this cipher is never given the track's `tenc`, and ISO/IEC 23001-7
/// §10.1/§12.2 gives it nothing else to fall back on: a `cenc` track carries a
/// real 8- or 16-byte IV per sample in `senc`, and `default_constant_IV` is a
/// `cbcs` construct. So an empty IV can only mean one of two broken things,
/// both of which must fail loudly:
///
/// - **encrypting**, it would build an all-zero counter block reused for every
///   sample of the track — one keystream for the whole track (a two-time pad),
///   and output no conformant decryptor can read;
/// - **decrypting**, it would "decrypt" a file whose `tenc` declares
///   `default_per_sample_iv_size == 0` plus a `default_constant_IV` against
///   that same all-zero counter and return `Ok` over garbage. Silent wrong
///   output is worse than a failure, so this returns an error instead.
pub(crate) fn apply_ctr(
    iv: &[u8],
    key: &[u8; KEY_LEN],
    subsamples: &[SubSampleEntry],
    data: &mut [u8],
) -> Result<()> {
    if iv.is_empty() {
        return Err(Error::InvalidInput(
            "cenc (AES-CTR) sample has no per-sample IV: an all-zero counter block would reuse \
             one keystream for every sample. cenc requires a per-sample IV in senc — a \
             tenc.default_constant_IV (default_per_sample_iv_size == 0) is cbcs-only",
        ));
    }
    check_iv_len(iv.len())?;
    // Validate the whole subsample map before touching a byte, so a malformed
    // map cannot leave a half-keystreamed sample behind (see
    // `rewrite_in_place`'s contract and `validate_subsample_map`).
    if !subsamples.is_empty() {
        validate_subsample_map(subsamples, data.len())?;
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
    // spans the whole sample. Every range below is already proven in bounds
    // by `validate_subsample_map`, so this loop cannot fail or panic.
    let mut offset = 0usize;
    for sub in subsamples {
        offset += sub.bytes_of_clear_data as usize;
        let end = offset + sub.bytes_of_protected_data as usize;
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
    check_iv_len(src.len())?;
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
/// `chain_iv` in/out: the CBC chain input for `range`'s first en/decrypted
/// block — the caller ([`cbcs_sample`]) always seeds this from the sample's
/// resolved IV fresh for every subsample's `range` (see the module docs for
/// why the chain resets per subsample rather than carrying over). *Within*
/// this one call, skipped (pattern) bytes are excluded from the chain
/// entirely — they are never fed to the cipher and never update `chain_iv` —
/// so the *next* en/decrypted block's CBC input is always the immediately
/// *preceding* block's ciphertext, not the fresh `chain_iv` a naive
/// per-pattern-run reading of ISO/IEC 23001-7 §10.2 might suggest (see the
/// module docs for why encrypt and decrypt both chain on ciphertext).
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
                // encrypted block follows later in this same range —
                // possibly across an intervening skip run (never across a
                // subsample boundary: each subsample gets its own fresh
                // `cbcs_pattern` call in `cbcs_sample`).
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
/// subsample map (or the whole sample, if unset), running one fresh
/// [`cbcs_pattern`] call *per subsample*, each reseeded from the same
/// resolved IV — see the module docs for why the chain resets at every
/// subsample boundary rather than carrying over.
pub(crate) fn cbcs_sample(
    tenc: &TrackEncryptionBox,
    entry: &SampleEncryptionEntry,
    key: &[u8; KEY_LEN],
    data: &mut [u8],
    op: CbcsOp,
) -> Result<()> {
    let crypt_blocks = tenc.default_crypt_byte_block;
    let skip_blocks = tenc.default_skip_byte_block;
    // `crypt_byte_block == 0` with a nonzero `skip_byte_block` is not the
    // "no pattern configured" case (that's 0:0, remapped to a 1:0 whole-range
    // run by `cbcs_pattern`) — it is a pattern whose crypt run length is zero,
    // so `cbcs_pattern`'s `want` computes to 0 and its loop's very first
    // `run_len` is `0`, breaking immediately and leaving the whole range
    // clear while `tenc.default_is_protected` still claims it is protected
    // (ISO/IEC 23001-7 §12.2). Reject rather than silently ship/accept
    // unprotected data — reachable both when building `tenc` for encryption
    // and when parsing an untrusted file's `tenc` for decryption.
    if crypt_blocks == 0 && skip_blocks != 0 {
        return Err(Error::InvalidInput(
            "cbcs pattern crypt_byte_block=0 with nonzero skip leaves data unprotected",
        ));
    }
    let seed_iv = resolve_cbcs_iv(entry, tenc)?;
    // Validate the whole subsample map before touching a byte (see
    // `rewrite_in_place`'s contract and `validate_subsample_map`) — including
    // that it covers the sample exactly, so no tail of the sample is silently
    // passed through unprotected/undecrypted.
    if !entry.subsamples.is_empty() {
        validate_subsample_map(&entry.subsamples, data.len())?;
    }

    if entry.subsamples.is_empty() {
        let mut chain_iv = seed_iv;
        cbcs_pattern(key, &mut chain_iv, crypt_blocks, skip_blocks, data, op);
        return Ok(());
    }

    // Every range below is already proven in bounds by
    // `validate_subsample_map`, so this loop cannot fail or panic.
    let mut offset = 0usize;
    for sub in &entry.subsamples {
        offset += sub.bytes_of_clear_data as usize;
        let end = offset + sub.bytes_of_protected_data as usize;
        // Reset the chain to the sample's seed IV at the start of every
        // subsample's protected range (see the module docs) — within this
        // one subsample, `cbcs_pattern` still chains correctly across its own
        // crypt/skip runs.
        let mut chain_iv = seed_iv;
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

    /// A `tenc` with `default_crypt_byte_block == 0` and a nonzero
    /// `default_skip_byte_block` (e.g. a hostile or malformed file) must be
    /// rejected on the **decrypt** path too — without this guard,
    /// `cbcs_pattern`'s crypt-run length computes to 0, its loop breaks
    /// immediately, and the "protected" range is returned untouched
    /// (ciphertext masquerading as plaintext) while `default_is_protected`
    /// still claims protection.
    #[test]
    fn cbcs_sample_decrypt_rejects_zero_crypt_nonzero_skip() {
        let tenc = TrackEncryptionBox {
            version: 1,
            default_crypt_byte_block: 0,
            default_skip_byte_block: 9,
            default_is_protected: 1,
            default_per_sample_iv_size: 8,
            default_kid: [0u8; KEY_LEN],
            default_constant_iv: None,
        };
        let entry = SampleEncryptionEntry {
            initialization_vector: IV8.to_vec(),
            subsamples: Vec::new(),
        };
        let mut data: Vec<u8> = (0u8..64).collect();
        let err = cbcs_sample(&tenc, &entry, &KEY, &mut data, CbcsOp::Decrypt).unwrap_err();
        assert!(matches!(err, Error::InvalidInput(_)));
    }

    /// `cenc` (AES-CTR) with **no** per-sample IV must error rather than build
    /// an all-zero counter block. On the decrypt side this is a real
    /// conformant-file case: `tenc.default_per_sample_iv_size == 0` +
    /// `default_constant_IV` (a `cbcs` construct) yields empty `senc` IVs, and
    /// the `cenc` path is never handed `tenc`, so without this guard it would
    /// return `Ok(())` over garbage.
    #[test]
    fn ctr_rejects_empty_iv() {
        let mut data: Vec<u8> = (0u8..64).collect();
        let untouched = data.clone();
        let err = apply_ctr(&[], &KEY, &[], &mut data).unwrap_err();
        assert!(matches!(err, Error::InvalidInput(_)));
        assert_eq!(data, untouched, "a rejected call must not cipher anything");
    }

    /// F3: `apply_ctr` must reject any IV length other than 8 or 16 bytes —
    /// not just empty or `> 16` (ISO/IEC 23001-7 §9.2 permits no other
    /// length). A 12-byte IV used to be silently zero-padded to a 16-byte
    /// counter block indistinguishable from a genuine (differently-valued)
    /// 8- or 16-byte IV that happened to share the same non-zero bytes.
    #[test]
    fn ctr_rejects_non_8_or_16_byte_iv() {
        for len in [1usize, 7, 9, 12, 15, 17, 20] {
            let iv = alloc::vec![0x42u8; len];
            let mut data: Vec<u8> = (0u8..64).collect();
            let untouched = data.clone();
            let err = apply_ctr(&iv, &KEY, &[], &mut data).unwrap_err();
            assert!(
                matches!(err, Error::InvalidValue { .. }),
                "len {len}: expected InvalidValue, got {err:?}"
            );
            assert_eq!(
                data, untouched,
                "len {len}: a rejected call must not cipher anything"
            );
        }
    }

    /// F3: `cbcs_sample`/`resolve_cbcs_iv` must reject the same non-8/16-byte
    /// lengths, whether the IV comes from a per-sample `senc` entry or from
    /// `tenc.default_constant_IV` — both are validated by the same shared
    /// `check_iv_len`.
    #[test]
    fn cbcs_rejects_non_8_or_16_byte_iv() {
        let tenc = TrackEncryptionBox {
            version: 1,
            default_crypt_byte_block: 1,
            default_skip_byte_block: 9,
            default_is_protected: 1,
            default_per_sample_iv_size: 12,
            default_kid: [0u8; KEY_LEN],
            default_constant_iv: None,
        };
        let entry = SampleEncryptionEntry {
            initialization_vector: alloc::vec![0x42u8; 12],
            subsamples: Vec::new(),
        };
        let mut data: Vec<u8> = (0u8..64).collect();
        let err = cbcs_sample(&tenc, &entry, &KEY, &mut data, CbcsOp::Decrypt).unwrap_err();
        assert!(matches!(err, Error::InvalidValue { .. }), "got {err:?}");

        // Same rejection when the 12-byte IV instead comes from
        // `tenc.default_constant_IV` (empty per-sample IV, constant-IV
        // fallback).
        let tenc_constant = TrackEncryptionBox {
            default_per_sample_iv_size: 0,
            default_constant_iv: Some(alloc::vec![0x42u8; 12]),
            ..tenc
        };
        let entry_empty = SampleEncryptionEntry {
            initialization_vector: Vec::new(),
            subsamples: Vec::new(),
        };
        let mut data2: Vec<u8> = (0u8..64).collect();
        let err = cbcs_sample(
            &tenc_constant,
            &entry_empty,
            &KEY,
            &mut data2,
            CbcsOp::Decrypt,
        )
        .unwrap_err();
        assert!(matches!(err, Error::InvalidValue { .. }), "got {err:?}");
    }

    /// A subsample map covering only part of the sample must error — ISO/IEC
    /// 23001-7 §9.3 requires it to account for every byte. Without the check,
    /// the uncovered tail is handed back untouched: on decrypt, 900 bytes of
    /// ciphertext presented as plaintext with an `Ok`.
    #[test]
    fn ctr_rejects_partial_subsample_coverage() {
        let mut data: Vec<u8> = (0u8..=255).cycle().take(1000).collect();
        let untouched = data.clone();
        let subsamples = alloc::vec![SubSampleEntry {
            bytes_of_clear_data: 4,
            bytes_of_protected_data: 96,
        }];
        let err = apply_ctr(&IV8, &KEY, &subsamples, &mut data).unwrap_err();
        assert!(matches!(err, Error::InvalidInput(_)), "got {err:?}");
        assert_eq!(
            data, untouched,
            "the sample must be left byte-identical, not partially keystreamed"
        );
    }

    /// The same coverage rule on the `cbcs` path.
    #[test]
    fn cbcs_rejects_partial_subsample_coverage() {
        let tenc = TrackEncryptionBox {
            version: 1,
            default_crypt_byte_block: 1,
            default_skip_byte_block: 9,
            default_is_protected: 1,
            default_per_sample_iv_size: 8,
            default_kid: [0u8; KEY_LEN],
            default_constant_iv: None,
        };
        let entry = SampleEncryptionEntry {
            initialization_vector: IV8.to_vec(),
            subsamples: alloc::vec![SubSampleEntry {
                bytes_of_clear_data: 4,
                bytes_of_protected_data: 96,
            }],
        };
        let mut data: Vec<u8> = (0u8..=255).cycle().take(1000).collect();
        let untouched = data.clone();
        let err = cbcs_sample(&tenc, &entry, &KEY, &mut data, CbcsOp::Decrypt).unwrap_err();
        assert!(matches!(err, Error::InvalidInput(_)), "got {err:?}");
        assert_eq!(data, untouched, "the sample must be left byte-identical");
    }

    /// **The half-encrypted-payload regression test.** A `senc` subsample map
    /// whose *second* subsample overruns the sample is exactly the shape that
    /// used to keystream the first subsample, fail on the second, and still
    /// commit the half-encrypted buffer into `sample.data`. The cipher core now
    /// validates the whole map first, so the sample comes back byte-identical
    /// — verified here through [`rewrite_in_place`] (i.e. on a real
    /// `Bytes`-backed sample, both schemes).
    #[test]
    fn second_subsample_overrun_leaves_the_sample_unchanged() {
        let plaintext: Vec<u8> = (0u8..=255).cycle().take(200).collect();
        let overrunning = alloc::vec![
            SubSampleEntry {
                bytes_of_clear_data: 4,
                bytes_of_protected_data: 60, // fine on its own
            },
            SubSampleEntry {
                bytes_of_clear_data: 4,
                bytes_of_protected_data: 4096, // runs past the 200-byte sample
            },
        ];
        let tenc = TrackEncryptionBox {
            version: 1,
            default_crypt_byte_block: 1,
            default_skip_byte_block: 9,
            default_is_protected: 1,
            default_per_sample_iv_size: 8,
            default_kid: [0u8; KEY_LEN],
            default_constant_iv: None,
        };
        let entry = SampleEncryptionEntry {
            initialization_vector: IV8.to_vec(),
            subsamples: overrunning.clone(),
        };

        for label in ["cenc", "cbcs"] {
            let mut data = Bytes::from(plaintext.clone());
            let err = rewrite_in_place(&mut data, |buf| {
                if label == "cenc" {
                    apply_ctr(&IV8, &KEY, &overrunning, buf)
                } else {
                    cbcs_sample(&tenc, &entry, &KEY, buf, CbcsOp::Encrypt)
                }
            })
            .expect_err("an overrunning subsample map must be rejected");
            assert!(
                matches!(err, Error::BufferTooShort { .. }),
                "{label}: {err:?}"
            );
            assert_eq!(
                &data[..],
                &plaintext[..],
                "{label}: sample.data must be unchanged — not half-encrypted, not empty"
            );
        }
    }

    /// A failing closure must never leave `sample.data` **empty**: the
    /// `mem::take` inside [`rewrite_in_place`] hands the storage to the closure,
    /// and the buffer has to be put back on the error path too.
    #[test]
    fn rewrite_in_place_restores_the_buffer_on_err() {
        let original: Vec<u8> = (0u8..32).collect();
        let mut data = Bytes::from(original.clone());
        let err = rewrite_in_place(&mut data, |_buf| {
            Err(Error::InvalidInput("closure failed before mutating"))
        })
        .expect_err("the closure's error must propagate");
        assert!(matches!(err, Error::InvalidInput(_)));
        assert_eq!(
            &data[..],
            &original[..],
            "a validate-then-mutate closure's failure must leave the sample intact"
        );
    }

    // -----------------------------------------------------------------
    // `rewrite_in_place` (media plane step 2b, G12) — the try_into_mut
    // fast-path mechanism itself. Proved directly here (not trusted): the
    // MANDATORY MEASUREMENT in `tests/alloc_measurement.rs` covers the
    // whole-pipeline allocation count; these two prove the *mechanism*
    // picks the right branch and never corrupts a genuinely shared buffer.
    // -----------------------------------------------------------------

    /// A uniquely-owned `Bytes` (refcount 1, the common case — a sample that
    /// hasn't been fanned out to any other consumer yet) must take the
    /// zero-copy `try_into_mut` fast path.
    #[test]
    fn rewrite_in_place_takes_fast_path_when_unique() {
        let mut data = Bytes::from(alloc::vec![1u8, 2, 3, 4]);
        let took_fast_path = rewrite_in_place(&mut data, |buf| {
            buf[0] = 0xFF;
            Ok(())
        })
        .expect("rewrite ok");
        assert!(
            took_fast_path,
            "uniquely-owned Bytes must take the zero-copy try_into_mut fast path"
        );
        assert_eq!(&data[..], &[0xFF, 2, 3, 4]);
    }

    /// A shared `Bytes` (another handle holds a clone — refcount > 1) must
    /// NOT take the fast path, and the fallback copy must not corrupt the
    /// other holder's view (proving the copy-on-write branch is genuinely
    /// safe, not just "doesn't panic").
    #[test]
    fn rewrite_in_place_copies_and_leaves_other_holder_untouched_when_shared() {
        let original = Bytes::from(alloc::vec![9u8, 9, 9, 9]);
        let mut shared_handle = original.clone(); // refcount 2: original + shared_handle
        let took_fast_path = rewrite_in_place(&mut shared_handle, |buf| {
            buf[0] = 0x00;
            Ok(())
        })
        .expect("rewrite ok");
        assert!(
            !took_fast_path,
            "shared Bytes must not take the fast path — that would alias/corrupt the other holder"
        );
        assert_eq!(
            &original[..],
            &[9, 9, 9, 9],
            "the other holder's bytes must be untouched by shared_handle's rewrite"
        );
        assert_eq!(
            &shared_handle[..],
            &[0x00, 9, 9, 9],
            "the rewriting handle's own view must reflect the mutation"
        );
    }
}
