//! Biting tests for the fMP4/CMAF conformance validator (issue #481).
//!
//! Each Error check gets BOTH a positive (clean → no such issue) and a negative
//! (broken → the exact `code`) assertion, so it cannot be satisfied by an
//! always-pass or always-fail validator. Clean segments are built from the real
//! codec configs + real coded samples of `h264-aac-frag.mp4` via the crate's own
//! `build_init_segment` / `build_media_segment`.

use crate::box_walk::find_box_range;
use broadcast_common::Parse;
use broadcast_common::Serialize;
use broadcast_common::timescale::Timescale;
use isobmff::EsdsBox;
use isobmff::movie_fragment::{
    MovieFragmentBox, MovieFragmentHeaderBox, TrackFragmentBaseMediaDecodeTimeBox,
    TrackFragmentBox, TrackFragmentHeaderBox, TrackFragmentRunBox, TrunSample,
};
use isobmff::movie_fragment::{
    TFHD_DEFAULT_BASE_IS_MOOF, TFHD_DEFAULT_SAMPLE_FLAGS_PRESENT, TRUN_DATA_OFFSET_PRESENT,
    TRUN_FIRST_SAMPLE_FLAGS_PRESENT, TRUN_SAMPLE_DURATION_PRESENT, TRUN_SAMPLE_FLAGS_PRESENT,
    TRUN_SAMPLE_SIZE_PRESENT,
};
use isobmff::segments::{MediaDataBox, SegmentTypeBox};
use transmux::TrackId;
use transmux::{
    CodecConfig, FragmentTrackData, MovieBox, Sample, SampleEntryVariant, Severity, StblChild,
    TrackSpec, build_init_segment, build_media_segment, validate::SegmentOrigin,
    validate_cmaf_track, validate_init_segment, validate_media_segment,
};

// ---------------------------------------------------------------------------
// Fixture plumbing (mirrors tests/it/pipeline.rs)
// ---------------------------------------------------------------------------

fn timescale(value: u32) -> Timescale {
    Timescale::try_new(value).expect("a non-zero timescale")
}

fn fixture() -> Vec<u8> {
    test_fixtures::read("mp4/h264-aac-frag.mp4")
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

fn track_timescale(moov: &MovieBox, track_idx: usize) -> Timescale {
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
        TrackId::new(1),
        track_timescale(moov, 0),
        CodecConfig::Avc {
            config: avc.config.clone(),
            width: avc.visual.width,
            height: avc.visual.height,
        },
    )
}

/// Build a single-audio-track spec from the fixture's real esds.
fn audio_spec(moov: &MovieBox) -> TrackSpec {
    let mp4a = match track_stsd_entry(moov, 1) {
        SampleEntryVariant::Mp4a(m) => m,
        _ => panic!("expected mp4a"),
    };
    let esds_body = mp4a
        .config_boxes
        .iter()
        .find(|b| &b.box_type == b"esds")
        .expect("mp4a must carry esds");
    let mut esds_full = Vec::with_capacity(8 + esds_body.data.len());
    esds_full.extend_from_slice(&((8 + esds_body.data.len()) as u32).to_be_bytes());
    esds_full.extend_from_slice(b"esds");
    esds_full.extend_from_slice(&esds_body.data);
    let esds = EsdsBox::parse_box(&esds_full).expect("esds must parse");
    TrackSpec::new(
        TrackId::new(2),
        track_timescale(moov, 1),
        CodecConfig::Aac {
            esds,
            channel_count: mp4a.channelcount,
            sample_rate: mp4a.samplerate >> 16,
            sample_size: mp4a.samplesize,
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
    // Absolute dts from the running trun duration sum (media plane step 2c);
    // pts folds in the trun composition offset.
    let mut next_dts: i64 = 0;
    for (i, ts) in trun.samples.iter().enumerate() {
        let size = ts.sample_size.expect("sample_size") as usize;
        let dur = ts.sample_duration.unwrap_or(3000);
        let co = ts.sample_composition_time_offset.unwrap_or(0);
        samples.push(
            Sample::new(
                data[cursor..cursor + size].to_vec(),
                Some(next_dts),
                Some(next_dts + co),
                Some(dur),
                i == 0,
            )
            .expect("valid timing"),
        );
        next_dts += i64::from(dur);
        cursor += size;
    }
    samples
}

/// A clean init segment (single video track) built from the fixture.
fn clean_init() -> Vec<u8> {
    let data = fixture();
    let (_, moov_bytes) = find_top_box(&data, b"moov");
    let moov = MovieBox::parse(&moov_bytes).unwrap();
    build_init_segment(&[video_spec(&moov)], timescale(1000)).expect("build init")
}

/// A clean media segment with the given sequence_number + base decode time,
/// carrying the real video samples.
fn clean_media(seq: u32, base_time: u64, samples: &[Sample]) -> Vec<u8> {
    build_media_segment(
        seq,
        &[FragmentTrackData::new(TrackId::new(1), base_time, samples)],
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
    let (start, size) =
        find_box_range(data, 0, data.len(), fourcc, CONTAINERS).expect("box to strip present");
    // Copy, then decrement the size of every ancestor whose original span
    // contained [start, start+size), before splicing the bytes out.
    let mut out = data.to_vec();
    shrink_ancestors(&mut out, 0, data.len(), start, size);
    out.drain(start..start + size);
    out
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

/// Rename the first descendant box of `from` to `to` (same-length 4-CC),/// Rename the first descendant box of `from` to `to` (same-length 4-CC),
/// leaving every size unchanged. Used to "remove" a box structurally (the
/// validator no longer sees it) without disturbing sibling offsets — e.g.
/// turning `tfdt` into `free` so the trun `data_offset` arithmetic still holds.
fn rename_box(data: &[u8], from: &[u8; 4], to: &[u8; 4]) -> Vec<u8> {
    let (start, _) =
        find_box_range(data, 0, data.len(), from, CONTAINERS).expect("box to rename present");
    let mut out = data.to_vec();
    out[start + 4..start + 8].copy_from_slice(to);
    out
}

/// The byte range of the `n`th (0-based) `trun` in `data`.
fn nth_trun(data: &[u8], n: usize) -> (usize, usize) {
    let mut found = find_box_range(data, 0, data.len(), b"trun", CONTAINERS).expect("trun present");
    for _ in 0..n {
        found = find_box_range(data, found.0 + found.1, data.len(), b"trun", CONTAINERS)
            .expect("trun present");
    }
    found
}

/// Overwrite the `data_offset` of the `trun` starting at `trun_start`.
fn set_trun_data_offset(data: &mut [u8], trun_start: usize, offset: i32) {
    // FullBox word, sample_count, then data_offset.
    let at = trun_start + 16;
    data[at..at + 4].copy_from_slice(&offset.to_be_bytes());
}

fn has_code(issues: &[transmux::ConformanceIssue], code: &str) -> bool {
    issues.iter().any(|i| i.code == code)
}

/// Rebuild `segment` with a leading `styp` carrying the given brands, keeping
/// every other box byte-identical (`trun.data_offset` is relative to the `moof`,
/// so the leading box size does not affect resolution).
fn with_styp(segment: &[u8], major: &[u8; 4], compatible: &[[u8; 4]]) -> Vec<u8> {
    let mut out = Vec::new();
    let size = 16 + 4 * compatible.len();
    out.extend_from_slice(&(size as u32).to_be_bytes());
    out.extend_from_slice(b"styp");
    out.extend_from_slice(major);
    out.extend_from_slice(&0u32.to_be_bytes());
    for brand in compatible {
        out.extend_from_slice(brand);
    }
    let styp_size = top_boxes(segment)
        .first()
        .filter(|(_, _, t)| t == b"styp")
        .map(|(_, s, _)| *s)
        .expect("the segment begins with styp");
    out.extend_from_slice(&segment[styp_size..]);
    out
}

fn warnings(issues: &[transmux::ConformanceIssue]) -> Vec<&str> {
    issues
        .iter()
        .filter(|i| i.severity == Severity::Warning)
        .map(|i| i.code)
        .collect()
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
    let issues = validate_media_segment(&media, SegmentOrigin::FromIr);
    assert!(
        errors(&issues).is_empty(),
        "clean media flagged errors: {:?}",
        errors(&issues)
    );
}

#[test]
fn styp_brand_rule_bites() {
    let data = fixture();
    let samples = video_samples(&data);
    let media = clean_media(1, 0, &samples);

    // A good styp (the builder's own msdh/msdh+msix) yields no warnings at all.
    let clean = validate_media_segment(&media, SegmentOrigin::FromIr);
    assert_eq!(
        warnings(&clean),
        Vec::<&str>::new(),
        "a clean segment must carry no warning"
    );

    let validate = |major: &[u8; 4], compat: &[[u8; 4]]| {
        let seg = with_styp(&media, major, compat);
        validate_media_segment(&seg, SegmentOrigin::FromIr)
    };

    // (b)/(c) a DASH segment brand as the major brand is accepted.
    assert!(
        !has_code(&validate(b"msdh", &[]), "media.styp.brand"),
        "msdh must be a recognised segment brand"
    );
    assert!(
        !has_code(&validate(b"msix", &[]), "media.styp.brand"),
        "msix must be a recognised segment brand"
    );
    // (d) any cmf* brand is accepted.
    assert!(
        !has_code(&validate(b"cmfc", &[]), "media.styp.brand"),
        "cmfc must be a recognised segment brand"
    );
    // (e) the compatible-brand list alone can carry the segment brand.
    assert!(
        !has_code(
            &validate(b"isom", &[*b"iso6", *b"cmfs"]),
            "media.styp.brand"
        ),
        "a compatible cmf* brand is enough"
    );

    // (a) a brand set with no segment brand is reported.
    let bad = validate(b"isom", &[*b"iso6"]);
    assert!(
        has_code(&bad, "media.styp.brand"),
        "isom+iso6 must warn: {:?}",
        warnings(&bad)
    );
    // (f) a near-miss brand is not a segment brand.
    for major in [*b"xmsd", *b"cmxx"] {
        assert!(
            has_code(&validate(&major, &[]), "media.styp.brand"),
            "a near-miss major brand must warn"
        );
    }

    // A segment with no styp at all reports the missing box, and only that.
    let no_styp = strip_box(&media, b"styp");
    let issues = validate_media_segment(&no_styp, SegmentOrigin::FromIr);
    assert!(
        has_code(&issues, "media.styp.missing"),
        "no styp must be reported: {issues:?}"
    );
}

#[test]
fn severity_labels_are_pinned() {
    assert_eq!(Severity::Error.name(), "error");
    assert_eq!(Severity::Warning.name(), "warning");
    assert_eq!(Severity::Error.to_string(), "error");
    assert_eq!(Severity::Warning.to_string(), "warning");
}

#[test]
fn clean_track_has_no_errors() {
    let data = fixture();
    let samples = video_samples(&data);
    let init = clean_init();
    let total: u64 = samples
        .iter()
        .map(|s| s.timing.and_then(|t| t.duration()).unwrap_or(0) as u64)
        .sum();
    let seg1 = clean_media(1, 0, &samples);
    let seg2 = clean_media(2, total, &samples);
    let issues = validate_cmaf_track(&init, &[&seg1, &seg2], SegmentOrigin::FromIr);
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
        &validate_media_segment(&media, SegmentOrigin::FromIr),
        "media.tfdt.missing"
    ));

    // negative: turn the tfdt into a free box (same size → offsets intact) so
    // ONLY the missing-tfdt error fires, not a flood of size-driven ones.
    let broken = rename_box(&media, b"tfdt", b"free");
    let issues = validate_media_segment(&broken, SegmentOrigin::FromIr);
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
        &validate_media_segment(&media, SegmentOrigin::FromIr),
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

    let issues = validate_media_segment(&broken, SegmentOrigin::FromIr);
    assert!(
        has_code(&issues, "media.mdat.overrun"),
        "expected media.mdat.overrun, got {:?}",
        errors(&issues)
    );
}

#[test]
fn mdat_overrun_lower_bound_bites() {
    // issue #991: a `data_offset` that points *before* the mdat payload (into
    // the moof itself, or the mdat's own box header) must be rejected even
    // when the sample range still fits comfortably under the upper bound —
    // an overrun check that only compares against `mdat_payload_end` misses
    // this entirely.
    let data = fixture();
    let samples = video_samples(&data);
    let media = clean_media(1, 0, &samples);

    // positive: intact data_offset → no overrun.
    assert!(!has_code(
        &validate_media_segment(&media, SegmentOrigin::FromIr),
        "media.mdat.overrun"
    ));

    // negative: rewrite trun.data_offset to 0 (the moof's own first byte).
    // The mdat is left completely untouched, so `end` (0 + total_size) still
    // sits well inside the real mdat payload — only the lower-bound check
    // added for #991 catches this.
    let (trun_start, _) =
        find_box_range(&media, 0, media.len(), b"trun", CONTAINERS).expect("trun present");
    let mut broken = media.clone();
    // trun layout: box header(8) + FullBox version/flags(4) + sample_count(4)
    // + data_offset(4) — see `TrackFragmentRunBox::parse_body`.
    let data_offset_off = trun_start + 8 + 4 + 4;
    broken[data_offset_off..data_offset_off + 4].copy_from_slice(&0i32.to_be_bytes());

    let issues = validate_media_segment(&broken, SegmentOrigin::FromIr);
    assert!(
        has_code(&issues, "media.mdat.overrun"),
        "expected media.mdat.overrun (lower bound), got {:?}",
        errors(&issues)
    );
}

#[test]
fn moof_without_mdat_bites() {
    let data = fixture();
    let samples = video_samples(&data);
    let media = clean_media(1, 0, &samples);

    // positive: moof+mdat present → no pairing error.
    let clean = validate_media_segment(&media, SegmentOrigin::FromIr);
    assert!(!has_code(&clean, "media.mdat.missing"));
    assert!(!has_code(&clean, "media.mdat.orphan"));

    // negative: drop the trailing mdat entirely (styp + moof only).
    let boxes = top_boxes(&media);
    let (mdat_off, _, _) = *boxes.iter().find(|(_, _, t)| t == b"mdat").unwrap();
    let broken = media[..mdat_off].to_vec();

    let issues = validate_media_segment(&broken, SegmentOrigin::FromIr);
    assert!(
        has_code(&issues, "media.mdat.missing"),
        "expected media.mdat.missing, got {:?}",
        errors(&issues)
    );
}

#[test]
fn moof_followed_by_a_non_mdat_box_bites() {
    let data = fixture();
    let samples = video_samples(&data);
    let media = clean_media(1, 0, &samples);

    // A same-size `free` where the mdat was: the moof's next box exists, but it
    // is not an mdat.
    let renamed = rename_box(&media, b"mdat", b"free");
    let issues = validate_media_segment(&renamed, SegmentOrigin::FromIr);
    assert!(
        has_code(&issues, "media.mdat.missing"),
        "a moof followed by free must report a missing mdat, got {:?}",
        errors(&issues)
    );

    // A free box between the moof and the mdat.
    let (mdat_off, _, _) = *top_boxes(&media)
        .iter()
        .find(|(_, _, t)| t == b"mdat")
        .expect("mdat present");
    let mut separated = media[..mdat_off].to_vec();
    separated.extend_from_slice(&8u32.to_be_bytes());
    separated.extend_from_slice(b"free");
    separated.extend_from_slice(&media[mdat_off..]);
    let issues = validate_media_segment(&separated, SegmentOrigin::FromIr);
    assert!(
        has_code(&issues, "media.mdat.missing"),
        "a box between moof and mdat must report a missing mdat, got {:?}",
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

    let issues = validate_media_segment(&buf, SegmentOrigin::FromIr);
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
    let total: u64 = samples
        .iter()
        .map(|s| s.timing.and_then(|t| t.duration()).unwrap_or(0) as u64)
        .sum();
    let init = clean_init();

    // positive: two contiguous segments → no continuity error.
    let seg1 = clean_media(1, 0, &samples);
    let seg2_ok = clean_media(2, total, &samples);
    let ok = validate_cmaf_track(&init, &[&seg1, &seg2_ok], SegmentOrigin::FromIr);
    assert!(
        !has_code(&ok, "track.tfdt.discontinuity"),
        "contiguous segments must not flag discontinuity: {:?}",
        errors(&ok)
    );

    // negative: segment 2 tfdt < segment 1 (rewinds) → continuity error.
    let seg2_bad = clean_media(2, 0, &samples); // same base time as seg1
    let bad = validate_cmaf_track(&init, &[&seg1, &seg2_bad], SegmentOrigin::FromIr);
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
    let total: u64 = samples
        .iter()
        .map(|s| s.timing.and_then(|t| t.duration()).unwrap_or(0) as u64)
        .sum();
    let init = clean_init();

    let seg1 = clean_media(5, 0, &samples);
    // Second segment with a NON-increasing sequence number (same as first).
    let seg2 = clean_media(5, total, &samples);
    let issues = validate_cmaf_track(&init, &[&seg1, &seg2], SegmentOrigin::FromIr);
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
    // negative: force a zero-duration sample, as a track the init segment does
    // not type as video writes one: a video track's zero is an unstated span
    // (issue #2788), any other track's zero states a zero length.
    samples[0].timing = Some(transmux::ir::SampleTiming::timed(0, 0));
    let media = build_media_segment(1, &[FragmentTrackData::new(TrackId::new(2), 0, &samples)])
        .expect("build media");
    let issues = validate_media_segment(&media, SegmentOrigin::External);
    assert!(
        has_code(&issues, "media.sample.zero-duration"),
        "expected media.sample.zero-duration, got {issues:?}"
    );
}

#[test]
fn ir_video_zero_span_with_init_is_a_warning_not_an_error() {
    let data = fixture();
    let (_, moov_bytes) = find_top_box(&data, b"moov");
    let moov = MovieBox::parse(&moov_bytes).expect("fixture moov");
    // Audio track first, video second: the handler, not the trak position,
    // decides which track is video.
    let init = build_init_segment(&[audio_spec(&moov), video_spec(&moov)], timescale(1000))
        .expect("init with audio first, video second");

    // A video IR sample with no stated span is written as a zero duration.
    let mut video: Vec<Sample> = video_samples(&data).into_iter().take(3).collect();
    for sample in &mut video {
        let timing = sample.timing.expect("timed sample");
        sample.timing = Some(transmux::ir::SampleTiming::timed(
            timing.dts().expect("dts"),
            timing.pts().expect("pts"),
        ));
    }
    let video_seg = clean_media(1, 0, &video);
    let issues = validate_cmaf_track(&init, &[&video_seg], SegmentOrigin::FromIr);
    let timing = issues
        .iter()
        .find(|i| i.code == "media.sample.timing")
        .unwrap_or_else(|| panic!("video zero span must be an unstated span: {issues:?}"));
    assert!(
        timing.message.contains("3 sample(s)"),
        "the warning must name the three samples: {}",
        timing.message
    );
    assert!(
        !has_code(&issues, "media.sample.zero-duration"),
        "a video track's zero span is unstated, not a declared zero: {issues:?}"
    );

    // The same zero span on a track the init types as audio states a zero
    // length, which is an error.
    let audio: Vec<Sample> = (0..3)
        .map(|i| Sample::timed(vec![0u8; 8], i * 1024, i * 1024, None))
        .collect();
    let audio_seg = build_media_segment(1, &[FragmentTrackData::new(TrackId::new(2), 0, &audio)])
        .expect("audio media segment");
    let issues = validate_cmaf_track(&init, &[&audio_seg], SegmentOrigin::FromIr);
    let zero = issues
        .iter()
        .find(|i| i.code == "media.sample.zero-duration")
        .unwrap_or_else(|| panic!("an audio zero span is a declared zero: {issues:?}"));
    assert!(
        zero.message.contains("3 sample(s)"),
        "the error must name the three samples: {}",
        zero.message
    );
    assert!(
        !has_code(&issues, "media.sample.timing"),
        "an audio zero is not an unstated video span: {issues:?}"
    );

    // A clean init + segment pair carries no warning at all.
    let clean = clean_media(1, 0, &video_samples(&data));
    let issues = validate_cmaf_track(&init, &[&clean], SegmentOrigin::FromIr);
    assert_eq!(
        warnings(&issues),
        Vec::<&str>::new(),
        "a clean track must carry no warning: {issues:?}"
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
    let i2 = validate_media_segment(&garbage, SegmentOrigin::FromIr);
    // No ftyp/moov → init errors.
    assert!(has_code(&i1, "init.ftyp.missing"));
    assert!(has_code(&i1, "init.moov.missing"));
    // No moof → media error.
    assert!(has_code(&i2, "media.moof.missing"));
}

#[test]
fn empty_buffer_no_panic() {
    assert!(!validate_init_segment(&[]).is_empty());
    assert!(!validate_media_segment(&[], SegmentOrigin::FromIr).is_empty());
    // cross-segment on empties must not panic.
    let _ = validate_cmaf_track(&[], &[&[], &[]], SegmentOrigin::FromIr);
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

// ---------------------------------------------------------------------------
// Audit r05-W30 — per-trun ranges, multi-moof, track identity, issue dedup
// ---------------------------------------------------------------------------

/// (a) The overrun check must consider **every** `trun`'s own range, not just the
/// smallest offset: a `trun` whose own `data_offset` leaves the mdat used to pass
/// whenever the aggregate fitted.
#[test]
fn mdat_overrun_checks_every_trun_range() {
    let data = fixture();
    let samples = video_samples(&data);
    let media = clean_media(1, 0, &samples);

    // Baseline is clean.
    assert!(!has_code(
        &validate_media_segment(&media, SegmentOrigin::FromIr),
        "media.mdat.overrun"
    ));

    // Corrupt *only* the trun's `data_offset`: point it well past the mdat.
    // The sample sizes are unchanged, so the only way to see this is to resolve
    // each trun's own range.
    let (trun_start, _) =
        find_box_range(&media, 0, media.len(), b"trun", CONTAINERS).expect("trun present");
    let mut broken = media.clone();
    // trun layout: box header(8) + version/flags(4) + sample_count(4) +
    // data_offset(4).
    let data_offset_off = trun_start + 8 + 4 + 4;
    broken[data_offset_off..data_offset_off + 4].copy_from_slice(&0x00FF_FFFFu32.to_be_bytes());

    let issues = validate_media_segment(&broken, SegmentOrigin::FromIr);
    assert!(
        has_code(&issues, "media.mdat.overrun"),
        "an out-of-range trun data_offset must be caught, got {:?}",
        errors(&issues)
    );

    // And the reverse: the original media (same sizes, in-range offset) is
    // clean — so the check is on the range, not merely on the presence of a
    // trun.
    assert!(!has_code(
        &validate_media_segment(&media, SegmentOrigin::FromIr),
        "media.mdat.overrun"
    ));
}

/// One `moof`/`mdat` media segment with the given fragment-wide and per-run
/// sample flags (ISO/IEC 14496-12:2026 §8.8.7/§8.8.8).
fn flagged_segment(
    tfhd_default: Option<u32>,
    first_sample: Option<u32>,
    sample_flags: Option<u32>,
) -> Vec<u8> {
    let sample = TrunSample {
        sample_duration: Some(1000),
        sample_size: Some(1),
        sample_flags,
        sample_composition_time_offset: None,
    };
    let mut tr_flags =
        TRUN_DATA_OFFSET_PRESENT | TRUN_SAMPLE_DURATION_PRESENT | TRUN_SAMPLE_SIZE_PRESENT;
    if sample_flags.is_some() {
        tr_flags |= TRUN_SAMPLE_FLAGS_PRESENT;
    }
    if first_sample.is_some() {
        tr_flags |= TRUN_FIRST_SAMPLE_FLAGS_PRESENT;
    }
    let run = TrackFragmentRunBox {
        version: 0,
        tr_flags,
        data_offset: Some(0),
        first_sample_flags: first_sample,
        samples: vec![sample, sample],
    };
    let mut tfhd_flags = TFHD_DEFAULT_BASE_IS_MOOF;
    if tfhd_default.is_some() {
        tfhd_flags |= TFHD_DEFAULT_SAMPLE_FLAGS_PRESENT;
    }
    let tfhd = TrackFragmentHeaderBox {
        flags: tfhd_flags,
        track_id: TrackId::new(1).into(),
        base_data_offset: None,
        sample_description_index: None,
        default_sample_duration: None,
        default_sample_size: None,
        default_sample_flags: tfhd_default,
    };
    let traf = TrackFragmentBox::new(
        tfhd,
        Some(TrackFragmentBaseMediaDecodeTimeBox::new_v1(0)),
        vec![run],
    );
    let mut moof = MovieFragmentBox::new(MovieFragmentHeaderBox::new(1), vec![traf]);
    moof.traf[0].trun[0].data_offset =
        Some(i32::try_from(moof.serialized_len() + 8).expect("small moof"));
    let styp = SegmentTypeBox {
        major_brand: *b"msdh",
        minor_version: 0,
        compatible_brands: vec![*b"msdh", *b"msix"],
    };
    let mdat = MediaDataBox { data: vec![0u8, 0] };
    let mut out = vec![0u8; styp.serialized_len() + moof.serialized_len() + mdat.serialized_len()];
    let mut c = 0;
    c += styp.serialize_into(&mut out[c..]).expect("styp");
    c += moof.serialize_into(&mut out[c..]).expect("moof");
    c += mdat.serialize_into(&mut out[c..]).expect("mdat");
    out.truncate(c);
    out
}

/// The three-level sample-flag precedence decides whether a fragment begins at
/// a sync sample (§8.8.8.1).
#[test]
fn non_sync_first_sample_precedence() {
    /// `sample_is_non_sync_sample`: bit `[16]` (§8.8.3.1).
    const NON_SYNC: u32 = 0x0001_0000;
    const SYNC: u32 = 0x0200_0000;
    let has = |segment: &[u8]| {
        has_code(
            &validate_media_segment(segment, SegmentOrigin::FromIr),
            "media.sample.non-sync-first",
        )
    };

    assert!(
        has(&flagged_segment(None, Some(NON_SYNC), None)),
        "first_sample_flags = non-sync must warn"
    );
    assert!(
        !has(&flagged_segment(None, Some(SYNC), Some(NON_SYNC))),
        "first_sample_flags wins over sample_flags"
    );
    assert!(
        has(&flagged_segment(None, None, Some(NON_SYNC))),
        "sample_flags = non-sync must warn when first_sample_flags is absent"
    );
    assert!(
        has(&flagged_segment(Some(NON_SYNC), None, None)),
        "tfhd.default_sample_flags = non-sync must warn as the last fallback"
    );
    assert!(
        !has(&flagged_segment(Some(SYNC), None, None)),
        "a sync tfhd.default_sample_flags must not warn"
    );
    assert!(
        !has(&flagged_segment(Some(SYNC), None, Some(SYNC))),
        "all-sync flags must not warn"
    );

    // Only bit 16 marks a non-sync sample: the neighbouring bits do not.
    assert!(!has(&flagged_segment(None, Some(0x0002_0000), None)));
    assert!(!has(&flagged_segment(None, Some(0x0000_8000), None)));
    assert!(!has(&flagged_segment(None, Some(0x0000_0100), None)));
}

/// CMAF requires `tfhd.default-base-is-moof`.
#[test]
fn default_base_is_moof_flag_bites() {
    let samples = video_samples(&fixture());
    let media = clean_media(1, 0, &samples);
    let clean = validate_media_segment(&media, SegmentOrigin::FromIr);
    assert!(
        !has_code(&clean, "media.tfhd.default-base-is-moof"),
        "the builder sets the flag: {clean:?}"
    );

    let (tfhd, _) =
        find_box_range(&media, 0, media.len(), b"tfhd", CONTAINERS).expect("tfhd present");
    let mut broken = media.clone();
    // Box header, then the FullBox version byte (0) and its three flag bytes.
    broken[tfhd + 9] = 0;
    broken[tfhd + 10] = 0;
    broken[tfhd + 11] = 0;
    let issues = validate_media_segment(&broken, SegmentOrigin::FromIr);
    assert!(
        has_code(&issues, "media.tfhd.default-base-is-moof"),
        "a tfhd without default-base-is-moof must be an error: {issues:?}"
    );
}

/// The single-track CMAF fragment warning fires only for two or more `traf`s.
#[test]
fn multi_traf_warning_needs_two_trafs() {
    let samples = video_samples(&fixture());

    // Zero trafs: a moof with only an mfhd.
    let mut none = Vec::new();
    none.extend_from_slice(&20u32.to_be_bytes());
    none.extend_from_slice(b"styp");
    none.extend_from_slice(b"msdh");
    none.extend_from_slice(&0u32.to_be_bytes());
    none.extend_from_slice(b"msdh");
    none.extend_from_slice(&20u32.to_be_bytes());
    none.extend_from_slice(b"moof");
    none.extend_from_slice(&12u32.to_be_bytes());
    none.extend_from_slice(b"mfhd");
    none.extend_from_slice(&[0, 0, 0, 0]);
    none.extend_from_slice(&1u32.to_be_bytes());
    none.extend_from_slice(&8u32.to_be_bytes());
    none.extend_from_slice(b"mdat");
    let issues = validate_media_segment(&none, SegmentOrigin::FromIr);
    assert!(
        has_code(&issues, "media.traf.missing"),
        "an empty moof must report a missing traf: {issues:?}"
    );
    assert!(
        !has_code(&issues, "media.moof.multi-traf"),
        "zero trafs is not a multi-track fragment: {issues:?}"
    );

    // One traf: no multi-track warning.
    let one = clean_media(1, 0, &samples);
    let issues = validate_media_segment(&one, SegmentOrigin::FromIr);
    assert!(
        !has_code(&issues, "media.moof.multi-traf"),
        "a single-traf fragment must not warn: {issues:?}"
    );

    // Two trafs: the warning fires exactly once.
    let two = build_media_segment(
        1,
        &[
            FragmentTrackData::new(TrackId::new(1), 0, &samples),
            FragmentTrackData::new(TrackId::new(2), 0, &samples),
        ],
    )
    .expect("two-track media segment");
    let issues = validate_media_segment(&two, SegmentOrigin::FromIr);
    assert_eq!(
        issues
            .iter()
            .filter(|i| i.code == "media.moof.multi-traf")
            .count(),
        1,
        "a two-traf fragment must warn once: {issues:?}"
    );
}

/// Two `trun`s claiming the same `mdat` bytes are rejected; adjacent and
/// disjoint runs are not.
#[test]
fn overlapping_trun_ranges_bite() {
    let data = fixture();
    let samples = video_samples(&data);
    let media = build_media_segment(
        1,
        &[
            FragmentTrackData::new(TrackId::new(1), 0, &samples),
            FragmentTrackData::new(TrackId::new(2), 0, &samples),
        ],
    )
    .expect("two-track media segment");

    // The builder lays the second run immediately after the first: adjacent
    // ranges (b_lo == a_hi) must not overlap.
    let clean = validate_media_segment(&media, SegmentOrigin::FromIr);
    assert!(
        !has_code(&clean, "media.mdat.overlap"),
        "adjacent runs must not overlap: {clean:?}"
    );

    let (second, _) = nth_trun(&media, 1);
    let second_off = i32::from_be_bytes([
        media[second + 16],
        media[second + 17],
        media[second + 18],
        media[second + 19],
    ]);

    // One byte of overlap.
    let mut overlap = media.clone();
    set_trun_data_offset(&mut overlap, second, second_off - 1);
    let issues = validate_media_segment(&overlap, SegmentOrigin::FromIr);
    assert!(
        has_code(&issues, "media.mdat.overlap"),
        "a one-byte overlap must be reported: {issues:?}"
    );

    // A gap in the mdat is not an overlap; grow the mdat so it still holds the
    // shifted run.
    let gap = 8usize;
    let (mdat_off, mdat_size, _) = *top_boxes(&media)
        .iter()
        .find(|(_, _, t)| t == b"mdat")
        .expect("mdat present");
    let mut disjoint = media.clone();
    disjoint[mdat_off..mdat_off + 4].copy_from_slice(
        &u32::try_from(mdat_size + gap)
            .expect("small mdat")
            .to_be_bytes(),
    );
    disjoint.extend_from_slice(&vec![0u8; gap]);
    set_trun_data_offset(&mut disjoint, second, second_off + gap as i32);
    let issues = validate_media_segment(&disjoint, SegmentOrigin::FromIr);
    assert!(
        !has_code(&issues, "media.mdat.overlap"),
        "disjoint runs must not overlap: {issues:?}"
    );
    assert!(
        !has_code(&issues, "media.mdat.overrun"),
        "the widened mdat must hold the shifted run: {issues:?}"
    );
}

/// (b) A segment holding **two** `moof`s must not report a false
/// `track.tfdt.discontinuity`: the cross-segment check must use the segment's
/// last fragment end, not its first tfdt.
#[test]
fn multi_moof_segment_has_no_false_tfdt_discontinuity() {
    let data = fixture();
    let all = video_samples(&data);
    let half = all.len() / 2;
    let (a, b) = all.split_at(half);

    let seg1 = clean_media(1, 0, a);
    let dur_a: u64 = a
        .iter()
        .map(|s| u64::from(s.timing.and_then(|t| t.duration()).unwrap_or(0)))
        .sum();
    let seg2 = clean_media(2, dur_a, b);

    // Concatenating two media segments gives one byte string with two moofs —
    // exactly a multi-fragment segment. The track's decode end after the second
    // fragment is dur_a + dur_b.
    let mut multi = seg1.clone();
    multi.extend_from_slice(&seg2);

    let dur_b: u64 = b
        .iter()
        .map(|s| u64::from(s.timing.and_then(|t| t.duration()).unwrap_or(0)))
        .sum();
    let seg3 = clean_media(3, dur_a + dur_b, a);

    let issues = validate_cmaf_track(&clean_init(), &[&multi, &seg3], SegmentOrigin::FromIr);
    assert!(
        !has_code(&issues, "track.tfdt.discontinuity"),
        "a multi-moof segment must not report a false tfdt discontinuity: {:?}",
        errors(&issues)
    );

    // And a genuinely discontinuous next segment still bites.
    let seg_bad = clean_media(3, dur_a + dur_b + 999_999, a);
    let issues = validate_cmaf_track(&clean_init(), &[&multi, &seg_bad], SegmentOrigin::FromIr);
    assert!(
        has_code(&issues, "track.tfdt.discontinuity"),
        "a real discontinuity must still be reported"
    );
}

/// (c) A media segment naming a track the init segment does not declare is an
/// error.
#[test]
fn unknown_track_id_bites() {
    let data = fixture();
    let samples = video_samples(&data);
    // Track id 7 is not in `clean_init()` (which carries the fixture's own id).
    let media = build_media_segment(1, &[FragmentTrackData::new(TrackId::new(7), 0, &samples)])
        .expect("build media with an undeclared track id");

    let issues = validate_cmaf_track(&clean_init(), &[&media], SegmentOrigin::FromIr);
    assert!(
        has_code(&issues, "track.tfhd.unknown-track"),
        "an undeclared tfhd.track_id must be reported: {:?}",
        errors(&issues)
    );
}

/// (d) A fragment whose samples all have a zero span reports **one**
/// `media.sample.zero-duration` issue, not one per sample. A span is a
/// positive tick count, so "no span" is stated by leaving the timing's
/// `duration` at `None`: the fragment writer emits a zero `trun` duration for
/// it (a non-zero span cannot be stated with `SampleTiming::with_duration`,
/// and a zero is not silently kept or cleared).
#[test]
fn unknown_span_is_reported_not_conflated_with_a_zero_one() {
    // A video IR sample whose span no source stated is written as a zero
    // `trun` duration. A video frame cannot have a zero length, so the
    // validator reports an unstated span, not a declared zero length. With no
    // init segment the track cannot be typed at all, and the span is still
    // reported as unstated rather than as a declared zero (issue #2788).
    let samples = vec![
        Sample::from_raw(vec![0x65, 0x88], Some(0), Some(0), None).expect("timestamps, no span"),
    ];
    let media = clean_media(1, 0, &samples);
    let issues = validate_media_segment(&media, SegmentOrigin::FromIr);
    let codes: Vec<&str> = issues.iter().map(|i| i.code).collect();
    assert!(
        codes.contains(&"media.sample.timing"),
        "an untyped track's zero duration is an unstated span: {codes:?}"
    );
    assert!(
        !codes.contains(&"media.sample.zero-duration"),
        "an unstated span is not a declared zero length: {codes:?}"
    );
}

#[test]
fn untyped_zero_span_counts_every_sample() {
    // Two samples with no stated span on an untyped track: the aggregate
    // warning must name both, not a product of the counter (0) or nothing.
    let samples = vec![
        Sample::from_raw(vec![0x65, 0x88], Some(0), Some(0), None).expect("timestamps, no span"),
        Sample::from_raw(vec![0x65, 0x88], Some(1_000), Some(1_000), None)
            .expect("timestamps, no span"),
    ];
    let media = clean_media(1, 0, &samples);
    let issues = validate_media_segment(&media, SegmentOrigin::FromIr);
    let timing = issues
        .iter()
        .find(|i| i.code == "media.sample.timing")
        .unwrap_or_else(|| panic!("an untyped zero span must be reported: {issues:?}"));
    assert!(
        timing.message.contains("2 sample(s)"),
        "the warning must name both samples: {}",
        timing.message
    );
}

#[test]
fn unknown_video_span_is_reported_without_the_init_segment() {
    // TEMP probe
    let samples = video_samples(&fixture());
    let mut unstated = samples.clone();
    unstated[1] = Sample::timed(unstated[1].data.clone(), 512, 512, None);
    let media = clean_media(1, 0, &unstated);
    let issues = validate_media_segment(&media, SegmentOrigin::FromIr);
    assert!(
        has_code(&issues, "media.sample.timing"),
        "an unstated video span must be reported without the init segment: {issues:?}"
    );
}
