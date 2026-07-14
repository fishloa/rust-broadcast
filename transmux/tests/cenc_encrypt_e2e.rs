//! CENC/CBCS encrypt path — end-to-end correctness proof (issue #564 Task 4).
//!
//! This is the gate the design doc (`docs/superpowers/specs/2026-07-13-cenc-encrypt-design.md`
//! §"Testing") calls out as the ungameable one: it exercises the *entire*
//! pipeline through public APIs only —
//!
//! ```text
//! cleartext Media (TsDemux over fixtures/ts/h264/main.ts, AVC track)
//!   -> CencEncryptor::encrypt            (Task 2)
//!   -> CmafMux::package                  (existing, untouched)
//!   -> protect_init_segment + protect_media_segment (Task 3)
//!   -> a single standalone fMP4 file written to a temp path
//! ```
//!
//! and then proves that file is *actually* a standards-compliant encrypted
//! CMAF, two independent ways:
//!
//! 1. **Self round-trip**: `CencDecryptor::from_fmp4` + `Decrypt::decrypt`
//!    (this crate's own decrypt path, shipped and tested independently)
//!    recovers samples byte-identical to the pre-encryption snapshot.
//! 2. **Golden interop**: Bento4's `mp4decrypt` — a real, independent,
//!    third-party CENC/CBCS decryptor — decrypts the same file, and its
//!    output (re-demuxed with this crate's plain, non-CENC `Fmp4Demux`) is
//!    also byte-identical to the snapshot. This is the authoritative check of
//!    the `saio` moof-relative anchor decision made in Task 3 (see
//!    `.superpowers/sdd/task-3-report.md`): if the anchor were wrong,
//!    `mp4decrypt` would either fail outright or silently decrypt garbage
//!    (XORing the wrong ciphertext bytes against the right keystream), while
//!    the self round-trip (which shares the same, possibly-wrong, offset
//!    convention on both write and read sides internal to this crate) would
//!    still falsely pass.
//!
//! `mp4decrypt` is optional at test-run time (skipped with a printed reason
//! if absent from `PATH`), mirroring the established pattern in
//! `tests/cenc_fragmented_fixture.rs` / `tests/golden_gate.rs`.

#![cfg(feature = "cenc")]

use std::path::PathBuf;
use std::process::Command;

use broadcast_common::{Decrypt, Encrypt, Package, Unpackage};
use transmux::init_segment::protect_init_segment;
use transmux::movie_fragment::{FragmentProtection, protect_media_segment};
use transmux::{
    CencDecryptor, CencEncryptor, CencScheme, CmafMux, CodecConfig, EncryptConfig, Fmp4Demux,
    IvGen, KeyMap, Media, SubsamplePolicy, TrackEncryption,
};

/// Constant `cbcs` IV — the standard real-world `cbcs` convention (confirmed
/// against Bento4's `mp4encrypt`, which always emits a `tenc.default_constant_IV`
/// for `cbcs` regardless of the `--key` IV given it; a per-sample `cbcs` IV, if
/// used at all, must be this same 16 bytes, not `cenc`'s 8-byte counter
/// convention — Bento4's `mp4decrypt` silently no-ops on an 8-byte `cbcs`
/// per-sample IV). 16 arbitrary bytes, distinct from the KID/KEY below so a
/// transposition bug between them would not accidentally cancel out.
const CBCS_CONSTANT_IV: [u8; 16] = [
    0xf0, 0xf1, 0xf2, 0xf3, 0xf4, 0xf5, 0xf6, 0xf7, 0xf8, 0xf9, 0xfa, 0xfb, 0xfc, 0xfd, 0xfe, 0xff,
];

/// Test KID: `00112233445566778899aabbccddeeff` (32 hex chars).
const KID: [u8; 16] = [
    0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff,
];
/// Test AES-128 content key: `000102030405060708090a0b0c0d0e0f` (32 hex chars).
const KEY: [u8; 16] = [
    0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f,
];

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/ts/h264/main.ts")
}

/// The cleartext fixture, narrowed to its single AVC video track (mirrors
/// `tests/cenc_encrypt.rs`/`tests/cenc_mux.rs`'s identical helper).
fn clear_video_media() -> Option<Media> {
    let path = fixture_path();
    if !path.exists() {
        eprintln!(
            "cenc_encrypt_e2e tests: SKIPPED — {path:?} not found (expected committed public fixture)."
        );
        return None;
    }
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}"));
    let mut demux = transmux::TsDemux::new();
    let media = demux.unpackage(bytes.as_slice()).expect("demux main.ts");
    Some(
        media
            .select_tracks_by(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
            .expect("AVC video track present"),
    )
}

/// Snapshot every sample's bytes for the (single) track, in decode order.
fn snapshot(media: &Media) -> Vec<Vec<u8>> {
    media.tracks[0]
        .samples
        .iter()
        .map(|s| s.data.clone())
        .collect()
}

fn to_hex(bytes: &[u8; 16]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn keys() -> KeyMap {
    KeyMap::new().with_key(KID, KEY)
}

/// True if `mp4decrypt` (Bento4) is on `PATH` — same detection strategy as
/// `tests/cenc_fragmented_fixture.rs::mp4decrypt_available` (Bento4 CLIs
/// print their usage banner to stderr and exit non-zero with no arguments).
fn mp4decrypt_available() -> bool {
    Command::new("mp4decrypt").output().is_ok_and(|o| {
        String::from_utf8_lossy(&o.stdout).contains("MP4 Decrypter")
            || String::from_utf8_lossy(&o.stderr).contains("MP4 Decrypter")
    })
}

/// Encrypt `media` in place per `cfg`, mux it through the real `CmafMux`
/// packager, then apply both Task-3 protection passes. Returns the fully
/// protected, standalone (ftyp+moov+styp+moof+mdat) fMP4 bytes.
fn build_protected_fmp4(media: &mut Media, cfg: &EncryptConfig) -> Vec<u8> {
    CencEncryptor
        .encrypt(&mut *media, cfg)
        .expect("CencEncryptor::encrypt");
    let track_id = media.tracks[0].spec.track_id;
    let enc: TrackEncryption = media.tracks[0]
        .encryption
        .as_ref()
        .expect("Track::encryption populated by CencEncryptor")
        .clone();

    let raw = CmafMux::new(1).package(&*media).expect("CmafMux::package");
    let with_protected_init =
        protect_init_segment(&raw, track_id, &enc).expect("protect_init_segment");
    let fragment_protection = FragmentProtection {
        track_id,
        entries: &enc.samples,
        per_sample_iv_size: enc.tenc.default_per_sample_iv_size,
    };
    protect_media_segment(&with_protected_init, &[fragment_protection])
        .expect("protect_media_segment")
}

/// Write `bytes` to a fresh temp file and return its path.
fn write_temp(bytes: &[u8], tag: &str) -> PathBuf {
    let path =
        std::env::temp_dir().join(format!("cenc_encrypt_e2e_{tag}_{}.mp4", std::process::id()));
    std::fs::write(&path, bytes).unwrap_or_else(|e| panic!("write {path:?}: {e}"));
    path
}

/// The full pipeline for one scheme: encrypt -> mux -> protect -> write ->
/// self round-trip via `CencDecryptor` -> golden interop via `mp4decrypt`
/// (skipped cleanly if the binary is absent).
fn run_e2e(
    scheme: CencScheme,
    iv: IvGen,
    pattern: Option<(u8, u8)>,
    subsample: SubsamplePolicy,
    tag: &str,
) {
    let Some(mut media) = clear_video_media() else {
        return;
    };
    let original = snapshot(&media);
    assert!(
        original.len() > 1,
        "fixture must carry more than one sample to bite"
    );

    let cfg = EncryptConfig {
        scheme,
        kid: KID,
        key: KEY,
        iv,
        pattern,
        subsample,
    };
    let protected_bytes = build_protected_fmp4(&mut media, &cfg);

    let in_path = write_temp(&protected_bytes, tag);

    // ── Self round-trip ─────────────────────────────────────────────────
    let dec = CencDecryptor::from_fmp4(&protected_bytes)
        .unwrap_or_else(|e| panic!("{tag}: CencDecryptor::from_fmp4: {e}"));
    assert_eq!(
        dec.scheme(),
        Some(scheme),
        "{tag}: scheme recovered from sinf/schm"
    );
    let mut recovered = dec.demux().unwrap_or_else(|e| panic!("{tag}: demux: {e}"));
    dec.decrypt(&mut recovered, &keys())
        .unwrap_or_else(|e| panic!("{tag}: decrypt: {e}"));
    let self_round_trip = snapshot(&recovered);
    assert_eq!(
        self_round_trip, original,
        "{tag}: self round-trip (CencDecryptor) must recover byte-identical samples"
    );

    // ── Golden interop (Bento4 mp4decrypt) ──────────────────────────────
    if !mp4decrypt_available() {
        eprintln!(
            "SKIP cenc_encrypt_e2e::{tag}: mp4decrypt (Bento4) not found on PATH \
             (install via `brew install bento4`) — golden-interop cross-check not run"
        );
        let _ = std::fs::remove_file(&in_path);
        return;
    }

    let out_path = std::env::temp_dir().join(format!(
        "cenc_encrypt_e2e_{tag}_out_{}.mp4",
        std::process::id()
    ));
    let key_arg = format!("{}:{}", to_hex(&KID), to_hex(&KEY));
    let status = Command::new("mp4decrypt")
        .arg("--key")
        .arg(&key_arg)
        .arg(&in_path)
        .arg(&out_path)
        .status()
        .expect("spawn mp4decrypt");
    assert!(
        status.success(),
        "{tag}: mp4decrypt failed for {in_path:?} (key={key_arg})"
    );

    let ref_bytes = std::fs::read(&out_path).expect("read mp4decrypt output");
    let mut demux = Fmp4Demux::new();
    let ref_media = demux
        .unpackage(&ref_bytes)
        .expect("demux mp4decrypt reference output");
    let ref_video = ref_media
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
        .expect("reference output must carry a video (AVC) track");
    let ref_samples: Vec<Vec<u8>> = ref_video.samples.iter().map(|s| s.data.clone()).collect();

    assert_eq!(
        ref_samples, original,
        "{tag}: Bento4 mp4decrypt reference output must be byte-identical to the \
         pre-encryption cleartext samples"
    );

    let _ = std::fs::remove_file(&in_path);
    let _ = std::fs::remove_file(&out_path);
}

#[test]
fn cenc_end_to_end_round_trip_and_mp4decrypt_interop() {
    run_e2e(
        CencScheme::Cenc,
        IvGen::Counter { base: 0 },
        None,
        SubsamplePolicy::Video,
        "cenc",
    );
}

/// `cbcs` uses [`SubsamplePolicy::Video`] — a real per-NAL multi-subsample
/// map — plus [`IvGen::Constant`] (the standard `cbcs` constant-IV
/// convention). This is deliberately the case that previously exposed a real
/// `cenc_crypto.rs` bug (issue #564): the `cbcs` CBC chain was incorrectly
/// carried over from one subsample's last crypt block into the *next*
/// subsample's first crypt block, rather than resetting to the sample's seed
/// IV at the start of every subsample's protected range (see
/// `cenc_crypto.rs`'s module docs for the fixed rule, triangulated against
/// Bento4/Shaka). With that fix, this test — using the exact
/// multi-subsample shape that used to diverge from Bento4 — now proves both
/// the box/wire plumbing (`tenc`/`sinf`/`senc`/`saiz`/`saio` emission, the
/// `saio` moof-relative anchor, and the constant-IV wire convention Bento4
/// requires for `cbcs`) AND the cipher-core chain-reset fix, end to end
/// against the real Bento4 `mp4decrypt` oracle.
#[test]
fn cbcs_end_to_end_round_trip_and_mp4decrypt_interop() {
    run_e2e(
        CencScheme::Cbcs,
        IvGen::Constant(CBCS_CONSTANT_IV),
        Some((1, 9)),
        SubsamplePolicy::Video,
        "cbcs",
    );
}
