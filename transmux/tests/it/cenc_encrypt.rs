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

use broadcast_common::timescale::Timescale;
use transmux::TrackId;

use broadcast_common::{Encrypt, Serialize, Unpackage};
use bytes::Bytes;
use transmux::{
    CencEncryptor, CencScheme, CodecConfig, EncryptConfig, IvGen, SubsamplePolicy, Track,
};
use transmux::{Media, TsDemux};
use transmux::media::Fmp4Demux;

fn timescale(value: u32) -> Timescale {
    Timescale::try_new(value).expect("a non-zero timescale")
}

const KID_A: [u8; 16] = [
    0xa7, 0xe6, 0x1c, 0x37, 0x3e, 0x21, 0x90, 0x33, 0xc2, 0x10, 0x91, 0xfa, 0x60, 0x7b, 0xf3, 0xb8,
];
const KEY_A: [u8; 16] = [
    0x76, 0xa6, 0xc6, 0x5c, 0x5e, 0xa7, 0x62, 0x04, 0x6b, 0xd7, 0x49, 0xa2, 0xe6, 0x32, 0xcc, 0xbb,
];
const KEY_B: [u8; 16] = [
    0xff, 0xa6, 0xc6, 0x5c, 0x5e, 0xa7, 0x62, 0x04, 0x6b, 0xd7, 0x49, 0xa2, 0xe6, 0x32, 0xcc, 0xbb,
];

/// A real **two-track** (H.264 video + AAC audio) MPEG-2 TS capture, with
/// *both* tracks kept — the shape a real caller encrypts, and the only shape
/// that can catch cross-track IV reuse (see
/// [`counter_ivs_are_unique_across_every_track`]).
fn clear_multi_track_media() -> Media {
    let bytes = test_fixtures::read("ts/h264-aac.ts");
    TsDemux::new()
        .unpackage(bytes.as_slice())
        .expect("demux h264-aac.ts")
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
    let mut media = clear_multi_track_media();
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
fn clear_video_media() -> Media {
    let bytes = test_fixtures::read("ts/h264/main.ts");
    let media = TsDemux::new()
        .unpackage(bytes.as_slice())
        .expect("demux main.ts");
    media
        .select_tracks_by(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
        .expect("AVC video track present")
}

fn snapshot(media: &Media) -> Vec<Bytes> {
    media.tracks[0]
        .samples
        .iter()
        .map(|s| s.data.clone())
        .collect()
}

fn cenc_cfg() -> EncryptConfig {
    EncryptConfig::new(CencScheme::Cenc, KID_A, SubsamplePolicy::Video)
}

/// `cenc` IR round trip: two encrypt passes with the identical deterministic
/// config reproduce the original cleartext, byte-for-byte, purely through the
/// public `Encrypt` API (see module docs for why this proves a real
/// self-inverse cipher, not a passthrough).
#[test]
fn cenc_double_encrypt_reproduces_cleartext() {
    let mut media = clear_video_media();
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
    let mut media_a = clear_video_media();
    let original = snapshot(&media_a);

    let cfg_a =
        EncryptConfig::new(CencScheme::Cbcs, KID_A, SubsamplePolicy::Video).with_pattern((1, 9));
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
    let mut media_b = clear_video_media();
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
    let mut media = clear_video_media();
    let cfg = cenc_cfg().with_subsample(SubsamplePolicy::WholeSample);
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
    let mut media = clear_video_media();
    let n = media.tracks[0].samples.len();
    assert!(n > 1, "fixture must have more than one sample to bite");
    let cfg = cenc_cfg().with_iv(IvGen::Explicit(vec![vec![0u8; 8]; n - 1]));
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
    let mut media = clear_video_media();
    let n = media.tracks[0].samples.len();
    let cfg = cenc_cfg().with_iv(IvGen::Explicit(vec![vec![0u8; 17]; n]));
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
    let mut media = clear_video_media();
    let n = media.tracks[0].samples.len();
    let cfg = cenc_cfg().with_iv(IvGen::Explicit(vec![vec![]; n]));
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
    let mut media = clear_video_media();
    let n = media.tracks[0].samples.len();
    let cfg = cenc_cfg().with_iv(IvGen::Explicit(vec![vec![0u8; 12]; n]));
    let err = CencEncryptor::new(KEY_A)
        .encrypt(&mut media, &cfg)
        .unwrap_err();
    assert!(
        matches!(err, transmux::Error::InvalidInput(_)),
        "expected InvalidInput, got {err:?}"
    );
}

/// `n` distinct IVs of `len` bytes each — a valid `IvGen::Explicit` list, since
/// an IV must be unique per content key.
///
/// The IV is the big-endian counter block (low-order bytes last), so the index
/// goes into bits 32..63 of the low half: consecutive samples then start 2^32
/// counter blocks apart, keeping their AES-CTR ranges disjoint under `cenc`
/// (see `cenc_encrypt`'s `assert_ctr_ranges_disjoint`).
fn distinct_ivs(n: usize, len: usize) -> Vec<Vec<u8>> {
    (0..n)
        .map(|i| {
            let mut iv = vec![0xABu8; len];
            iv[len - 5] = (i & 0xFF) as u8;
            iv[len - 6] = ((i >> 8) & 0xFF) as u8;
            iv[len - 7] = ((i >> 16) & 0xFF) as u8;
            iv[len - 8] = ((i >> 24) & 0xFF) as u8;
            iv
        })
        .collect()
}

/// `IvGen::Explicit` with valid 8-byte and 16-byte per-sample IVs is accepted.
#[test]
fn explicit_iv_valid_lengths_are_ok() {
    for len in [8usize, 16] {
        let mut media = clear_video_media();
        let n = media.tracks[0].samples.len();
        let cfg = cenc_cfg().with_iv(IvGen::Explicit(distinct_ivs(n, len)));
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
    let mut media = clear_video_media();
    let n = media.tracks[0].samples.len();
    assert!(n > 1, "fixture must have more than one sample to bite");
    let mut ivs = distinct_ivs(n, 8);
    ivs[n - 1] = ivs[0].clone(); // one repeat, everything else distinct
    let cfg = cenc_cfg().with_iv(IvGen::Explicit(ivs));
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
    let mut media = clear_multi_track_media();
    assert!(media.tracks.len() > 1, "fixture must be multi-track");
    let first_track_n = media.tracks[0].samples.len();
    let total: usize = media.tracks.iter().map(|t| t.samples.len()).sum();
    assert!(
        first_track_n < total,
        "the per-track and per-Media counts must differ for this test to bite"
    );

    // Old (per-track) meaning: rejected.
    let cfg = cenc_cfg().with_iv(IvGen::Explicit(distinct_ivs(first_track_n, 8)));
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
    let cfg = cenc_cfg().with_iv(IvGen::Explicit(ivs.clone()));
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
    let mut media = clear_video_media();
    let cfg = cenc_cfg().with_iv(IvGen::Constant([0x5Au8; 16]));
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
    let mut media = clear_video_media();
    const CONSTANT_IV: [u8; 16] = [
        0xf0, 0xf1, 0xf2, 0xf3, 0xf4, 0xf5, 0xf6, 0xf7, 0xf8, 0xf9, 0xfa, 0xfb, 0xfc, 0xfd, 0xfe,
        0xff,
    ];
    let original = snapshot(&media);
    let cfg = cenc_cfg()
        .with_scheme(CencScheme::Cbcs)
        .with_iv(IvGen::Constant(CONSTANT_IV))
        .with_pattern((1, 9));
    CencEncryptor::new(KEY_A)
        .encrypt(&mut media, &cfg)
        .expect("cbcs + constant IV is the standard convention and must be accepted");
    let enc = media.tracks[0].encryption.as_ref().expect("Some");
    assert_eq!(enc.tenc.default_per_sample_iv_size, 16);
    assert_eq!(
        enc.tenc.default_constant_iv.as_deref(),
        Some(&CONSTANT_IV[..])
    );
    assert!(
        enc.samples
            .iter()
            .all(|e| e.initialization_vector == CONSTANT_IV),
        "default constant_iv_senc=Emit must carry the constant IV in every senc entry"
    );
    assert_ne!(snapshot(&media), original, "cbcs must change bytes");
}

/// A `cbcs` pattern with `crypt_byte_block == 0` and a nonzero
/// `skip_byte_block` must error rather than silently leave the "protected"
/// range in cleartext.
#[test]
fn cbcs_pattern_zero_crypt_nonzero_skip_errors() {
    let mut media = clear_video_media();
    let cfg = cenc_cfg()
        .with_scheme(CencScheme::Cbcs)
        .with_pattern((0, 9));
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
    let mut media = clear_video_media();
    let cfg = cenc_cfg()
        .with_scheme(CencScheme::Cbcs)
        .with_pattern((17, 9));
    let err = CencEncryptor::new(KEY_A)
        .encrypt(&mut media, &cfg)
        .unwrap_err();
    assert!(
        matches!(err, transmux::Error::InvalidInput(_)),
        "expected InvalidInput, got {err:?}"
    );
}

// ---------------------------------------------------------------------------
// Atomicity of a rejected call (r05-C5)
// ---------------------------------------------------------------------------

/// One 4-byte-length-prefixed NAL, as an AVC [`Sample`] expects.
fn lp_nal(payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + payload.len());
    out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    out.extend_from_slice(payload);
    out
}

/// An AVC track spec — the same minimal shape `tests/it/splice.rs` builds.
fn avc_spec(track_id: u32) -> transmux::TrackSpec {
    let record = transmux::AVCDecoderConfigurationRecord {
        configuration_version: 1,
        profile_indication: 66,
        profile_compatibility: 0,
        level_indication: 30,
        length_size_minus_one: 3,
        sps: vec![transmux::AvcSps(vec![0x67, 0x42, 0x00, 0x1e])],
        pps: vec![transmux::AvcPps(vec![0x68, 0xce, 0x3c, 0x80])],
        chroma_format: None,
        bit_depth_luma_minus8: None,
        bit_depth_chroma_minus8: None,
        sps_ext: vec![],
    };
    transmux::TrackSpec::new(
        TrackId::new(track_id),
        timescale(90_000),
        CodecConfig::Avc {
            config: transmux::AVCConfigurationBox::new(record),
            width: 16,
            height: 16,
        },
    )
}

/// A synthetic AVC `Media`: `valid_count` well-formed length-prefixed samples
/// followed by one sample whose 4-byte NAL length prefix declares far more
/// bytes than the sample holds — the shape that made the mid-loop
/// `nal_subsamples(..)?` in `CencEncryptor::encrypt` fail *after* earlier
/// samples had already been keystreamed in place.
fn avc_media(valid_count: usize, with_bad_sample: bool) -> Media {
    let mut samples: Vec<transmux::Sample> = (0..valid_count)
        .map(|i| {
            let body = lp_nal(
                &[0x65, i as u8]
                    .into_iter()
                    .chain(0..40)
                    .collect::<Vec<u8>>(),
            );
            transmux::Sample::new(
                body,
                Some(i as i64 * 100),
                Some(i as i64 * 100),
                Some(100),
                i == 0,
            )
        })
        .collect::<std::result::Result<Vec<_>, _>>()
        .expect("valid timing");
    if with_bad_sample {
        // Declares a 512-byte NAL but carries only 8 payload bytes.
        let mut bad = vec![0x00, 0x00, 0x02, 0x00];
        bad.extend_from_slice(&[0x41; 8]);
        samples.push(
            transmux::Sample::new(
                bad,
                Some(valid_count as i64 * 100),
                Some(valid_count as i64 * 100),
                Some(100),
                false,
            )
            .expect("valid timing"),
        );
    }
    Media::new(vec![Track::new(avc_spec(1), samples)], timescale(90_000))
}

/// Every sample's bytes of every track, in (track, sample) order.
fn snapshot_all(media: &Media) -> Vec<Bytes> {
    media
        .tracks
        .iter()
        .flat_map(|t| t.samples.iter().map(|s| s.data.clone()))
        .collect()
}

/// **A rejected `encrypt` must be atomic** (the method's own docs promise it):
/// a sample whose NAL length prefix overruns the sample must leave every
/// *other* sample byte-identical and no `Track::encryption` populated — not
/// samples `0..k` silently ciphertext with an `Err` returned. And because the
/// call was rejected, the [`IvGen::Counter`] state must be exactly as it was:
/// a later successful call on the *same* instance must hand out precisely the
/// IVs a brand-new instance would (a failed call may neither consume nor skip
/// counter state — reusing an IV under the same key is a two-time pad).
#[test]
fn rejected_encrypt_leaves_media_and_counter_untouched() {
    let mut media = avc_media(4, true);
    let pristine = avc_media(4, true);
    let before = snapshot_all(&media);

    let err = CencEncryptor::new(KEY_A)
        .encrypt(&mut media, &cenc_cfg())
        .expect_err("a sample whose NAL length prefix overruns must be rejected");
    assert!(
        matches!(err, transmux::Error::BufferTooShort(..))
            || matches!(err, transmux::Error::InvalidInput(_)),
        "expected a buffer/input error, got {err:?}"
    );

    // The whole point of the fix: no earlier sample was touched.
    assert_eq!(
        snapshot_all(&media),
        before,
        "a rejected encrypt must leave every sample byte-identical"
    );
    assert_eq!(snapshot_all(&media), snapshot_all(&pristine));
    for track in &media.tracks {
        assert!(
            track.encryption.is_none(),
            "a rejected encrypt must not populate Track::encryption"
        );
    }

    // The counter survived the failed call: the same instance, on fresh valid
    // media, produces exactly the IVs a brand-new one does.
    let mut used_encryptor = CencEncryptor::new(KEY_A);
    let mut good_used = avc_media(4, false);
    used_encryptor
        .encrypt(&mut good_used, &cenc_cfg())
        .expect("valid media must encrypt after a rejected call");

    let mut fresh = CencEncryptor::new(KEY_A);
    let mut good_fresh = avc_media(4, false);
    fresh
        .encrypt(&mut good_fresh, &cenc_cfg())
        .expect("valid media must encrypt");

    assert_eq!(
        all_recorded_ivs(&good_used),
        all_recorded_ivs(&good_fresh),
        "a rejected call must not consume or skip IvGen::Counter state"
    );
    assert_eq!(used_encryptor.next_counter(), fresh.next_counter());
}

// ---------------------------------------------------------------------------
// audit 22 — cbcs `0:0` means full-sample protection (ISO/IEC 23001-7 §10.2)
// ---------------------------------------------------------------------------

use aes::cipher::{Block, BlockModeEncrypt, KeyIvInit};

type Aes128CbcEnc = cbc::Encryptor<aes::Aes128>;

fn whole_sample_media(data: Vec<u8>) -> Media {
    let record = transmux::AVCDecoderConfigurationRecord {
        configuration_version: 1,
        profile_indication: 66,
        profile_compatibility: 0,
        level_indication: 30,
        length_size_minus_one: 3,
        sps: Vec::new(),
        pps: Vec::new(),
        chroma_format: None,
        bit_depth_luma_minus8: None,
        bit_depth_chroma_minus8: None,
        sps_ext: Vec::new(),
    };
    let spec = transmux::TrackSpec::new(
        TrackId::new(1),
        timescale(1000),
        CodecConfig::Avc {
            config: transmux::AVCConfigurationBox::new(record),
            width: 16,
            height: 16,
        },
    );
    let sample =
        transmux::Sample::new(data, Some(0), Some(0), Some(1000), true).expect("valid timing");
    Media::new(vec![Track::new(spec, vec![sample])], timescale(1000))
}

/// A single-AVC-track [`Media`] with `sample_count` identical 16-byte samples
/// (one AES-CTR block each), built on [`whole_sample_media`]'s track.
fn counter_media(sample_count: usize) -> Media {
    let mut media = whole_sample_media((0..16u8).collect());
    let first = media.tracks[0]
        .samples
        .first()
        .expect("whole_sample_media builds one sample")
        .clone();
    for _ in 1..sample_count {
        media.tracks[0].samples.push(first.clone());
    }
    media
}

fn cbcs_encrypt(data: Vec<u8>, pattern: (u8, u8)) -> Result<Vec<u8>, transmux::Error> {
    const KEY: [u8; 16] = [0x42; 16];
    const IV: [u8; 16] = [0x11; 16];
    const KID: [u8; 16] = [0x22; 16];
    let mut media = whole_sample_media(data);
    let cfg = EncryptConfig::new(CencScheme::Cbcs, KID, SubsamplePolicy::WholeSample)
        .with_iv(IvGen::Constant(IV))
        .with_pattern(pattern);
    CencEncryptor::new(KEY).encrypt(&mut media, &cfg)?;
    Ok(media.tracks[0].samples[0].data.to_vec())
}

fn independent_cbc(data: &[u8], blocks: usize) -> Vec<u8> {
    const KEY: [u8; 16] = [0x42; 16];
    const IV: [u8; 16] = [0x11; 16];
    let mut out = data.to_vec();
    let mut enc = Aes128CbcEnc::new((&KEY).into(), (&IV).into());
    for chunk in out[..blocks * 16].chunks_exact_mut(16) {
        let block: &mut Block<aes::Aes128> = chunk.try_into().expect("16-byte block");
        enc.encrypt_block(block);
    }
    out
}

#[test]
fn cbcs_zero_zero_pattern_encrypts_the_whole_sample() {
    let plain: Vec<u8> = (0..87u8).collect();
    let got = cbcs_encrypt(plain.clone(), (0, 0)).expect("full-sample protection");
    let want = independent_cbc(&plain, 5);
    assert_eq!(got, want, "5 whole blocks encrypted, 7-byte tail clear");
}

#[test]
fn cbcs_one_nine_pattern_encrypts_only_the_crypt_run() {
    let plain: Vec<u8> = (0..87u8).collect();
    let got = cbcs_encrypt(plain.clone(), (1, 9)).expect("1:9 pattern");
    let want = independent_cbc(&plain, 1);
    assert_eq!(got, want, "first block encrypted, the rest clear");
}

#[test]
fn cbcs_zero_crypt_with_nonzero_skip_is_rejected() {
    assert!(matches!(
        cbcs_encrypt(vec![0u8; 87], (0, 5)),
        Err(transmux::Error::InvalidInput(_))
    ));
}

// ---------------------------------------------------------------------------
// issue #3661 — the counter must also fit *after* the call, not just the IV
// ---------------------------------------------------------------------------

/// `IvGen::Counter` hands out `next_counter + i`; the counter must also be
/// representable *after* the call. A final IV of exactly `u64::MAX` is a
/// valid IV, but leaves nothing to advance to, so the call must be rejected
/// up front. (The old bare `+=` wrapped to 0 in release builds, and the next
/// call replayed IVs 0.. under the same key: AES-CTR keystream reuse.)
#[test]
fn counter_iv_at_u64_max_is_rejected_and_leaves_state_untouched() {
    let mut media = whole_sample_media((0..16u8).collect());
    let before = snapshot(&media);
    let mut enc = CencEncryptor::resume(KEY_A, u64::MAX);
    let cfg = cenc_cfg().with_subsample(SubsamplePolicy::WholeSample);

    let err = enc
        .encrypt(&mut media, &cfg)
        .expect_err("an IV of exactly u64::MAX cannot be advanced past");
    assert!(
        matches!(err, transmux::Error::InvalidInput(_)),
        "expected InvalidInput, got {err:?}"
    );
    assert_eq!(enc.next_counter(), u64::MAX);
    assert_eq!(
        snapshot(&media),
        before,
        "a rejected encrypt must leave every sample byte-identical"
    );
    assert!(media.tracks[0].encryption.is_none());
}

/// The largest representable *start* is still usable once: `u64::MAX - 1`
/// with one sample hands out IV `u64::MAX - 1` and leaves the counter at
/// `u64::MAX`; the next one-sample call then has no room and is rejected.
#[test]
fn counter_iv_u64_max_minus_one_is_usable_once_then_rejects() {
    let cfg = cenc_cfg().with_subsample(SubsamplePolicy::WholeSample);

    let mut media = whole_sample_media((0..16u8).collect());
    let mut enc = CencEncryptor::resume(KEY_A, u64::MAX - 1);
    enc.encrypt(&mut media, &cfg)
        .expect("u64::MAX - 1 is a valid IV and leaves the counter at u64::MAX");
    assert_eq!(enc.next_counter(), u64::MAX);
    let iv = media.tracks[0]
        .encryption
        .as_ref()
        .expect("track.encryption Some")
        .samples[0]
        .initialization_vector
        .clone();
    assert_eq!(
        iv,
        (u64::MAX - 1).to_be_bytes().to_vec(),
        "the 8-byte counter IV is the literal u64::MAX - 1"
    );

    let mut again = whole_sample_media((0..16u8).collect());
    let before = snapshot(&again);
    let err = enc
        .encrypt(&mut again, &cfg)
        .expect_err("the counter is already at u64::MAX");
    assert!(
        matches!(err, transmux::Error::InvalidInput(_)),
        "expected InvalidInput, got {err:?}"
    );
    assert_eq!(enc.next_counter(), u64::MAX);
    assert_eq!(
        snapshot(&again),
        before,
        "a rejected encrypt must leave every sample byte-identical"
    );
}

/// A multi-sample call whose *last* IV fits but whose advance (`+ 2`) does
/// not must be rejected before either sample is ciphered: the old code
/// checked only `next_counter + (total - 1)` and then wrapped (or panicked)
/// on `next_counter += total`.
#[test]
fn counter_iv_using_the_last_iv_in_a_multi_sample_call_is_rejected() {
    let mut media = counter_media(2);
    let before = snapshot(&media);
    let mut enc = CencEncryptor::resume(KEY_A, u64::MAX - 1);
    let cfg = cenc_cfg().with_subsample(SubsamplePolicy::WholeSample);

    let err = enc
        .encrypt(&mut media, &cfg)
        .expect_err("next_counter + 2 overflows, so the call must be rejected");
    assert!(
        matches!(err, transmux::Error::InvalidInput(_)),
        "expected InvalidInput, got {err:?}"
    );
    assert_eq!(enc.next_counter(), u64::MAX - 1);
    assert_eq!(
        snapshot(&media),
        before,
        "a rejected encrypt must leave every sample byte-identical"
    );
}

// ---------------------------------------------------------------------------
// issue #3662 — next_counter() is the real snapshot callers resume from
// ---------------------------------------------------------------------------

/// `next_counter()` must report the exact number of counter IVs consumed, not
/// just "some value that also moves for a fresh instance": a wrong snapshot
/// makes `resume()` reuse IVs after a restart (AES-CTR keystream reuse).
#[test]
fn next_counter_reports_samples_consumed_and_resume_continues_from_it() {
    let cfg = cenc_cfg().with_subsample(SubsamplePolicy::WholeSample);

    let mut media = counter_media(3);
    let mut enc = CencEncryptor::resume(KEY_A, 7);
    assert_eq!(enc.next_counter(), 7, "resume takes the given counter");
    enc.encrypt(&mut media, &cfg).expect("encrypt 3 samples");
    assert_eq!(enc.next_counter(), 10, "3 samples consumed counters 7,8,9");

    let ivs: Vec<Vec<u8>> = all_recorded_ivs(&media);
    assert_eq!(
        ivs,
        vec![
            7u64.to_be_bytes().to_vec(),
            8u64.to_be_bytes().to_vec(),
            9u64.to_be_bytes().to_vec(),
        ],
        "the recorded IVs are the literal counters 7, 8, 9"
    );

    let mut more = counter_media(1);
    enc.encrypt(&mut more, &cfg).expect("encrypt one more");
    assert_eq!(
        all_recorded_ivs(&more),
        vec![10u64.to_be_bytes().to_vec()],
        "a second call continues at the advanced counter, never 0 or 1"
    );
}

// ---------------------------------------------------------------------------
// issue #3663 — cbcs pattern components are 4-bit fields, edges included
// ---------------------------------------------------------------------------

fn cbcs_pattern_config(pattern: (u8, u8)) -> EncryptConfig {
    EncryptConfig::new(CencScheme::Cbcs, KID_A, SubsamplePolicy::WholeSample)
        .with_iv(IvGen::Constant([0x11u8; 16]))
        .with_pattern(pattern)
}

/// 15 is the largest value a `cbcs` pattern component may take (ISO/IEC
/// 23001-7 §12.2 packs crypt and skip into one byte, one nibble each), so
/// `15:15` must be accepted and the `tenc` pattern byte must be the literal
/// `(15 << 4) | 15 == 0xFF`.
#[test]
fn cbcs_pattern_15_15_is_accepted_and_packs_0xff() {
    let mut media = whole_sample_media(vec![0u8; 32]);
    CencEncryptor::new(KEY_A)
        .encrypt(&mut media, &cbcs_pattern_config((15, 15)))
        .expect("15:15 is the maximum legal cbcs pattern");
    let tenc = media.tracks[0]
        .encryption
        .as_ref()
        .expect("track.encryption Some")
        .tenc
        .clone();
    assert_eq!(tenc.default_crypt_byte_block, 15);
    assert_eq!(tenc.default_skip_byte_block, 15);
    let mut buf = vec![0u8; tenc.serialized_len()];
    tenc.serialize_into(&mut buf).expect("serialize tenc");
    // tenc body: reserved (1) + pattern byte (1) — after the 8-byte box header
    // and the 4-byte FullBox version/flags.
    assert_eq!(buf[13], 0xFF, "tenc pattern byte is (15 << 4) | 15");
}

/// Any component above 15 must be rejected rather than truncated into its low
/// nibble by `tenc`'s `(crypt << 4) | skip` packing.
#[test]
fn cbcs_pattern_component_above_15_is_rejected() {
    for pattern in [(1u8, 16u8), (1, 200), (16, 1)] {
        let mut media = whole_sample_media(vec![0u8; 32]);
        let err = CencEncryptor::new(KEY_A)
            .encrypt(&mut media, &cbcs_pattern_config(pattern))
            .expect_err("a cbcs pattern component above 15 must be rejected");
        assert!(
            matches!(err, transmux::Error::InvalidInput(_)),
            "pattern {pattern:?}: expected InvalidInput, got {err:?}"
        );
        assert!(media.tracks[0].encryption.is_none());
    }
}

// ---------------------------------------------------------------------------
// issue #3664 — an exactly-spaced CTR IV list is valid, one block closer is not
// ---------------------------------------------------------------------------

fn media_with_samples(samples: Vec<Vec<u8>>) -> Media {
    let mut media = whole_sample_media(samples[0].clone());
    media.tracks[0].samples.clear();
    for data in samples {
        media.tracks[0].samples.push(
            transmux::Sample::new(data, Some(0), Some(0), Some(1000), true)
                .expect("valid timing"),
        );
    }
    media
}

/// A 16-byte big-endian counter-block IV whose counter value is `value`.
fn ctr_iv(value: u64) -> Vec<u8> {
    let mut iv = vec![0u8; 16];
    iv[8..].copy_from_slice(&value.to_be_bytes());
    iv
}

fn cenc_explicit_config(ivs: Vec<Vec<u8>>) -> EncryptConfig {
    EncryptConfig::new(CencScheme::Cenc, KID_A, SubsamplePolicy::WholeSample).with_iv(IvGen::Explicit(ivs))
}

/// One length-prefixed AVC NAL unit: 4-byte length prefix, 1-byte header,
/// `payload` bytes of protected payload.
fn avc_nal_sample(payload: usize) -> Vec<u8> {
    let nal_len = 1 + payload;
    let mut data = Vec::with_capacity(4 + nal_len);
    data.extend_from_slice(&(nal_len as u32).to_be_bytes());
    data.extend(std::iter::repeat_n(0x65u8, nal_len));
    data
}

/// Under `cenc` a sample consumes one counter block per 16 protected bytes
/// (`ceil(protected / 16)`, ISO/IEC 23001-7 §10.1), so IV spacing must be at
/// least the previous sample's block count. Exactly that spacing is valid
/// (adjacent ranges); one block closer overlaps. Samples of 16, 17 and 32
/// bytes (1, 2 and 2 blocks) pin the ceiling arithmetic at a non-multiple.
#[test]
fn ctr_iv_spacing_must_be_at_least_the_previous_samples_block_count() {
    let lens = [16usize, 17, 32];

    let mut ok = media_with_samples(lens.iter().map(|&len| vec![0u8; len]).collect());
    CencEncryptor::new(KEY_A)
        .encrypt(&mut ok, &cenc_explicit_config(vec![ctr_iv(0), ctr_iv(1), ctr_iv(3)]))
        .expect("IVs spaced by exactly the previous sample's protected block count are adjacent, not overlapping");

    let mut overlap = media_with_samples(lens.iter().map(|&len| vec![0u8; len]).collect());
    let err = CencEncryptor::new(KEY_A)
        .encrypt(
            &mut overlap,
            &cenc_explicit_config(vec![ctr_iv(0), ctr_iv(1), ctr_iv(2)]),
        )
        .expect_err("IV 2 starts inside the 17-byte sample's range [1, 3)");
    assert!(matches!(err, transmux::Error::InvalidInput(_)), "got {err:?}");
}

/// A 17-byte sample consumes two counter blocks (ceil(17/16)), so the next IV
/// must be two blocks later: one block is an overlap.
#[test]
fn ctr_iv_spacing_for_a_17_byte_sample_is_two_blocks() {
    let mut ok = media_with_samples(vec![vec![0u8; 17], vec![0u8; 17]]);
    CencEncryptor::new(KEY_A)
        .encrypt(&mut ok, &cenc_explicit_config(vec![ctr_iv(0), ctr_iv(2)]))
        .expect("ceil(17 / 16) == 2 blocks, so spacing 2 is adjacent");

    let mut overlap = media_with_samples(vec![vec![0u8; 17], vec![0u8; 17]]);
    let err = CencEncryptor::new(KEY_A)
        .encrypt(&mut overlap, &cenc_explicit_config(vec![ctr_iv(0), ctr_iv(1)]))
        .expect_err("a 17-byte sample covers blocks 0 and 1, so IV 1 overlaps");
    assert!(matches!(err, transmux::Error::InvalidInput(_)), "got {err:?}");
}

/// The block count comes from the *protected* bytes of the subsample map, not
/// the sample length: a single 33-byte AVC NAL (4-byte prefix + 1-byte
/// header clear) protects 32 bytes = 2 blocks, so spacing 2 is valid and
/// spacing 1 overlaps even though the sample itself is 37 bytes.
#[test]
fn ctr_iv_spacing_counts_subsample_protected_bytes() {
    let mut ok = media_with_samples(vec![avc_nal_sample(32), avc_nal_sample(32)]);
    let cfg = cenc_explicit_config(vec![ctr_iv(0), ctr_iv(2)])
        .with_subsample(SubsamplePolicy::Video);
    CencEncryptor::new(KEY_A)
        .encrypt(&mut ok, &cfg)
        .expect("32 protected bytes are two counter blocks");
    let entries = &ok.tracks[0].encryption.as_ref().expect("Some").samples;
    assert_eq!(entries[0].subsamples.len(), 1, "one subsample per NAL");
    assert_eq!(entries[0].subsamples[0].bytes_of_clear_data, 5);
    assert_eq!(entries[0].subsamples[0].bytes_of_protected_data, 32);

    let mut overlap = media_with_samples(vec![avc_nal_sample(32), avc_nal_sample(32)]);
    let cfg = cenc_explicit_config(vec![ctr_iv(0), ctr_iv(1)])
        .with_subsample(SubsamplePolicy::Video);
    let err = CencEncryptor::new(KEY_A)
        .encrypt(&mut overlap, &cfg)
        .expect_err("32 protected bytes cover blocks 0 and 1, so IV 1 overlaps");
    assert!(matches!(err, transmux::Error::InvalidInput(_)), "got {err:?}");
}

// ---------------------------------------------------------------------------
// issue #3665 — NAL-aware subsampling under HEVC and VVC, not just AVC
// ---------------------------------------------------------------------------

fn hevc_media() -> Media {
    let bytes = test_fixtures::read("ts/hevc/main.ts");
    let media = TsDemux::new()
        .unpackage(bytes.as_slice())
        .expect("demux ts/hevc/main.ts");
    media
        .select_tracks_by(|t| matches!(t.spec.config, CodecConfig::Hevc { .. }))
        .expect("HEVC video track present")
}

fn vvc_media() -> Media {
    let bytes = test_fixtures::read("mp4/frag/vvc.frag.mp4");
    let media = Fmp4Demux::new()
        .unpackage(bytes.as_slice())
        .expect("demux mp4/frag/vvc.frag.mp4");
    media
        .select_tracks_by(|t| matches!(t.spec.config, CodecConfig::Vvc { .. }))
        .expect("VVC video track present")
}

/// Assert every NAL unit of every sample carries a subsample entry that clears
/// exactly the 4-byte length prefix plus `header_len` header bytes (2 for
/// HEVC/VVC — ITU-T H.265 §7.3.1.2 / H.266 §7.3.1.2) and protects the rest,
/// and that the clear bytes are byte-identical to the cleartext.
fn assert_nal_subsample_map(original: &Media, encrypted: &Media, header_len: usize) {
    let mut nals = 0usize;
    for (track, orig_track) in encrypted.tracks.iter().zip(&original.tracks) {
        let enc = track
            .encryption
            .as_ref()
            .expect("every sample must record encryption metadata");
        assert_eq!(enc.samples.len(), track.samples.len());
        for ((sample, orig), entry) in track
            .samples
            .iter()
            .zip(&orig_track.samples)
            .zip(&enc.samples)
        {
            assert!(
                !entry.subsamples.is_empty(),
                "a NAL-aware track must not fall back to whole-sample protection"
            );
            let mut off = 0usize;
            for sub in &entry.subsamples {
                let nal_len = u32::from_be_bytes(
                    sample.data[off..off + 4]
                        .try_into()
                        .expect("4-byte NAL length"),
                ) as usize;
                assert_eq!(
                    sub.bytes_of_clear_data as usize,
                    4 + header_len,
                    "clear run is the length prefix plus the {header_len}-byte NAL header"
                );
                assert_eq!(
                    sub.bytes_of_protected_data as usize,
                    nal_len - header_len,
                    "protected run is the NAL payload"
                );
                assert_eq!(
                    &sample.data[off..off + 4 + header_len],
                    &orig.data[off..off + 4 + header_len],
                    "length prefix and NAL header must stay in the clear"
                );
                off += 4 + nal_len;
                nals += 1;
            }
            assert_eq!(
                off,
                sample.data.len(),
                "the subsample map must cover the whole sample"
            );
        }
    }
    assert!(nals > 0, "fixture must carry length-prefixed NAL units");
}

#[test]
fn nal_aware_subsampling_clears_the_two_byte_hevc_header() {
    let original = hevc_media();
    let mut media = hevc_media();
    let cfg = EncryptConfig::new(CencScheme::Cbcs, KID_A, SubsamplePolicy::Video);
    CencEncryptor::new(KEY_A)
        .encrypt(&mut media, &cfg)
        .expect("encrypt HEVC");
    assert_nal_subsample_map(&original, &media, 2);
}

#[test]
fn nal_aware_subsampling_clears_the_two_byte_vvc_header() {
    let original = vvc_media();
    let mut media = vvc_media();
    let cfg = EncryptConfig::new(CencScheme::Cbcs, KID_A, SubsamplePolicy::Video);
    CencEncryptor::new(KEY_A)
        .encrypt(&mut media, &cfg)
        .expect("encrypt VVC");
    assert_nal_subsample_map(&original, &media, 2);
}
