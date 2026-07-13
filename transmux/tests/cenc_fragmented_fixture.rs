//! Fragmented CENC/CBCS fixture tests (issue #564).
//!
//! Oracle: `fixtures/transmux/h264_{cenc,cbcs}.mp4` — real fragmented CMAF
//! files produced by **Bento4's `mp4encrypt`** (an independent, third-party
//! tool, not this project's own code) against the already-committed real
//! fixture `fixtures/transmux/h264_aac_frag.mp4`. Full provenance + the exact
//! key/IV/KID material used below is in
//! `fixtures/transmux/h264_cenc-PROVENANCE.md`.
//!
//! Both fixtures are genuinely fragmented (`moov` + 3 `moof`/`mdat` pairs,
//! `mp4dump`-verified): track 1 (H.264 video) is protected, track 2 (AAC
//! audio) is left cleartext by Bento4's default single-track behavior — this
//! is exactly the real-world mixed-protection multiplex `cenc_decrypt.rs`'s
//! fragmented-CMAF support (issue #564) targets.
//!
//! The golden-interop cross-check shells out to Bento4's `mp4decrypt` (the
//! read side of the same independent tool) and demuxes ITS output — which
//! Bento4 rewrites back to a plain (`avc1`, no `sinf`) fragmented fMP4 — with
//! this crate's own regular, non-CENC [`Fmp4Demux`], then compares NAL
//! payloads byte-for-byte against this module's own `CencDecryptor` output.
//! `mp4decrypt` is optional at test-run time (skipped with a printed reason if
//! absent from `PATH`), matching the established pattern for
//! external-tool-backed tests elsewhere in this crate (see
//! `tests/golden_gate.rs`'s `ffprobe_available`/`skip_unless!`).

use std::process::Command;

use broadcast_common::{Decrypt, Unpackage};
use transmux::annexb::iter_length_prefixed_nals;
use transmux::cenc_decrypt::{CencDecryptor, CencScheme, KeyMap};
use transmux::{CodecConfig, Fmp4Demux, Sample};

const CENC_MP4: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../fixtures/transmux/h264_cenc.mp4"
);
const CBCS_MP4: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../fixtures/transmux/h264_cbcs.mp4"
);

// Key material from `fixtures/transmux/h264_cenc-PROVENANCE.md`: the same
// 16 bytes serve as both the KID and the AES-128 content key (a fixture
// convenience, not real-world practice). The provenance file's third value
// (`100102030405060708090a0b0c0d0e0f`) is the encryption-time IV passed to
// `mp4encrypt`'s 3-part `--key <track>:<key>:<iv>` — it is not needed here:
// `CencDecryptor` recovers the real per-sample IV from each sample's own
// `senc` entry, and `mp4decrypt`'s 2-part `--key <id>:<k>` takes no IV at all.
const KEY: [u8; 16] = [
    0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f,
];
const KID: [u8; 16] = KEY;

fn read(path: &str) -> Vec<u8> {
    std::fs::read(path).unwrap_or_else(|e| panic!("fixture {path}: {e}"))
}

fn keys() -> KeyMap {
    KeyMap::new().with_key(KID, KEY)
}

fn to_hex(bytes: &[u8; 16]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Collect every NAL payload across every video sample of a track, in order.
fn nal_payloads(samples: &[Sample]) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    for s in samples {
        for nal in iter_length_prefixed_nals(&s.data).expect("length-prefixed NALs") {
            out.push(nal.to_vec());
        }
    }
    out
}

// ── External-tool availability gate (mirrors tests/golden_gate.rs) ─────────

/// True if `mp4decrypt` (Bento4) is on `PATH`. Bento4 CLIs print their usage
/// banner to **stderr** and exit non-zero when run with no arguments, so
/// availability is checked by output content (on stderr), not exit status
/// (same reasoning as `golden_gate.rs`'s `ffprobe_available`).
fn mp4decrypt_available() -> bool {
    Command::new("mp4decrypt").output().is_ok_and(|o| {
        String::from_utf8_lossy(&o.stdout).contains("MP4 Decrypter")
            || String::from_utf8_lossy(&o.stderr).contains("MP4 Decrypter")
    })
}

// ── Test 1 + 2: fragmented demux + decrypt succeeds (the actual bug fixed) ─

#[test]
fn cenc_fragmented_demux_and_decrypt_succeeds() {
    let file = read(CENC_MP4);
    let dec = CencDecryptor::from_fmp4(&file).expect("harvest fragmented CENC metadata");
    assert_eq!(dec.scheme(), Some(CencScheme::Cenc), "scheme must be cenc");
    assert_eq!(&dec.original_format(), b"avc1");

    let mut media = dec.demux().expect("demux fragmented protected fMP4");
    assert_eq!(
        media.tracks.len(),
        1,
        "only the protected video track is carried (Bento4 left track 2 cleartext)"
    );
    // 3 moof fragments: 25 + 25 video samples (fragment 3 carries only the
    // audio track's traf, mp4dump-verified) — proves fragments were walked
    // and concatenated, not just the first one.
    assert_eq!(
        media.tracks[0].samples.len(),
        50,
        "video samples across all fragments must be concatenated in file order"
    );

    dec.decrypt(&mut media, &keys())
        .expect("decrypt fragmented cenc samples");
    let nals = nal_payloads(&media.tracks[0].samples);
    assert!(!nals.is_empty(), "decrypted output must carry NALs");
    assert_eq!(
        nals[0][0] & 0x80,
        0,
        "decrypted NAL header sane (forbidden_zero_bit clear)"
    );
}

#[test]
fn cbcs_fragmented_demux_and_decrypt_succeeds() {
    let file = read(CBCS_MP4);
    let dec = CencDecryptor::from_fmp4(&file).expect("harvest fragmented CBCS metadata");
    assert_eq!(dec.scheme(), Some(CencScheme::Cbcs), "scheme must be cbcs");
    assert_eq!(&dec.original_format(), b"avc1");

    let tenc = dec.track_encryption().expect("tenc present");
    assert_eq!(
        tenc.default_per_sample_iv_size, 0,
        "cbcs uses a constant IV"
    );
    assert!(
        tenc.default_constant_iv.is_some(),
        "tenc must carry default_constant_IV when per_sample_iv_size == 0"
    );
    assert_eq!(tenc.default_crypt_byte_block, 1);
    assert_eq!(tenc.default_skip_byte_block, 9);

    let mut media = dec.demux().expect("demux fragmented protected fMP4");
    assert_eq!(media.tracks.len(), 1);
    assert_eq!(media.tracks[0].samples.len(), 50);

    dec.decrypt(&mut media, &keys())
        .expect("decrypt fragmented cbcs samples");
    let nals = nal_payloads(&media.tracks[0].samples);
    assert!(!nals.is_empty());
    assert_eq!(
        nals[0][0] & 0x80,
        0,
        "decrypted NAL header sane (forbidden_zero_bit clear)"
    );
}

// ── Test 3: golden-interop cross-check against Bento4's mp4decrypt ─────────

/// Decrypt `fixture_path` with Bento4's `mp4decrypt`, demux ITS (now plain
/// `avc1`) output with this crate's own regular fragmented demux
/// ([`Fmp4Demux`]), and assert our own `CencDecryptor` path reproduces the
/// exact same NAL payloads. Skips cleanly (printing why) if `mp4decrypt`
/// isn't on `PATH`.
fn assert_matches_bento4_reference(fixture_path: &str, tag: &str) {
    if !mp4decrypt_available() {
        eprintln!(
            "SKIP cenc_fragmented_fixture::{tag}: mp4decrypt (Bento4) not found on PATH \
             (install via `brew install bento4`) — golden-interop cross-check not run"
        );
        return;
    }

    let out_path = std::env::temp_dir().join(format!(
        "cenc_fragmented_fixture_{tag}_{}.mp4",
        std::process::id()
    ));
    // `mp4decrypt --key <id>:<k>` takes a KID-or-track-ID and the AES-128
    // *content key* — unlike `mp4encrypt`, it takes no IV argument at all (the
    // IV is self-describing, recovered from each sample's own `senc` entry).
    // Passing the encryption-time IV here (as one might expect by analogy
    // with `mp4encrypt`'s 3-part `--key <track>:<key>:<iv>`) silently decrypts
    // with the wrong key material instead of erroring — caught during
    // development by manually re-deriving one sample's plaintext with a
    // known-good AES-CTR reference and comparing against `mp4decrypt`'s own
    // output before trusting it as the oracle.
    let key_arg = format!("{}:{}", to_hex(&KID), to_hex(&KEY));
    let status = Command::new("mp4decrypt")
        .arg("--key")
        .arg(&key_arg)
        .arg(fixture_path)
        .arg(&out_path)
        .status()
        .expect("spawn mp4decrypt");
    assert!(
        status.success(),
        "mp4decrypt failed for {fixture_path} (key={key_arg})"
    );

    // Reference: demux Bento4's own decrypted output — a plain (avc1, no
    // sinf) fragmented fMP4 with the identical sample bytes our own decrypt
    // path should produce — with this crate's regular, non-CENC fMP4 demux.
    let ref_bytes = std::fs::read(&out_path).expect("read mp4decrypt output");
    let mut demux = Fmp4Demux::new();
    let ref_media = demux
        .unpackage(&ref_bytes)
        .expect("demux mp4decrypt reference output");
    let ref_video = ref_media
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
        .expect("reference output must carry a video track");
    let ref_nals = nal_payloads(&ref_video.samples);
    let _ = std::fs::remove_file(&out_path);

    // Ours.
    let file = read(fixture_path);
    let dec = CencDecryptor::from_fmp4(&file).expect("harvest crypto metadata");
    let mut media = dec.demux().expect("demux protected fMP4");
    dec.decrypt(&mut media, &keys()).expect("decrypt");
    let our_nals = nal_payloads(&media.tracks[0].samples);

    assert_eq!(
        our_nals.len(),
        ref_nals.len(),
        "{tag}: NAL count must match the Bento4 mp4decrypt reference"
    );
    for (i, (ours, reference)) in our_nals.iter().zip(ref_nals.iter()).enumerate() {
        assert_eq!(
            ours, reference,
            "{tag}: NAL {i} must be byte-identical to the Bento4 mp4decrypt reference"
        );
    }
}

#[test]
fn cenc_matches_bento4_mp4decrypt_reference() {
    assert_matches_bento4_reference(CENC_MP4, "cenc");
}

#[test]
fn cbcs_matches_bento4_mp4decrypt_reference() {
    assert_matches_bento4_reference(CBCS_MP4, "cbcs");
}

// ── Test 4: mutation-bite test — a corrupted ciphertext byte must NOT ──────
// ── silently decrypt to the same plaintext (proves real decryption). ──────

/// Absolute file offset of the first top-level `mdat` box's body (its first
/// encrypted-sample byte). Hand-rolled 32-bit box walk (test-only): every box
/// in these fixtures uses the compact 32-bit size form (mp4dump-verified), so
/// no largesize/uuid handling is needed here.
fn first_mdat_body_offset(file: &[u8]) -> usize {
    let mut offset = 0usize;
    loop {
        assert!(offset + 8 <= file.len(), "mdat not found before EOF");
        let size = u32::from_be_bytes(file[offset..offset + 4].try_into().unwrap()) as usize;
        let box_type = &file[offset + 4..offset + 8];
        if box_type == b"mdat" {
            return offset + 8;
        }
        assert!(
            size >= 8,
            "unexpected non-positive box size while scanning for mdat"
        );
        offset += size;
    }
}

fn assert_mutation_changes_output(fixture_path: &str, tag: &str) {
    let mut file = read(fixture_path);

    // Baseline (golden) decrypt. Compare raw sample bytes rather than parsed
    // NAL payloads: a mutated ciphertext byte decrypts to garbage that need
    // not even be a valid length-prefixed NAL stream, so parsing it (as
    // `nal_payloads` does) could itself error instead of just differing.
    let dec = CencDecryptor::from_fmp4(&file).expect("harvest crypto metadata");
    let mut good = dec.demux().expect("demux protected fMP4");
    dec.decrypt(&mut good, &keys()).expect("decrypt (baseline)");
    let good_first_sample = good.tracks[0].samples[0].data.clone();

    // Flip one byte inside the first mdat's body — squarely inside the first
    // fragment's encrypted video sample bytes (mp4dump-verified: the first
    // traf's trun.data_offset for both fixtures places track 1's first
    // sample right at the start of the first mdat's body).
    let mutate_at = first_mdat_body_offset(&file);
    file[mutate_at] ^= 0xFF;

    let dec_mutated =
        CencDecryptor::from_fmp4(&file).expect("harvest crypto metadata (mutated file)");
    let mut bad = dec_mutated.demux().expect("demux protected fMP4 (mutated)");
    dec_mutated
        .decrypt(&mut bad, &keys())
        .expect("decrypt (mutated)");
    let bad_first_sample = &bad.tracks[0].samples[0].data;

    assert_eq!(
        good_first_sample.len(),
        bad_first_sample.len(),
        "{tag}: mutation must not resize the sample"
    );
    assert_ne!(
        &good_first_sample, bad_first_sample,
        "{tag}: a corrupted ciphertext byte must yield different plaintext \
         (a no-op passthrough would leave this unchanged)"
    );
}

#[test]
fn cenc_mutation_bite() {
    assert_mutation_changes_output(CENC_MP4, "cenc");
}

#[test]
fn cbcs_mutation_bite() {
    assert_mutation_changes_output(CBCS_MP4, "cbcs");
}
