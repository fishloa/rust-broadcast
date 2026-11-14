//! c5 R5-2: a hostile `TimestampScale` (zero, or too large for the i64 the pts
//! arithmetic uses) must be reported as a degraded `Info`, never applied: a zero
//! scale collapses every pts to 0 and a scale >= 2^63 wraps negative.

use broadcast_common::Unpackage;
use transmux::WebmDemux;
use transmux::ir::InputDegradation;

const TIMESTAMP_SCALE_ID: [u8; 3] = [0x2A, 0xD7, 0xB1];

fn webm_with_scale(value: &[u8]) -> Vec<u8> {
    let element_len = TIMESTAMP_SCALE_ID.len() + 1 + value.len();
    let mut info = vec![0x15, 0x49, 0xA9, 0x66, 0x80 | element_len as u8];
    info.extend_from_slice(&TIMESTAMP_SCALE_ID);
    info.push(0x80 | value.len() as u8);
    info.extend_from_slice(value);
    let mut file = vec![0x1A, 0x45, 0xDF, 0xA3, 0x80];
    file.extend_from_slice(&[0x18, 0x53, 0x80, 0x67, 0x80 | info.len() as u8]);
    file.extend_from_slice(&info);
    file
}

fn degraded_of(value: &[u8]) -> Vec<InputDegradation> {
    let file = webm_with_scale(value);
    WebmDemux::new()
        .unpackage(&file[..])
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
    for value in [
        &[0x0F, 0x42, 0x40][..],
        &[0x01][..],
        &[0x7F, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF][..],
    ] {
        assert!(degraded_of(value).is_empty(), "{value:?}");
    }
}

#[test]
fn zeroing_the_scale_of_a_real_file_keeps_its_timeline() {
    let clean = test_fixtures::read("mkv/vp9-opus.webm");
    let at = clean
        .windows(TIMESTAMP_SCALE_ID.len())
        .position(|w| w == TIMESTAMP_SCALE_ID)
        .expect("fixture has a TimestampScale");
    let len = usize::from(clean[at + 3] & 0x7F);
    let mut hostile = clean.clone();
    hostile[at + 4..at + 4 + len].fill(0);

    let want = WebmDemux::new().unpackage(&clean[..]).unwrap();
    let got = WebmDemux::new().unpackage(&hostile[..]).unwrap();
    assert!(want.degraded.is_empty());
    assert_eq!(got.degraded.len(), 1, "{:?}", got.degraded);
    assert!(matches!(
        got.degraded[0],
        InputDegradation::MalformedUnit { .. }
    ));
    let times = |m: &transmux::Media| -> Vec<Vec<Option<i64>>> {
        m.tracks
            .iter()
            .map(|t| {
                t.samples
                    .iter()
                    .map(|s| s.timing.and_then(|t| t.pts()))
                    .collect()
            })
            .collect()
    };
    assert_eq!(times(&got), times(&want));
    assert!(times(&got).iter().flatten().flatten().any(|&p| p > 0));
}
