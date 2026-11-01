//! Audit T16 (#1144) — WebM/Matroska: one corrupt Cluster / TrackEntry must not
//! fail the whole file. Real committed fixture, damage injected in the middle,
//! valid data after it must still be decoded byte-identically.

use broadcast_common::Unpackage;
use transmux::WebmDemux;
use transmux::ir::InputDegradation;
use transmux::media::Media;

const CLUSTER_ID: [u8; 4] = [0x1F, 0x43, 0xB6, 0x75];

fn fixture(rel: &str) -> Vec<u8> {
    let path = format!("{}/../fixtures/{rel}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}"))
}

fn cluster_offsets(bytes: &[u8]) -> Vec<usize> {
    bytes
        .windows(4)
        .enumerate()
        .filter(|(_, w)| *w == CLUSTER_ID)
        .map(|(i, _)| i)
        .collect()
}

fn samples_of(m: &Media, track: usize) -> Vec<Vec<u8>> {
    m.tracks[track]
        .samples
        .iter()
        .map(|s| s.data.to_vec())
        .collect()
}

#[test]
fn corrupt_cluster_size_in_the_middle_resyncs_at_next_cluster() {
    let clean_bytes = fixture("mkv/h264_aac.mkv");
    let clean = WebmDemux::new().unpackage(&clean_bytes[..]).unwrap();
    assert!(clean.degraded.is_empty());
    let clusters = cluster_offsets(&clean_bytes);
    assert_eq!(clusters.len(), 3, "fixture has three clusters");
    let victim = clusters[1];

    // The cluster size VINT (3 bytes, marker 0x20) now claims ~1 MiB, far past
    // the end of the file: the element framing is lost.
    let mut bytes = clean_bytes.clone();
    bytes[victim + 4..victim + 7].copy_from_slice(&[0x2F, 0xFF, 0xFE]);

    let damaged = WebmDemux::new()
        .unpackage(&bytes[..])
        .expect("one corrupt cluster must not fail the file");
    assert_eq!(damaged.degraded.len(), 1, "{:?}", damaged.degraded);
    assert!(matches!(
        damaged.degraded[0],
        InputDegradation::MalformedUnit { .. }
    ));
    assert_eq!(damaged.tracks.len(), clean.tracks.len());
    for t in 0..clean.tracks.len() {
        let (d, c) = (samples_of(&damaged, t), samples_of(&clean, t));
        assert!(d.len() < c.len(), "track {t}: the middle cluster is lost");
        assert_eq!(d.first(), c.first());
        // The third cluster is intact: the damaged stream ends like the clean one.
        assert_eq!(d.last(), c.last());
    }
}

#[test]
fn corrupt_track_entry_is_dropped_and_the_other_track_survives() {
    let clean_bytes = fixture("mkv/h264_aac.mkv");
    let clean = WebmDemux::new().unpackage(&clean_bytes[..]).unwrap();
    assert_eq!(clean.tracks.len(), 2);

    // The second TrackEntry's first child is TrackNumber = 2 (D7 81 02),
    // followed by TrackUID (73 C5). Replace the child size `81` with a 2-byte
    // VINT 0x40FF (255) that overruns the entry.
    let pat = [0xD7u8, 0x81, 0x02, 0x73, 0xC5];
    let at = clean_bytes
        .windows(pat.len())
        .position(|w| w == pat)
        .expect("second TrackEntry");
    let mut bytes = clean_bytes.clone();
    bytes[at + 1] = 0x40;
    bytes[at + 2] = 0xFF;

    let damaged = WebmDemux::new()
        .unpackage(&bytes[..])
        .expect("one corrupt TrackEntry must not fail the file");
    assert_eq!(damaged.degraded.len(), 1, "{:?}", damaged.degraded);
    assert_eq!(damaged.tracks.len(), 1);
    assert_eq!(samples_of(&damaged, 0), samples_of(&clean, 0));
}

#[test]
fn corrupt_block_inside_a_cluster_is_dropped_the_rest_of_the_cluster_survives() {
    let clean_bytes = fixture("mkv/h264_aac.mkv");
    let clean = WebmDemux::new().unpackage(&clean_bytes[..]).unwrap();
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

    let damaged = WebmDemux::new()
        .unpackage(&bytes[..])
        .expect("one corrupt block must not fail the file");
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
