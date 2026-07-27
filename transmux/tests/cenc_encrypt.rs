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
use bytes::Bytes;
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

/// A real **two-track** (H.264 video + AAC audio) MPEG-2 TS capture, with
/// *both* tracks kept — the shape a real caller encrypts, and the only shape
/// that can catch cross-track IV reuse (see
/// [`counter_ivs_are_unique_across_every_track`]).
fn multi_track_fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/ts/h264_aac.ts")
}

/// The two-track cleartext fixture, demuxed with **every** track kept.
fn clear_multi_track_media() -> Option<Media> {
    let path = multi_track_fixture_path();
    if !path.exists() {
        eprintln!(
            "cenc_encrypt tests: SKIPPED — {path:?} not found (expected committed public fixture)."
        );
        return None;
    }
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}"));
    let mut demux = TsDemux::new();
    Some(
        demux
            .unpackage(bytes.as_slice())
            .expect("demux h264_aac.ts"),
    )
}

/// Every per-sample IV recorded across every track of `media`, in (track,
/// sample) order.
fn all_recorded_ivs(media: &Media) -> Vec<Vec<u8>> {
    media
        .tracks
        .iter()
        .flat_map(|t| {
            t.encryption
                .as_ref()
                .expect("every track must record encryption metadata")
                .samples
                .iter()
                .map(|e| e.initialization_vector.clone())
        })
        .collect()
}

/// **The two-time-pad regression test (the centrepiece).**
///
/// ISO/IEC 23001-7 §9.2 requires a sample's IV to be unique **per key**, not
/// per track. `EncryptConfig` applies one key to every track, so the
/// per-sample IV counter must run across the whole [`Media`] — if it restarts
/// at `base` for each track, video sample *i* and audio sample *i* are
/// encrypted with the same AES-CTR key **and** the same counter block, and
/// `ciphertext_video XOR ciphertext_audio == plaintext_video XOR
/// plaintext_audio`: a classic two-time pad that discloses both plaintexts
/// without the key.
///
/// A single-track fixture cannot catch this class of bug — which is exactly
/// why every other test in this file narrows the fixture to one track, and
/// exactly why the defect shipped with a green suite.
#[test]
fn counter_ivs_are_unique_across_every_track() {
    let Some(mut media) = clear_multi_track_media() else {
        return;
    };
    assert!(
        media.tracks.len() > 1,
        "this test is meaningless on a single-track Media: the fixture must \
         carry at least two tracks (got {})",
        media.tracks.len()
    );
    let total_samples: usize = media.tracks.iter().map(|t| t.samples.len()).sum();

    CencEncryptor::new(KEY_A)
        .encrypt(&mut media, &cenc_cfg())
        .expect("encrypt a multi-track Media");

    let ivs = all_recorded_ivs(&media);
    assert_eq!(
        ivs.len(),
        total_samples,
        "one recorded IV per sample, across every track"
    );
    let unique: std::collections::BTreeSet<&Vec<u8>> = ivs.iter().collect();
    assert_eq!(
        unique.len(),
        ivs.len(),
        "AES-CTR keystream reuse: {} of {} per-sample IVs are duplicates across \
         the Media's tracks — under one shared key that is a two-time pad",
        ivs.len() - unique.len(),
        ivs.len()
    );
}

/// The cleartext fixture, narrowed to its single AVC video track — a
/// deterministic single-track `Media` for the per-scheme cipher/metadata tests
/// below, whose subject is one track's own behaviour.
///
/// Nothing about IV *uniqueness* can be tested here: that property spans tracks
/// (`IvGen`'s counter and IV list are both indexed across the whole `Media`), so
/// it is covered by [`clear_multi_track_media`]'s tests — chiefly
/// [`counter_ivs_are_unique_across_every_track`]. Narrowing every test to one
/// track is precisely how the cross-track keystream-reuse defect shipped green.
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

fn snapshot(media: &Media) -> Vec<Bytes> {
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
        iv: IvGen::Counter,
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

    CencEncryptor::new(KEY_A)
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

    CencEncryptor::new(KEY_A)
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
        iv: IvGen::Counter,
        pattern: Some((1, 9)),
        subsample: SubsamplePolicy::Video,
    };
    CencEncryptor::new(KEY_A)
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
    let cfg_b = cfg_a.clone();
    CencEncryptor::new(KEY_B)
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
    CencEncryptor::new(KEY_A)
        .encrypt(&mut media, &cfg)
        .expect("encrypt");
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
    let err = CencEncryptor::new(KEY_A)
        .encrypt(&mut media, &cfg)
        .unwrap_err();
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
    let err = CencEncryptor::new(KEY_A)
        .encrypt(&mut media, &cfg)
        .unwrap_err();
    assert!(
        matches!(err, transmux::Error::InvalidInput(_)),
        "expected InvalidInput, got {err:?}"
    );
}

/// An `IvGen::Explicit` list of empty (0-byte) per-sample IVs must error —
/// only 8- or 16-byte IVs are valid (ISO/IEC 23001-7 §9.2/§12.2). Without this
/// guard, an empty IV silently builds an all-zero AES-CTR counter, reusing the
/// same keystream for every sample (a two-time-pad).
#[test]
fn explicit_iv_empty_errors() {
    let Some(mut media) = clear_video_media() else {
        return;
    };
    let n = media.tracks[0].samples.len();
    let cfg = EncryptConfig {
        iv: IvGen::Explicit(vec![vec![]; n]),
        ..cenc_cfg()
    };
    let err = CencEncryptor::new(KEY_A)
        .encrypt(&mut media, &cfg)
        .unwrap_err();
    assert!(
        matches!(err, transmux::Error::InvalidInput(_)),
        "expected InvalidInput, got {err:?}"
    );
}

/// An `IvGen::Explicit` list of uniform 12-byte per-sample IVs must error —
/// only 8 or 16 bytes are valid lengths on the wire.
#[test]
fn explicit_iv_wrong_uniform_length_errors() {
    let Some(mut media) = clear_video_media() else {
        return;
    };
    let n = media.tracks[0].samples.len();
    let cfg = EncryptConfig {
        iv: IvGen::Explicit(vec![vec![0u8; 12]; n]),
        ..cenc_cfg()
    };
    let err = CencEncryptor::new(KEY_A)
        .encrypt(&mut media, &cfg)
        .unwrap_err();
    assert!(
        matches!(err, transmux::Error::InvalidInput(_)),
        "expected InvalidInput, got {err:?}"
    );
}

/// `n` distinct IVs of `len` bytes each (the last byte counts up) — a valid
/// `IvGen::Explicit` list, since an IV must be unique per content key.
fn distinct_ivs(n: usize, len: usize) -> Vec<Vec<u8>> {
    (0..n)
        .map(|i| {
            let mut iv = vec![0xABu8; len];
            iv[len - 1] = i as u8;
            iv[len - 2] = (i >> 8) as u8;
            iv
        })
        .collect()
}

/// `IvGen::Explicit` with valid 8-byte and 16-byte per-sample IVs is accepted.
#[test]
fn explicit_iv_valid_lengths_are_ok() {
    for len in [8usize, 16] {
        let Some(mut media) = clear_video_media() else {
            return;
        };
        let n = media.tracks[0].samples.len();
        let cfg = EncryptConfig {
            iv: IvGen::Explicit(distinct_ivs(n, len)),
            ..cenc_cfg()
        };
        CencEncryptor::new(KEY_A)
            .encrypt(&mut media, &cfg)
            .unwrap_or_else(|e| panic!("{len}-byte explicit IV must be accepted: {e:?}"));
    }
}

/// `IvGen::Explicit` must reject a list that repeats an IV, even if every
/// length is valid: an IV is unique **per content key** (ISO/IEC 23001-7
/// §9.2), and under `cenc` (AES-CTR) two samples sharing one IV share one
/// keystream (a two-time pad). This is the guard that makes the whole
/// keystream-reuse class unreachable rather than fixing one instance of it.
#[test]
fn explicit_duplicate_ivs_error() {
    let Some(mut media) = clear_video_media() else {
        return;
    };
    let n = media.tracks[0].samples.len();
    assert!(n > 1, "fixture must have more than one sample to bite");
    let mut ivs = distinct_ivs(n, 8);
    ivs[n - 1] = ivs[0].clone(); // one repeat, everything else distinct
    let cfg = EncryptConfig {
        iv: IvGen::Explicit(ivs),
        ..cenc_cfg()
    };
    let err = CencEncryptor::new(KEY_A)
        .encrypt(&mut media, &cfg)
        .unwrap_err();
    assert!(
        matches!(err, transmux::Error::InvalidInput(_)),
        "expected InvalidInput, got {err:?}"
    );
}

/// `IvGen::Explicit`'s count is measured against the **whole `Media`**, not
/// one track: a list sized to the *first* track's sample count (the old,
/// unsafe per-track meaning, which replayed the same IVs for every track) must
/// now be rejected on a multi-track `Media`.
#[test]
fn explicit_iv_count_is_per_media_not_per_track() {
    let Some(mut media) = clear_multi_track_media() else {
        return;
    };
    assert!(media.tracks.len() > 1, "fixture must be multi-track");
    let first_track_n = media.tracks[0].samples.len();
    let total: usize = media.tracks.iter().map(|t| t.samples.len()).sum();
    assert!(
        first_track_n < total,
        "the per-track and per-Media counts must differ for this test to bite"
    );

    // Old (per-track) meaning: rejected.
    let cfg = EncryptConfig {
        iv: IvGen::Explicit(distinct_ivs(first_track_n, 8)),
        ..cenc_cfg()
    };
    let err = CencEncryptor::new(KEY_A)
        .encrypt(&mut media, &cfg)
        .unwrap_err();
    assert!(
        matches!(err, transmux::Error::InvalidInput(_)),
        "expected InvalidInput for a per-track-sized IV list, got {err:?}"
    );

    // New (per-Media) meaning: accepted, and every recorded IV is the one
    // supplied for that (track, sample) position, in order.
    let ivs = distinct_ivs(total, 8);
    let cfg = EncryptConfig {
        iv: IvGen::Explicit(ivs.clone()),
        ..cenc_cfg()
    };
    CencEncryptor::new(KEY_A)
        .encrypt(&mut media, &cfg)
        .expect("a Media-wide IV list must be accepted");
    assert_eq!(
        all_recorded_ivs(&media),
        ivs,
        "explicit IVs must be consumed in (track, sample) order across the whole Media"
    );
}

/// `IvGen::Constant` is `cbcs`-only: under `cenc` (AES-CTR) a constant IV
/// derives one counter block for every sample, so the same keystream protects
/// the whole track (and the previous behaviour was worse still — the `cenc`
/// path never saw `tenc.default_constant_IV`, so it encrypted with an
/// all-zero counter and produced output no conformant decryptor could read).
#[test]
fn constant_iv_under_cenc_errors() {
    let Some(mut media) = clear_video_media() else {
        return;
    };
    let cfg = EncryptConfig {
        iv: IvGen::Constant([0x5Au8; 16]),
        ..cenc_cfg()
    };
    let err = CencEncryptor::new(KEY_A)
        .encrypt(&mut media, &cfg)
        .unwrap_err();
    assert!(
        matches!(err, transmux::Error::InvalidInput(_)),
        "expected InvalidInput, got {err:?}"
    );
    assert!(
        media.tracks[0].encryption.is_none(),
        "a rejected config must leave the Media untouched, not half-encrypted"
    );
}

/// The companion to [`constant_iv_under_cenc_errors`]: `IvGen::Constant` under
/// `cbcs` is unchanged and still works (it is the standard `cbcs` convention),
/// recording the constant IV in `tenc.default_constant_IV` with
/// `default_per_sample_iv_size == 0` and no per-sample `senc` IV.
#[test]
fn constant_iv_under_cbcs_still_works() {
    let Some(mut media) = clear_video_media() else {
        return;
    };
    const CONSTANT_IV: [u8; 16] = [
        0xf0, 0xf1, 0xf2, 0xf3, 0xf4, 0xf5, 0xf6, 0xf7, 0xf8, 0xf9, 0xfa, 0xfb, 0xfc, 0xfd, 0xfe,
        0xff,
    ];
    let original = snapshot(&media);
    let cfg = EncryptConfig {
        scheme: CencScheme::Cbcs,
        iv: IvGen::Constant(CONSTANT_IV),
        pattern: Some((1, 9)),
        ..cenc_cfg()
    };
    CencEncryptor::new(KEY_A)
        .encrypt(&mut media, &cfg)
        .expect("cbcs + constant IV is the standard convention and must be accepted");
    let enc = media.tracks[0].encryption.as_ref().expect("Some");
    assert_eq!(enc.tenc.default_per_sample_iv_size, 0);
    assert_eq!(
        enc.tenc.default_constant_iv.as_deref(),
        Some(&CONSTANT_IV[..])
    );
    assert!(
        enc.samples
            .iter()
            .all(|e| e.initialization_vector.is_empty()),
        "a constant-IV track carries no per-sample senc IV"
    );
    assert_ne!(snapshot(&media), original, "cbcs must change bytes");
}

/// A `cbcs` pattern with `crypt_byte_block == 0` and a nonzero
/// `skip_byte_block` must error rather than silently leave the "protected"
/// range in cleartext.
#[test]
fn cbcs_pattern_zero_crypt_nonzero_skip_errors() {
    let Some(mut media) = clear_video_media() else {
        return;
    };
    let cfg = EncryptConfig {
        scheme: CencScheme::Cbcs,
        pattern: Some((0, 9)),
        ..cenc_cfg()
    };
    let err = CencEncryptor::new(KEY_A)
        .encrypt(&mut media, &cfg)
        .unwrap_err();
    assert!(
        matches!(err, transmux::Error::InvalidInput(_)),
        "expected InvalidInput, got {err:?}"
    );
}

/// A `cbcs` pattern component above 15 must error rather than silently
/// truncate to its low 4 bits when packed into `tenc`.
#[test]
fn cbcs_pattern_component_too_large_errors() {
    let Some(mut media) = clear_video_media() else {
        return;
    };
    let cfg = EncryptConfig {
        scheme: CencScheme::Cbcs,
        pattern: Some((17, 9)),
        ..cenc_cfg()
    };
    let err = CencEncryptor::new(KEY_A)
        .encrypt(&mut media, &cfg)
        .unwrap_err();
    assert!(
        matches!(err, transmux::Error::InvalidInput(_)),
        "expected InvalidInput, got {err:?}"
    );
}
