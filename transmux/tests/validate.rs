//! Biting tests for the fMP4/CMAF conformance validator (issue #481).
//!
//! Each Error check gets BOTH a positive (clean → no such issue) and a negative
//! (broken → the exact `code`) assertion, so it cannot be satisfied by an
//! always-pass or always-fail validator. Clean segments are built from the real
//! codec configs + real coded samples of `h264_aac_frag.mp4` via the crate's own
//! `build_init_segment` / `build_media_segment`.

use broadcast_common::Parse;
use transmux::{
    CodecConfig, FragmentTrackData, MovieBox, MovieFragmentBox, Sample, SampleEntryVariant,
    Severity, StblChild, TrackSpec, build_init_segment, build_media_segment, validate_cmaf_track,
    validate_init_segment, validate_media_segment,
};

// ---------------------------------------------------------------------------
// Fixture plumbing (mirrors tests/pipeline.rs)
// ---------------------------------------------------------------------------

fn fixture() -> Vec<u8> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../fixtures/transmux/h264_aac_frag.mp4"
    );
    std::fs::read(path).expect("fixture file must exist")
}

fn find_top_box(data: &[u8], fourcc: &[u8; 4]) -> (usize, Vec<u8>) {
    let mut off = 0usize;
    while off + 8 <= data.len() {
        let size =
            u32::from_be_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]]) as usize;
        if size < 8 {
            break;
        }
        if &data[off + 4..off + 8] == fourcc {
            return (off, data[off..off + size].to_vec());
        }
        off += size;
    }
    panic!("box {:?} not found", std::str::from_utf8(fourcc).unwrap());
}

fn track_stsd_entry(moov: &MovieBox, track_idx: usize) -> SampleEntryVariant {
    let stbl = moov.tracks[track_idx]
        .mdia
        .as_ref()
        .unwrap()
        .minf
        .as_ref()
        .unwrap()
        .stbl
        .as_ref()
        .unwrap();
    stbl.children
        .iter()
        .find_map(|c| match c {
            StblChild::Stsd(s) => Some(s),
            _ => None,
        })
        .unwrap()
        .entries[0]
        .clone()
}

fn track_timescale(moov: &MovieBox, track_idx: usize) -> u32 {
    moov.tracks[track_idx]
        .mdia
        .as_ref()
        .unwrap()
        .mdhd
        .as_ref()
        .unwrap()
        .timescale
}

/// Build a single-video-track spec from the fixture's real avcC.
fn video_spec(moov: &MovieBox) -> TrackSpec {
    let avc = match track_stsd_entry(moov, 0) {
        SampleEntryVariant::Avc1(a) => a,
        _ => panic!("expected avc1"),
    };
    TrackSpec::new(
        1,
        track_timescale(moov, 0),
        CodecConfig::Avc {
            config: avc.config.clone(),
            width: avc.visual.width,
            height: avc.visual.height,
        },
    )
}

/// Extract the first moof's video samples (track index 0).
fn video_samples(data: &[u8]) -> Vec<Sample> {
    let (moof_off, moof_bytes) = find_top_box(data, b"moof");
    let moof = MovieFragmentBox::parse_body(&moof_bytes[8..]).expect("moof");
    let traf = &moof.traf[0];
    let trun = &traf.trun[0];
    let base = moof_off + trun.data_offset.expect("data_offset") as usize;
    let mut samples = Vec::new();
    let mut cursor = base;
    for (i, ts) in trun.samples.iter().enumerate() {
        let size = ts.sample_size.expect("sample_size") as usize;
        samples.push(Sample::new(
            data[cursor..cursor + size].to_vec(),
            ts.sample_duration.unwrap_or(3000),
            i == 0,
            ts.sample_composition_time_offset.unwrap_or(0),
        ));
        cursor += size;
    }
    samples
}

/// A clean init segment (single video track) built from the fixture.
fn clean_init() -> Vec<u8> {
    let data = fixture();
    let (_, moov_bytes) = find_top_box(&data, b"moov");
    let moov = MovieBox::parse(&moov_bytes).unwrap();
    build_init_segment(&[video_spec(&moov)], 1000).expect("build init")
}

/// A clean media segment with the given sequence_number + base decode time,
/// carrying the real video samples.
fn clean_media(seq: u32, base_time: u64, samples: &[Sample]) -> Vec<u8> {
    build_media_segment(
        seq,
        &[FragmentTrackData {
            track_id: 1,
            base_media_decode_time: base_time,
            samples,
        }],
    )
    .expect("build media")
}

// ---------------------------------------------------------------------------
// Box-tree mutation helpers (test-only)
// ---------------------------------------------------------------------------

/// Walk top-level boxes, returning (offset, size, fourcc) for each.
fn top_boxes(data: &[u8]) -> Vec<(usize, usize, [u8; 4])> {
    let mut out = Vec::new();
    let mut off = 0usize;
    while off + 8 <= data.len() {
        let size =
            u32::from_be_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]]) as usize;
        if size < 8 || off + size > data.len() {
            break;
        }
        out.push((
            off,
            size,
            [data[off + 4], data[off + 5], data[off + 6], data[off + 7]],
        ));
        off += size;
    }
    out
}

const CONTAINERS: &[&[u8; 4]] = &[
    b"moov", b"trak", b"mdia", b"minf", b"stbl", b"mvex", b"moof", b"traf", b"edts", b"dinf",
];

/// Remove the first descendant box of `fourcc` found anywhere inside `data`
/// (recursive descent through container boxes), shrinking every enclosing box's
/// size field. Returns the mutated buffer. Simple byte-splice; sufficient for
/// test stimuli.
fn strip_box(data: &[u8], fourcc: &[u8; 4]) -> Vec<u8> {
    // Locate the box in the ORIGINAL coordinate space.
    let (start, size) = find_box_range(data, 0, data.len(), fourcc).expect("box to strip present");
    // Copy, then decrement the size of every ancestor whose original span
    // contained [start, start+size), before splicing the bytes out.
    let mut out = data.to_vec();
    shrink_ancestors(&mut out, 0, data.len(), start, size);
    out.drain(start..start + size);
    out
}

/// Recursively locate a box of `fourcc` within [lo, hi). Returns (abs_offset,
/// box_size). Descends into known container boxes.
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
        if CONTAINERS.contains(&&t) {
            if let Some(found) = find_box_range(data, off + 8, off + size, fourcc) {
                return Some(found);
            }
        }
        off += size;
    }
    None
}

/// Decrement (in place, ORIGINAL coordinate space) the 32-bit size of every box
/// in [lo, hi) whose span contains the region [cut_start, cut_start+cut_len).
fn shrink_ancestors(data: &mut [u8], lo: usize, hi: usize, cut_start: usize, cut_len: usize) {
    let mut off = lo;
    while off + 8 <= hi {
        let size =
            u32::from_be_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]]) as usize;
        if size < 8 || off + size > hi {
            break;
        }
        let t = [data[off + 4], data[off + 5], data[off + 6], data[off + 7]];
        // Ancestor if it strictly contains the cut (but is not the cut box).
        if off < cut_start && cut_start + cut_len <= off + size {
            let new_size = (size - cut_len) as u32;
            data[off..off + 4].copy_from_slice(&new_size.to_be_bytes());
            if CONTAINERS.contains(&&t) {
                shrink_ancestors(data, off + 8, off + size, cut_start, cut_len);
            }
        }
        off += size;
    }
}

/// Rename the first descendant box of `from` to `to` (same-length 4-CC),
/// leaving every size unchanged. Used to "remove" a box structurally (the
/// validator no longer sees it) without disturbing sibling offsets — e.g.
/// turning `tfdt` into `free` so the trun `data_offset` arithmetic still holds.
fn rename_box(data: &[u8], from: &[u8; 4], to: &[u8; 4]) -> Vec<u8> {
    let (start, _) = find_box_range(data, 0, data.len(), from).expect("box to rename present");
    let mut out = data.to_vec();
    out[start + 4..start + 8].copy_from_slice(to);
    out
}

fn has_code(issues: &[transmux::ConformanceIssue], code: &str) -> bool {
    issues.iter().any(|i| i.code == code)
}

fn errors(issues: &[transmux::ConformanceIssue]) -> Vec<&str> {
    issues
        .iter()
        .filter(|i| i.severity == Severity::Error)
        .map(|i| i.code)
        .collect()
}

// ---------------------------------------------------------------------------
// 1. Clean segments pass (no Errors)
// ---------------------------------------------------------------------------

#[test]
fn clean_init_has_no_errors() {
    let init = clean_init();
    let issues = validate_init_segment(&init);
    assert!(
        errors(&issues).is_empty(),
        "clean init flagged errors: {:?}",
        errors(&issues)
    );
}

#[test]
fn clean_media_has_no_errors() {
    let data = fixture();
    let samples = video_samples(&data);
    let media = clean_media(1, 0, &samples);
    let issues = validate_media_segment(&media);
    assert!(
        errors(&issues).is_empty(),
        "clean media flagged errors: {:?}",
        errors(&issues)
    );
}

#[test]
fn clean_track_has_no_errors() {
    let data = fixture();
    let samples = video_samples(&data);
    let init = clean_init();
    let total: u64 = samples.iter().map(|s| s.duration as u64).sum();
    let seg1 = clean_media(1, 0, &samples);
    let seg2 = clean_media(2, total, &samples);
    let issues = validate_cmaf_track(&init, &[&seg1, &seg2]);
    assert!(
        errors(&issues).is_empty(),
        "clean track flagged errors: {:?}",
        errors(&issues)
    );
}

// ---------------------------------------------------------------------------
// 2. Each Error check bites (positive + negative)
// ---------------------------------------------------------------------------

#[test]
fn tfdt_missing_bites() {
    let data = fixture();
    let samples = video_samples(&data);
    let media = clean_media(1, 0, &samples);

    // positive: clean media → no tfdt.missing.
    assert!(!has_code(
        &validate_media_segment(&media),
        "media.tfdt.missing"
    ));

    // negative: turn the tfdt into a free box (same size → offsets intact) so
    // ONLY the missing-tfdt error fires, not a flood of size-driven ones.
    let broken = rename_box(&media, b"tfdt", b"free");
    let issues = validate_media_segment(&broken);
    assert!(
        has_code(&issues, "media.tfdt.missing"),
        "expected media.tfdt.missing, got {:?}",
        errors(&issues)
    );
    // Structural sanity: no unrelated moof/mfhd/traf/trun/mdat errors.
    let errs = errors(&issues);
    for bad in [
        "media.moof.missing",
        "media.mfhd.missing",
        "media.traf.missing",
        "media.trun.missing",
        "media.mdat.missing",
        "media.mdat.overrun",
    ] {
        assert!(!errs.contains(&bad), "unexpected {bad} in {errs:?}");
    }
}

#[test]
fn mdat_overrun_bites() {
    let data = fixture();
    let samples = video_samples(&data);
    let media = clean_media(1, 0, &samples);

    // positive: intact mdat → no overrun.
    assert!(!has_code(
        &validate_media_segment(&media),
        "media.mdat.overrun"
    ));

    // negative: truncate the mdat payload so trun sizes overrun it.
    // Shrink the mdat box size field and drop trailing bytes.
    let boxes = top_boxes(&media);
    let (mdat_off, mdat_size, _) = *boxes.iter().find(|(_, _, t)| t == b"mdat").unwrap();
    let cut = mdat_size / 2; // remove half the payload
    let new_mdat_size = (mdat_size - cut) as u32;
    let mut broken = Vec::new();
    broken.extend_from_slice(&media[..mdat_off]);
    broken.extend_from_slice(&new_mdat_size.to_be_bytes());
    broken.extend_from_slice(&media[mdat_off + 4..mdat_off + 8]); // "mdat"
    broken.extend_from_slice(&media[mdat_off + 8..mdat_off + mdat_size - cut]);
    // (anything after mdat is dropped)

    let issues = validate_media_segment(&broken);
    assert!(
        has_code(&issues, "media.mdat.overrun"),
        "expected media.mdat.overrun, got {:?}",
        errors(&issues)
    );
}

#[test]
fn moof_without_mdat_bites() {
    let data = fixture();
    let samples = video_samples(&data);
    let media = clean_media(1, 0, &samples);

    // positive: moof+mdat present → no pairing error.
    let clean = validate_media_segment(&media);
    assert!(!has_code(&clean, "media.mdat.missing"));
    assert!(!has_code(&clean, "media.mdat.orphan"));

    // negative: drop the trailing mdat entirely (styp + moof only).
    let boxes = top_boxes(&media);
    let (mdat_off, _, _) = *boxes.iter().find(|(_, _, t)| t == b"mdat").unwrap();
    let broken = media[..mdat_off].to_vec();

    let issues = validate_media_segment(&broken);
    assert!(
        has_code(&issues, "media.mdat.missing"),
        "expected media.mdat.missing, got {:?}",
        errors(&issues)
    );
}

#[test]
fn mdat_orphan_bites() {
    // A bare mdat with no preceding moof.
    let mut buf = Vec::new();
    // styp
    let styp: [u8; 4] = *b"styp";
    buf.extend_from_slice(&24u32.to_be_bytes());
    buf.extend_from_slice(&styp);
    buf.extend_from_slice(b"msdh");
    buf.extend_from_slice(&0u32.to_be_bytes());
    buf.extend_from_slice(b"msix");
    buf.extend_from_slice(b"msdh");
    // mdat with no moof before it
    buf.extend_from_slice(&12u32.to_be_bytes());
    buf.extend_from_slice(b"mdat");
    buf.extend_from_slice(&[0xAA, 0xBB, 0xCC, 0xDD]);

    let issues = validate_media_segment(&buf);
    assert!(
        has_code(&issues, "media.mdat.orphan"),
        "expected media.mdat.orphan, got {:?}",
        errors(&issues)
    );
}

#[test]
fn tfdt_continuity_bites() {
    let data = fixture();
    let samples = video_samples(&data);
    let total: u64 = samples.iter().map(|s| s.duration as u64).sum();
    let init = clean_init();

    // positive: two contiguous segments → no continuity error.
    let seg1 = clean_media(1, 0, &samples);
    let seg2_ok = clean_media(2, total, &samples);
    let ok = validate_cmaf_track(&init, &[&seg1, &seg2_ok]);
    assert!(
        !has_code(&ok, "track.tfdt.discontinuity"),
        "contiguous segments must not flag discontinuity: {:?}",
        errors(&ok)
    );

    // negative: segment 2 tfdt < segment 1 (rewinds) → continuity error.
    let seg2_bad = clean_media(2, 0, &samples); // same base time as seg1
    let bad = validate_cmaf_track(&init, &[&seg1, &seg2_bad]);
    assert!(
        has_code(&bad, "track.tfdt.discontinuity"),
        "expected track.tfdt.discontinuity, got {:?}",
        errors(&bad)
    );
}

#[test]
fn mfhd_sequence_bites() {
    let data = fixture();
    let samples = video_samples(&data);
    let total: u64 = samples.iter().map(|s| s.duration as u64).sum();
    let init = clean_init();

    let seg1 = clean_media(5, 0, &samples);
    // Second segment with a NON-increasing sequence number (same as first).
    let seg2 = clean_media(5, total, &samples);
    let issues = validate_cmaf_track(&init, &[&seg1, &seg2]);
    assert!(
        issues
            .iter()
            .any(|i| i.code == "track.mfhd.sequence" && i.severity == Severity::Warning),
        "expected track.mfhd.sequence warning, got {issues:?}"
    );
}

#[test]
fn zero_duration_sample_bites() {
    let data = fixture();
    let mut samples = video_samples(&data);
    // positive path already covered by clean_media_has_no_errors.
    // negative: force a zero-duration sample.
    samples[0].duration = 0;
    let media = clean_media(1, 0, &samples);
    let issues = validate_media_segment(&media);
    assert!(
        has_code(&issues, "media.sample.zero-duration"),
        "expected media.sample.zero-duration, got {:?}",
        errors(&issues)
    );
}

// ---------------------------------------------------------------------------
// Init-segment error checks bite
// ---------------------------------------------------------------------------

#[test]
fn init_missing_moov_bites() {
    let init = clean_init();
    // positive
    assert!(!has_code(
        &validate_init_segment(&init),
        "init.moov.missing"
    ));
    // negative: strip moov.
    let broken = strip_box(&init, b"moov");
    let issues = validate_init_segment(&broken);
    assert!(
        has_code(&issues, "init.moov.missing"),
        "expected init.moov.missing, got {:?}",
        errors(&issues)
    );
}

#[test]
fn init_missing_mvhd_bites() {
    let init = clean_init();
    assert!(!has_code(
        &validate_init_segment(&init),
        "init.mvhd.missing"
    ));
    let broken = strip_box(&init, b"mvhd");
    let issues = validate_init_segment(&broken);
    assert!(
        has_code(&issues, "init.mvhd.missing"),
        "expected init.mvhd.missing, got {:?}",
        errors(&issues)
    );
}

#[test]
fn init_incomplete_trak_bites() {
    let init = clean_init();
    assert!(!has_code(
        &validate_init_segment(&init),
        "init.trak.incomplete"
    ));
    // strip the stsd (deep inside trak>mdia>minf>stbl).
    let broken = strip_box(&init, b"stsd");
    let issues = validate_init_segment(&broken);
    assert!(
        has_code(&issues, "init.trak.incomplete"),
        "expected init.trak.incomplete, got {:?}",
        errors(&issues)
    );
}

#[test]
fn init_missing_mvex_warns() {
    let init = clean_init();
    // clean fragmented init HAS mvex → no warning.
    assert!(!has_code(
        &validate_init_segment(&init),
        "init.mvex.missing"
    ));
    // strip mvex → warning (not error).
    let broken = strip_box(&init, b"mvex");
    let issues = validate_init_segment(&broken);
    assert!(
        issues
            .iter()
            .any(|i| i.code == "init.mvex.missing" && i.severity == Severity::Warning),
        "expected init.mvex.missing warning, got {issues:?}"
    );
    assert!(
        errors(&issues).is_empty(),
        "mvex absence must not be an error"
    );
}

// ---------------------------------------------------------------------------
// 3. Garbage / malformed input → issues, no panic
// ---------------------------------------------------------------------------

#[test]
fn garbage_buffer_no_panic() {
    let garbage = [0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x11, 0x22, 0x33, 0x44, 0x55];
    let i1 = validate_init_segment(&garbage);
    let i2 = validate_media_segment(&garbage);
    // No ftyp/moov → init errors.
    assert!(has_code(&i1, "init.ftyp.missing"));
    assert!(has_code(&i1, "init.moov.missing"));
    // No moof → media error.
    assert!(has_code(&i2, "media.moof.missing"));
}

#[test]
fn empty_buffer_no_panic() {
    assert!(!validate_init_segment(&[]).is_empty());
    assert!(!validate_media_segment(&[]).is_empty());
    // cross-segment on empties must not panic.
    let _ = validate_cmaf_track(&[], &[&[], &[]]);
}

#[test]
fn wrong_brand_moov_only_errors_no_panic() {
    // A buffer whose first box is not ftyp (a bare moov-less mess).
    let mut buf = Vec::new();
    buf.extend_from_slice(&16u32.to_be_bytes());
    buf.extend_from_slice(b"free"); // filler first, then a truncated moov
    buf.extend_from_slice(&[0u8; 8]);
    buf.extend_from_slice(&16u32.to_be_bytes());
    buf.extend_from_slice(b"XXXX");
    buf.extend_from_slice(&[0u8; 8]);
    let issues = validate_init_segment(&buf);
    assert!(has_code(&issues, "init.ftyp.missing"));
    assert!(has_code(&issues, "init.moov.missing"));
}
