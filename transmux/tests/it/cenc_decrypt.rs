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

use crate::box_walk::{SAMPLE_TABLE_CONTAINERS, find_box_range};
use broadcast_bitstream::annexb::iter_length_prefixed_nals;
use test_harness::collect::Collected as Media;
use transmux::CodecConfig;
use transmux::cenc_decrypt::{CencDecryptStage, CencScheme, KeyMap};

const CENC_MP4: &str = "mp4/cenc.mp4";
const CLEAR_TS: &str = "ts/h264/main.ts";

const KID: [u8; 16] = [
    0xa7, 0xe6, 0x1c, 0x37, 0x3e, 0x21, 0x90, 0x33, 0xc2, 0x10, 0x91, 0xfa, 0x60, 0x7b, 0xf3, 0xb8,
];
const CONTENT_KEY: [u8; 16] = [
    0x76, 0xa6, 0xc6, 0x5c, 0x5e, 0xa7, 0x62, 0x04, 0x6b, 0xd7, 0x49, 0xa2, 0xe6, 0x32, 0xcc, 0xbb,
];

fn read(path: &str) -> Vec<u8> {
    std::fs::read(test_fixtures::path(path)).unwrap_or_else(|e| panic!("fixture {path}: {e}"))
}

fn keys() -> KeyMap {
    KeyMap::new().with_key(KID, CONTENT_KEY)
}

fn demux(bytes: &[u8]) -> Media {
    test_harness::collect::media_from_mp4(bytes, test_harness::collect::CollectMode::Seekable)
        .expect("demux the protected fMP4")
}

fn decrypt(media: &Media, keys: &KeyMap) -> Media {
    let mut stage = CencDecryptStage::new(keys.clone());
    crate::cenc_support::run(&mut stage, media).expect("decrypt")
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
    let media = demux(&file);
    let track = media.tracks.first().expect("one protected track");
    let protection = track.spec.protection.as_ref().expect("sinf/tenc");
    assert_eq!(protection.scheme, CencScheme::Cenc, "scheme must be cenc");
    assert!(
        matches!(track.spec.config, CodecConfig::Avc { .. }),
        "frma restores the original avc1 codec"
    );
    assert_eq!(
        protection.tenc.default_is_protected, 1,
        "default_isProtected"
    );
    assert_eq!(
        protection.tenc.default_per_sample_iv_size, 8,
        "per_sample_IV_size"
    );
    assert_eq!(protection.tenc.default_kid, KID, "default_KID");
    assert_eq!(track.samples.len(), 15, "15 per-sample senc entries");
    let first = track.samples[0].protection.as_ref().expect("senc entry");
    assert_eq!(first.iv.len(), 8, "8-byte IV");
    assert!(!first.subsamples.is_empty(), "subsample encryption present");
}

/// Test 2 (ungameable oracle): decrypting the protected samples reproduces the
/// cleartext TS NAL payloads, sample-for-sample.
#[test]
fn decrypt_matches_cleartext_ts() {
    let file = read(CENC_MP4);
    let media = demux(&file);
    let decrypted = nal_payloads(&decrypt(&media, &keys()).tracks[0].samples);

    let ts = read(CLEAR_TS);

    let clear_media = { test_harness::collect::media_from_ts(&ts).expect("demux cleartext TS") };
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
    let media = demux(&file);
    let good_nals = nal_payloads(&decrypt(&media, &keys()).tracks[0].samples);

    let mut wrong_key = CONTENT_KEY;
    wrong_key[0] ^= 0xFF;
    let bad = decrypt(&media, &KeyMap::new().with_key(KID, wrong_key));
    let bad_nals = nal_payloads(&bad.tracks[0].samples);

    assert_ne!(
        good_nals, bad_nals,
        "wrong key must yield different (garbage) plaintext"
    );

    let ts = read(CLEAR_TS);

    let clear =
        { nal_payloads(&test_harness::collect::media_from_ts(&ts).unwrap().tracks[0].samples) };
    assert_ne!(bad_nals, clear, "wrong key must not reproduce cleartext");
}

/// Test 4: subsample clear ranges are left untouched; only protected ranges change.
#[test]
fn subsample_boundaries_respected() {
    let file = read(CENC_MP4);
    let before = demux(&file);
    let after = decrypt(&before, &keys());

    let mut checked_clear = false;
    let mut checked_protected = false;
    for (idx, sample) in before.tracks[0].samples.iter().enumerate() {
        let entry = sample.protection.as_ref().expect("senc entry");
        let pre = &sample.data;
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
            assert_eq!(
                &pre[off..off + clear],
                &post[off..off + clear],
                "clear range of sample {idx} must be untouched"
            );
            if clear > 0 {
                checked_clear = true;
            }
            off += clear;
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

/// Test 5: decryption runs through the per-sample stage and the resulting
/// samples carry no protection and parse as valid NALs.
#[test]
fn decrypts_through_the_stage() {
    let file = read(CENC_MP4);
    let media = decrypt(&demux(&file), &keys());
    let track = &media.tracks[0];
    assert!(track.samples.iter().all(|s| s.protection.is_none()));
    assert!(track.spec.protection.is_none());
    let nals = nal_payloads(&track.samples);
    assert!(!nals.is_empty());
    assert_eq!(nals[0][0] & 0x80, 0, "decrypted NAL header sane");
}

/// Audit r05-W6: a real CENC asset with an `encv` **and** an `enca` track
/// (the ordinary CENC shape) must demux. Both entries now map to their
/// wrapped codec and keep their protection, so the audio leg is no longer
/// dropped.
const CENC_AV_ENCA_MP4: &str = "mp4/cenc-av-enca.mp4";

#[test]
fn demux_maps_both_protected_tracks_of_a_cenc_asset() {
    let file = read(CENC_AV_ENCA_MP4);
    let media = demux(&file);

    assert_eq!(media.tracks.len(), 2, "both encv and enca reconstruct");
    assert!(
        media
            .tracks
            .iter()
            .all(|t| !matches!(t.spec.config, CodecConfig::Data { .. })),
        "neither protected entry stays a data track"
    );
    assert!(
        media.tracks.iter().all(|t| t.spec.protection.is_some()),
        "both tracks keep their tenc"
    );
    assert!(
        media
            .tracks
            .iter()
            .any(|t| matches!(t.spec.config, CodecConfig::Avc { .. })),
        "the video track is AVC"
    );
    assert!(
        media.tracks.iter().all(|t| !t.samples.is_empty()),
        "both tracks carry samples"
    );
    assert!(media.skipped.is_empty(), "no track is skipped");

    let dump = std::env::temp_dir().join(format!("cenc_av_enca_{}.xml", std::process::id()));
    match std::process::Command::new("MP4Box")
        .args([
            "-diso",
            test_fixtures::path(CENC_AV_ENCA_MP4).to_str().unwrap(),
            "-out",
        ])
        .arg(&dump)
        .output()
    {
        Ok(_) => {
            let text = std::fs::read_to_string(&dump).unwrap_or_default();
            let _ = std::fs::remove_file(&dump);
            assert!(
                text.contains("enca") && text.contains("encv"),
                "fixture must carry encv and enca (MP4Box -diso oracle)"
            );
            assert_eq!(
                text.matches(r#"Type="tenc""#).count(),
                2,
                "MP4Box must report two TrackEncryptionBoxes"
            );
        }
        Err(_) => eprintln!(
            "SKIP demux_maps_both_protected_tracks_of_a_cenc_asset MP4Box cross-check: MP4Box not on PATH"
        ),
    }
}

/// A clear track next to a protected one is still demuxed (it simply has no
/// protection), so nothing is silently dropped.
#[test]
fn a_clear_track_beside_a_protected_one_is_kept() {
    const MIXED: &str = "mp4/cenc-mixed-tracks.mp4";
    let file = read(MIXED);
    let media = demux(&file);

    assert_eq!(media.tracks.len(), 2, "both tracks are carried");
    assert!(
        media.tracks.iter().any(|t| t.spec.protection.is_some()),
        "the protected AVC track keeps its protection"
    );
    assert!(
        media.tracks.iter().any(
            |t| t.spec.protection.is_none() && matches!(t.spec.config, CodecConfig::Aac { .. })
        ),
        "the clear audio track has no protection"
    );
    assert!(media.skipped.is_empty(), "no track is skipped");
}

/// W6 follow-up: the reconstructed AVC track's decrypted samples must equal
/// what the independent decryptor (`mp4decrypt`) produces from the **same**
/// file. Uses the W7 layout fixture (a Bento4-encrypted file mp4decrypt is
/// known to handle). Skips loudly when the tool is absent.
#[test]
fn demux_avc_track_matches_mp4decrypt() {
    if !mp4decrypt_available() {
        eprintln!(
            "SKIP demux_avc_track_matches_mp4decrypt: mp4decrypt (Bento4) not on PATH —              independent cross-check not run"
        );
        return;
    }
    const LAY: &str = "mp4/cenc-layouts";
    let path = test_fixtures::path(&format!("{}/enc-base-moof-none.mp4", LAY));
    let file = std::fs::read(&path).expect("read layout fixture");

    let ours = decrypt(&demux(&file), &layout_keys());
    let our_video: Vec<Vec<u8>> = ours
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
        .expect("our video track")
        .samples
        .iter()
        .map(|s| s.data.to_vec())
        .collect();

    let out = std::env::temp_dir().join(format!("w7_dec_{}.mp4", std::process::id()));
    let key_arg = format!(
        "{}:{}",
        test_fixtures::hex_lower(&LAYOUT_KID),
        test_fixtures::hex_lower(&LAYOUT_KEY)
    );
    let status = std::process::Command::new("mp4decrypt")
        .arg("--key")
        .arg(&key_arg)
        .arg(&path)
        .arg(&out)
        .status()
        .expect("spawn mp4decrypt");
    assert!(status.success(), "mp4decrypt failed");
    let bytes = std::fs::read(&out).expect("read mp4decrypt output");
    let _ = std::fs::remove_file(&out);

    let ref_media = demux(&bytes);
    let ref_video: Vec<Vec<u8>> = ref_media
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
        .expect("reference video track")
        .samples
        .iter()
        .map(|s| s.data.to_vec())
        .collect();

    assert!(
        !ref_video.is_empty(),
        "mp4decrypt output must carry samples"
    );
    assert_eq!(
        our_video, ref_video,
        "our decrypted AVC samples must equal mp4decrypt's byte-for-byte"
    );
}

/// The W7 layout fixture's KID/KEY.
const LAYOUT_KID: [u8; 16] = [
    0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef,
];
const LAYOUT_KEY: [u8; 16] = [
    0x00, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff,
];

fn layout_keys() -> KeyMap {
    KeyMap::new().with_key(LAYOUT_KID, LAYOUT_KEY)
}

/// True if `mp4decrypt` (Bento4) is on `PATH`.
fn mp4decrypt_available() -> bool {
    std::process::Command::new("mp4decrypt")
        .output()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout).contains("MP4 Decrypter")
                || String::from_utf8_lossy(&o.stderr).contains("MP4 Decrypter")
        })
        .unwrap_or(false)
}

/// Grow (in place, ORIGINAL coordinate space) the 32-bit size of every
/// container box whose original span contains the insertion point `at`.
fn grow_ancestors(data: &mut [u8], lo: usize, hi: usize, at: usize, grow: usize) {
    let mut off = lo;
    while off + 8 <= hi {
        let size =
            u32::from_be_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]]) as usize;
        if size < 8 || off + size > hi {
            break;
        }
        let t = [data[off + 4], data[off + 5], data[off + 6], data[off + 7]];
        if SAMPLE_TABLE_CONTAINERS.contains(&&t) && off < at && at <= off + size {
            let new_size = (size + grow) as u32;
            data[off..off + 4].copy_from_slice(&new_size.to_be_bytes());
            grow_ancestors(data, off + 8, off + size, at, grow);
        }
        off += size;
    }
}

fn insert_growing_ancestors(data: &[u8], at: usize, extra: &[u8]) -> Vec<u8> {
    let mut out = data.to_vec();
    grow_ancestors(&mut out, 0, data.len(), at, extra.len());
    out.splice(at..at, extra.iter().copied());
    out
}

/// Real `cenc.mp4` fixture, but with a `seig`-typed `sgpd` box appended as
/// the last child of the protected track's `stbl`.
fn cenc_mp4_with_seig_sgpd() -> Vec<u8> {
    use broadcast_core::Serialize;
    use isobmff::sample_groups::{GROUPING_TYPE_SEIG, SampleGroupDescriptionBox, SgpdEntry};

    let file = read(CENC_MP4);
    let (stbl_start, stbl_size) =
        find_box_range(&file, 0, file.len(), b"stbl", SAMPLE_TABLE_CONTAINERS)
            .expect("fixture has a stbl");
    let insert_at = stbl_start + stbl_size;

    let sgpd = SampleGroupDescriptionBox {
        version: 1,
        flags: 0,
        grouping_type: GROUPING_TYPE_SEIG,
        default_length: 0,
        default_group_description_index: None,
        entries: vec![SgpdEntry::Unknown(vec![0u8; 20])],
    };
    let sgpd_bytes = sgpd.try_to_bytes().unwrap();

    insert_growing_ancestors(&file, insert_at, &sgpd_bytes)
}

/// A `seig` sample group in `stbl` is key rotation this crate does not
/// implement: no sample may be handed out decrypted with the track's default
/// `tenc.default_KID` alone. The track is dropped and reported; a validating
/// read refuses the file instead.
#[test]
fn seig_sample_group_is_dropped_not_silently_decrypted() {
    let file = cenc_mp4_with_seig_sgpd();
    match test_harness::collect::media_from_mp4(
        file.as_slice(),
        test_harness::collect::CollectMode::Seekable,
    ) {
        Err(err) => assert!(
            matches!(err, transmux::Error::UnsupportedFeature(_)),
            "expected Error::UnsupportedFeature, got {err:?}"
        ),
        Ok(media) => {
            assert!(
                media.tracks.iter().all(|track| track.samples.is_empty()),
                "no sample of a rotated-key track is delivered"
            );
            assert!(
                media
                    .damages
                    .iter()
                    .any(|damage| damage.kind() == broadcast_core::damage::DamageKind::TrackDropped)
                    || !media.skipped.is_empty(),
                "the dropped track is reported: {:?}",
                media.damages
            );
        }
    }
}

/// Sanity: the *unmodified* real fixture (no seig group) still decrypts fine.
#[test]
fn non_seig_content_is_unaffected() {
    let file = read(CENC_MP4);
    decrypt(&demux(&file), &keys());
}
