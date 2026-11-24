//! ISOBMFF box-walk drift guard (slice 7).
//!
//! `container-probe` keeps its own box header decode and size-driven walk: it
//! must depend only on `broadcast-common` at runtime, so it cannot call
//! `isobmff`'s iterator. This guard pins the two together. It covers the
//! direction an integration test can see — upstream `isobmff` still declares
//! the header constants and walks the same number of top-level boxes on every
//! ISO BMFF fixture. The other direction (this crate's own private walk visits
//! the same `(type, offset, size)` sequence) lives in the `isobmff::drift`
//! unit-test module inside `src/isobmff.rs`, where the private constants and
//! `decode_header` are visible, exactly as `drift_guard.rs` does for MPEG-TS.

use container_probe::{Detail, Probe};

/// Every committed ISO BMFF fixture, workspace-relative under `fixtures/`.
const FIXTURES: &[&str] = &[
    "mp4/h264-high.mp4",
    "mp4/hevc-main.mp4",
    "mp4/av1.mp4",
    "mp4/vp9.mp4",
    "mp4/opus.mp4",
    "mp4/flac.mp4",
    "mp4/cenc.mp4",
    "mp4/stpp.mp4",
    "mp4/prft.mp4",
    "mp4/colr-hdr.mp4",
    "mp4/aac-sgpd.mp4",
    "mp4/cmaf/av-frag.mp4",
    "mp4/progressive/av-prog.mp4",
    "mp4/progressive/av-faststart.mp4",
    "mp4/frag/av1.frag.mp4",
    "mp4/frag/vvc.frag.mp4",
    "mp4/llcmaf/ll-chunked.mp4",
    "mp4/h264-aac-prog.mp4",
];

/// Upstream `isobmff` still declares the header values this crate's ISOBMFF
/// prober was written against.
#[test]
fn upstream_isobmff_still_declares_the_expected_header_constants() {
    assert_eq!(
        isobmff::box_types::BOX_HEADER_MIN_SIZE,
        8,
        "isobmff changed the minimum box header size; re-review container-probe/src/isobmff.rs"
    );
    assert_eq!(
        isobmff::box_types::LARGESIZE_SIZE,
        8,
        "isobmff changed the largesize width"
    );
    assert_eq!(
        isobmff::box_types::BOX_HEADER_LARGESIZE_SIZE,
        16,
        "isobmff changed the largesize header size"
    );
    assert_eq!(
        isobmff::box_types::SIZE_TO_EOF,
        0,
        "isobmff changed the to-EOF size sentinel"
    );
    assert_eq!(
        isobmff::box_types::SIZE_INDICATES_LARGESIZE,
        1,
        "isobmff changed the largesize marker"
    );
}

/// The prober's walk visits as many top-level boxes as `isobmff`'s iterator on
/// every ISO BMFF fixture (the exact offsets and sizes are pinned by the
/// in-source `isobmff::drift` unit test).
#[test]
fn prober_walk_counts_match_isobmff_on_every_fixture() {
    for &rel in FIXTURES {
        let data = test_fixtures::read(rel);
        let expected = isobmff::BoxIter::new(&data)
            .filter(|step| step.is_ok())
            .count()
            .min(usize::from(u8::MAX));
        match container_probe::probe(&data) {
            Probe::Identified {
                detail: Detail::Isobmff { boxes_walked, .. },
                ..
            } => assert_eq!(
                usize::from(boxes_walked),
                expected,
                "{rel}: prober and isobmff must walk the same number of boxes"
            ),
            other => panic!("{rel}: expected an Isobmff probe, got {other:?}"),
        }
    }
}
