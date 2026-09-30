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

use broadcast_common::{Decrypt, Unpackage};
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

/// Audit r05-W6: a real CENC asset with an `encv` **and** an `enca` track
/// (the ordinary CENC shape) must demux. The code used to `return Err` on the
/// first protected track whose `frma` is not `avc1`, so such a file could not
/// be demuxed at all — nor could a protected HEVC file. The non-AVC protected
/// track is now skipped, with the AVC one still reconstructed.
const CENC_AV_ENCA_MP4: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../fixtures/mp4/cenc_av_enca.mp4"
);

#[test]
fn demux_skips_non_avc_protected_track() {
    let file = read(CENC_AV_ENCA_MP4);
    let dec = CencDecryptor::from_fmp4(&file).expect("harvest a two-track CENC asset");

    // Demuxing must succeed and yield exactly the AVC track.
    let media = dec
        .demux()
        .expect("a CENC asset with a protected audio track must demux");
    assert_eq!(media.tracks.len(), 1, "only the AVC track is reconstructed");
    assert!(
        matches!(
            media.tracks[0].spec.config,
            transmux::CodecConfig::Avc { .. }
        ),
        "the reconstructed track must be the AVC one"
    );
    assert!(
        !media.tracks[0].samples.is_empty(),
        "the AVC track must carry samples"
    );

    // The skip must be REPORTED, never silent: exactly one entry, naming the
    // dropped track's original sample-entry FourCC (audit r05-W6 follow-up).
    assert_eq!(
        media.skipped.len(),
        1,
        "the skipped protected audio track must be recorded in Media::skipped"
    );
    assert_eq!(
        media.skipped[0].fourcc, "mp4a",
        "the skipped entry must name the audio track's original format"
    );
    assert!(
        media.skipped[0].reason.contains("avc1"),
        "the reason must explain the AVC-only limit: {}",
        media.skipped[0].reason
    );

    // Independent oracle: GPAC's own dumper must show both protected sample
    // entries in the source file, proving the fixture really has the
    // encv+enca shape this test is about (and that the skip is not a
    // single-track file passing by accident). `MP4Box -diso` writes the XML to
    // a file, so give it one in the temp dir.
    let dump = std::env::temp_dir().join(format!("cenc_av_enca_{}.xml", std::process::id()));
    match std::process::Command::new("MP4Box")
        .args(["-diso", CENC_AV_ENCA_MP4, "-out"])
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
            "SKIP demux_skips_non_avc_protected_track MP4Box cross-check: MP4Box not on PATH"
        ),
    }
}

/// W6 follow-up: an **unprotected** track next to a protected one must also be
/// reported in `Media::skipped` (audit r05-W6 follow-up) — dropping it silently
/// is what made the earlier "never silent" claim false.
///
/// `cenc_mixed_tracks.mp4` is one `encv` (video, CENC-protected) + one clear
/// `mp4a` (audio) — GPAC `MP4Box -crypt` with only track 1 encrypted.
#[test]
fn demux_reports_unprotected_track_as_skipped() {
    const MIXED: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/cenc_mixed_tracks.mp4"
    );
    let file = read(MIXED);
    let dec = CencDecryptor::from_fmp4(&file).expect("harvest");
    let media = dec
        .demux()
        .expect("a mixed clear/protected file must demux");

    assert_eq!(
        media.tracks.len(),
        1,
        "only the protected AVC track is built"
    );
    assert!(
        matches!(
            media.tracks[0].spec.config,
            transmux::CodecConfig::Avc { .. }
        ),
        "the built track must be the AVC one"
    );
    assert_eq!(
        media.skipped.len(),
        1,
        "the unprotected audio track must be recorded in Media::skipped"
    );
    assert_eq!(
        media.skipped[0].fourcc, "mp4a",
        "the skipped entry must name the clear track's sample-entry FourCC"
    );
    assert!(
        media.skipped[0].reason.contains("unprotected"),
        "the reason must say the track is unprotected: {}",
        media.skipped[0].reason
    );
}

/// W6 follow-up: the reconstructed AVC track's decrypted samples must equal
/// what the independent decryptor (`mp4decrypt`) produces from the **same**
/// file. Uses the W7 layout fixture (a Bento4-encrypted file mp4decrypt is
/// known to handle; the ffmpeg-muxed `cenc_av_enca.mp4` is rejected, see the
/// W7 fixture README C6). Skips loudly when the tool is absent.
#[test]
fn demux_avc_track_matches_mp4decrypt() {
    if !mp4decrypt_available() {
        eprintln!(
            "SKIP demux_avc_track_matches_mp4decrypt: mp4decrypt (Bento4) not on PATH —              independent cross-check not run"
        );
        return;
    }
    const LAY: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/cenc_frag_layouts"
    );
    let path = format!("{LAY}/enc_base_moof_none.mp4");
    let file = std::fs::read(&path).expect("read layout fixture");

    // Ours.
    let dec = CencDecryptor::from_fmp4(&file).expect("harvest");
    let mut ours = dec.demux().expect("demux");
    dec.decrypt(&mut ours, &layout_keys()).expect("decrypt");
    let our_video: Vec<Vec<u8>> = ours.tracks[0]
        .samples
        .iter()
        .map(|s| s.data.to_vec())
        .collect();

    // mp4decrypt's.
    let out = std::env::temp_dir().join(format!("w7_dec_{}.mp4", std::process::id()));
    let key_arg = format!("{}:{}", hex(&LAYOUT_KID), hex(&LAYOUT_KEY));
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

    let mut fd = transmux::Fmp4Demux::new();
    let ref_media = fd.unpackage(bytes.as_slice()).expect("demux reference");
    let ref_video: Vec<Vec<u8>> = ref_media
        .tracks
        .iter()
        .find(|t| matches!(t.spec.config, transmux::CodecConfig::Avc { .. }))
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

/// Hex-encode 16 bytes for `mp4decrypt --key`.
fn hex(b: &[u8; 16]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// True if `mp4decrypt` (Bento4) is on `PATH`.
fn mp4decrypt_available() -> bool {
    // Bento4 CLIs print their banner and exit non-zero with no arguments, so
    // gate on spawning at all, not on the exit status.
    std::process::Command::new("mp4decrypt")
        .output()
        .map(|o| {
            String::from_utf8_lossy(&o.stdout).contains("MP4 Decrypter")
                || String::from_utf8_lossy(&o.stderr).contains("MP4 Decrypter")
        })
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// issue #990: a `seig` (CENC key-rotation) sample group must be rejected
// explicitly, not silently decrypted with only the track's default KID.
// ---------------------------------------------------------------------------

const CONTAINERS: &[&[u8; 4]] = &[b"moov", b"trak", b"mdia", b"minf", b"stbl"];

/// Recursively locate a box of `fourcc` within `[lo, hi)`. Returns
/// `(abs_offset, box_size)`. Descends into known container boxes.
fn find_box_range(data: &[u8], lo: usize, hi: usize, fourcc: &[u8; 4]) -> Option<(usize, usize)> {
    let mut off = lo;
    while off + 8 <= hi {
        let size =
            u32::from_be_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]]) as usize;
        if size < 8 || off + size > hi {
            break;
        }
        let t = [data[off + 4], data[off + 5], data[off + 6], data[off + 7]];
        if &t == fourcc {
            return Some((off, size));
        }
        if CONTAINERS.contains(&&t)
            && let Some(found) = find_box_range(data, off + 8, off + size, fourcc)
        {
            return Some(found);
        }
        off += size;
    }
    None
}

/// Grow (in place, ORIGINAL coordinate space) the 32-bit size of every
/// container box whose original span contains the insertion point `at`
/// (interior, or exactly at the box's own end — i.e. "append a new last
/// child").
fn grow_ancestors(data: &mut [u8], lo: usize, hi: usize, at: usize, grow: usize) {
    let mut off = lo;
    while off + 8 <= hi {
        let size =
            u32::from_be_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]]) as usize;
        if size < 8 || off + size > hi {
            break;
        }
        let t = [data[off + 4], data[off + 5], data[off + 6], data[off + 7]];
        // Only an actual container can be a genuine ancestor: a leaf box
        // (e.g. `saiz`) whose span happens to *end* exactly at the insertion
        // point (because it's the last child of the container we're
        // inserting into) must NOT have its own size grown — only real
        // ancestors up the tree do.
        if CONTAINERS.contains(&&t) && off < at && at <= off + size {
            let new_size = (size + grow) as u32;
            data[off..off + 4].copy_from_slice(&new_size.to_be_bytes());
            grow_ancestors(data, off + 8, off + size, at, grow);
        }
        off += size;
    }
}

/// Insert `extra` bytes at absolute offset `at` in `data`, growing every
/// ancestor container's size field so the file stays structurally valid.
fn insert_growing_ancestors(data: &[u8], at: usize, extra: &[u8]) -> Vec<u8> {
    let mut out = data.to_vec();
    grow_ancestors(&mut out, 0, data.len(), at, extra.len());
    out.splice(at..at, extra.iter().copied());
    out
}

/// Real `cenc.mp4` fixture, but with a `seig`-typed `sgpd` box appended as
/// the last child of the (single, protected) track's `stbl` — the shape a
/// real CENC key-rotation asset would carry (ISO/IEC 23001-7
/// CencSampleEncryptionInformationGroupEntry). Built by growing the real
/// fixture's box tree in place (real `moov`/`trak`/`stbl` bytes, not a
/// hand-rolled file), so this exercises the actual box-navigation path
/// `harvest_track` walks, not a synthetic stand-in for it.
fn cenc_mp4_with_seig_sgpd() -> Vec<u8> {
    use broadcast_common::Serialize;
    use transmux::sample_groups::{GROUPING_TYPE_SEIG, SampleGroupDescriptionBox, SgpdEntry};

    let file = read(CENC_MP4);
    let (stbl_start, stbl_size) =
        find_box_range(&file, 0, file.len(), b"stbl").expect("fixture has a stbl");
    let insert_at = stbl_start + stbl_size; // append as stbl's new last child

    let sgpd = SampleGroupDescriptionBox {
        version: 1,
        flags: 0,
        grouping_type: GROUPING_TYPE_SEIG,
        default_length: 0,
        default_sample_description_index: None,
        // The internal seig entry layout (KID/IV-size/pattern override) is
        // not typed by this crate (see `sample_groups.rs` docs) — only
        // `grouping_type` needs to be real for the detector under test, so
        // an opaque placeholder payload is enough here.
        entries: vec![SgpdEntry::Unknown(vec![0u8; 20])],
    };
    let sgpd_bytes = sgpd.to_bytes();

    insert_growing_ancestors(&file, insert_at, &sgpd_bytes)
}

/// A `seig` sample group in `stbl` must be rejected outright, not silently
/// decrypted with the track's default `tenc.default_KID` alone.
#[test]
fn seig_sample_group_is_rejected_not_silently_decrypted() {
    let file = cenc_mp4_with_seig_sgpd();

    let err = CencDecryptor::from_fmp4(&file).expect_err(
        "a track with a seig sample group must be rejected, not silently \
         accepted for single-key decryption",
    );
    assert!(
        matches!(err, transmux::Error::UnsupportedFeature(_)),
        "expected Error::UnsupportedFeature, got {err:?}"
    );
}

/// Sanity: the *unmodified* real fixture (no seig group) still decrypts fine
/// — the detector must not be a false-positive trap that rejects every
/// sgpd-bearing track regardless of grouping type.
#[test]
fn non_seig_content_is_unaffected() {
    let file = read(CENC_MP4);
    CencDecryptor::from_fmp4(&file).expect("plain cenc.mp4 (no seig) must still decrypt");
}
