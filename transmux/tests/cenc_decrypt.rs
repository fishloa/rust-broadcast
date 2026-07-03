//! CENC decrypt integration tests (#465) — AES-CTR sample decryption.
//!
//! Oracle: `fixtures/mp4/cenc.mp4` is a real ffmpeg `cenc-aes-ctr` protected
//! fMP4 built from the cleartext `fixtures/ts/h264/main.ts`. Decrypting the
//! protected samples with the known content key must reproduce, byte-for-byte,
//! the NAL payloads that `TsDemux` recovers from the cleartext source.
//!
//! Content key = `76a6c65c5ea762046bd749a2e632ccbb`
//! KID         = `a7e61c373e219033c21091fa607bf3b8`

#![cfg(feature = "cenc")]

use broadcast_common::Decrypt;
use transmux::TsDemux;
use transmux::annexb::iter_length_prefixed_nals;
use transmux::cenc_decrypt::{CencDecryptor, CencScheme, KeyMap};

const CENC_MP4: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/mp4/cenc.mp4");
const CLEAR_TS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../fixtures/ts/h264/main.ts");

const KID: [u8; 16] = [
    0xa7, 0xe6, 0x1c, 0x37, 0x3e, 0x21, 0x90, 0x33, 0xc2, 0x10, 0x91, 0xfa, 0x60, 0x7b, 0xf3, 0xb8,
];
const CONTENT_KEY: [u8; 16] = [
    0x76, 0xa6, 0xc6, 0x5c, 0x5e, 0xa7, 0x62, 0x04, 0x6b, 0xd7, 0x49, 0xa2, 0xe6, 0x32, 0xcc, 0xbb,
];

fn read(path: &str) -> Vec<u8> {
    std::fs::read(path).unwrap_or_else(|e| panic!("fixture {path}: {e}"))
}

fn keys() -> KeyMap {
    KeyMap::new().with_key(KID, CONTENT_KEY)
}

/// Collect every NAL payload across every video sample of a track, in order.
fn nal_payloads(samples: &[transmux::Sample]) -> Vec<Vec<u8>> {
    let mut out = Vec::new();
    for s in samples {
        for nal in iter_length_prefixed_nals(&s.data).expect("length-prefixed NALs") {
            out.push(nal.to_vec());
        }
    }
    out
}

/// Test 1: the CENC boxes are recognised and carry the expected metadata.
#[test]
fn boxes_recognised() {
    let file = read(CENC_MP4);
    let dec = CencDecryptor::from_fmp4(&file).expect("harvest CENC metadata");

    assert_eq!(dec.scheme(), Some(CencScheme::Cenc), "scheme must be cenc");
    assert_eq!(&dec.original_format(), b"avc1", "frma original format");

    let tenc = dec.track_encryption().expect("tenc present");
    assert_eq!(tenc.default_is_protected, 1, "default_isProtected");
    assert_eq!(tenc.default_per_sample_iv_size, 8, "per_sample_IV_size");
    assert_eq!(tenc.default_kid, KID, "default_KID");

    let entries = dec.sample_entries();
    assert_eq!(entries.len(), 15, "15 per-sample senc entries");
    assert_eq!(entries[0].initialization_vector.len(), 8, "8-byte IV");
    assert!(
        !entries[0].subsamples.is_empty(),
        "subsample encryption present"
    );
}

/// Test 2 (ungameable oracle): decrypting the protected samples reproduces the
/// cleartext TS NAL payloads, sample-for-sample.
#[test]
fn decrypt_matches_cleartext_ts() {
    // Decrypt side.
    let file = read(CENC_MP4);
    let dec = CencDecryptor::from_fmp4(&file).unwrap();
    let mut media = dec.demux().expect("demux protected fMP4");
    dec.decrypt(&mut media, &keys()).expect("decrypt");
    let decrypted = nal_payloads(&media.tracks[0].samples);

    // Cleartext oracle.
    let ts = read(CLEAR_TS);
    let mut td = TsDemux::new();
    let clear_media = {
        use broadcast_common::Unpackage;
        td.unpackage(&ts).expect("demux cleartext TS")
    };
    let clear = nal_payloads(&clear_media.tracks[0].samples);

    assert_eq!(
        decrypted.len(),
        clear.len(),
        "same NAL count ({} decrypted vs {} cleartext)",
        decrypted.len(),
        clear.len()
    );
    for (i, (d, c)) in decrypted.iter().zip(clear.iter()).enumerate() {
        assert_eq!(d, c, "NAL {i} must be byte-identical to cleartext");
    }
}

/// Test 3: a wrong key produces bytes that do NOT match the cleartext (proves
/// decryption is real, not a passthrough).
#[test]
fn wrong_key_does_not_match() {
    let file = read(CENC_MP4);
    let dec = CencDecryptor::from_fmp4(&file).unwrap();

    // Right key → matches (baseline).
    let mut good = dec.demux().unwrap();
    dec.decrypt(&mut good, &keys()).unwrap();
    let good_nals = nal_payloads(&good.tracks[0].samples);

    // Wrong key → different plaintext.
    let mut wrong_key = CONTENT_KEY;
    wrong_key[0] ^= 0xFF;
    let mut bad = dec.demux().unwrap();
    dec.decrypt(&mut bad, &KeyMap::new().with_key(KID, wrong_key))
        .unwrap();
    let bad_nals = nal_payloads(&bad.tracks[0].samples);

    assert_ne!(
        good_nals, bad_nals,
        "wrong key must yield different (garbage) plaintext"
    );

    // And the wrong-key output must not match the cleartext TS.
    let ts = read(CLEAR_TS);
    let mut td = TsDemux::new();
    let clear = {
        use broadcast_common::Unpackage;
        nal_payloads(&td.unpackage(&ts).unwrap().tracks[0].samples)
    };
    assert_ne!(bad_nals, clear, "wrong key must not reproduce cleartext");
}

/// Test 4: subsample clear ranges are left untouched; only protected ranges change.
#[test]
fn subsample_boundaries_respected() {
    let file = read(CENC_MP4);
    let dec = CencDecryptor::from_fmp4(&file).unwrap();

    let before = dec.demux().unwrap();
    let mut after = dec.demux().unwrap();
    dec.decrypt(&mut after, &keys()).unwrap();

    let entries = dec.sample_entries();
    // Find a sample that has a non-empty subsample map with a clear region and a
    // protected region, and assert clear==unchanged, protected==changed.
    let mut checked_clear = false;
    let mut checked_protected = false;
    for (idx, entry) in entries.iter().enumerate() {
        let pre = &before.tracks[0].samples[idx].data;
        let post = &after.tracks[0].samples[idx].data;
        assert_eq!(
            pre.len(),
            post.len(),
            "decrypt must not resize sample {idx}"
        );
        let mut off = 0usize;
        for sub in &entry.subsamples {
            let clear = sub.bytes_of_clear_data as usize;
            let protected = sub.bytes_of_protected_data as usize;
            // Clear range: identical pre/post.
            assert_eq!(
                &pre[off..off + clear],
                &post[off..off + clear],
                "clear range of sample {idx} must be untouched"
            );
            if clear > 0 {
                checked_clear = true;
            }
            off += clear;
            // Protected range: must differ (encrypted vs decrypted).
            if protected > 0 {
                assert_ne!(
                    &pre[off..off + protected],
                    &post[off..off + protected],
                    "protected range of sample {idx} must change"
                );
                checked_protected = true;
            }
            off += protected;
        }
    }
    assert!(checked_clear, "test must exercise a clear range");
    assert!(checked_protected, "test must exercise a protected range");
}

/// Test 5: decryption is driven through the `broadcast_common::Decrypt` trait,
/// not an inherent method — invoke via a trait object bound.
#[test]
fn decrypt_via_trait() {
    let file = read(CENC_MP4);
    let dec = CencDecryptor::from_fmp4(&file).unwrap();
    let mut media = dec.demux().unwrap();

    // Bind through the trait explicitly so this only compiles/runs if the
    // `Decrypt` impl (with its associated types) is wired up.
    fn run_decrypt<D: Decrypt<Media = transmux::Media, Keys = KeyMap, Error = transmux::Error>>(
        d: &D,
        m: &mut transmux::Media,
        k: &KeyMap,
    ) -> Result<(), transmux::Error> {
        d.decrypt(m, k)
    }
    run_decrypt(&dec, &mut media, &keys()).expect("decrypt via trait");

    // Sanity: the trait path produced valid, parseable NALs.
    let nals = nal_payloads(&media.tracks[0].samples);
    assert!(!nals.is_empty());
    // First NAL should be a valid AUD/SPS-class NAL (top bit zero: forbidden_zero_bit).
    assert_eq!(nals[0][0] & 0x80, 0, "decrypted NAL header sane");
}
