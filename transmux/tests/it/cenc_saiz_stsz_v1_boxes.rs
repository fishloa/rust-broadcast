//! Real-fixture, independent-oracle tests for the W4 box-level spec-misread
//! fixes (issue #1131 theme T3): `saiz`/`stsz` uniform-size `sample_count`
//! collapse (#1013, #1018), `enca` re-serialized as `mp4a` (#1017), and v1
//! `mvhd`/`tkhd` wrong size/offsets (#1015, #1016).
//!
//! Every fixture here is generated with an independent tool (ffmpeg and/or
//! GPAC's MP4Box — see `../fixtures/mp4/cenc-boxes/PROVENANCE.md` for the exact
//! commands) and the expected values are recorded from that tool's own
//! inspector (`MP4Box -diso`), never derived from this crate's own
//! serializer.

use broadcast_common::timescale::Timescale;
use broadcast_common::{Parse, Serialize};
use isobmff::TrackId;
use isobmff::cenc::SampleAuxInfoSizesBox;
use isobmff::init_segment::{MovieHeaderBox, Mp4aSampleEntry, SampleSizeBox, TrackHeaderBox};

fn timescale(value: u32) -> Timescale {
    Timescale::try_new(value).expect("a non-zero timescale")
}

fn fixture(name: &str) -> Vec<u8> {
    let path = test_fixtures::path(&format!("mp4/cenc-boxes/{name}"));
    std::fs::read(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

/// Find the first (full, including its 8-byte header) box of the given
/// four-CC anywhere in `data`. Sufficient here: each four-CC of interest
/// appears exactly once as a real box tag in these small fixtures, and an
/// ASCII four-CC recurring by chance inside compressed audio/video payload
/// bytes is not a realistic risk (the same approach `tests/it/cenc.rs` uses).
fn find_box<'a>(data: &'a [u8], fourcc: &[u8; 4]) -> &'a [u8] {
    let pos = data
        .windows(4)
        .position(|w| w == fourcc)
        .unwrap_or_else(|| {
            panic!(
                "{} four-CC must be present",
                std::str::from_utf8(fourcc).unwrap()
            )
        });
    let start = pos - 4;
    let size = u32::from_be_bytes([
        data[start],
        data[start + 1],
        data[start + 2],
        data[start + 3],
    ]) as usize;
    &data[start..start + size]
}

/// #1013 / #1017: a real MP4Box CENC AES-CTR (`cenc`) encrypted AAC track —
/// whole-sample protected, 8-byte per-sample IV, no subsamples, the common
/// case `build_cenc_fragment_boxes` picks the uniform `saiz` form for.
///
/// Oracle (`MP4Box -diso`, see the fixture README):
/// `<SampleAuxiliaryInfoSizeBox ... default_sample_info_size="8" sample_count="88">`
/// and `<SampleEncryptionBox ... sampleCount="88">`.
#[test]
fn saiz_uniform_sample_count_matches_mp4box_diso() {
    let data = fixture("aac-cenc.mp4");
    let saiz_bytes = find_box(&data, b"saiz");

    let saiz = SampleAuxInfoSizesBox::parse_box(saiz_bytes).expect("parse saiz");
    assert_eq!(saiz.version, 0);
    assert_eq!(
        saiz.default_sample_info_size, 8,
        "uniform aux-info size == the tenc per-sample IV size (8)"
    );
    assert!(
        saiz.sample_info_sizes.is_empty(),
        "uniform form: no per-sample size table"
    );
    assert_eq!(
        saiz.sample_count, 88,
        "sample_count must survive the uniform (default_sample_info_size != 0) form \
         — this collapsed to 0 pre-fix (issue #1013)"
    );

    // Byte-exact round trip against the real file bytes.
    let mut out = vec![0u8; saiz.serialized_len()];
    let n = saiz.serialize_into(&mut out).unwrap();
    assert_eq!(&out[..n], saiz_bytes, "saiz round-trip byte-exact");
}

/// #1017: the `enca` sample entry (CENC-protected AAC) must keep its `enca`
/// four-CC through parse -> serialize, not silently become `mp4a`.
///
/// Oracle (`MP4Box -diso`): `<AudioSampleDescriptionBox ... Type="enca" ...>`.
#[test]
fn enca_sample_entry_keeps_its_four_cc() {
    let data = fixture("aac-cenc.mp4");
    let enca_bytes = find_box(&data, b"enca");

    let entry = Mp4aSampleEntry::parse(enca_bytes).expect("parse enca sample entry");
    assert_eq!(
        &entry.codec_type, b"enca",
        "parsed codec_type must record enca, not the mp4a base type"
    );
    // A real sinf child must have parsed through as an opaque config box.
    assert!(
        entry.config_boxes.iter().any(|b| &b.box_type == b"sinf"),
        "enca entry must carry its sinf child"
    );

    let mut out = vec![0u8; entry.serialized_len()];
    let n = entry.serialize_into(&mut out).unwrap();
    assert_eq!(
        &out[..n],
        enca_bytes,
        "enca sample entry round-trip byte-exact (four-CC included)"
    );
}

/// #1018: a real, uncompressed-PCM MP4 (`ipcm`) where every sample is
/// exactly 2 bytes (16-bit mono @ 8 kHz) — the genuinely constant-size case
/// `stsz`'s uniform `sample_size` field exists for.
///
/// Oracle (`MP4Box -diso`): `<SampleSizeBox ... SampleCount="8000" ConstantSampleSize="2">`.
#[test]
fn stsz_uniform_sample_count_matches_mp4box_diso() {
    let data = fixture("pcm-clear.mp4");
    let stsz_bytes = find_box(&data, b"stsz");

    let stsz = SampleSizeBox::parse(stsz_bytes).expect("parse stsz");
    assert_eq!(stsz.version, 0);
    assert_eq!(
        stsz.sample_size, 2,
        "constant sample size (16-bit mono PCM)"
    );
    assert!(
        stsz.entries.is_empty(),
        "uniform form: no per-sample size table"
    );
    assert_eq!(
        stsz.sample_count, 8000,
        "sample_count must survive the uniform (sample_size != 0) form \
         — this collapsed to 0 pre-fix (issue #1018)"
    );

    let mut out = vec![0u8; stsz.serialized_len()];
    let n = stsz.serialize_into(&mut out).unwrap();
    assert_eq!(&out[..n], stsz_bytes, "stsz round-trip byte-exact");
}

/// #1015: a real v1 `mvhd` (movie timescale forced to 2 GHz so the 3-second
/// duration in ticks exceeds `u32::MAX`) must parse at its real 120-byte
/// size with `next_track_id` at the right offset, and round-trip
/// byte-exact — not the previously-hard-coded (wrong) 124.
///
/// Oracle (`MP4Box -diso`):
/// `<MovieHeaderBox Size="120" Version="1" TimeScale="2000000000" Duration="6000000000" NextTrackID="2">`.
#[test]
fn mvhd_v1_matches_mp4box_diso() {
    let data = fixture("v1-mvhd.mp4");
    let mvhd_bytes = find_box(&data, b"mvhd");
    assert_eq!(mvhd_bytes.len(), 120, "real v1 mvhd is 120 bytes, not 124");

    let mvhd = MovieHeaderBox::parse(mvhd_bytes).expect("parse v1 mvhd");
    assert_eq!(mvhd.version, 1);
    assert_eq!(mvhd.timescale, timescale(2_000_000_000));
    assert_eq!(mvhd.duration, 6_000_000_000);
    assert_eq!(
        mvhd.next_track_id,
        TrackId::new(2),
        "next_track_id must be read from byte 116, not 120 (issue #1015)"
    );

    let mut out = vec![0u8; mvhd.serialized_len()];
    let n = mvhd.serialize_into(&mut out).unwrap();
    assert_eq!(n, 120);
    assert_eq!(&out[..n], mvhd_bytes, "v1 mvhd round-trip byte-exact");
}

/// #1016: the matching real v1 `tkhd` from the same fixture — track_ID(32)
/// and a single 32-bit reserved field (not two) precede `duration`, so
/// `duration` starts at byte 36, not 40; every field after it was
/// previously read 4 bytes early.
///
/// Oracle (`MP4Box -diso`):
/// `<TrackHeaderBox Size="104" Version="1" Duration="6000000000" Width="64.00" Height="64.00">`.
#[test]
fn tkhd_v1_matches_mp4box_diso() {
    let data = fixture("v1-mvhd.mp4");
    let tkhd_bytes = find_box(&data, b"tkhd");
    assert_eq!(tkhd_bytes.len(), 104);

    let tkhd = TrackHeaderBox::parse(tkhd_bytes).expect("parse v1 tkhd");
    assert_eq!(tkhd.version, 1);
    assert_eq!(tkhd.track_id, TrackId::new(1));
    assert_eq!(
        tkhd.duration, 6_000_000_000,
        "duration must be read from byte 36, not 40 (issue #1016)"
    );
    assert_eq!(tkhd.width, 0x0040_0000, "64.00 in 16.16 fixed point");
    assert_eq!(tkhd.height, 0x0040_0000);

    let mut out = vec![0u8; tkhd.serialized_len()];
    let n = tkhd.serialize_into(&mut out).unwrap();
    assert_eq!(n, 104);
    assert_eq!(&out[..n], tkhd_bytes, "v1 tkhd round-trip byte-exact");
}

/// `saio.offset[0]` is the moof-relative position of the matching `senc`'s
/// first entry (ISO/IEC 23001-7 §12.3): it has to count every byte written
/// before that `traf`, a moof-level opaque child and a preceding `traf`
/// included, or a decryptor reads its aux info from the wrong bytes.
#[test]
fn protect_media_segment_anchors_each_saio_at_its_own_senc() {
    use isobmff::cenc::SampleAuxInfoOffsetsBox;
    use transmux::fragment_build::{FragmentProtection, protect_media_segment};

    fn boxed(fourcc: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut b = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        b.extend_from_slice(fourcc);
        b.extend_from_slice(body);
        b
    }
    fn traf(track_id: u32) -> Vec<u8> {
        let mut body = Vec::new();
        body.extend_from_slice(&boxed(b"tfhd", &[0, 0, 0, 0, 0, 0, 0, track_id as u8]));
        body.extend_from_slice(&boxed(b"trun", &[0, 0, 0, 0, 0, 0, 0, 1]));
        boxed(b"traf", &body)
    }

    let mut moof_body = Vec::new();
    moof_body.extend_from_slice(&boxed(b"mfhd", &[0, 0, 0, 0, 0, 0, 0, 1]));
    // A moof-level opaque child between the `mfhd` and the first `traf`.
    moof_body.extend_from_slice(&boxed(b"free", &[0xAA; 12]));
    moof_body.extend_from_slice(&traf(1));
    moof_body.extend_from_slice(&traf(2));
    let moof = boxed(b"moof", &moof_body);

    let entries = |iv: u8| {
        vec![isobmff::cenc::SampleEncryptionEntry {
            initialization_vector: vec![iv; 8],
            subsamples: Vec::new(),
        }]
    };
    let a = entries(0x11);
    let b = entries(0x22);
    let protections = [
        FragmentProtection {
            track_id: TrackId::new(1).into(),
            entries: &a,
            per_sample_iv_size: 8,
        },
        FragmentProtection {
            track_id: TrackId::new(2).into(),
            entries: &b,
            per_sample_iv_size: 8,
        },
    ];
    let out = protect_media_segment(&moof, &protections).expect("protect both trafs");
    assert_eq!(out[4..8], *b"moof");

    // Walk the rebuilt moof's children by hand so the expected offsets come
    // from where the bytes actually are, not from the code under test.
    let body = &out[8..];
    let mut senc_at = Vec::new();
    let mut saio = Vec::new();
    let mut off = 0usize;
    for step in isobmff::box_types::box_iter(body) {
        let (child, consumed) = step.expect("walk the moof");
        if child.header.box_type.is(b"traf") {
            let traf_body_at = 8 + off + 8;
            let mut inner = 0usize;
            for step in isobmff::box_types::box_iter(child.body) {
                let (grandchild, size) = step.expect("walk the traf");
                if grandchild.header.box_type.is(b"senc") {
                    senc_at.push(traf_body_at + inner);
                } else if grandchild.header.box_type.is(b"saio") {
                    let at = traf_body_at + inner;
                    saio.push(SampleAuxInfoOffsetsBox::parse_box(&out[at..at + size]).unwrap());
                }
                inner += size;
            }
        }
        off += consumed;
    }
    assert_eq!(senc_at.len(), 2, "one senc per protected traf");
    assert_eq!(saio.len(), 2, "one saio per protected traf");
    for (i, saio) in saio.iter().enumerate() {
        assert_eq!(saio.offsets.len(), 1);
        assert_eq!(
            saio.offsets[0],
            (senc_at[i] + 16) as u64,
            "traf {} must anchor its saio at its own senc's first entry",
            i + 1
        );
    }
    assert!(
        senc_at[0] > 36,
        "the mfhd and the moof-level free child precede the first senc"
    );
}
