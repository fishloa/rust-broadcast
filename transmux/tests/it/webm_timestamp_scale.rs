//! c5 R5-2: a hostile `TimestampScale` (zero, or far outside the range a
//! millisecond timeline can express) must never be applied wrapped into a
//! negative pts. The Reader rejects a zero scale outright, computes block
//! timestamps in `i128`, and the demux stage converts to the IR with a checked
//! narrowing, so a value the old i64 multiply would have wrapped is either
//! rejected or represented exactly.

use test_harness::collect::{CollectMode, media_from_matroska};
use transmux::ir::InputDegradation;

/// `Info` element id.
const INFO_ID: u32 = 0x1549_A966;
/// `TimestampScale` element id.
const TIMESTAMP_SCALE_ID: u32 = 0x2A_D7_B1;
/// `TimestampScale` element id bytes, for locating it in a real fixture.
const TIMESTAMP_SCALE_BYTES: [u8; 3] = [0x2A, 0xD7, 0xB1];

/// A one-block WebM whose `Info` carries `scale`.
fn file_with_scale(scale: &[u8]) -> Vec<u8> {
    use crate::webm_build::{EBML_HEADER, SEGMENT, element, encode_size, track_entry};
    let info = element(INFO_ID, &element(TIMESTAMP_SCALE_ID, scale));
    let tracks = element(crate::webm_build::TRACKS, &track_entry(1, 1, b"V_VP9", &[]));
    let mut block = encode_size(1);
    block.extend_from_slice(&0i16.to_be_bytes());
    block.push(0x80);
    block.extend_from_slice(&[1, 2, 3, 4]);
    let mut cluster_body = element(crate::webm_build::CLUSTER_TIMESTAMP, &[1u8]);
    cluster_body.extend(element(crate::webm_build::SIMPLE_BLOCK, &block));
    let cluster = element(crate::webm_build::CLUSTER, &cluster_body);
    let mut segment_body = info;
    segment_body.extend_from_slice(&tracks);
    segment_body.extend_from_slice(&cluster);
    let mut out = element(EBML_HEADER, &[]);
    out.extend(element(SEGMENT, &segment_body));
    out
}

fn degraded_of(value: &[u8]) -> Vec<InputDegradation> {
    let file = file_with_scale(value);
    media_from_matroska(&file, CollectMode::Seekable)
        .expect("a bad Info degrades, it does not fail the file")
        .degraded
}

#[test]
fn zero_timestamp_scale_is_reported_as_a_malformed_info() {
    for value in [&[0u8][..], &[0, 0, 0, 0, 0, 0, 0, 0][..]] {
        let degraded = degraded_of(value);
        assert_eq!(degraded.len(), 1, "{degraded:?}");
        assert!(matches!(
            degraded[0],
            InputDegradation::MalformedUnit { .. }
        ));
    }
}

#[test]
fn timestamp_scale_beyond_i64_is_reported_as_a_malformed_info() {
    for value in [
        [0x80, 0, 0, 0, 0, 0, 0, 0],
        [0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF],
    ] {
        let degraded = degraded_of(&value);
        assert_eq!(degraded.len(), 1, "{value:?}: {degraded:?}");
        assert!(matches!(
            degraded[0],
            InputDegradation::MalformedUnit { .. }
        ));
    }
}

#[test]
fn in_range_timestamp_scales_are_accepted() {
    for (value, expected_pts) in [
        (&[0x0F, 0x42, 0x40][..], 1i64),
        (&[0x01][..], 0),
        (
            &[0x7F, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF][..],
            ((i64::MAX as i128) / 1_000_000) as i64,
        ),
    ] {
        let file = file_with_scale(value);
        let media = media_from_matroska(&file, CollectMode::Seekable).expect("in range");
        assert!(media.degraded.is_empty(), "{value:?}");
        assert_eq!(
            media.tracks[0].samples[0]
                .timing
                .and_then(|timing| timing.pts()),
            Some(expected_pts),
            "{value:?}"
        );
    }
}

#[test]
fn zeroing_the_scale_of_a_real_file_keeps_its_timeline() {
    let clean = test_fixtures::read("mkv/vp9-opus.webm");
    let at = clean
        .windows(TIMESTAMP_SCALE_BYTES.len())
        .position(|window| window == TIMESTAMP_SCALE_BYTES)
        .expect("fixture has a TimestampScale");
    let len = usize::from(clean[at + 3] & 0x7F);
    let mut hostile = clean.clone();
    hostile[at + 4..at + 4 + len].fill(0);

    let want = media_from_matroska(&clean, CollectMode::Seekable).unwrap();
    let got = media_from_matroska(&hostile, CollectMode::Seekable).unwrap();
    assert!(want.degraded.is_empty());
    assert_eq!(got.degraded.len(), 1, "{:?}", got.degraded);
    assert!(matches!(
        got.degraded[0],
        InputDegradation::MalformedUnit { .. }
    ));
    let times = |media: &test_harness::collect::Collected| -> Vec<Vec<Option<i64>>> {
        media
            .tracks
            .iter()
            .map(|track| {
                track
                    .samples
                    .iter()
                    .map(|sample| sample.timing.and_then(|timing| timing.pts()))
                    .collect()
            })
            .collect()
    };
    assert_eq!(times(&got), times(&want));
    assert!(times(&got).iter().flatten().flatten().any(|&pts| pts > 0));
}
