//! HLS Sample-AES + full-segment AES-128 content protection.
//!
//! Apple's HLS-native encryption (distinct from ISO/IEC 23001-7 CENC): either
//! the whole segment is AES-128-CBC encrypted (`METHOD=AES-128`) or each
//! audio/video sample carries a clear leader with the remainder encrypted in a
//! 16-byte-block skip pattern (`METHOD=SAMPLE-AES`).
//!
//! # Byte layouts
//!
//! | Stream        | Clear prefix                 | Encrypted region                        |
//! |---------------|------------------------------|-----------------------------------------|
//! | H.264 NAL 1/5 | 1 hdr byte + 31 payload (32) | 16-byte block, then ≤144 clear (~10%)   |
//! | AAC (ADTS)    | ADTS header + 16-byte leader | 16-byte CBC blocks, `<16` trailer clear |
//! | AC-3 / E-AC-3 | 16-byte leader               | 16-byte CBC blocks, `<16` trailer clear |
//! | AES-128       | none (whole segment)         | AES-128-CBC over all bytes, PKCS#7 pad   |
//!
//! # Spec citations
//!
//! - **Apple "MPEG-2 Stream Encryption Format for HTTP Live Streaming"** — the
//!   Sample-AES sample byte patterns and IV rules; transcribed in
//!   `transmux/docs/drm/hls-sample-aes.md` (§2 AES-128, §3 H.264, §4 AAC,
//!   §5 AC-3, §6 E-AC-3, §9 EXT-X-KEY, §10 IV derivation, §11 cipher details).
//! - **ffmpeg `libavformat/hls_sample_aes.c`** — the CBC chaining the H.264 and
//!   E-AC-3 fixtures pin (chained across a NAL's blocks, reset per NAL; reset
//!   per E-AC-3 syncframe). See `tests/fixtures/ORACLES.md`.
//! - **RFC 8216 §4.3.2.4** — the `EXT-X-KEY` tag (`METHOD`/`URI`/`IV`/
//!   `KEYFORMAT`/`KEYFORMATVERSIONS`).
//! - **NIST SP 800-38A** — AES-128-CBC.
//!
//! No AES is rolled by hand: the [`aes`] block cipher + the [`cbc`] mode crate
//! do the work. The whole module is gated on the `sample-aes` feature; the
//! default `no_std` core build carries no crypto.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use aes::cipher::{
    BlockDecryptMut, BlockEncryptMut, KeyIvInit, block_padding::Pkcs7, generic_array::GenericArray,
};

use broadcast_hls::AttrValue;

use crate::error::{Error, Result};

/// AES-128-CBC encryptor (NIST SP 800-38A).
type Aes128CbcEnc = cbc::Encryptor<aes::Aes128>;
/// AES-128-CBC decryptor (NIST SP 800-38A).
type Aes128CbcDec = cbc::Decryptor<aes::Aes128>;

/// AES-128 key length, in bytes (`docs/drm/hls-sample-aes.md` §11).
pub const KEY_LEN: usize = 16;
/// AES block size, in bytes (`docs/drm/hls-sample-aes.md` §11).
pub const BLOCK_LEN: usize = 16;

// --- H.264 SAMPLE-AES (docs/drm/hls-sample-aes.md §3) ---------------------

/// H.264 NAL unit type carried in the low 5 bits of the NAL header byte:
/// coded slice of a non-IDR picture (`docs/drm/hls-sample-aes.md` §3.1).
pub const NAL_TYPE_NON_IDR_SLICE: u8 = 1;
/// H.264 NAL unit type: coded slice of an IDR picture
/// (`docs/drm/hls-sample-aes.md` §3.1).
pub const NAL_TYPE_IDR_SLICE: u8 = 5;
/// Mask selecting the 5-bit `nal_unit_type` from the NAL header byte.
pub const NAL_TYPE_MASK: u8 = 0x1F;
/// Minimum NAL length (bytes, header included) for encryption to apply; NALs of
/// this length or shorter are left entirely clear (`docs/drm/hls-sample-aes.md` §3.1).
pub const H264_MIN_ENCRYPTED_NAL_LEN: usize = 48;
/// Clear prefix of an encrypted H.264 NAL: 1 NAL-header byte + 31 payload bytes
/// (`docs/drm/hls-sample-aes.md` §3.2).
pub const H264_CLEAR_PREFIX_LEN: usize = 32;
/// Bytes left clear after each encrypted block: up to 9 blocks = 144 bytes
/// (`docs/drm/hls-sample-aes.md` §3.3).
pub const H264_SKIP_LEN: usize = 144;

// --- Audio SAMPLE-AES (docs/drm/hls-sample-aes.md §4–§6) ------------------

/// Clear leader after the ADTS header for AAC / after the syncframe start for
/// AC-3 / E-AC-3 (`docs/drm/hls-sample-aes.md` §4.1, §5, §6).
pub const AUDIO_CLEAR_LEADER_LEN: usize = 16;
/// ADTS fixed+variable header length with no CRC (`docs/drm/hls-sample-aes.md` §4.1).
pub const ADTS_HEADER_LEN_NO_CRC: usize = 7;
/// ADTS header length with CRC protection (`docs/drm/hls-sample-aes.md` §4.1).
pub const ADTS_HEADER_LEN_WITH_CRC: usize = 9;

// --------------------------------------------------------------------------
// Raw AES-128-CBC primitives.
// --------------------------------------------------------------------------

/// Encrypt whole 16-byte blocks of `data` in place with AES-128-CBC.
///
/// `data.len()` must be a non-zero multiple of [`BLOCK_LEN`]; any trailing
/// partial block is the caller's responsibility (Sample-AES leaves it clear).
fn cbc_encrypt_blocks_in_place(key: &[u8; KEY_LEN], iv: &[u8; BLOCK_LEN], data: &mut [u8]) {
    if data.is_empty() {
        return;
    }
    let mut enc = Aes128CbcEnc::new(key.into(), iv.into());
    for chunk in data.chunks_exact_mut(BLOCK_LEN) {
        let block = GenericArray::from_mut_slice(chunk);
        enc.encrypt_block_mut(block);
    }
}

/// Decrypt whole 16-byte blocks of `data` in place with AES-128-CBC.
fn cbc_decrypt_blocks_in_place(key: &[u8; KEY_LEN], iv: &[u8; BLOCK_LEN], data: &mut [u8]) {
    if data.is_empty() {
        return;
    }
    let mut dec = Aes128CbcDec::new(key.into(), iv.into());
    for chunk in data.chunks_exact_mut(BLOCK_LEN) {
        let block = GenericArray::from_mut_slice(chunk);
        dec.decrypt_block_mut(block);
    }
}

/// Encrypt whole 16-byte blocks of `data` in place with AES-128-CBC, returning
/// the running IV the next call must use (the final ciphertext block).
///
/// Used by the H.264 skip-pattern ([`h264_transform_pattern`]), whose CBC
/// chain runs across the encrypted blocks *within one NAL* and is reset per NAL
/// (`docs/drm/hls-sample-aes.md` §3.4; verified against ffmpeg's
/// `hls_sample_aes.c` decryptor). The E-AC-3 path does **not** use these: it
/// resets the IV at every syncframe (§6).
fn cbc_encrypt_blocks_chained(
    key: &[u8; KEY_LEN],
    iv: &[u8; BLOCK_LEN],
    data: &mut [u8],
) -> [u8; BLOCK_LEN] {
    let mut state = *iv;
    if data.is_empty() {
        return state;
    }
    let mut enc = Aes128CbcEnc::new(key.into(), iv.into());
    for chunk in data.chunks_exact_mut(BLOCK_LEN) {
        let block = GenericArray::from_mut_slice(chunk);
        enc.encrypt_block_mut(block);
    }
    if let Some(last) = data.chunks_exact(BLOCK_LEN).next_back() {
        state.copy_from_slice(last);
    }
    state
}

/// Decrypt whole 16-byte blocks of `data` in place with AES-128-CBC, returning
/// the running IV the next call must use.
///
/// AES-CBC decryption chains on the *ciphertext*, so the carried value is the
/// last ciphertext block of `data`, captured before the in-place decrypt.
fn cbc_decrypt_blocks_chained(
    key: &[u8; KEY_LEN],
    iv: &[u8; BLOCK_LEN],
    data: &mut [u8],
) -> [u8; BLOCK_LEN] {
    let mut state = *iv;
    if data.is_empty() {
        return state;
    }
    if let Some(last) = data.chunks_exact(BLOCK_LEN).next_back() {
        state.copy_from_slice(last);
    }
    let mut dec = Aes128CbcDec::new(key.into(), iv.into());
    for chunk in data.chunks_exact_mut(BLOCK_LEN) {
        let block = GenericArray::from_mut_slice(chunk);
        dec.decrypt_block_mut(block);
    }
    state
}

// --------------------------------------------------------------------------
// AES-128 full-segment mode (docs/drm/hls-sample-aes.md §2).
// --------------------------------------------------------------------------

/// Encrypt a whole segment with AES-128-CBC and PKCS#7 padding
/// (`METHOD=AES-128`, `docs/drm/hls-sample-aes.md` §2).
///
/// The output length is `plaintext.len()` rounded up to the next multiple of
/// [`BLOCK_LEN`] (a full padding block is appended when already aligned).
pub fn aes128_encrypt_segment(
    key: &[u8; KEY_LEN],
    iv: &[u8; BLOCK_LEN],
    plaintext: &[u8],
) -> Vec<u8> {
    let enc = Aes128CbcEnc::new(key.into(), iv.into());
    enc.encrypt_padded_vec_mut::<Pkcs7>(plaintext)
}

/// Decrypt an AES-128 full-segment ciphertext, stripping PKCS#7 padding
/// (`docs/drm/hls-sample-aes.md` §2).
///
/// Returns [`Error::InvalidInput`] if the ciphertext length is not a positive
/// multiple of [`BLOCK_LEN`] or the padding is malformed.
pub fn aes128_decrypt_segment(
    key: &[u8; KEY_LEN],
    iv: &[u8; BLOCK_LEN],
    ciphertext: &[u8],
) -> Result<Vec<u8>> {
    if ciphertext.is_empty() || !ciphertext.len().is_multiple_of(BLOCK_LEN) {
        return Err(Error::InvalidInput(
            "AES-128 segment ciphertext length not a positive multiple of 16",
        ));
    }
    let dec = Aes128CbcDec::new(key.into(), iv.into());
    dec.decrypt_padded_vec_mut::<Pkcs7>(ciphertext)
        .map_err(|_| Error::InvalidInput("AES-128 segment PKCS#7 padding invalid"))
}

// --------------------------------------------------------------------------
// H.264 SAMPLE-AES (docs/drm/hls-sample-aes.md §3).
// --------------------------------------------------------------------------

/// Whether a single H.264 NAL unit (header byte included, no start code, no
/// emulation-prevention bytes) is a Sample-AES-encryptable slice
/// (`docs/drm/hls-sample-aes.md` §3.1): NAL type 1 or 5 with length `> 48`.
pub fn h264_nal_is_encrypted(nal: &[u8]) -> bool {
    if nal.len() <= H264_MIN_ENCRYPTED_NAL_LEN || nal.is_empty() {
        return false;
    }
    let nal_type = nal[0] & NAL_TYPE_MASK;
    nal_type == NAL_TYPE_NON_IDR_SLICE || nal_type == NAL_TYPE_IDR_SLICE
}

/// Remove H.264 emulation-prevention bytes (unescape `00 00 03 XX` → `00 00 XX`
/// for `XX ∈ {00,01,02,03}`) from a raw byte-stream NAL
/// (`docs/drm/hls-sample-aes.md` §3.5).
fn h264_unescape(escaped: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(escaped.len());
    let mut i = 0;
    while i < escaped.len() {
        // A 00 00 03 sequence where the byte after 03 is <= 03 is an emulation
        // prevention byte: drop the 03.
        if i + 3 < escaped.len()
            && escaped[i] == 0x00
            && escaped[i + 1] == 0x00
            && escaped[i + 2] == 0x03
            && escaped[i + 3] <= 0x03
        {
            out.push(0x00);
            out.push(0x00);
            i += 3; // skip the 0x03; the following byte is copied next iteration
        } else {
            out.push(escaped[i]);
            i += 1;
        }
    }
    out
}

/// Re-insert H.264 emulation-prevention bytes (escape `00 00 XX` → `00 00 03 XX`
/// for `XX ∈ {00,01,02,03}`, and a trailing `00 00`) over a raw NAL
/// (`docs/drm/hls-sample-aes.md` §3.5).
fn h264_escape(unescaped: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(unescaped.len());
    let mut zero_run = 0usize; // consecutive 0x00 bytes already emitted
    for &b in unescaped {
        if zero_run >= 2 && b <= 0x03 {
            out.push(0x03);
            zero_run = 0;
        }
        out.push(b);
        if b == 0x00 {
            zero_run += 1;
        } else {
            zero_run = 0;
        }
    }
    out
}

/// Apply the H.264 skip-encrypt pattern (`docs/drm/hls-sample-aes.md` §3.2–§3.3)
/// to the *unescaped* NAL bytes in place. `encrypt = true` encrypts, `false`
/// decrypts; the same clear/encrypted block partition is used for both.
///
/// CBC **chains across the encrypted blocks within one NAL**: the first
/// encrypted block uses `iv`, and each later encrypted block uses the previous
/// encrypted block's ciphertext as its IV. The IV is reset only at the start of
/// each NAL (i.e. per call). This is what ffmpeg's Sample-AES decryptor does
/// (`libavformat/hls_sample_aes.c` `decrypt_nal_unit`: one IV per NAL, then
/// `av_aes_crypt(..., 1, ctx->iv, 1)` updating `ctx->iv` in place across the
/// blocks); decrypting a chained stream with a per-block IV reset yields
/// garbage. Issue #1080 r05-W1.
fn h264_transform_pattern(
    key: &[u8; KEY_LEN],
    iv: &[u8; BLOCK_LEN],
    nal: &mut [u8],
    encrypt: bool,
) {
    // Clear prefix: 1 NAL header byte + 31 payload bytes.
    let mut offset = H264_CLEAR_PREFIX_LEN;
    // Running CBC state, seeded from the NAL's IV and updated with each
    // encrypted block's ciphertext (the chain input for the next block).
    let mut chain = *iv;
    while offset < nal.len() {
        let remaining = nal.len() - offset;
        // Encrypt one 16-byte block only when MORE than a whole block
        // remains (`docs/drm/hls-sample-aes.md` §3.2: `bytes_remaining() >
        // 16`, strictly greater). A remainder of exactly 16 bytes is the
        // trailing block and stays clear — using `>=` here encrypted it,
        // corrupting the last 16 bytes of ~1/160 slice NALs (issue #1014).
        if remaining > BLOCK_LEN {
            let block = &mut nal[offset..offset + BLOCK_LEN];
            chain = if encrypt {
                cbc_encrypt_blocks_chained(key, &chain, block)
            } else {
                cbc_decrypt_blocks_chained(key, &chain, block)
            };
            offset += BLOCK_LEN;
        } else {
            // Trailing partial block (`< 16`): left clear.
            break;
        }
        // Skip up to 144 clear bytes before the next encrypted block.
        offset += H264_SKIP_LEN.min(nal.len().saturating_sub(offset));
    }
}

/// Sample-AES-encrypt one H.264 NAL unit (header byte first, no Annex B start
/// code) given as *raw byte-stream* bytes (emulation-prevention bytes present),
/// returning the re-escaped encrypted NAL (`docs/drm/hls-sample-aes.md` §3).
///
/// NALs that are not encryptable (type not 1/5, or `len <= 48` after
/// unescaping) are returned unchanged. The CBC chain is reset to `iv` at the
/// start of each NAL, then carried across that NAL's encrypted blocks.
pub fn h264_encrypt_nal(key: &[u8; KEY_LEN], iv: &[u8; BLOCK_LEN], nal: &[u8]) -> Vec<u8> {
    let mut raw = h264_unescape(nal);
    if !h264_nal_is_encrypted(&raw) {
        return nal.to_vec();
    }
    h264_transform_pattern(key, iv, &mut raw, true);
    h264_escape(&raw)
}

/// Sample-AES-decrypt one H.264 NAL unit produced by [`h264_encrypt_nal`],
/// returning the re-escaped cleartext NAL (`docs/drm/hls-sample-aes.md` §3).
///
/// The CBC chain is reset to `iv` at the start of each NAL, then carried across
/// that NAL's encrypted blocks (see [`h264_encrypt_nal`] for the chaining rule).
pub fn h264_decrypt_nal(key: &[u8; KEY_LEN], iv: &[u8; BLOCK_LEN], nal: &[u8]) -> Vec<u8> {
    let mut raw = h264_unescape(nal);
    if !h264_nal_is_encrypted(&raw) {
        return nal.to_vec();
    }
    h264_transform_pattern(key, iv, &mut raw, false);
    h264_escape(&raw)
}

// --------------------------------------------------------------------------
// Audio SAMPLE-AES: shared leader + block loop (docs/drm/hls-sample-aes.md §4–§6).
// --------------------------------------------------------------------------

/// Encrypt the whole-block region of an audio frame after `clear_prefix` clear
/// bytes, leaving the `< 16` trailer clear.
fn audio_transform(
    key: &[u8; KEY_LEN],
    iv: &[u8; BLOCK_LEN],
    frame: &mut [u8],
    clear_prefix: usize,
    encrypt: bool,
) {
    if frame.len() <= clear_prefix {
        return;
    }
    let body = &mut frame[clear_prefix..];
    let whole = (body.len() / BLOCK_LEN) * BLOCK_LEN;
    if whole == 0 {
        return;
    }
    let blocks = &mut body[..whole];
    if encrypt {
        cbc_encrypt_blocks_in_place(key, iv, blocks);
    } else {
        cbc_decrypt_blocks_in_place(key, iv, blocks);
    }
}

/// The ADTS header length (7 or 9 bytes) implied by the `protection_absent` bit
/// (bit 0 of byte 1) of an ADTS frame (`docs/drm/hls-sample-aes.md` §4.1).
///
/// Returns [`Error::InvalidInput`] if `frame` is too short to hold the header.
pub fn adts_header_len(frame: &[u8]) -> Result<usize> {
    if frame.len() < ADTS_HEADER_LEN_NO_CRC {
        return Err(Error::InvalidInput("ADTS frame shorter than 7-byte header"));
    }
    // protection_absent == 1 → no CRC (7-byte header); == 0 → CRC (9 bytes).
    let protection_absent = frame[1] & 0x01;
    Ok(if protection_absent == 1 {
        ADTS_HEADER_LEN_NO_CRC
    } else {
        ADTS_HEADER_LEN_WITH_CRC
    })
}

/// Sample-AES-encrypt one AAC ADTS frame: ADTS header + 16-byte leader clear,
/// then 16-byte CBC blocks, `< 16` trailer clear
/// (`docs/drm/hls-sample-aes.md` §4). IV is reset per frame (§4.2).
pub fn aac_encrypt_frame(
    key: &[u8; KEY_LEN],
    iv: &[u8; BLOCK_LEN],
    frame: &[u8],
) -> Result<Vec<u8>> {
    let hdr = adts_header_len(frame)?;
    let mut out = frame.to_vec();
    audio_transform(key, iv, &mut out, hdr + AUDIO_CLEAR_LEADER_LEN, true);
    Ok(out)
}

/// Sample-AES-decrypt one AAC ADTS frame produced by [`aac_encrypt_frame`]
/// (`docs/drm/hls-sample-aes.md` §4).
pub fn aac_decrypt_frame(
    key: &[u8; KEY_LEN],
    iv: &[u8; BLOCK_LEN],
    frame: &[u8],
) -> Result<Vec<u8>> {
    let hdr = adts_header_len(frame)?;
    let mut out = frame.to_vec();
    audio_transform(key, iv, &mut out, hdr + AUDIO_CLEAR_LEADER_LEN, false);
    Ok(out)
}

/// Sample-AES-encrypt one AC-3 or E-AC-3 frame: 16-byte leader clear, then
/// 16-byte CBC blocks, `< 16` trailer clear
/// (`docs/drm/hls-sample-aes.md` §5, §6). IV is reset per frame.
pub fn ac3_encrypt_frame(key: &[u8; KEY_LEN], iv: &[u8; BLOCK_LEN], frame: &[u8]) -> Vec<u8> {
    let mut out = frame.to_vec();
    audio_transform(key, iv, &mut out, AUDIO_CLEAR_LEADER_LEN, true);
    out
}

/// Sample-AES-decrypt one AC-3 or E-AC-3 frame produced by [`ac3_encrypt_frame`]
/// (`docs/drm/hls-sample-aes.md` §5, §6).
pub fn ac3_decrypt_frame(key: &[u8; KEY_LEN], iv: &[u8; BLOCK_LEN], frame: &[u8]) -> Vec<u8> {
    let mut out = frame.to_vec();
    audio_transform(key, iv, &mut out, AUDIO_CLEAR_LEADER_LEN, false);
    out
}

/// The protected block of an E-AC-3 stream is a **single syncframe**.
///
/// `eac3_encrypt_frame`/`eac3_decrypt_frame` walk the syncframes of a frame via
/// `crate::ac3::split_eac3_syncframe_ranges` (ETSI TS 102 366 §E.1.3.1.3
/// `frmsiz`), applying a 16-byte clear leader and a whole-block CBC pass to
/// each, with the **IV reset at every syncframe**.
///
/// The reset is what the independent oracle shows
/// (`tests/fixtures/sample_aes_eac3/`): ffmpeg decodes the reset reference 9/9
/// audio frames identical to clear but the carried-IV variant only 1/9, and
/// Bento4's `mp4hls` output is byte-identical to the reset reference. See that
/// fixture's `README.md` / `proof.log`.
///
/// The per-*syncframe* reset differs from the "IV is not reset at syncframe
/// boundaries within an audio frame" reading: every stream ffmpeg or Bento4 can
/// produce has one independent syncframe per audio frame, so both tools reset
/// there. IV carried across an independent + dependent syncframe pair has **NO
/// independent oracle** (see `ORACLES.md` B3) — those dependent syncframes are
/// treated like independent ones (reset) until an oracle says otherwise.
///
/// # Errors
///
/// A payload that is not a clean run of syncframes (no sync word, or a
/// truncated trailing syncframe) is [`Error::InvalidInput`] rather than being
/// returned partly encrypted: an encryptor must never emit plaintext or a
/// half-encrypted access unit, which no conformant client can decrypt.
///
/// NO independent oracle for the dependent-syncframe case above.
fn eac3_transform(
    key: &[u8; KEY_LEN],
    iv: &[u8; BLOCK_LEN],
    frame: &[u8],
    encrypt: bool,
) -> Result<Vec<u8>> {
    let ranges = crate::ac3::split_eac3_syncframe_ranges(frame);
    if ranges.is_empty() {
        return Err(Error::InvalidInput(
            "E-AC-3 frame does not start with a valid syncframe",
        ));
    }
    // The walk must cover the whole frame: a truncated trailing syncframe would
    // otherwise be left in the clear while the head is encrypted.
    if ranges.last().map(|&(_, end)| end) != Some(frame.len()) {
        return Err(Error::InvalidInput(
            "E-AC-3 frame has a trailing partial syncframe (walk did not cover the frame)",
        ));
    }
    let mut out = frame.to_vec();
    for (start, end) in ranges {
        // IV reset at every syncframe (see the oracle note above).
        audio_transform(
            key,
            iv,
            &mut out[start..end],
            AUDIO_CLEAR_LEADER_LEN,
            encrypt,
        );
    }
    Ok(out)
}

/// Sample-AES-encrypt one E-AC-3 audio frame, walking its syncframes with a
/// 16-byte clear leader each and resetting the IV at every syncframe
/// (`docs/drm/hls-sample-aes.md` §6).
///
/// Prefer this over [`ac3_encrypt_frame`] for any E-AC-3 stream: a frame with
/// more than one syncframe (the norm for 5.1 at 1536 samples) needs a leader
/// per syncframe, which the single-leader form skips.
///
/// # Unverified corner
///
/// Carrying the IV across an independent syncframe *and its dependent
/// syncframes* (a 7.1 access unit) has **no independent oracle** — every stream
/// ffmpeg or Bento4 can produce has one independent syncframe per audio frame,
/// and both reset there (`ORACLES.md` B3). This implementation resets at every
/// syncframe, including a dependent one; treat that case as unverified.
///
/// # Errors
///
/// [`Error::InvalidInput`] if the frame does not begin with a valid syncframe,
/// or if the syncframe walk does not cover the whole frame (a truncated or
/// trailing partial syncframe) — never a partial or plaintext output.
pub fn eac3_encrypt_frame(
    key: &[u8; KEY_LEN],
    iv: &[u8; BLOCK_LEN],
    frame: &[u8],
) -> Result<Vec<u8>> {
    eac3_transform(key, iv, frame, true)
}

/// Sample-AES-decrypt one E-AC-3 audio frame produced by
/// [`eac3_encrypt_frame`] (`docs/drm/hls-sample-aes.md` §6).
///
/// Same unverified corner and error contract as [`eac3_encrypt_frame`]: the IV
/// resets at every syncframe, and a payload that is not a clean run of
/// syncframes is [`Error::InvalidInput`].
pub fn eac3_decrypt_frame(
    key: &[u8; KEY_LEN],
    iv: &[u8; BLOCK_LEN],
    frame: &[u8],
) -> Result<Vec<u8>> {
    eac3_transform(key, iv, frame, false)
}

// --------------------------------------------------------------------------
// EXT-X-KEY tag rendering (RFC 8216 §4.3.2.4, docs/drm/hls-sample-aes.md §9).
// --------------------------------------------------------------------------

/// HLS encryption method for the `EXT-X-KEY` `METHOD` attribute
/// (RFC 8216 §4.3.2.4, `docs/drm/hls-sample-aes.md` §9).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum HlsEncryptionMethod {
    /// `AES-128` — full-segment AES-128-CBC.
    Aes128,
    /// `SAMPLE-AES` — per-sample AES-128-CBC.
    SampleAes,
}

impl HlsEncryptionMethod {
    /// The `METHOD` token as it appears in the tag (`"AES-128"` / `"SAMPLE-AES"`).
    pub fn name(&self) -> &'static str {
        match self {
            HlsEncryptionMethod::Aes128 => "AES-128",
            HlsEncryptionMethod::SampleAes => "SAMPLE-AES",
        }
    }
}

broadcast_common::impl_spec_display!(HlsEncryptionMethod);

/// Format a 16-byte IV as the `0x`-prefixed 32-hex-digit `EXT-X-KEY` `IV`
/// attribute value (`docs/drm/hls-sample-aes.md` §9).
pub fn format_iv(iv: &[u8; BLOCK_LEN]) -> String {
    let mut s = String::with_capacity(2 + 2 * BLOCK_LEN);
    s.push_str("0x");
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for &b in iv {
        s.push(HEX[(b >> 4) as usize] as char);
        s.push(HEX[(b & 0x0F) as usize] as char);
    }
    s
}

/// Derive the implicit IV from the media sequence number: the sequence number
/// as a 128-bit big-endian integer, zero-padded to 16 bytes
/// (`docs/drm/hls-sample-aes.md` §10).
pub fn iv_from_sequence_number(media_sequence: u128) -> [u8; BLOCK_LEN] {
    media_sequence.to_be_bytes()
}

/// An `EXT-X-KEY` tag (RFC 8216 §4.3.2.4, `docs/drm/hls-sample-aes.md` §9).
///
/// Renders to the tag line via [`to_tag`](ExtXKey::to_tag), which validates
/// `uri`/`keyformat`/`keyformatversions` (issue #1140).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ExtXKey {
    /// `METHOD` — the encryption method.
    pub method: HlsEncryptionMethod,
    /// `URI` — key-server URL, `skd://asset-id`, or `data:` URI.
    pub uri: String,
    /// `IV` — explicit 16-byte IV, or `None` to derive it from the media
    /// sequence number at playback.
    pub iv: Option<[u8; BLOCK_LEN]>,
    /// `KEYFORMAT` — e.g. `com.apple.streamingkeydelivery` / `identity`.
    pub keyformat: Option<String>,
    /// `KEYFORMATVERSIONS` — e.g. `1`.
    pub keyformatversions: Option<String>,
}

impl ExtXKey {
    /// A `METHOD=SAMPLE-AES` FairPlay tag with `skd://` URI and the
    /// `com.apple.streamingkeydelivery` key format
    /// (`docs/drm/hls-sample-aes.md` §9).
    pub fn fairplay_sample_aes(skd_uri: impl Into<String>) -> Self {
        Self {
            method: HlsEncryptionMethod::SampleAes,
            uri: skd_uri.into(),
            iv: None,
            keyformat: Some(String::from("com.apple.streamingkeydelivery")),
            keyformatversions: Some(String::from("1")),
        }
    }

    /// A `METHOD=AES-128` full-segment tag with the given key `URI` and explicit
    /// `IV` (`docs/drm/hls-sample-aes.md` §2, §9).
    pub fn aes128(uri: impl Into<String>, iv: [u8; BLOCK_LEN]) -> Self {
        Self {
            method: HlsEncryptionMethod::Aes128,
            uri: uri.into(),
            iv: Some(iv),
            keyformat: None,
            keyformatversions: None,
        }
    }

    /// Render the `#EXT-X-KEY:...` tag line (RFC 8216 §4.3.2.4). Attribute order
    /// is `METHOD`, `URI`, `IV`, `KEYFORMAT`, `KEYFORMATVERSIONS`; absent
    /// optional attributes are omitted.
    ///
    /// Builds the attribute list through `broadcast_hls`'s checked
    /// `AttrValue` constructors and shared [`broadcast_hls::render_attribute_list`]
    /// renderer, rather than hand-formatting `,NAME="VALUE"` (issue #1140 /
    /// audit r05-W10): `uri`/`keyformat`/`keyformatversions` are
    /// caller-supplied (often assembled from a key-server request), so a
    /// `"`, CR or LF in any of them previously terminated the attribute and
    /// injected arbitrary playlist tags. Returns
    /// [`Error::HlsAttrValue`] instead.
    pub fn to_tag(&self) -> Result<String> {
        let mut s = String::from("#EXT-X-KEY:METHOD=");
        s.push_str(self.method.name());
        let mut attrs = alloc::vec![("URI".to_string(), AttrValue::quoted(&self.uri)?)];
        if let Some(iv) = self.iv {
            // A hex token this crate itself formats (`format_iv`), never
            // caller-freeform text, so `bare` cannot fail.
            attrs.push(("IV".to_string(), AttrValue::bare(format_iv(&iv))?));
        }
        if let Some(ref kf) = self.keyformat {
            attrs.push(("KEYFORMAT".to_string(), AttrValue::quoted(kf.clone())?));
        }
        if let Some(ref kfv) = self.keyformatversions {
            attrs.push((
                "KEYFORMATVERSIONS".to_string(),
                AttrValue::quoted(kfv.clone())?,
            ));
        }
        broadcast_hls::render_attribute_list(&mut s, &attrs);
        Ok(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // NIST SP 800-38A F.2.1/F.2.2 CBC-AES128 vector.
    const NIST_KEY: [u8; 16] = [
        0x2b, 0x7e, 0x15, 0x16, 0x28, 0xae, 0xd2, 0xa6, 0xab, 0xf7, 0x15, 0x88, 0x09, 0xcf, 0x4f,
        0x3c,
    ];
    const NIST_IV: [u8; 16] = [
        0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e,
        0x0f,
    ];
    const NIST_PT: [u8; 16] = [
        0x6b, 0xc1, 0xbe, 0xe2, 0x2e, 0x40, 0x9f, 0x96, 0xe9, 0x3d, 0x7e, 0x11, 0x73, 0x93, 0x17,
        0x2a,
    ];
    const NIST_CT: [u8; 16] = [
        0x76, 0x49, 0xab, 0xac, 0x81, 0x19, 0xb2, 0x46, 0xce, 0xe9, 0x8e, 0x9b, 0x12, 0xe9, 0x19,
        0x7d,
    ];

    #[test]
    fn aes128_cbc_known_answer() {
        // Encrypting one raw block must match the published NIST ciphertext.
        let mut block = NIST_PT;
        cbc_encrypt_blocks_in_place(&NIST_KEY, &NIST_IV, &mut block);
        assert_eq!(block, NIST_CT, "AES-128-CBC block != NIST vector");

        let mut back = NIST_CT;
        cbc_decrypt_blocks_in_place(&NIST_KEY, &NIST_IV, &mut back);
        assert_eq!(back, NIST_PT, "decrypt did not recover plaintext");
    }

    #[test]
    fn aes128_segment_pkcs7_round_trip() {
        // 37 bytes → pads to 48 (next multiple of 16).
        let segment: Vec<u8> = (0u8..37).collect();
        let ct = aes128_encrypt_segment(&NIST_KEY, &NIST_IV, &segment);
        assert_eq!(ct.len(), 48, "ciphertext must be block-padded");
        assert_ne!(&ct[..segment.len()], &segment[..], "must actually encrypt");
        let pt = aes128_decrypt_segment(&NIST_KEY, &NIST_IV, &ct).unwrap();
        assert_eq!(pt, segment, "segment round-trip mismatch");

        // Already-aligned input gets a full extra padding block.
        let aligned = vec![0xABu8; 32];
        let ct2 = aes128_encrypt_segment(&NIST_KEY, &NIST_IV, &aligned);
        assert_eq!(ct2.len(), 48);
        assert_eq!(
            aes128_decrypt_segment(&NIST_KEY, &NIST_IV, &ct2).unwrap(),
            aligned
        );

        // Malformed ciphertext length is rejected, not panicked.
        assert!(aes128_decrypt_segment(&NIST_KEY, &NIST_IV, &[0u8; 15]).is_err());
        assert!(aes128_decrypt_segment(&NIST_KEY, &NIST_IV, &[]).is_err());
    }

    #[test]
    fn h264_short_nal_untouched() {
        // NAL type 5 but <= 48 bytes: left entirely clear.
        let mut nal = vec![0x65u8]; // header: type 5
        nal.extend((0u8..40).map(|i| i.wrapping_mul(7)));
        assert!(nal.len() <= H264_MIN_ENCRYPTED_NAL_LEN);
        let out = h264_encrypt_nal(&NIST_KEY, &NIST_IV, &nal);
        assert_eq!(out, nal, "short NAL must not be encrypted");
    }

    #[test]
    fn h264_wrong_type_untouched() {
        // NAL type 7 (SPS), long: never encrypted.
        let mut nal = vec![0x67u8];
        nal.extend((0u8..80).map(|i| i.wrapping_add(1)));
        assert!(nal.len() > H264_MIN_ENCRYPTED_NAL_LEN);
        let out = h264_encrypt_nal(&NIST_KEY, &NIST_IV, &nal);
        assert_eq!(out, nal, "non-slice NAL must not be encrypted");
    }

    #[test]
    fn h264_pattern_and_round_trip() {
        // A NAL with no emulation sequences so escaped == raw: type 5, 200 bytes.
        // Fill payload with a value that never forms 00 00 0x.
        let mut nal = vec![0x65u8];
        nal.extend(core::iter::repeat_n(0xAAu8, 199));
        assert_eq!(nal.len(), 200);
        assert!(h264_nal_is_encrypted(&nal));

        let enc = h264_encrypt_nal(&NIST_KEY, &NIST_IV, &nal);
        // No emulation bytes introduced (0xAA ciphertext rarely forms 00 00 0x,
        // but assert length preserved when none are needed by re-deriving).
        // First 32 bytes (clear prefix) unchanged.
        assert_eq!(&enc[..H264_CLEAR_PREFIX_LEN], &nal[..H264_CLEAR_PREFIX_LEN]);

        // Pattern offsets (no escaping expansion here → offsets are byte-exact):
        // [32 clear][16 enc][144 clear][16 enc][... ] over 200 bytes.
        // block1 = [32,48), skip = [48,192), block2 = [192,200) → only 8 bytes
        // remain (<16) so block2 is NOT encrypted (trailing partial clear).
        // Encrypted region [32,48) must differ; [48,192) and [192,200) clear.
        // Because escaping could shift bytes, compare on the unescaped form:
        let raw = h264_unescape(&nal);
        let enc_raw = h264_unescape(&enc);
        assert_eq!(raw.len(), enc_raw.len());
        assert_ne!(&enc_raw[32..48], &raw[32..48], "block1 must be encrypted");
        assert_eq!(
            &enc_raw[48..192],
            &raw[48..192],
            "skip region must be clear"
        );
        assert_eq!(
            &enc_raw[192..200],
            &raw[192..200],
            "trailing <16 must be clear"
        );

        let dec = h264_decrypt_nal(&NIST_KEY, &NIST_IV, &enc);
        assert_eq!(dec, nal, "H.264 NAL round-trip mismatch");
    }

    #[test]
    fn h264_exact_16_byte_trailer_left_clear() {
        // Oracle: `docs/drm/hls-sample-aes.md` §3.2 (transcribed from Apple's
        // "MPEG-2 Stream Encryption Format for HTTP Live Streaming" §3.2),
        // `while (bytes_remaining() > 0) { if (bytes_remaining() > 16) {
        // encrypted_block } unencrypted_block [min(144, bytes_remaining())] }`
        // — the comparison is strictly `> 16`, so a remainder of *exactly*
        // 16 bytes at a block boundary is the trailing partial-block case
        // and must stay clear. Issue #1014: this crate used `>= 16`, so it
        // encrypted that last block instead (confirmed by temporarily
        // reverting the fix to `>=`: `enc_raw[192..208]` then differs from
        // `raw[192..208]` and the assertion below fails).
        //
        // Layout: 32 clear prefix + 16-byte encrypted block1 [32,48) +
        // 144-byte skip [48,192) + exactly 16 bytes left [192,208) — the
        // *second* loop iteration's `bytes_remaining()` is exactly 16 there,
        // which is the case the spec leaves clear.
        let mut nal = vec![0x65u8]; // type 5 (IDR slice)
        nal.extend(core::iter::repeat_n(0xAAu8, 207)); // 1 + 207 = 208 bytes, no 00 00 0x runs
        assert_eq!(nal.len(), 208);
        assert!(h264_nal_is_encrypted(&nal));

        let enc = h264_encrypt_nal(&NIST_KEY, &NIST_IV, &nal);
        let raw = h264_unescape(&nal);
        let enc_raw = h264_unescape(&enc);
        assert_eq!(raw.len(), enc_raw.len());
        assert_ne!(&enc_raw[32..48], &raw[32..48], "block1 must be encrypted");
        assert_eq!(
            &enc_raw[48..192],
            &raw[48..192],
            "144-byte skip region must be clear"
        );
        assert_eq!(
            &enc_raw[192..208],
            &raw[192..208],
            "exact-16-byte trailer must stay clear (spec: bytes_remaining() > 16)"
        );

        let dec = h264_decrypt_nal(&NIST_KEY, &NIST_IV, &enc);
        assert_eq!(dec, nal, "H.264 NAL round-trip mismatch");
    }

    #[test]
    fn h264_emulation_prevention_round_trips() {
        // Build a NAL whose payload contains 00 00 03 sequences.
        let mut nal = vec![0x65u8]; // type 5
        // `00 00 03 01` is a canonical emulation-prevention triplet; the
        // trailing 0xFF 0xEE keep groups apart so no illegal 00-00-00 run forms.
        for _ in 0..30 {
            nal.extend_from_slice(&[0x00, 0x00, 0x03, 0x01, 0xFF, 0xEE]);
        }
        assert!(nal.len() > H264_MIN_ENCRYPTED_NAL_LEN);
        // Sanity: unescape then re-escape is identity for a valid stream.
        let round = h264_escape(&h264_unescape(&nal));
        assert_eq!(round, nal, "escape/unescape not identity on valid NAL");

        let enc = h264_encrypt_nal(&NIST_KEY, &NIST_IV, &nal);
        let dec = h264_decrypt_nal(&NIST_KEY, &NIST_IV, &enc);
        assert_eq!(dec, nal, "emulation-prevention NAL round-trip mismatch");
    }

    #[test]
    fn aac_round_trip_and_clear_leader() {
        // ADTS frame: byte1 bit0 = 1 → 7-byte header (no CRC).
        let mut frame = vec![0xFF, 0xF1, 0x00, 0x00, 0x00, 0x00, 0x00];
        frame.extend((0u8..100).map(|i| i.wrapping_mul(3)));
        let hdr = adts_header_len(&frame).unwrap();
        assert_eq!(hdr, ADTS_HEADER_LEN_NO_CRC);

        let enc = aac_encrypt_frame(&NIST_KEY, &NIST_IV, &frame).unwrap();
        assert_eq!(enc.len(), frame.len(), "AAC length preserved (no padding)");
        // Header + 16-byte leader clear.
        let clear = hdr + AUDIO_CLEAR_LEADER_LEN;
        assert_eq!(&enc[..clear], &frame[..clear], "leader must be clear");
        // Something in the body was encrypted.
        assert_ne!(&enc[clear..], &frame[clear..], "body must be encrypted");

        let dec = aac_decrypt_frame(&NIST_KEY, &NIST_IV, &enc).unwrap();
        assert_eq!(dec, frame, "AAC round-trip mismatch");
    }

    #[test]
    fn adts_header_len_with_crc() {
        // byte1 bit0 = 0 → 9-byte header (CRC present).
        let frame = vec![0xFF, 0xF0, 0, 0, 0, 0, 0, 0, 0];
        assert_eq!(adts_header_len(&frame).unwrap(), ADTS_HEADER_LEN_WITH_CRC);
        assert!(adts_header_len(&[0xFF]).is_err());
    }

    #[test]
    fn ac3_round_trip() {
        let mut frame = vec![0x0B, 0x77]; // AC-3 syncword
        frame.extend((0u8..90).map(|i| i.wrapping_add(5)));
        let enc = ac3_encrypt_frame(&NIST_KEY, &NIST_IV, &frame);
        assert_eq!(enc.len(), frame.len());
        assert_eq!(
            &enc[..AUDIO_CLEAR_LEADER_LEN],
            &frame[..AUDIO_CLEAR_LEADER_LEN],
            "16-byte leader clear"
        );
        let dec = ac3_decrypt_frame(&NIST_KEY, &NIST_IV, &enc);
        assert_eq!(dec, frame);
    }

    /// Audit r05-W2, corrected by the independent oracle: our E-AC-3 encryptor
    /// must reproduce `tests/fixtures/sample_aes_eac3/enc_reset.ec3`
    /// byte-for-byte (IV reset at **every** syncframe), and must NOT reproduce
    /// `enc_carried.ec3`. ffmpeg decodes the reset file 9/9 audio frames
    /// identical to clear and the carried one 1/9; Bento4's encryptor is
    /// byte-identical to the reset reference. See that fixture's `proof.log`.
    #[test]
    fn eac3_reset_per_syncframe_matches_oracle() {
        let dir = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/sample_aes_eac3"
        );
        let read = |name: &str| {
            std::fs::read(alloc::format!("{dir}/{name}"))
                .unwrap_or_else(|e| panic!("read {dir}/{name}: {e}"))
        };
        let clear = read("clear.ec3");
        let want_reset = read("enc_reset.ec3");
        let carried = read("enc_carried.ec3");

        // The fixture's key/IV (the playlist IV).
        const KEY: [u8; KEY_LEN] = [
            0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d,
            0x0e, 0x0f,
        ];
        const IV: [u8; BLOCK_LEN] = [
            0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d,
            0x1e, 0x1f,
        ];

        // One call for the whole frame: our walker resets per syncframe.
        let got = eac3_encrypt_frame(&KEY, &IV, &clear).expect("encrypt");
        assert_eq!(
            got, want_reset,
            "our E-AC-3 encryptor must reproduce the reset-per-syncframe reference byte-for-byte"
        );
        assert_ne!(
            got, carried,
            "our output must NOT equal the carried-IV negative control"
        );

        // Our decryptor inverts the reference exactly.
        let dec = eac3_decrypt_frame(&KEY, &IV, &want_reset).expect("decrypt");
        assert_eq!(dec, clear, "our decryptor must recover the clear E-AC-3 ES");

        // Bento4's independent encryptor output must decrypt to the clear ES.
        let bento4 = read("bento4/packed/bento4_encrypted_es.ec3");
        let dec_b = eac3_decrypt_frame(&KEY, &IV, &bento4).expect("decrypt bento4");
        assert_eq!(
            dec_b, clear,
            "Bento4's E-AC-3 output must decrypt to the clear ES (independent encryptor)"
        );
    }

    /// Audit r05-W2 fail-open fix: a payload that is not a clean run of
    /// syncframes must be an error, never returned unchanged/partly encrypted.
    #[test]
    fn eac3_unwalkable_payload_is_err() {
        // Not a syncframe at all.
        let garbage = alloc::vec![0x00u8; 64];
        assert!(eac3_encrypt_frame(&NIST_KEY, &NIST_IV, &garbage).is_err());
        assert!(eac3_decrypt_frame(&NIST_KEY, &NIST_IV, &garbage).is_err());

        // A truncated trailing syncframe must also be rejected, not left clear.
        let dir = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/sample_aes_eac3"
        );
        let Ok(clear) = std::fs::read(alloc::format!("{dir}/clear.ec3")) else {
            panic!("eac3 fixture missing at {dir}/clear.ec3");
        };
        // Chop 100 bytes off the end: the last syncframe no longer completes.
        let truncated = &clear[..clear.len() - 100];
        assert!(
            eac3_encrypt_frame(&NIST_KEY, &NIST_IV, truncated).is_err(),
            "a truncated trailing syncframe must be an error, not silently left clear"
        );

        // Trailing junk after the last complete syncframe: the walk stops at
        // the junk, so it no longer covers the frame — must be an error, never
        // a frame whose head is encrypted and tail left in the clear.
        let mut with_junk = clear.clone();
        with_junk.extend_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x11]);
        assert!(
            eac3_encrypt_frame(&NIST_KEY, &NIST_IV, &with_junk).is_err(),
            "trailing junk after the last syncframe must be an error"
        );
        assert!(
            eac3_decrypt_frame(&NIST_KEY, &NIST_IV, &with_junk).is_err(),
            "trailing junk must also be rejected on decrypt"
        );
    }

    /// A single-syncframe chunk still encrypts (the per-syncframe walk is a
    /// no-op distinction for one syncframe), and the IV reset is what a caller
    /// passing syncframes one at a time already got.
    #[test]
    fn eac3_single_syncframe_matches_ac3_form() {
        let dir = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/sample_aes_eac3"
        );
        let Ok(clear) = std::fs::read(alloc::format!("{dir}/clear.ec3")) else {
            panic!("eac3 fixture missing");
        };
        let ranges = crate::ac3::split_eac3_syncframe_ranges(&clear);
        let (s, e) = ranges[0];
        let one = &clear[s..e];
        let via_frame = eac3_encrypt_frame(&NIST_KEY, &NIST_IV, one).expect("encrypt one");
        let via_ac3 = ac3_encrypt_frame(&NIST_KEY, &NIST_IV, one);
        assert_eq!(
            via_frame, via_ac3,
            "one syncframe through either API must agree (both reset at the frame start)"
        );
    }

    #[test]
    fn ext_x_key_tag_strings() {
        let sample_aes = ExtXKey::fairplay_sample_aes("skd://asset-42");
        assert_eq!(
            sample_aes.to_tag().unwrap(),
            "#EXT-X-KEY:METHOD=SAMPLE-AES,URI=\"skd://asset-42\",\
             KEYFORMAT=\"com.apple.streamingkeydelivery\",KEYFORMATVERSIONS=\"1\""
        );

        let aes = ExtXKey::aes128(
            "https://keyserver.example.com/key",
            [
                0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xAA, 0xBB, 0xBB, 0xBB, 0xBB, 0xBB, 0xBB,
                0xBB, 0xBB,
            ],
        );
        assert_eq!(
            aes.to_tag().unwrap(),
            "#EXT-X-KEY:METHOD=AES-128,URI=\"https://keyserver.example.com/key\",\
             IV=0xaaaaaaaaaaaaaaaabbbbbbbbbbbbbbbb"
        );
    }

    /// Audit r05-W10: a `"` in the key URI must be rejected, not injected
    /// into the attribute list.
    #[test]
    fn to_tag_rejects_injection_in_uri() {
        let key = ExtXKey::fairplay_sample_aes("skd://asset\"\r\n#EXT-X-ENDLIST");
        assert!(key.to_tag().is_err());
    }

    #[test]
    fn iv_from_sequence_number_be() {
        assert_eq!(
            iv_from_sequence_number(7),
            [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 7]
        );
        assert_eq!(
            format_iv(&iv_from_sequence_number(5)),
            "0x00000000000000000000000000000005"
        );
    }
}
