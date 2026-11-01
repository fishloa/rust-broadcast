//! Oracle consumer: H.264 HLS Sample-AES CBC chaining (audit r05-W1).
//!
//! Consumes the committed, independently-generated fixtures under
//! `tests/fixtures/sample_aes_h264/` — see that directory's `README.md` and
//! `ORACLES.md`. The claim they pin:
//!
//! * AES-128-CBC **chains across the encrypted 16-byte blocks within one NAL**;
//!   the IV is reset only at the start of each NAL;
//! * the first 32 bytes of a NAL are clear, then 16 encrypted bytes out of every
//!   160 (16 encrypted + 144 clear);
//! * only slice NAL types 1 and 5 with **more than 48 bytes** are encrypted;
//! * a candidate block is encrypted only when **more than 16 bytes remain** (a
//!   tail of exactly 16 stays clear);
//! * the NAL is unescaped, encrypted, then re-escaped (`0x03` where the
//!   ciphertext contains `00 00 0[0-3]`).
//!
//! Proven by three independent implementations (python reference, Bento4
//! `mp4hls`, ffmpeg decryptor); `enc_reset.h264` (IV reset per block — the
//! pre-#1080 behaviour) is the negative control.
#![cfg(all(feature = "sample-aes", feature = "std"))]

use std::path::PathBuf;

use transmux::annexb::{iter_annexb_nals, length_prefixed_to_annexb};
use transmux::sample_aes::{BLOCK_LEN, KEY_LEN, h264_decrypt_nal, h264_encrypt_nal};

/// `fixtures/sample_aes_h264/key.bin`.
const KEY: [u8; KEY_LEN] = [
    0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f,
];
/// `IV=0x1011..1f` in the fixture playlists.
const IV: [u8; BLOCK_LEN] = [
    0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d, 0x1e, 0x1f,
];

fn fx(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/sample_aes_h264")
        .join(name)
}

fn read(name: &str) -> Vec<u8> {
    let p = fx(name);
    std::fs::read(&p).unwrap_or_else(|e| panic!("read {p:?}: {e}"))
}

/// Encrypt a whole Annex-B ES, NAL by NAL, emitting 4-byte start codes (the
/// form `clear.h264` uses).
fn encrypt_es(annexb: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(annexb.len());
    for nal in iter_annexb_nals(annexb) {
        let enc = h264_encrypt_nal(&KEY, &IV, nal);
        out.extend_from_slice(&[0, 0, 0, 1]);
        out.extend_from_slice(&enc);
    }
    out
}

/// Decrypt a whole Annex-B ES, NAL by NAL.
fn decrypt_es(annexb: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(annexb.len());
    for nal in iter_annexb_nals(annexb) {
        let dec = h264_decrypt_nal(&KEY, &IV, nal);
        out.extend_from_slice(&[0, 0, 0, 1]);
        out.extend_from_slice(&dec);
    }
    out
}

/// The core oracle: our encryptor must reproduce `enc_chained.h264`
/// byte-for-byte, and must NOT reproduce `enc_reset.h264`.
#[test]
fn encrypt_matches_chained_reference_bytes() {
    let clear = read("clear.h264");
    let want = read("enc_chained.h264");
    let reset = read("enc_reset.h264");

    let got = encrypt_es(&clear);
    assert_eq!(
        got.len(),
        want.len(),
        "encrypted ES length must match the chained reference"
    );
    assert_eq!(
        got, want,
        "our encryptor must reproduce the independently-generated chained ciphertext byte-for-byte"
    );
    assert_ne!(
        got, reset,
        "our output must NOT equal the per-block-IV-reset negative control (the pre-fix behaviour)"
    );
}

/// Round-trip: our decryptor must recover the clear ES from the chained
/// reference. Compared modulo emulation prevention, because ffmpeg's (and the
/// reference's) decryptor rewrites EP bytes — the fixture README documents this.
#[test]
fn decrypt_chained_reference_recovers_clear() {
    let clear = read("clear.h264");
    let enc = read("enc_chained.h264");
    let got = decrypt_es(&enc);
    assert_eq!(
        norm_ep(&got),
        norm_ep(&clear),
        "decrypting the chained reference must recover the clear slice NALs (modulo EP)"
    );
}

/// The tail rule (`>16`): `tailprobe_enc_gt` is what our encryptor produces;
/// `tailprobe_enc_ge` (the wrong rule) must not match.
#[test]
fn tail_rule_gt16_is_the_shipped_behaviour() {
    let clear = read("tailprobe_clear.h264");
    let want = read("tailprobe_enc_gt.h264");
    let wrong = read("tailprobe_enc_ge.h264");

    let got = encrypt_es(&clear);
    assert_eq!(got, want, "the `>16` tail rule must match the reference");
    assert_ne!(got, wrong, "the `>=16` tail rule must not match");
}

/// Emulation prevention is re-applied after encryption: our output must equal
/// `escprobe_enc.h264` byte-for-byte (its ciphertext block starts `00 00 01`,
/// forcing an inserted `0x03`).
#[test]
fn emulation_prevention_is_reapplied() {
    let clear = read("escprobe_clear.h264");
    let want = read("escprobe_enc.h264");
    let got = encrypt_es(&clear);
    assert_eq!(
        got, want,
        "the re-escape pass must match the reference (0x03 inserted after 00 00 01)"
    );
}

/// Bento4's own `mp4hls` encryptor is an independent ENCRYPTOR. Its ES
/// (extracted as Bento4 wrote it) must carry the same encrypted **slice** NALs
/// as the reference chained ES — Bento4 emits them differently packaged (its
/// file has 16 extra non-slice NALs, e.g. repeated AUD/SPS), so the comparison
/// is over the slice NALs (types 1/5), which is what `compare_es.py` compares.
/// Our decryptor must then recover the clear slices from it.
#[test]
fn bento4_encrypted_slices_match_and_decrypt() {
    let clear = read("clear.h264");
    let reference = read("enc_chained.h264");
    let bento4 = read("bento4_sample_aes/clear/bento4_encrypted.h264");

    let ref_slices = slice_nals(&reference);
    let bento4_slices = slice_nals(&bento4);
    assert!(
        !ref_slices.is_empty(),
        "the reference must contain slice NALs to compare"
    );
    assert_eq!(
        bento4_slices, ref_slices,
        "Bento4's SAMPLE-AES slice NALs must be byte-identical to the chained reference \
         (independent encryptor)"
    );

    let got = decrypt_es(&bento4);
    // Bento4's slice NALs decrypt to the clear slices (modulo EP), per the README.
    assert_eq!(
        slice_nals(&norm_ep(&got)),
        slice_nals(&norm_ep(&clear)),
        "our decryptor must recover the clear slices from Bento4's ES"
    );
}

/// The slice (types 1/5) NAL bytes of an Annex-B ES, in order.
fn slice_nals(annexb: &[u8]) -> Vec<Vec<u8>> {
    iter_annexb_nals(annexb)
        .filter(|n| matches!(n[0] & 0x1f, 1 | 5))
        .map(|n| n.to_vec())
        .collect()
}

/// A5 — `epclear_*`: EP bytes planted inside the CLEAR regions of a slice.
///
/// NO independent oracle: Bento4 double-escapes and ffmpeg emits unescaped
/// NALs, so neither can arbitrate (see `ORACLES.md`). This asserts the
/// spec-derived rule only (unescape before encrypt, re-escape after) by
/// round-tripping through our own code.
#[test]
fn epclear_round_trips_spec_derived() {
    // NO independent oracle: this is the python reference's Apple-text rule.
    let clear = read("epclear_clear.h264");
    let got = decrypt_es(&encrypt_es(&clear));
    assert_eq!(
        norm_ep(&got),
        norm_ep(&clear),
        "EP bytes in clear regions must survive a round-trip"
    );
}

/// `escprobe_enc.h264`'s ciphertext contains an inserted `0x03` (the escape
/// pass re-inserted it after `00 00 01`). Decrypting it must remove that byte
/// before the cipher runs and reproduce the clear wire bytes **exactly** — not
/// merely modulo EP.
#[test]
fn escprobe_decrypt_removes_inserted_escape_byte() {
    let clear = read("escprobe_clear.h264");
    let enc = read("escprobe_enc.h264");
    let got = decrypt_es(&enc);
    assert_eq!(
        got, clear,
        "decrypting escprobe_enc must reproduce escprobe_clear byte-for-byte, including the          re-escaped 0x03 removal"
    );
    // And the encrypted ES really does differ (the probe injected an escape).
    assert_ne!(enc, clear, "the probe must actually have changed the bytes");
}

/// Normalise NALs for comparison: unescape emulation-prevention triplets and
/// re-serialise as 4-byte-length-prefixed frames (mirrors `compare_es.py`).
fn norm_ep(annexb: &[u8]) -> Vec<u8> {
    let mut lp = Vec::with_capacity(annexb.len());
    for nal in iter_annexb_nals(annexb) {
        let raw = unescape(nal);
        lp.extend_from_slice(&(raw.len() as u32).to_be_bytes());
        lp.extend_from_slice(&raw);
    }
    lp
}

/// Drop emulation-prevention `0x03` bytes (`00 00 03 XX` → `00 00 XX` for
/// `XX <= 3`, the only values the escape inserts).
fn unescape(nal: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(nal.len());
    let mut i = 0;
    while i < nal.len() {
        if i + 3 < nal.len()
            && nal[i] == 0x00
            && nal[i + 1] == 0x00
            && nal[i + 2] == 0x03
            && nal[i + 3] <= 0x03
        {
            out.push(0x00);
            out.push(0x00);
            i += 3;
        } else {
            out.push(nal[i]);
            i += 1;
        }
    }
    out
}

/// Sanity: the fixture's own decoded reference and the round-trip agree at the
/// Annex-B length-prefix level too (guards the helper, not the crypto).
#[test]
fn annexb_helper_round_trips() {
    let clear = read("clear.h264");
    let lp = {
        let mut v = Vec::new();
        for nal in iter_annexb_nals(&clear) {
            v.extend_from_slice(&(nal.len() as u32).to_be_bytes());
            v.extend_from_slice(nal);
        }
        v
    };
    let back = length_prefixed_to_annexb(&lp).expect("to annexb");
    assert_eq!(back, clear, "length-prefix <-> Annex B must round-trip");
}
