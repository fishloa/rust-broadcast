//! Synthetic fragmented CENC fixtures for decryptor behaviour that lives in
//! the fragment's own boxes: `senc` flags, a missing `senc`, `tfdt` seeding
//! and `trun` sample flags (audit 23 findings 173-178).
//!
//! The files are produced by this crate's own packager (`CmafMux` +
//! `protect_init_segment` + `protect_media_segment`) from the committed
//! `ts/h264/main.ts` fixture, then patched at the box level where a specific
//! (malformed or unusual) fragment shape is the subject under test.
#![cfg(feature = "cenc")]

use broadcast_common::{Decrypt, Encrypt, Package, Unpackage};
use transmux::TrackFragmentRunBox;
use transmux::init_segment::protect_init_segment;
use transmux::movie_fragment::{FragmentProtection, MovieFragmentBox, protect_media_segment};
use transmux::{
    CencDecryptor, CencEncryptor, CencScheme, CmafMux, CodecConfig, EncryptConfig, KeyMap, Media,
    Sample, SubsamplePolicy, Track, TrackId, TrackSpec, TsDemux,
};

const KID: [u8; 16] = [
    0xa7, 0xe6, 0x1c, 0x37, 0x3e, 0x21, 0x90, 0x33, 0xc2, 0x10, 0x91, 0xfa, 0x60, 0x7b, 0xf3, 0xb8,
];
const KEY: [u8; 16] = [
    0x76, 0xa6, 0xc6, 0x5c, 0x5e, 0xa7, 0x62, 0x04, 0x6b, 0xd7, 0x49, 0xa2, 0xe6, 0x32, 0xcc, 0xbb,
];

fn clear_video_media() -> Media {
    let bytes = test_fixtures::read("ts/h264/main.ts");
    let media = TsDemux::new()
        .unpackage(bytes.as_slice())
        .expect("demux ts/h264/main.ts");
    media
        .select_tracks_by(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
        .expect("AVC video track present")
}

fn cenc_cfg(subsample: SubsamplePolicy) -> EncryptConfig {
    EncryptConfig::new(CencScheme::Cenc, KID, subsample)
}

fn cbcs_cfg(subsample: SubsamplePolicy) -> EncryptConfig {
    EncryptConfig::new(CencScheme::Cbcs, KID, subsample)
}

fn keys() -> KeyMap {
    KeyMap::new().with_key(KID, KEY)
}

/// `(offset, size, fourcc)` of every box directly in `body`.
fn children(body: &[u8]) -> Vec<(usize, usize, [u8; 4])> {
    let mut out = Vec::new();
    let mut off = 0usize;
    while off + 8 <= body.len() {
        let size =
            u32::from_be_bytes([body[off], body[off + 1], body[off + 2], body[off + 3]]) as usize;
        if size < 8 || off + size > body.len() {
            break;
        }
        let ty: [u8; 4] = body[off + 4..off + 8]
            .try_into()
            .expect("four-byte box type");
        out.push((off, size, ty));
        off += size;
    }
    out
}

fn find_top(data: &[u8], fourcc: &[u8; 4]) -> (usize, usize) {
    children(data)
        .into_iter()
        .find(|(_, _, ty)| ty == fourcc)
        .map(|(off, size, _)| (off, size))
        .unwrap_or_else(|| panic!("top-level box {:?} present", core::str::from_utf8(fourcc)))
}

/// Mux `media` through the real `CmafMux` packager, protect the init segment
/// of every track, then apply the fragment protection for every track.
fn protect_all(media: &Media) -> Vec<u8> {
    let mut out = CmafMux::new(1).package(media).expect("CmafMux::package");
    for track in &media.tracks {
        let enc = track
            .encryption
            .as_ref()
            .expect("track.encryption populated by CencEncryptor");
        out = protect_init_segment(&out, track.spec.track_id, enc).expect("protect_init_segment");
    }
    let protections: Vec<FragmentProtection<'_>> = media
        .tracks
        .iter()
        .map(|t| {
            let enc = t
                .encryption
                .as_ref()
                .expect("track.encryption populated by CencEncryptor");
            FragmentProtection {
                track_id: t.spec.track_id,
                entries: &enc.samples,
                per_sample_iv_size: enc.tenc.default_per_sample_iv_size,
            }
        })
        .collect();
    protect_media_segment(&out, &protections).expect("protect_media_segment")
}

/// The `(offset, size)` of the *body* of the `traf` for `track_id` inside the
/// first `moof`, located by matching the typed parser's `tfhd.track_id` order.
fn traf_range(file: &[u8], track_id: u32) -> (usize, usize) {
    let (moof_off, moof_size) = find_top(file, b"moof");
    let moof_body = &file[moof_off + 8..moof_off + moof_size];
    let moof = MovieFragmentBox::parse_body(moof_body).expect("parse moof");
    let index = moof
        .traf
        .iter()
        .position(|t| t.tfhd.track_id == TrackId::new(track_id))
        .expect("traf for the requested track");
    let trafs: Vec<(usize, usize)> = children(moof_body)
        .into_iter()
        .filter(|(_, _, ty)| ty == b"traf")
        .map(|(off, size, _)| (off, size))
        .collect();
    let (off, size) = trafs[index];
    (moof_off + 8 + off + 8, size - 8)
}

fn traf_bytes(file: &[u8], track_id: u32) -> &[u8] {
    let (off, size) = traf_range(file, track_id);
    &file[off..off + size]
}

fn sample_bytes(media: &Media, track_id: u32) -> Vec<Vec<u8>> {
    media
        .tracks
        .iter()
        .find(|t| t.spec.track_id == TrackId::new(track_id))
        .unwrap_or_else(|| panic!("track {track_id} present"))
        .samples
        .iter()
        .map(|s| s.data.to_vec())
        .collect()
}

// ---------------------------------------------------------------------------
// finding 173 (#3654) — senc FullBox flags are read from their own bytes
// ---------------------------------------------------------------------------

/// A fragmented `senc` with `UseSubSampleEncryption` (0x2) set and real
/// clear/protected runs must decrypt back to the cleartext byte-for-byte.
/// The flag bytes are `senc[9..12]` (after size, type and version); reading
/// any other offset drops the subsample maps and "decrypts" the clear NAL
/// headers too.
#[test]
fn subsample_senc_flags_are_read_from_the_fullbox_flag_bytes() {
    let mut media = clear_video_media();
    let clear = sample_bytes(&media, 1);
    CencEncryptor::new(KEY)
        .encrypt(&mut media, &cenc_cfg(SubsamplePolicy::Video))
        .expect("encrypt with a per-NAL subsample map");
    let protected = protect_all(&media);
    assert!(
        clear.iter().any(|s| s.len() > 20),
        "fixture must carry samples longer than a NAL header"
    );

    let traf = traf_bytes(&protected, 1);
    let (senc_off, senc_size, _) = children(traf)
        .into_iter()
        .find(|(_, _, ty)| ty == b"senc")
        .expect("senc present in the protected traf");
    let senc = &traf[senc_off..senc_off + senc_size];
    let flags = u32::from_be_bytes([0, senc[9], senc[10], senc[11]]);
    assert_eq!(
        flags, 0x0000_0002,
        "senc FullBox flags carry UseSubSampleEncryption"
    );

    let dec = CencDecryptor::from_fmp4(&protected).expect("harvest the protected fragment");
    let mut out = dec.demux().expect("demux");
    dec.decrypt(&mut out, &keys()).expect("decrypt");
    assert_eq!(
        sample_bytes(&out, 1),
        clear,
        "clear runs stay byte-identical and protected runs decrypt to the known plaintext"
    );
}

// ---------------------------------------------------------------------------
// finding 174 (#3655) — a per-sample-IV track with no senc must not get
// synthesized empty-IV entries
// ---------------------------------------------------------------------------

/// Rename the `traf`'s `senc` to an unknown four-CC, leaving every box size
/// and offset intact. The track still declares `default_per_sample_iv_size =
/// 8`, so there is no constant IV to fall back on: the harvest must leave the
/// entry list empty and `decrypt` must fail the sample-count check.
#[test]
fn per_sample_iv_track_without_senc_errors_instead_of_empty_iv_entries() {
    let mut media = clear_video_media();
    CencEncryptor::new(KEY)
        .encrypt(&mut media, &cenc_cfg(SubsamplePolicy::WholeSample))
        .expect("encrypt");
    let mut protected = protect_all(&media);
    assert_eq!(
        media.tracks[0]
            .encryption
            .as_ref()
            .expect("tenc")
            .tenc
            .default_per_sample_iv_size,
        8,
        "the fixture must need per-sample IVs (no constant-IV fallback)"
    );

    let (traf_off, traf_size) = traf_range(&protected, 1);
    let traf = &protected[traf_off..traf_off + traf_size];
    let (senc_off, _, _) = children(traf)
        .into_iter()
        .find(|(_, _, ty)| ty == b"senc")
        .expect("senc present before the patch");
    let ty = traf_off + senc_off + 4;
    protected[ty..ty + 4].copy_from_slice(b"sken");

    let dec = CencDecryptor::from_fmp4(&protected).expect("harvest still succeeds");
    assert!(
        dec.sample_entries().is_empty(),
        "a per-sample-IV track with no senc must not synthesize placeholder entries"
    );
    let mut out = dec.demux().expect("demux");
    let before = sample_bytes(&out, 1);
    let err = dec
        .decrypt(&mut out, &keys())
        .expect_err("decrypt must fail the sample-count check");
    assert!(
        matches!(err, transmux::Error::InvalidInput(_)),
        "expected InvalidInput, got {err:?}"
    );
    assert_eq!(
        sample_bytes(&out, 1),
        before,
        "a rejected decrypt must leave every sample byte-identical"
    );
}

// ---------------------------------------------------------------------------
// finding 177 (#3658) — the first sample's dts comes from the *target*
// track's tfdt
// ---------------------------------------------------------------------------

fn timescale(value: u32) -> broadcast_common::timescale::Timescale {
    broadcast_common::timescale::Timescale::try_new(value).expect("non-zero timescale")
}

fn retimed_track(base: &Track, track_id: u32, first_dts: i64) -> Track {
    let spec = TrackSpec::new(
        TrackId::new(track_id),
        base.spec.timescale,
        base.spec.config.clone(),
    );
    let samples = base
        .samples
        .iter()
        .enumerate()
        .map(|(i, sample)| {
            let dts = first_dts + i as i64 * 1000;
            Sample::new(
                sample.data.clone(),
                Some(dts),
                Some(dts),
                Some(1000),
                sample.flags.is_sync,
            )
            .expect("valid timing")
        })
        .collect();
    Track::new_at(spec, samples, first_dts as u64)
}

fn dts_of(sample: &Sample) -> i64 {
    match sample.timing.expect("fragment samples are timed") {
        transmux::ir::SampleTiming::Timed { dts, .. } => dts,
        other => panic!("expected a Timed sample, got {other:?}"),
    }
}

/// Two protected AVC tracks with `tfdt` 90 000 and 500 000: each track's
/// decrypted samples must be seeded from its own `tfdt`, never 0 and never
/// the other track's base time.
#[test]
fn first_sample_dts_is_seeded_from_the_target_tracks_tfdt() {
    let base = clear_video_media();
    assert!(
        base.tracks[0].samples.len() > 1,
        "fixture must carry more than one sample"
    );
    let mut media = Media::new(
        vec![
            retimed_track(&base.tracks[0], 1, 90_000),
            retimed_track(&base.tracks[0], 2, 500_000),
        ],
        timescale(1000),
    );
    CencEncryptor::new(KEY)
        .encrypt(&mut media, &cbcs_cfg(SubsamplePolicy::WholeSample))
        .expect("encrypt both tracks");
    let protected = protect_all(&media);

    let dec = CencDecryptor::from_fmp4(&protected).expect("harvest");
    let mut out = dec.demux().expect("demux");
    dec.decrypt(&mut out, &keys()).expect("decrypt");

    let find = |id: u32| {
        out.tracks
            .iter()
            .find(|t| t.spec.track_id == TrackId::new(id))
            .unwrap_or_else(|| panic!("track {id} decrypted"))
    };
    let target = find(1);
    assert_eq!(
        dts_of(&target.samples[0]),
        90_000,
        "the first sample's dts is track 1's own tfdt"
    );
    assert_eq!(
        dts_of(&target.samples[1]),
        91_000,
        "the next sample continues from 90000 + duration[0]"
    );
    assert_eq!(
        dts_of(&find(2).samples[0]),
        500_000,
        "the other track keeps its own tfdt"
    );
}

// ---------------------------------------------------------------------------
// finding 178 (#3659) — `is_sync` comes from the trun sample flags
// ---------------------------------------------------------------------------

/// A fragment carrying per-sample flags (first sample `0x02000000` sync, the
/// rest exactly `0x01010000` non-sync) must decrypt to those `is_sync`
/// values: inverting or masking the non-sync bit breaks seeking and
/// segmenting of the decrypted output.
#[test]
fn trun_sample_flags_drive_is_sync_after_decryption() {
    let base = clear_video_media();
    let spec = TrackSpec::new(
        TrackId::new(1),
        base.tracks[0].spec.timescale,
        base.tracks[0].spec.config.clone(),
    );
    let samples: Vec<Sample> = base.tracks[0]
        .samples
        .iter()
        .enumerate()
        .map(|(i, sample)| {
            let dts = i as i64 * 1000;
            Sample::new(
                sample.data.clone(),
                Some(dts),
                Some(dts),
                Some(1000),
                i == 0,
            )
            .expect("valid timing")
        })
        .collect();
    assert!(samples.len() > 1, "need a sync sample plus non-sync ones");
    let mut media = Media::new(vec![Track::new_at(spec, samples, 0)], timescale(1000));
    CencEncryptor::new(KEY)
        .encrypt(&mut media, &cbcs_cfg(SubsamplePolicy::WholeSample))
        .expect("encrypt");
    let protected = protect_all(&media);

    let traf = traf_bytes(&protected, 1);
    let (trun_off, trun_size, _) = children(traf)
        .into_iter()
        .find(|(_, _, ty)| ty == b"trun")
        .expect("trun present");
    let trun = TrackFragmentRunBox::parse_body(&traf[trun_off + 8..trun_off + trun_size])
        .expect("parse trun");
    let raw_flags: Vec<u32> = trun
        .samples
        .iter()
        .map(|s| s.sample_flags.expect("per-sample flags present"))
        .collect();
    assert_eq!(raw_flags[0], 0x0200_0000, "first sample is a sync sample");
    assert!(
        raw_flags[1..].iter().all(|&f| f == 0x0101_0000),
        "the remaining samples carry exactly the non-sync word"
    );

    let dec = CencDecryptor::from_fmp4(&protected).expect("harvest");
    let mut out = dec.demux().expect("demux");
    dec.decrypt(&mut out, &keys()).expect("decrypt");
    let got: Vec<bool> = out.tracks[0]
        .samples
        .iter()
        .map(|s| s.flags.is_sync)
        .collect();
    assert!(got[0], "the sync sample stays a sync sample");
    assert!(
        got[1..].iter().all(|&is_sync| !is_sync),
        "0x01010000 means non-sync for every following sample"
    );
}
