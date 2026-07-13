//! `CencEncryptor` integration tests (issue #564) — public-API-only coverage.
//!
//! Complements the byte-exact IR-level round-trip unit tests inside
//! `src/cenc_encrypt.rs` (which reverse via the shared, crate-internal cipher
//! core `cenc_crypto::apply_ctr`/`cbcs_sample` directly — those functions are
//! `pub(crate)`, so an integration test file like this one, compiled as a
//! separate crate linking only `transmux`'s public API, cannot call them).
//! This file instead exercises exactly what a caller of the crate can reach:
//! [`CencEncryptor`]/[`Encrypt`], [`EncryptConfig`], [`IvGen`],
//! [`SubsamplePolicy`].
//!
//! For **`cenc`** (AES-CTR), the round trip is still verified **byte-for-byte
//! identical** through the public API alone: AES-CTR keystream XOR is its own
//! inverse, so re-running [`CencEncryptor::encrypt`] a *second* time with the
//! identical deterministic config (same KID/key/`IvGen::Counter` base, same
//! subsample policy) reproduces the original cleartext exactly — the
//! ciphertext's NAL length-prefixes and headers are left clear by
//! construction, so the subsample map recomputed from the ciphertext is
//! identical to the one computed from the cleartext, so the recomputed
//! per-sample IVs/keystreams line up and the second XOR cancels the first.
//!
//! For **`cbcs`** (AES-CBC pattern), that trick does not apply (CBC chaining
//! is not self-inverse), and there is no public decrypt-from-`Media` entry
//! point yet (`CencDecryptor::from_fmp4` needs a real protected fMP4 file —
//! the muxer doesn't emit `sinf`/`senc` until Tasks 3/4 land). This file
//! therefore verifies `cbcs` behaviourally: real bytes change, the recorded
//! [`transmux::TrackEncryption`] is well-formed (one entry per sample, the
//! configured pattern, a well-formed subsample map), and two different keys
//! produce different ciphertexts (proving genuine encryption, not a
//! passthrough) — the true byte-exact `cbcs` reversal is covered by the
//! in-crate unit test.
//!
//! Skips cleanly if the (normally-committed) cleartext fixture is absent.

#![cfg(feature = "cenc")]

use std::path::PathBuf;

use broadcast_common::{Encrypt, Unpackage};
use transmux::{CencEncryptor, CencScheme, CodecConfig, EncryptConfig, IvGen, SubsamplePolicy};
use transmux::{Media, TsDemux};

const KID_A: [u8; 16] = [
    0xa7, 0xe6, 0x1c, 0x37, 0x3e, 0x21, 0x90, 0x33, 0xc2, 0x10, 0x91, 0xfa, 0x60, 0x7b, 0xf3, 0xb8,
];
const KEY_A: [u8; 16] = [
    0x76, 0xa6, 0xc6, 0x5c, 0x5e, 0xa7, 0x62, 0x04, 0x6b, 0xd7, 0x49, 0xa2, 0xe6, 0x32, 0xcc, 0xbb,
];
const KEY_B: [u8; 16] = [
    0xff, 0xa6, 0xc6, 0x5c, 0x5e, 0xa7, 0x62, 0x04, 0x6b, 0xd7, 0x49, 0xa2, 0xe6, 0x32, 0xcc, 0xbb,
];

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/ts/h264/main.ts")
}

/// The cleartext fixture, narrowed to its single AVC video track (main.ts may
/// also carry an audio track with a *different* sample count, and
/// `IvGen::Explicit` supplies one IV list shared across every track — see
/// `EncryptConfig`'s docs — so a single-track `Media` keeps every test in
/// this file unambiguous).
fn clear_video_media() -> Option<Media> {
    let path = fixture_path();
    if !path.exists() {
        eprintln!(
            "cenc_encrypt tests: SKIPPED — {path:?} not found (expected committed public fixture)."
        );
        return None;
    }
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}"));
    let mut demux = TsDemux::new();
    let media = demux.unpackage(bytes.as_slice()).expect("demux main.ts");
    Some(
        media
            .select_tracks_by(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
            .expect("AVC video track present"),
    )
}

fn snapshot(media: &Media) -> Vec<Vec<u8>> {
    media.tracks[0]
        .samples
        .iter()
        .map(|s| s.data.clone())
        .collect()
}

fn cenc_cfg() -> EncryptConfig {
    EncryptConfig {
        scheme: CencScheme::Cenc,
        kid: KID_A,
        key: KEY_A,
        iv: IvGen::Counter { base: 0 },
        pattern: None,
        subsample: SubsamplePolicy::Video,
    }
}

/// `cenc` IR round trip: two encrypt passes with the identical deterministic
/// config reproduce the original cleartext, byte-for-byte, purely through the
/// public `Encrypt` API (see module docs for why this proves a real
/// self-inverse cipher, not a passthrough).
#[test]
fn cenc_double_encrypt_reproduces_cleartext() {
    let Some(mut media) = clear_video_media() else {
        return;
    };
    let original = snapshot(&media);
    let cfg = cenc_cfg();

    CencEncryptor
        .encrypt(&mut media, &cfg)
        .expect("first encrypt");
    let after_first = snapshot(&media);
    assert_ne!(after_first, original, "first encrypt must change bytes");

    let enc = media.tracks[0]
        .encryption
        .as_ref()
        .expect("track.encryption populated");
    assert_eq!(enc.scheme, CencScheme::Cenc);
    assert_eq!(enc.samples.len(), media.tracks[0].samples.len());
    for entry in &enc.samples {
        assert_eq!(entry.initialization_vector.len(), 8, "8-byte counter IV");
    }

    CencEncryptor
        .encrypt(&mut media, &cfg)
        .expect("second encrypt (self-inverse)");
    let after_second = snapshot(&media);
    assert_eq!(
        after_second, original,
        "re-encrypting with the identical deterministic config must reproduce \
         the cleartext (AES-CTR keystream XOR is its own inverse)"
    );
}

/// `cbcs`: real encryption happens (bytes change, differ by key), and the
/// recorded [`transmux::TrackEncryption`] is well-formed — one entry per
/// sample, the configured `1:9` pattern, and a subsample map whose
/// clear+protected byte counts sum to each sample's length.
#[test]
fn cbcs_encrypt_changes_bytes_and_records_well_formed_metadata() {
    let Some(mut media_a) = clear_video_media() else {
        return;
    };
    let original = snapshot(&media_a);

    let cfg_a = EncryptConfig {
        scheme: CencScheme::Cbcs,
        kid: KID_A,
        key: KEY_A,
        iv: IvGen::Counter { base: 0 },
        pattern: Some((1, 9)),
        subsample: SubsamplePolicy::Video,
    };
    CencEncryptor
        .encrypt(&mut media_a, &cfg_a)
        .expect("encrypt (key A)");
    let encrypted_a = snapshot(&media_a);
    assert_ne!(encrypted_a, original, "cbcs encrypt must change bytes");

    let enc = media_a.tracks[0]
        .encryption
        .as_ref()
        .expect("track.encryption populated");
    assert_eq!(enc.scheme, CencScheme::Cbcs);
    assert_eq!(enc.tenc.default_crypt_byte_block, 1);
    assert_eq!(enc.tenc.default_skip_byte_block, 9);
    assert_eq!(enc.samples.len(), media_a.tracks[0].samples.len());
    for (sample, entry) in media_a.tracks[0].samples.iter().zip(enc.samples.iter()) {
        let covered: usize = entry
            .subsamples
            .iter()
            .map(|s| s.bytes_of_clear_data as usize + s.bytes_of_protected_data as usize)
            .sum();
        assert_eq!(
            covered,
            sample.data.len(),
            "subsample map must cover the whole sample"
        );
    }

    // A different key must produce different ciphertext (proves real AES-CBC
    // encryption, not an identity/passthrough).
    let Some(mut media_b) = clear_video_media() else {
        return;
    };
    let cfg_b = EncryptConfig {
        key: KEY_B,
        ..cfg_a
    };
    CencEncryptor
        .encrypt(&mut media_b, &cfg_b)
        .expect("encrypt (key B)");
    let encrypted_b = snapshot(&media_b);
    assert_ne!(
        encrypted_a, encrypted_b,
        "different keys must yield different ciphertext"
    );
}

/// [`SubsamplePolicy::WholeSample`] records an empty subsample map (whole
/// sample protected, ISO/IEC 23001-7 §9.3).
#[test]
fn whole_sample_policy_yields_empty_subsample_map() {
    let Some(mut media) = clear_video_media() else {
        return;
    };
    let cfg = EncryptConfig {
        subsample: SubsamplePolicy::WholeSample,
        ..cenc_cfg()
    };
    CencEncryptor.encrypt(&mut media, &cfg).expect("encrypt");
    let enc = media.tracks[0].encryption.as_ref().expect("Some");
    assert!(
        enc.samples.iter().all(|e| e.subsamples.is_empty()),
        "WholeSample policy must record an empty subsample map"
    );
}

/// `IvGen::Explicit` with a list whose length doesn't match the track's
/// sample count must error, not silently truncate/pad.
#[test]
fn explicit_iv_count_mismatch_errors() {
    let Some(mut media) = clear_video_media() else {
        return;
    };
    let n = media.tracks[0].samples.len();
    assert!(n > 1, "fixture must have more than one sample to bite");
    let cfg = EncryptConfig {
        iv: IvGen::Explicit(vec![vec![0u8; 8]; n - 1]),
        ..cenc_cfg()
    };
    let err = CencEncryptor.encrypt(&mut media, &cfg).unwrap_err();
    assert!(
        matches!(err, transmux::Error::InvalidInput(_)),
        "expected InvalidInput, got {err:?}"
    );
}

/// An `IvGen::Explicit` IV longer than 16 bytes must error.
#[test]
fn explicit_iv_too_long_errors() {
    let Some(mut media) = clear_video_media() else {
        return;
    };
    let n = media.tracks[0].samples.len();
    let cfg = EncryptConfig {
        iv: IvGen::Explicit(vec![vec![0u8; 17]; n]),
        ..cenc_cfg()
    };
    let err = CencEncryptor.encrypt(&mut media, &cfg).unwrap_err();
    assert!(
        matches!(err, transmux::Error::InvalidInput(_)),
        "expected InvalidInput, got {err:?}"
    );
}
