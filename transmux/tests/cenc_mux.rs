//! CMAF muxer CENC box-emission tests (issue #564 Task 3).
//!
//! Verifies `transmux::init_segment::protect_init_segment` and
//! `transmux::movie_fragment::protect_media_segment` — the post-processing
//! passes that turn an already-muxed clear CMAF `CmafMux` output into a
//! standards-compliant CENC-protected one, reading the crypto metadata
//! [`CencEncryptor`] records on `Track::encryption`.
//!
//! This is a **structural/byte-exact box round trip**: build a `Media`,
//! encrypt it, mux it, protect the muxed bytes, then parse the emitted boxes
//! back and assert their shape. The full decrypt round trip + `mp4decrypt`
//! interop (confirming the `saio` moof-relative anchor choice against a real
//! external decryptor) is issue #564 Task 4 — out of scope here.
//!
//! Skips cleanly if the (normally-committed) cleartext fixture is absent.

#![cfg(feature = "cenc")]

use std::path::PathBuf;

use broadcast_common::{Encrypt, Package, Parse, Unpackage};
use transmux::cenc::{
    ProtectionSchemeInfoBox, SampleAuxInfoOffsetsBox, SampleAuxInfoSizesBox, SampleEncryptionBox,
};
use transmux::init_segment::{MovieBox, SampleEntryVariant, StblChild, protect_init_segment};
use transmux::movie_fragment::{FragmentProtection, MovieFragmentBox, protect_media_segment};
use transmux::{
    CencEncryptor, CencScheme, CmafMux, CodecConfig, EncryptConfig, IvGen, Media, SubsamplePolicy,
    TsDemux,
};

const KID: [u8; 16] = [
    0xa7, 0xe6, 0x1c, 0x37, 0x3e, 0x21, 0x90, 0x33, 0xc2, 0x10, 0x91, 0xfa, 0x60, 0x7b, 0xf3, 0xb8,
];
const KEY: [u8; 16] = [
    0x76, 0xa6, 0xc6, 0x5c, 0x5e, 0xa7, 0x62, 0x04, 0x6b, 0xd7, 0x49, 0xa2, 0xe6, 0x32, 0xcc, 0xbb,
];

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/ts/h264/main.ts")
}

/// The cleartext fixture, narrowed to its single AVC video track (mirrors
/// `tests/cenc_encrypt.rs`'s `clear_video_media`).
fn clear_video_media() -> Option<Media> {
    let path = fixture_path();
    if !path.exists() {
        eprintln!(
            "cenc_mux tests: SKIPPED — {path:?} not found (expected committed public fixture)."
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

// ---------------------------------------------------------------------------
// Raw box-walking helpers (byte-level, independent of the typed parsers under
// test — mirrors how `cenc_decrypt.rs` locates `sinf`/`senc` in a real file).
// ---------------------------------------------------------------------------

/// Find a top-level box by four-CC; returns `(file_offset, full_box_bytes)`.
fn find_top_box<'a>(data: &'a [u8], fourcc: &[u8; 4]) -> Option<(usize, &'a [u8])> {
    let mut off = 0usize;
    for step in transmux::box_iter(data) {
        let (box_ref, consumed) = step.ok()?;
        if box_ref.header.box_type.is(fourcc) {
            return Some((off, &data[off..off + consumed]));
        }
        off += consumed;
    }
    None
}

/// Find a child box by four-CC inside `data` (a box's body or another box's
/// full bytes scanned as a flat list); returns `(offset_in_data, full_box_bytes)`.
fn find_child_box<'a>(data: &'a [u8], fourcc: &[u8; 4]) -> Option<(usize, &'a [u8])> {
    let mut off = 0usize;
    while off + 8 <= data.len() {
        let size =
            u32::from_be_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]]) as usize;
        if size < 8 {
            break;
        }
        let end = (off + size).min(data.len());
        if &data[off + 4..off + 8] == fourcc {
            return Some((off, &data[off..end]));
        }
        off += size;
    }
    None
}

/// Find the `traf` (full bytes + its offset within the moof body) whose
/// `tfhd.track_id` matches.
fn find_traf_for_track(moof_body: &[u8], track_id: u32) -> Option<(usize, &[u8])> {
    let mut off = 0usize;
    while off + 8 <= moof_body.len() {
        let size = u32::from_be_bytes([
            moof_body[off],
            moof_body[off + 1],
            moof_body[off + 2],
            moof_body[off + 3],
        ]) as usize;
        if size < 8 {
            break;
        }
        let end = (off + size).min(moof_body.len());
        if &moof_body[off + 4..off + 8] == b"traf" {
            let traf_body = &moof_body[off + 8..end];
            if let Some((_, tfhd)) = find_child_box(traf_body, b"tfhd") {
                if let Ok(parsed) =
                    transmux::movie_fragment::TrackFragmentHeaderBox::parse_body(&tfhd[8..])
                {
                    if parsed.track_id == track_id {
                        return Some((off, &moof_body[off..end]));
                    }
                }
            }
        }
        off += size;
    }
    None
}

fn parse_senc(senc_bytes: &[u8], per_sample_iv_size: u8) -> SampleEncryptionBox {
    let version = senc_bytes[8];
    let flags = u32::from_be_bytes([0, senc_bytes[9], senc_bytes[10], senc_bytes[11]]);
    SampleEncryptionBox::parse_body(&senc_bytes[12..], version, flags, per_sample_iv_size)
        .expect("parse senc body")
}

fn cenc_cfg(subsample: SubsamplePolicy) -> EncryptConfig {
    EncryptConfig {
        scheme: CencScheme::Cenc,
        kid: KID,
        key: KEY,
        iv: IvGen::Counter { base: 0 },
        pattern: None,
        subsample,
    }
}

/// Mux `media` through the real `CmafMux` packager, then apply both
/// protection passes for `track_id`. Returns the fully protected CMAF bytes.
fn protect(media: &Media, track_id: u32) -> Vec<u8> {
    let raw = CmafMux::new(1).package(media).expect("CmafMux::package");
    let enc = media.tracks[0]
        .encryption
        .as_ref()
        .expect("track.encryption populated by CencEncryptor");

    let with_protected_init =
        protect_init_segment(&raw, track_id, enc).expect("protect_init_segment");

    let fragment_protection = FragmentProtection {
        track_id,
        entries: &enc.samples,
        per_sample_iv_size: enc.tenc.default_per_sample_iv_size,
    };
    protect_media_segment(&with_protected_init, &[fragment_protection])
        .expect("protect_media_segment")
}

/// The init-segment half: the sample entry becomes `encv` with a `sinf`
/// carrying the original four-CC, the configured scheme, and a `tenc`
/// matching `Track::encryption.tenc` exactly.
#[test]
fn protect_init_segment_emits_encv_sinf() {
    let Some(mut media) = clear_video_media() else {
        return;
    };
    let cfg = cenc_cfg(SubsamplePolicy::WholeSample);
    CencEncryptor.encrypt(&mut media, &cfg).expect("encrypt");
    let track_id = media.tracks[0].spec.track_id;

    let protected = protect(&media, track_id);

    let (_, moov_bytes) = find_top_box(&protected, b"moov").expect("moov present");
    let moov = MovieBox::parse(moov_bytes).expect("parse moov");
    let track = moov
        .tracks
        .iter()
        .find(|t| t.tkhd.track_id == track_id)
        .expect("track present");
    let stbl = track
        .mdia
        .as_ref()
        .unwrap()
        .minf
        .as_ref()
        .unwrap()
        .stbl
        .as_ref()
        .unwrap();
    let stsd = stbl
        .children
        .iter()
        .find_map(|c| match c {
            StblChild::Stsd(s) => Some(s),
            _ => None,
        })
        .expect("stsd present");
    assert_eq!(stsd.entries.len(), 1);

    let SampleEntryVariant::Unknown(entry) = &stsd.entries[0] else {
        panic!(
            "expected a protected (Unknown-wrapper) sample entry, got {:?}",
            stsd.entries[0]
        );
    };
    assert_eq!(&entry.box_type, b"encv", "sample entry renamed to encv");

    // `entry.data` is the encv box's body: the 78-byte fixed VisualSampleEntry
    // fields (ISO/IEC 14496-12:2015 §12.1.3) come first, unwrapped (not
    // themselves a box) — child boxes (avcC, then our appended sinf) start
    // right after, matching `crate::cenc_decrypt::find_sinf_in_stsd`'s own
    // `VISUAL_SAMPLE_ENTRY_HDR` skip.
    const VISUAL_SAMPLE_ENTRY_FIXED_LEN: usize = 78;
    let (_, sinf_bytes) = find_child_box(&entry.data[VISUAL_SAMPLE_ENTRY_FIXED_LEN..], b"sinf")
        .expect("sinf child present");
    let sinf = ProtectionSchemeInfoBox::parse(sinf_bytes).expect("parse sinf");
    assert_eq!(
        &sinf.original_format.data_format, b"avc1",
        "frma keeps the original codec four-CC"
    );
    let schm = sinf.scheme_type.expect("schm present");
    assert_eq!(&schm.scheme_type, b"cenc");
    assert_eq!(schm.scheme_version, 0x0001_0000);
    let schi = sinf.scheme_info.expect("schi present");
    let tenc = schi.tenc.expect("tenc present");
    let enc = media.tracks[0].encryption.as_ref().unwrap();
    assert_eq!(
        tenc, enc.tenc,
        "tenc matches Track::encryption.tenc exactly"
    );
}

/// The fragment half: `senc`/`saiz`/`saio` are appended to the protected
/// track's `traf`, `senc` carries exactly the fragment's per-sample IVs, and
/// `saio.offset[0]` genuinely points at the first sample's IV bytes.
#[test]
fn protect_media_segment_emits_senc_saiz_saio() {
    let Some(mut media) = clear_video_media() else {
        return;
    };
    let cfg = cenc_cfg(SubsamplePolicy::WholeSample);
    CencEncryptor.encrypt(&mut media, &cfg).expect("encrypt");
    let track_id = media.tracks[0].spec.track_id;
    let sample_count = media.tracks[0].samples.len();
    assert!(sample_count > 1, "fixture must carry more than one sample");

    let protected = protect(&media, track_id);
    let enc = media.tracks[0].encryption.as_ref().unwrap();

    let (moof_file_off, moof_bytes) = find_top_box(&protected, b"moof").expect("moof present");
    // Sanity: `protect_media_segment` really did grow the moof relative to a
    // freshly-built clear one (proves the boxes were actually appended, not a
    // silent no-op).
    let clear_raw = CmafMux::new(1)
        .package(&{
            let mut m = media.clone();
            m.tracks[0].encryption = None;
            m
        })
        .expect("clear CmafMux::package");
    let (_, clear_moof_bytes) = find_top_box(&clear_raw, b"moof").expect("clear moof present");
    assert!(
        moof_bytes.len() > clear_moof_bytes.len(),
        "protected moof must be larger than the clear one"
    );

    let moof_body = &moof_bytes[8..];
    let (traf_off_in_body, traf_bytes) =
        find_traf_for_track(moof_body, track_id).expect("traf for track present");
    let traf_body = &traf_bytes[8..];

    let moof = MovieFragmentBox::parse_body(moof_body).expect("parse moof");
    let traf = moof
        .traf
        .iter()
        .find(|t| t.tfhd.track_id == track_id)
        .expect("typed traf present");
    let typed_sample_count: usize = traf.trun.iter().map(|r| r.samples.len()).sum();
    assert_eq!(typed_sample_count, sample_count);

    let (senc_off, senc_bytes) = find_child_box(traf_body, b"senc").expect("senc present");
    let (_, saiz_bytes) = find_child_box(traf_body, b"saiz").expect("saiz present");
    let (_, saio_bytes) = find_child_box(traf_body, b"saio").expect("saio present");

    let senc = parse_senc(senc_bytes, enc.tenc.default_per_sample_iv_size);
    assert_eq!(senc.entries.len(), sample_count);
    assert_eq!(
        &senc.entries, &enc.samples,
        "senc entries match Track::encryption.samples exactly"
    );
    assert_eq!(
        senc.flags & transmux::SENC_FLAG_USE_SUBSAMPLE_ENCRYPTION,
        0,
        "WholeSample policy: no subsample flag"
    );

    let saiz = SampleAuxInfoSizesBox::parse_box(saiz_bytes).expect("parse saiz");
    assert_eq!(
        saiz.default_sample_info_size, enc.tenc.default_per_sample_iv_size,
        "uniform aux size (no subsamples) == IV size"
    );

    let saio = SampleAuxInfoOffsetsBox::parse_box(saio_bytes).expect("parse saio");
    assert_eq!(saio.offsets.len(), 1);

    // Byte-exact anchor check: moof-relative offset 0 == first byte of the
    // moof box, so `saio.offset[0]` must land exactly on the first sample's
    // IV inside `senc` (16 bytes past the senc box's own start).
    let senc_start_in_moof =
        8 /* moof header */ + traf_off_in_body + 8 /* traf header */ + senc_off;
    let expected_offset = senc_start_in_moof as u64 + 16;
    assert_eq!(
        saio.offsets[0], expected_offset,
        "saio.offset[0] must be the moof-relative byte position of senc's first IV"
    );
    // Cross-check against the actual file bytes: the first IV byte at that
    // moof-relative position must equal the first byte of the first entry's
    // recorded IV.
    let iv_pos_in_file = moof_file_off + saio.offsets[0] as usize;
    assert_eq!(
        protected[iv_pos_in_file], enc.samples[0].initialization_vector[0],
        "saio anchor resolves to the real first IV byte in the file"
    );
}

/// `SubsamplePolicy::Video` produces a real per-NAL subsample map; `senc`
/// must set the use-subsample-encryption flag and carry each sample's
/// subsample list, and `saiz` must record a per-sample size when sizes vary.
#[test]
fn protect_media_segment_sets_subsample_flag() {
    let Some(mut media) = clear_video_media() else {
        return;
    };
    let cfg = cenc_cfg(SubsamplePolicy::Video);
    CencEncryptor.encrypt(&mut media, &cfg).expect("encrypt");
    let track_id = media.tracks[0].spec.track_id;
    let enc = media.tracks[0].encryption.clone().unwrap();
    assert!(
        enc.samples.iter().any(|e| !e.subsamples.is_empty()),
        "fixture must produce at least one subsampled entry to bite"
    );

    let protected = protect(&media, track_id);
    let (_, moof_bytes) = find_top_box(&protected, b"moof").expect("moof present");
    let moof_body = &moof_bytes[8..];
    let (_, traf_bytes) = find_traf_for_track(moof_body, track_id).expect("traf present");
    let traf_body = &traf_bytes[8..];
    let (_, senc_bytes) = find_child_box(traf_body, b"senc").expect("senc present");

    let senc = parse_senc(senc_bytes, enc.tenc.default_per_sample_iv_size);
    assert_ne!(
        senc.flags & transmux::SENC_FLAG_USE_SUBSAMPLE_ENCRYPTION,
        0,
        "subsample flag must be set when any sample carries subsamples"
    );
    assert_eq!(senc.entries.len(), enc.samples.len());
    for (got, want) in senc.entries.iter().zip(enc.samples.iter()) {
        assert_eq!(got.subsamples, want.subsamples);
        assert_eq!(got.initialization_vector, want.initialization_vector);
    }
}

/// A track_id that isn't present in the muxed `moof` must error, not panic
/// or silently no-op.
#[test]
fn protect_media_segment_unknown_track_errors() {
    let Some(mut media) = clear_video_media() else {
        return;
    };
    let cfg = cenc_cfg(SubsamplePolicy::WholeSample);
    CencEncryptor.encrypt(&mut media, &cfg).expect("encrypt");
    let enc = media.tracks[0].encryption.clone().unwrap();

    let raw = CmafMux::new(1).package(&media).expect("CmafMux::package");
    let bogus_protection = FragmentProtection {
        track_id: 9999,
        entries: &enc.samples,
        per_sample_iv_size: enc.tenc.default_per_sample_iv_size,
    };
    let err = protect_media_segment(&raw, &[bogus_protection]).unwrap_err();
    assert!(matches!(err, transmux::Error::InvalidInput(_)));
}

/// An empty `protections` slice must return the input byte-identical
/// (the clear-mux path is never touched by these functions unless asked).
#[test]
fn protect_media_segment_empty_protections_is_identity() {
    let Some(media) = clear_video_media() else {
        return;
    };
    let raw = CmafMux::new(1).package(&media).expect("CmafMux::package");
    let out = protect_media_segment(&raw, &[]).expect("identity pass");
    assert_eq!(out, raw);
}
