//! Audit T16 (#1144) — WebM/Matroska: one corrupt Cluster / TrackEntry must not
//! fail the whole file. Real committed fixture, damage injected in the middle,
//! valid data after it must still be decoded byte-identically.

use test_harness::collect::{CollectMode, media_from_matroska};
use transmux::Media;
use transmux::ir::InputDegradation;

use crate::webm_build::{encode_size_width, read_id, read_size};

const CLUSTER_ID: [u8; 4] = [0x1F, 0x43, 0xB6, 0x75];
const SEGMENT_ID: [u8; 4] = [0x18, 0x53, 0x80, 0x67];

fn demux(bytes: &[u8]) -> Result<Media, transmux::Error> {
    media_from_matroska(bytes, CollectMode::Seekable)
}

fn fixture(rel: &str) -> Vec<u8> {
    let path = test_fixtures::path(rel.trim_start_matches("fixtures/"));
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn cluster_offsets(bytes: &[u8]) -> Vec<usize> {
    bytes
        .windows(4)
        .enumerate()
        .filter(|(_, w)| *w == CLUSTER_ID)
        .map(|(i, _)| i)
        .collect()
}

fn position(kind: &InputDegradation) -> u64 {
    match kind {
        InputDegradation::MalformedUnit { position }
        | InputDegradation::TruncatedUnit { position } => *position,
        _ => 0,
    }
}

fn samples_of(m: &Media, track: usize) -> Vec<Vec<u8>> {
    m.tracks[track]
        .samples
        .iter()
        .map(|s| s.data.to_vec())
        .collect()
}

#[test]
fn an_over_large_cluster_size_is_framed_by_element_id_without_data_loss() {
    let clean_bytes = fixture("mkv/h264-aac.mkv");
    let clean = demux(&clean_bytes).unwrap();
    assert!(clean.degraded.is_empty());
    let clusters = cluster_offsets(&clean_bytes);
    assert_eq!(clusters.len(), 3, "fixture has three clusters");
    let victim = clusters[1];

    // The cluster size VINT (3 bytes, marker 0x20) now claims ~1 MiB, far past
    // the end of the file. The reader frames the following bytes as that
    // Cluster's children and ends it at the next Segment-level id, so every
    // block is still recovered and no unit is lost.
    let mut bytes = clean_bytes.clone();
    bytes[victim + 4..victim + 7].copy_from_slice(&[0x2F, 0xFF, 0xFE]);

    let damaged = demux(&bytes).expect("one corrupt cluster must not fail the file");
    assert!(damaged.degraded.is_empty(), "{:?}", damaged.degraded);
    assert_eq!(damaged.tracks.len(), clean.tracks.len());
    for t in 0..clean.tracks.len() {
        assert_eq!(
            samples_of(&damaged, t),
            samples_of(&clean, t),
            "track {t}: the structural boundary must keep every block"
        );
    }
}

#[test]
fn corrupt_track_entry_is_reported_not_panicked() {
    let clean_bytes = fixture("mkv/h264-aac.mkv");
    let clean = demux(&clean_bytes).unwrap();
    assert_eq!(clean.tracks.len(), 2);

    // The second TrackEntry's first child is TrackNumber = 2 (D7 81 02),
    // followed by TrackUID (73 C5). Replace the child size `81` with a 2-byte
    // VINT 0x40FF (255) that overruns the entry, so the whole Tracks master
    // cannot be typed.
    let pat = [0xD7u8, 0x81, 0x02, 0x73, 0xC5];
    let at = clean_bytes
        .windows(pat.len())
        .position(|w| w == pat)
        .expect("second TrackEntry");
    let mut bytes = clean_bytes.clone();
    bytes[at + 1] = 0x40;
    bytes[at + 2] = 0xFF;

    let damaged = demux(&bytes).expect("one corrupt TrackEntry must not fail the file");
    assert_eq!(damaged.degraded.len(), 1, "{:?}", damaged.degraded);
    assert!(matches!(
        damaged.degraded[0],
        InputDegradation::MalformedUnit { .. }
    ));
    assert!(
        damaged.tracks.is_empty(),
        "an untypable Tracks master announces no track, never a fabricated one"
    );
}

#[test]
fn corrupt_block_inside_a_cluster_is_dropped_the_rest_of_the_cluster_survives() {
    let clean_bytes = fixture("mkv/h264-aac.mkv");
    let clean = demux(&clean_bytes).unwrap();
    let clusters = cluster_offsets(&clean_bytes);
    let victim = clusters[1];

    // Find a SimpleBlock (0xA3) well inside the middle cluster and clear its
    // body's track-number VINT to 0x00 (an invalid VINT: no marker bit).
    let region = &clean_bytes[victim + 64..clusters[2]];
    let rel = region
        .windows(2)
        .position(|w| w[0] == 0xA3 && w[1] & 0x80 == 0)
        .map(|i| i + 64);
    // SimpleBlock sizes in this fixture are multi-byte VINTs (>= 0x4000); the
    // body starts after the 2-byte size. Locate precisely and corrupt.
    let at = victim + rel.expect("a SimpleBlock with a 2-byte size in the middle cluster");
    let body = at + 3;
    let mut bytes = clean_bytes.clone();
    bytes[body] = 0x00;

    let damaged = demux(&bytes).expect("one corrupt block must not fail the file");
    assert_eq!(damaged.degraded.len(), 1, "{:?}", damaged.degraded);
    let total = |m: &Media| m.tracks.iter().map(|t| t.samples.len()).sum::<usize>();
    assert_eq!(total(&damaged) + 1, total(&clean), "exactly one frame lost");
    for t in 0..clean.tracks.len() {
        assert_eq!(
            samples_of(&damaged, t).last(),
            samples_of(&clean, t).last(),
            "later data still decoded"
        );
    }
}

/// Append `extra` to the Segment's body and grow its size field to match, so
/// the reader walks the appended bytes as Segment children.
fn append_to_segment(data: &[u8], extra: &[u8]) -> Vec<u8> {
    let mut bytes = data.to_vec();
    let seg = bytes
        .windows(4)
        .position(|w| w == SEGMENT_ID)
        .expect("fixture must contain a Segment");
    let (_id, id_len) = read_id(&bytes, seg);
    let (size, size_len) = read_size(&bytes, seg + id_len);
    let field = seg + id_len;
    bytes[field..field + size_len]
        .copy_from_slice(&encode_size_width(size + extra.len() as u64, size_len));
    bytes.extend_from_slice(extra);
    bytes
}

/// A Segment body ending inside an element ID (`1A 45`, a 4-byte ID with two
/// bytes present) is a truncated element, reported as a damaged unit; it must
/// not slice four bytes out of a two-byte tail.
#[test]
fn truncated_element_id_at_the_segment_end_is_reported_not_panicked() {
    let clean_bytes = fixture("mkv/h264-aac.mkv");
    let clean = demux(&clean_bytes).unwrap();

    let extra = [0x1Au8, 0x45];
    let bytes = append_to_segment(&clean_bytes, &extra);
    let damaged = demux(&bytes).expect("a truncated trailing element ID must not fail the file");
    let start = clean_bytes.len();
    assert_eq!(damaged.degraded.len(), 2, "{:?}", damaged.degraded);
    for entry in &damaged.degraded {
        assert!(matches!(entry, InputDegradation::MalformedUnit { .. }));
    }
    assert_eq!(
        position(&damaged.degraded[0]),
        start as u64,
        "the damage starts where the appended bytes start"
    );
    assert_eq!(
        position(&damaged.degraded[1]),
        (start + extra.len()) as u64,
        "the second report is the end of the truncated id"
    );
    for t in 0..clean.tracks.len() {
        assert_eq!(
            samples_of(&damaged, t),
            samples_of(&clean, t),
            "track {t} must be untouched by the trailing damage"
        );
    }
}

/// An element ID with five leading-zero-derived bytes (`0x08` starts a 5-byte
/// ID, but EBML IDs are 1-4 bytes) is rejected as a damaged unit, not decoded
/// as an element. A valid empty size byte follows it, so a loosened guard would
/// accept the bogus element silently.
#[test]
fn a_five_byte_element_id_is_reported_not_parsed() {
    let clean_bytes = fixture("mkv/h264-aac.mkv");
    let clean = demux(&clean_bytes).unwrap();

    let extra = [0x08u8, 0x01, 0x02, 0x03, 0x04, 0x80];
    let bytes = append_to_segment(&clean_bytes, &extra);
    let damaged = demux(&bytes).expect("an over-long element ID must not fail the file");
    let start = clean_bytes.len();
    assert_eq!(damaged.degraded.len(), 2, "{:?}", damaged.degraded);
    for entry in &damaged.degraded {
        assert!(matches!(entry, InputDegradation::MalformedUnit { .. }));
    }
    assert_eq!(position(&damaged.degraded[0]), start as u64);
    assert_eq!(position(&damaged.degraded[1]), (start + extra.len()) as u64);
    for t in 0..clean.tracks.len() {
        assert_eq!(
            samples_of(&damaged, t),
            samples_of(&clean, t),
            "track {t} must be untouched by the bogus element"
        );
    }
}

/// Audit 23 findings 109-110 (#3583/#3584): after framing is lost the walk
/// resumes at the next real Cluster, and each damage reports its own byte
/// offset — never a constant, never a duplicated or re-read Cluster.
///
/// The Tracks element deliberately ends with `0x1F`, the first byte of the
/// Cluster ID: searching for the resync boundary one byte too early
/// (`at - 1`) would splice that byte onto the garbage's `43 B6 75` into a
/// phantom Cluster that swallows every later element.
#[test]
fn resync_after_damage_reports_each_damage_at_its_own_offset() {
    use crate::webm_build::synth_mkv;

    let tracks = synth_mkv::tracks(&[synth_mkv::track_entry(
        1,
        2,
        b"A_OPUS",
        &synth_mkv::opus_head(),
        &synth_mkv::uint(synth_mkv::DEFAULT_DURATION, 0x1F),
    )]);
    let cluster_a = synth_mkv::cluster(
        0,
        &[synth_mkv::simple_block(1, 0, synth_mkv::KEYFRAME, &[0xAA])],
    );
    let cluster_b = synth_mkv::cluster(
        0,
        &[synth_mkv::simple_block(1, 0, synth_mkv::KEYFRAME, &[0xBB])],
    );

    let mut segment = tracks;
    // A 4-byte garbage run that decodes as a Cluster only when read from its
    // last preceding byte (`1F`); from its own first byte (`43`) it is a
    // truncated 2-byte element ID that cannot fit its declared body.
    let garbage_at = segment.len();
    let mut phantom = vec![0x43u8, 0xB6, 0x75, 0x80];
    segment.extend_from_slice(&phantom);
    segment.extend_from_slice(&cluster_a);
    segment.push(0x00);
    segment.extend_from_slice(&cluster_b);
    // Give the phantom Cluster a body that runs to the end of the Segment, so a
    // walk starting at `at - 1` swallows both real Clusters.
    let remaining = segment.len() - (garbage_at + 4);
    assert!(remaining < 0x40, "one-byte VINT size for the phantom");
    phantom[3] = 0x80 | remaining as u8;
    segment[garbage_at..garbage_at + 4].copy_from_slice(&phantom);

    let mut file = synth_mkv::element(synth_mkv::EBML_HEADER, &[]);
    file.extend(synth_mkv::element(synth_mkv::SEGMENT, &segment));

    let media = demux(&file).expect("both Clusters after the damage must still be read");
    assert_eq!(media.tracks.len(), 1);
    let data: Vec<Vec<u8>> = media.tracks[0]
        .samples
        .iter()
        .map(|s| s.data.to_vec())
        .collect();
    assert_eq!(
        data,
        vec![vec![0xBB]],
        "the walk resumes at a real Cluster and never re-reads one"
    );

    let cluster_starts: Vec<usize> = file
        .windows(4)
        .enumerate()
        .filter(|(_, w)| *w == CLUSTER_ID)
        .map(|(i, _)| i)
        .collect();
    assert!(
        cluster_starts.len() >= 2,
        "the two real Clusters must be present"
    );
    let body_start = file.len() - segment.len();
    assert!(media.degraded.len() >= 2, "{:?}", media.degraded);
    let mut positions: Vec<u64> = media.degraded.iter().map(position).collect();
    positions.sort_unstable();
    positions.dedup();
    assert_eq!(
        positions.len(),
        media.degraded.len(),
        "every damage reports its own byte offset, never a constant"
    );
    assert!(
        positions
            .iter()
            .all(|position| *position >= (body_start + garbage_at) as u64),
        "no damage is reported before the garbage: {positions:?}"
    );
}
