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

use broadcast_common::{Decrypt, Encrypt, Package, Parse, Serialize, Unpackage};
use transmux::cenc::{
    ProtectionSchemeInfoBox, SampleAuxInfoOffsetsBox, SampleAuxInfoSizesBox, SampleEncryptionBox,
};
use transmux::init_segment::{MovieBox, SampleEntryVariant, StblChild, protect_init_segment};
use transmux::movie_fragment::{FragmentProtection, MovieFragmentBox, protect_media_segment};
use transmux::{
    CencEncryptor, CencScheme, CmafMux, CodecConfig, ConstantIvSenc, EncryptConfig, IvGen, Media,
    SubsamplePolicy, TsDemux,
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
            if let Some((_, tfhd)) = find_child_box(traf_body, b"tfhd")
                && let Ok(parsed) =
                    transmux::movie_fragment::TrackFragmentHeaderBox::parse_body(&tfhd[8..])
                && parsed.track_id == track_id
            {
                return Some((off, &moof_body[off..end]));
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
        iv: IvGen::Counter,
        pattern: None,
        subsample,
        constant_iv_senc: ConstantIvSenc::default(),
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
    CencEncryptor::new(KEY)
        .encrypt(&mut media, &cfg)
        .expect("encrypt");
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
    CencEncryptor::new(KEY)
        .encrypt(&mut media, &cfg)
        .expect("encrypt");
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
    CencEncryptor::new(KEY)
        .encrypt(&mut media, &cfg)
        .expect("encrypt");
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
    CencEncryptor::new(KEY)
        .encrypt(&mut media, &cfg)
        .expect("encrypt");
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

// ---------------------------------------------------------------------------
// Fix wave H items 2/3: a `moof` (or `traf`) carrying opaque children
// ---------------------------------------------------------------------------

/// Splice `extra` into the first `moof`'s body right after its `mfhd`, so the
/// fragment has a moof-level opaque child. Returns the new buffer.
fn splice_into_moof(segment: &[u8], extra: &[u8]) -> Vec<u8> {
    let mut off = 0usize;
    while off + 8 <= segment.len() {
        let size = u32::from_be_bytes([
            segment[off],
            segment[off + 1],
            segment[off + 2],
            segment[off + 3],
        ]) as usize;
        if size < 8 || off + size > segment.len() {
            break;
        }
        if &segment[off + 4..off + 8] == b"moof" {
            // After the 8-byte `moof` header and the 16-byte `mfhd`.
            let at = off + 8 + 16;
            let mut out = segment[..at].to_vec();
            out.extend_from_slice(extra);
            out.extend_from_slice(&segment[at..]);
            let new_size = (size + extra.len()) as u32;
            out[off..off + 4].copy_from_slice(&new_size.to_be_bytes());
            // Growing the `moof` moves the `mdat` after it, while every
            // `trun.data_offset` stays moof-relative — so each inserted byte
            // must be added to the offsets that address data past the
            // insertion point.
            patch_trun_data_offsets(&mut out, off, new_size as usize, extra.len() as i32);
            return out;
        }
        off += size;
    }
    panic!("no moof in the segment");
}

/// Add `delta` to every `trun.data_offset` inside the `moof` at `moof_at`
/// (of `moof_size` bytes).
fn patch_trun_data_offsets(out: &mut [u8], moof_at: usize, moof_size: usize, delta: i32) {
    fn walk(out: &mut [u8], at: usize, size: usize, delta: i32) {
        let mut off = at + 8;
        while off + 8 <= at + size {
            let sz =
                u32::from_be_bytes([out[off], out[off + 1], out[off + 2], out[off + 3]]) as usize;
            if sz < 8 || off + sz > at + size {
                break;
            }
            if &out[off + 4..off + 8] == b"trun" {
                let flags =
                    u32::from_be_bytes([0, out[off + 9], out[off + 10], out[off + 11]]) & 0xFF_FFFF;
                if flags & 1 != 0 {
                    let at_off = off + 8 + 4 + 4;
                    let cur = i32::from_be_bytes([
                        out[at_off],
                        out[at_off + 1],
                        out[at_off + 2],
                        out[at_off + 3],
                    ]);
                    let next = cur.saturating_add(delta);
                    out[at_off..at_off + 4].copy_from_slice(&next.to_be_bytes());
                }
            } else if matches!(&out[off + 4..off + 8], b"traf") {
                walk(out, off, sz, delta);
            }
            off += sz;
        }
    }
    walk(out, moof_at, moof_size, delta);
}

/// Splice `extra` into the first `traf`'s body, after its `trun`.
fn splice_into_traf(segment: &[u8], extra: &[u8]) -> Vec<u8> {
    // Locate the `traf` inside the `moof`.
    let mut moof_off = None;
    let mut off = 0usize;
    while off + 8 <= segment.len() {
        let size = u32::from_be_bytes([
            segment[off],
            segment[off + 1],
            segment[off + 2],
            segment[off + 3],
        ]) as usize;
        if size < 8 || off + size > segment.len() {
            break;
        }
        if &segment[off + 4..off + 8] == b"moof" {
            moof_off = Some((off, size));
            break;
        }
        off += size;
    }
    let (moof_at, moof_size) = moof_off.expect("a moof");
    let mut traf = None;
    let mut k = moof_at + 8;
    while k + 8 <= moof_at + moof_size {
        let size = u32::from_be_bytes([segment[k], segment[k + 1], segment[k + 2], segment[k + 3]])
            as usize;
        if size < 8 || k + size > moof_at + moof_size {
            break;
        }
        if &segment[k + 4..k + 8] == b"traf" {
            traf = Some((k, size));
            break;
        }
        k += size;
    }
    let (traf_at, traf_size) = traf.expect("a traf");

    let mut out = Vec::new();
    // Insert after the whole `traf` (so its own addressing is untouched) —
    // the point is an *opaque traf child*, not where it sits.
    let after_traf = traf_at + traf_size;
    out.extend_from_slice(&segment[..after_traf]);
    // The inserted box grows the traf only if placed inside it; place it
    // inside, immediately after the traf's own children, and grow both
    // ancestors.
    out.extend_from_slice(extra);
    out.extend_from_slice(&segment[after_traf..]);
    // Grow the traf and the moof by the inserted length. This keeps every
    // `trun.data_offset` (moof-relative) addressing the same absolute data as
    // long as the segment's `mdat` follows the `moof`.
    let d = extra.len() as u32;
    out[traf_at..traf_at + 4].copy_from_slice(&(traf_size as u32 + d).to_be_bytes());
    out[moof_at..moof_at + 4].copy_from_slice(&(moof_size as u32 + d).to_be_bytes());
    out
}

/// A `moof` with an opaque moof-level child must still be rewritten: the
/// rebuilt length has to count that child (audit item 2). With the old
/// `header + mfhd + Σ traf` arithmetic the consistency check at the end
/// rejected the segment outright.
#[test]
fn protect_media_segment_keeps_a_moof_level_opaque_child() {
    let Some(mut media) = clear_video_media() else {
        return;
    };
    let cfg = cenc_cfg(SubsamplePolicy::WholeSample);
    CencEncryptor::new(KEY)
        .encrypt(&mut media, &cfg)
        .expect("encrypt");
    let raw = CmafMux::new(1).package(&media).expect("CmafMux::package");
    let enc = media.tracks[0].encryption.as_ref().expect("encryption");

    let with_protected_init = protect_init_segment(&raw, 1, enc).expect("protect_init_segment");
    // A real moof-level opaque child: a `pssh` (ISO/IEC 23001-7 §8.1.1) —
    // 8-byte header + FullBox(4) + system_id(16) + data_size(4) + no data.
    let mut pssh = Vec::new();
    pssh.extend_from_slice(&32u32.to_be_bytes());
    pssh.extend_from_slice(b"pssh");
    pssh.extend_from_slice(&[0, 0, 0, 0]); // version 0, flags 0
    pssh.extend_from_slice(&KID); // system_id (any 16 bytes)
    pssh.extend_from_slice(&0u32.to_be_bytes()); // data_size
    let spliced = splice_into_moof(&with_protected_init, &pssh);

    let fp = FragmentProtection {
        track_id: 1,
        entries: &enc.samples,
        per_sample_iv_size: enc.tenc.default_per_sample_iv_size,
    };
    let out = protect_media_segment(&spliced, &[fp]).expect("protect_media_segment");

    // The `pssh` must survive, and every `trun.data_offset` must still point
    // at the same absolute byte as in the clear `mdat`.
    let mut moof = None;
    let mut off = 0usize;
    while off + 8 <= out.len() {
        let size =
            u32::from_be_bytes([out[off], out[off + 1], out[off + 2], out[off + 3]]) as usize;
        if size < 8 || off + size > out.len() {
            break;
        }
        if &out[off + 4..off + 8] == b"moof" {
            moof = Some((off, size));
            break;
        }
        off += size;
    }
    let (moof_at, moof_size) = moof.expect("a moof in the output");
    let moof_bytes = &out[moof_at..moof_at + moof_size];
    assert!(
        moof_bytes.windows(4).any(|w| w == b"pssh"),
        "the moof-level pssh must survive the rewrite"
    );

    // The mdat data must be byte-identical to the pre-protection buffer's.
    let mdat_of = |d: &[u8]| -> Vec<u8> {
        let mut o = 0usize;
        while o + 8 <= d.len() {
            let s = u32::from_be_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]]) as usize;
            if s < 8 || o + s > d.len() {
                break;
            }
            if &d[o + 4..o + 8] == b"mdat" {
                return d[o + 8..o + s].to_vec();
            }
            o += s;
        }
        panic!("no mdat");
    };
    assert_eq!(
        mdat_of(&out),
        mdat_of(&spliced),
        "the protected segment's mdat payload is unchanged"
    );

    // The written `moof` length must equal its declared size (the check that
    // used to fail).
    let parsed = MovieFragmentBox::parse_body(&moof_bytes[8..]).expect("re-parse moof");
    let mut re = vec![0u8; parsed.serialized_len()];
    let n = parsed.serialize_into(&mut re).expect("re-serialize");
    assert_eq!(&re[..n], moof_bytes, "moof round-trips byte-exactly");
}

/// A `traf` that already carries `senc`/`saiz`/`saio` must be rejected rather
/// than given a second set (audit item 3): §12.3 defines one `senc` per traf,
/// and a decryptor cannot choose between two.
#[test]
fn protect_media_segment_rejects_a_traf_that_already_carries_senc() {
    let Some(mut media) = clear_video_media() else {
        return;
    };
    let cfg = cenc_cfg(SubsamplePolicy::WholeSample);
    CencEncryptor::new(KEY)
        .encrypt(&mut media, &cfg)
        .expect("encrypt");
    let raw = CmafMux::new(1).package(&media).expect("CmafMux::package");
    let enc = media.tracks[0].encryption.as_ref().expect("encryption");
    let with_protected_init = protect_init_segment(&raw, 1, enc).expect("protect_init_segment");

    // A 16-byte `senc` FullBox declaring zero samples.
    let mut senc = Vec::new();
    senc.extend_from_slice(&16u32.to_be_bytes());
    senc.extend_from_slice(b"senc");
    senc.extend_from_slice(&0u32.to_be_bytes()); // version 0, flags 0
    senc.extend_from_slice(&0u32.to_be_bytes()); // sample_count
    let spliced = splice_into_traf(&with_protected_init, &senc);

    let fp = FragmentProtection {
        track_id: 1,
        entries: &enc.samples,
        per_sample_iv_size: enc.tenc.default_per_sample_iv_size,
    };
    let err = protect_media_segment(&spliced, &[fp])
        .expect_err("a traf with an existing senc must be rejected");
    assert!(
        matches!(err, transmux::Error::InvalidInput(_)),
        "expected InvalidInput, got {err:?}"
    );
}

/// The `trun.data_offset` shift must be checked: a fragment whose offset would
/// leave the 32-bit field is an error, not a wrap.
#[test]
fn protect_media_segment_rejects_a_data_offset_that_would_overflow() {
    let Some(mut media) = clear_video_media() else {
        return;
    };
    let cfg = cenc_cfg(SubsamplePolicy::WholeSample);
    CencEncryptor::new(KEY)
        .encrypt(&mut media, &cfg)
        .expect("encrypt");
    let raw = CmafMux::new(1).package(&media).expect("CmafMux::package");
    let enc = media.tracks[0].encryption.as_ref().expect("encryption");
    let with_protected_init = protect_init_segment(&raw, 1, enc).expect("protect_init_segment");

    // Push the `data_offset` to `i32::MAX` so the added `delta` overflows.
    // The `trun` is nested (`moof` → `traf` → `trun`), so scan every level.
    fn find_box(data: &[u8], fourcc: &[u8; 4], base: usize) -> Option<(usize, usize)> {
        let mut off = base;
        while off + 8 <= data.len() {
            let size = u32::from_be_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]])
                as usize;
            if size < 8 || off + size > data.len() {
                return None;
            }
            if &data[off + 4..off + 8] == fourcc {
                return Some((off, size));
            }
            if let Some(hit) = find_box(data, fourcc, off + 8) {
                return Some(hit);
            }
            off += size;
        }
        None
    }
    let mut buf = with_protected_init.clone();
    let (trun_at, _) = find_box(&buf, b"trun", 0).expect("a trun in the segment");
    let flags =
        u32::from_be_bytes([0, buf[trun_at + 9], buf[trun_at + 10], buf[trun_at + 11]]) & 0xFF_FFFF;
    assert!(flags & 1 != 0, "the fixture's trun carries data_offset");
    let at = trun_at + 8 + 4 + 4;
    buf[at..at + 4].copy_from_slice(&i32::MAX.to_be_bytes());

    let fp = FragmentProtection {
        track_id: 1,
        entries: &enc.samples,
        per_sample_iv_size: enc.tenc.default_per_sample_iv_size,
    };
    let err = protect_media_segment(&buf, &[fp])
        .expect_err("an overflowing data_offset must be rejected");
    assert!(
        matches!(err, transmux::Error::InvalidInput(_)),
        "expected InvalidInput, got {err:?}"
    );
}

/// End-to-end oracle for item 2: protect a segment that carries an opaque
/// moof-level child, then decrypt it back with this crate's own
/// `CencDecryptor` (whose §8.8.7/§8.8.8 addressing is pinned against the
/// `cenc_frag_layouts` fixtures) and check the samples equal the clear ones.
/// That catches a wrong `saio`, a wrong `data_offset` shift, and a dropped
/// `senc` — none of which a structural check alone would.
#[test]
fn protected_segment_with_an_opaque_moof_child_decrypts_to_the_clear_samples() {
    use transmux::{CencDecryptor, KeyMap};

    let Some(mut media) = clear_video_media() else {
        return;
    };
    let clear_samples: Vec<Vec<u8>> = media.tracks[0]
        .samples
        .iter()
        .map(|s| s.data.to_vec())
        .collect();

    let cfg = cenc_cfg(SubsamplePolicy::WholeSample);
    CencEncryptor::new(KEY)
        .encrypt(&mut media, &cfg)
        .expect("encrypt");
    let raw = CmafMux::new(1).package(&media).expect("CmafMux::package");
    let enc = media.tracks[0].encryption.as_ref().expect("encryption");
    let with_protected_init = protect_init_segment(&raw, 1, enc).expect("protect_init_segment");

    // A `pssh` as a moof-level opaque child.
    let mut pssh = Vec::new();
    pssh.extend_from_slice(&32u32.to_be_bytes());
    pssh.extend_from_slice(b"pssh");
    pssh.extend_from_slice(&[0, 0, 0, 0]);
    pssh.extend_from_slice(&KID);
    pssh.extend_from_slice(&0u32.to_be_bytes());
    let spliced = splice_into_moof(&with_protected_init, &pssh);

    let fp = FragmentProtection {
        track_id: 1,
        entries: &enc.samples,
        per_sample_iv_size: enc.tenc.default_per_sample_iv_size,
    };
    let protected = protect_media_segment(&spliced, &[fp]).expect("protect_media_segment");

    let mut keys = KeyMap::new();
    keys.insert(KID, KEY);
    let dec = CencDecryptor::from_fmp4(&protected)
        .expect("CencDecryptor::from_fmp4 on the protected segment");
    let mut demuxed = dec.demux().expect("CencDecryptor::demux");
    dec.decrypt(&mut demuxed, &keys)
        .expect("decrypt the protected segment");
    let got: Vec<Vec<u8>> = demuxed.tracks[0]
        .samples
        .iter()
        .map(|s| s.data.to_vec())
        .collect();
    assert_eq!(got.len(), clear_samples.len(), "sample count");
    assert_eq!(
        got, clear_samples,
        "decrypting the protected segment (with its moof-level pssh) must \
         reproduce the clear samples exactly"
    );
}

/// Independent oracle for item 2: Bento4 `mp4decrypt` must decrypt this
/// crate's protected segment (the one carrying a moof-level `pssh`) to the
/// same samples `CencDecryptor` produces — a decryptor sharing no code with
/// the `saio`/`data_offset` arithmetic under test. Skips loudly when the tool
/// is absent.
#[test]
fn protected_segment_with_an_opaque_moof_child_matches_mp4decrypt() {
    if std::process::Command::new("mp4decrypt")
        .arg("--version")
        .output()
        .is_err()
    {
        eprintln!(
            "SKIP protected_segment_with_an_opaque_moof_child_matches_mp4decrypt: mp4decrypt (Bento4) not on PATH - independent cross-check not run"
        );
        return;
    }
    use transmux::{CencDecryptor, KeyMap};

    let Some(mut media) = clear_video_media() else {
        return;
    };
    let cfg = cenc_cfg(SubsamplePolicy::WholeSample);
    CencEncryptor::new(KEY)
        .encrypt(&mut media, &cfg)
        .expect("encrypt");
    let raw = CmafMux::new(1).package(&media).expect("CmafMux::package");
    let enc = media.tracks[0].encryption.as_ref().expect("encryption");
    let with_protected_init = protect_init_segment(&raw, 1, enc).expect("protect_init_segment");

    let mut pssh = Vec::new();
    pssh.extend_from_slice(&32u32.to_be_bytes());
    pssh.extend_from_slice(b"pssh");
    pssh.extend_from_slice(&[0, 0, 0, 0]);
    pssh.extend_from_slice(&KID);
    pssh.extend_from_slice(&0u32.to_be_bytes());
    let spliced = splice_into_moof(&with_protected_init, &pssh);

    let fp = FragmentProtection {
        track_id: 1,
        entries: &enc.samples,
        per_sample_iv_size: enc.tenc.default_per_sample_iv_size,
    };
    let protected = protect_media_segment(&spliced, &[fp]).expect("protect_media_segment");

    // Ours.
    let dec = CencDecryptor::from_fmp4(&protected).expect("harvest");
    let mut ours = dec.demux().expect("demux");
    dec.decrypt(&mut ours, &{
        let mut k = KeyMap::new();
        k.insert(KID, KEY);
        k
    })
    .expect("decrypt");
    let our_samples: Vec<Vec<u8>> = ours.tracks[0]
        .samples
        .iter()
        .map(|s| s.data.to_vec())
        .collect();

    // mp4decrypt's.
    let dir = std::env::temp_dir().join(format!("transmux-h2-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let input = dir.join("protected.mp4");
    let output = dir.join("decrypted.mp4");
    std::fs::write(&input, &protected).expect("write");
    let hex = |b: &[u8; 16]| b.iter().map(|x| format!("{x:02x}")).collect::<String>();
    let status = std::process::Command::new("mp4decrypt")
        .arg("--key")
        .arg(format!("{}:{}", hex(&KID), hex(&KEY)))
        .arg(&input)
        .arg(&output)
        .status()
        .expect("spawn mp4decrypt");
    assert!(status.success(), "mp4decrypt failed");
    let bytes = std::fs::read(&output).expect("read mp4decrypt output");
    let _ = std::fs::remove_dir_all(&dir);

    let ref_media = transmux::Fmp4Demux::new()
        .unpackage(bytes.as_slice())
        .expect("demux mp4decrypt output");
    let ref_samples: Vec<Vec<u8>> = ref_media.tracks[0]
        .samples
        .iter()
        .map(|s| s.data.to_vec())
        .collect();
    assert_eq!(
        our_samples, ref_samples,
        "our decryption of the protected segment must equal mp4decrypt's"
    );
}

// ---------------------------------------------------------------------------
// Round-3 item 7: several trafs, opaque children after them, aux_info_type
// ---------------------------------------------------------------------------

/// A protected segment with **two** tracks (two `traf`s) and an opaque child
/// placed after the last `traf` must still be rewritten correctly: the rebuilt
/// `moof` length, each `saio`'s moof-relative offset, and every
/// `trun.data_offset` shift all depend on the wire order. Decrypting the result
/// with this crate's own `CencDecryptor` (whose addressing is pinned against
/// the `cenc_frag_layouts` fixtures) must reproduce the clear samples.
#[test]
fn two_trafs_and_a_trailing_opaque_child_decrypt_to_the_clear_samples() {
    use transmux::{CencDecryptor, KeyMap};

    // AVC + AAC: `fixtures/ts/h264/main.ts` is video-only, so this test needs
    // the A/V capture.
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/ts/h264_aac.ts");
    if !path.exists() {
        eprintln!("cenc_mux tests: SKIPPED — {path:?} not found.");
        return;
    }
    let bytes = std::fs::read(&path).expect("read fixture");
    let media = TsDemux::new()
        .unpackage(bytes.as_slice())
        .expect("demux h264_aac.ts");
    // AVC video + AAC audio: two tracks, so the moof carries two trafs.
    let mut media = media
        .select_tracks_by(|t| {
            matches!(t.spec.config, CodecConfig::Avc { .. })
                || matches!(t.spec.config, CodecConfig::Aac { .. })
        })
        .expect("AVC + AAC tracks present");
    assert_eq!(media.tracks.len(), 2, "two tracks selected");

    let clear_samples: Vec<Vec<Vec<u8>>> = media
        .tracks
        .iter()
        .map(|t| t.samples.iter().map(|s| s.data.to_vec()).collect())
        .collect();

    let cfg = cenc_cfg(SubsamplePolicy::WholeSample);
    CencEncryptor::new(KEY)
        .encrypt(&mut media, &cfg)
        .expect("encrypt");
    let raw = CmafMux::new(1).package(&media).expect("CmafMux::package");

    let mut with_init = raw.clone();
    for track in &media.tracks {
        let enc = track.encryption.as_ref().expect("encryption");
        with_init = protect_init_segment(&with_init, track.spec.track_id, enc)
            .expect("protect_init_segment");
    }

    // A `pssh` appended *after* the last `traf` (the wire-order case the
    // running `saio` offset and moof length must both survive).
    let mut pssh = Vec::new();
    pssh.extend_from_slice(&32u32.to_be_bytes());
    pssh.extend_from_slice(b"pssh");
    pssh.extend_from_slice(&[0, 0, 0, 0]);
    pssh.extend_from_slice(&KID);
    pssh.extend_from_slice(&0u32.to_be_bytes());
    let spliced = splice_into_moof(&with_init, &pssh);

    let protections: Vec<FragmentProtection<'_>> = media
        .tracks
        .iter()
        .map(|t| {
            let enc = t.encryption.as_ref().expect("encryption");
            FragmentProtection {
                track_id: t.spec.track_id,
                entries: &enc.samples,
                per_sample_iv_size: enc.tenc.default_per_sample_iv_size,
            }
        })
        .collect();
    let protected = protect_media_segment(&spliced, &protections).expect("protect_media_segment");

    let mut keys = KeyMap::new();
    keys.insert(KID, KEY);
    let dec = CencDecryptor::from_fmp4(&protected).expect("harvest");
    let mut out = dec.demux().expect("demux");
    dec.decrypt(&mut out, &keys).expect("decrypt");

    // `CencDecryptor` reconstructs the AVC track; the point of this test is
    // that the *rewrite* kept both `traf`s addressable and the appended
    // `pssh` in place, so the video track must decrypt to its clear samples
    // even though a second `traf` (and an opaque child after it) shifted every
    // moof-relative offset.
    let video = out
        .tracks
        .iter()
        .position(|t| matches!(t.spec.config, CodecConfig::Avc { .. }))
        .expect("the decrypted video track");
    let got: Vec<Vec<u8>> = out.tracks[video]
        .samples
        .iter()
        .map(|s| s.data.to_vec())
        .collect();
    assert_eq!(
        got, clear_samples[0],
        "the video track must decrypt to its clear samples"
    );
    assert!(
        protected.windows(4).any(|w| w == b"pssh"),
        "the appended pssh survives"
    );
}

/// A `saiz`/`saio` pair typed for a *different* scheme is not CENC's and must
/// not make `protect_media_segment` refuse the segment (audit item 7). A CENC
/// one still must.
#[test]
fn non_cenc_typed_saiz_is_not_treated_as_an_existing_cenc_triple() {
    use transmux::cenc::{SampleAuxInfoOffsetsBox, SampleAuxInfoSizesBox};

    // `saiz` v0, aux_info_type present (flag 0x1) and *not* a CENC four-CC.
    let saiz = SampleAuxInfoSizesBox {
        version: 0,
        flags: 0x01,
        aux_info_type: Some(u32::from_be_bytes(*b"rocb")),
        aux_info_type_parameter: Some(1),
        default_sample_info_size: 0,
        sample_count: 0,
        sample_info_sizes: Vec::new(),
    };
    let saio = SampleAuxInfoOffsetsBox {
        version: 0,
        flags: 0x01,
        aux_info_type: Some(u32::from_be_bytes(*b"rocb")),
        aux_info_type_parameter: Some(1),
        offsets: Vec::new(),
    };
    let mut extra = Vec::new();
    extra.extend_from_slice(&saiz.to_bytes());
    extra.extend_from_slice(&saio.to_bytes());
    // And a CENC-typed pair for the contrast.
    let cenc_saiz = SampleAuxInfoSizesBox {
        aux_info_type: Some(u32::from_be_bytes(*b"cenc")),
        ..saiz
    };
    let mut cenc_extra = Vec::new();
    cenc_extra.extend_from_slice(&cenc_saiz.to_bytes());

    let Some(mut media) = clear_video_media() else {
        return;
    };
    let cfg = cenc_cfg(SubsamplePolicy::WholeSample);
    CencEncryptor::new(KEY)
        .encrypt(&mut media, &cfg)
        .expect("encrypt");
    let raw = CmafMux::new(1).package(&media).expect("CmafMux::package");
    let enc = media.tracks[0].encryption.as_ref().expect("encryption");
    let with_init = protect_init_segment(&raw, 1, enc).expect("protect_init_segment");
    let fp = || FragmentProtection {
        track_id: 1,
        entries: &enc.samples,
        per_sample_iv_size: enc.tenc.default_per_sample_iv_size,
    };

    // A non-CENC pair is preserved and the segment is still protected.
    let with_non_cenc = splice_into_traf(&with_init, &extra);
    let out = protect_media_segment(&with_non_cenc, &[fp()])
        .expect("a non-CENC saiz/saio must not block protection");
    assert!(
        out.windows(4).any(|w| w == b"rocb"),
        "the non-CENC pair survives the rewrite"
    );

    // A CENC-typed one still collides.
    let with_cenc = splice_into_traf(&with_init, &cenc_extra);
    let err =
        protect_media_segment(&with_cenc, &[fp()]).expect_err("a CENC saiz must still be refused");
    assert!(
        matches!(err, transmux::Error::InvalidInput(_)),
        "expected InvalidInput, got {err:?}"
    );
}
