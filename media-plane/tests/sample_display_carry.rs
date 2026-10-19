//! `transmux::Sample::display` reaches egress through the `Trunk` unchanged
//! (`docs/display-geometry/DESIGN.md` §5.3c, Phase 2).
//!
//! The `Trunk` stores `transmux::Sample` as the single copy of the data; it
//! neither strips nor rebuilds it, so the per-access-unit display tag (an
//! `Arc<DisplayInfo>`) a demuxer attached must be the **same allocation** on
//! the subscriber side (`Arc::ptr_eq`), not merely an equal value. A pair of
//! samples with different geometries pins that the tag is read per sample.

use std::num::NonZeroUsize;
use std::sync::Arc;

use bytes::Bytes;
use media_plane::{RetentionClass, SampleCursorItem, Trunk, TrunkConfig};
use transmux::ir::{AspectRatioIdc, DisplayGeometry, DisplayInfo};
use transmux::{Sample, TrackId};

fn nz(v: usize) -> NonZeroUsize {
    NonZeroUsize::new(v).expect("non-zero")
}

fn info(width: u32, aspect_ratio: AspectRatioIdc) -> Arc<DisplayInfo> {
    Arc::new(DisplayInfo::new(Some(DisplayGeometry::new(
        width,
        576,
        Some(aspect_ratio),
    ))))
}

fn video_sample(byte: u8, display: Option<Arc<DisplayInfo>>) -> Sample {
    let sample = Sample::new(Bytes::from(vec![byte; 4]), Some(0), Some(0), Some(1), true);
    match display {
        Some(d) => sample.with_display(d),
        None => sample,
    }
}

#[test]
fn the_per_sample_display_tag_survives_the_trunk() {
    let trunk = Trunk::new(TrunkConfig::new(nz(100), nz(10), nz(4), nz(8), nz(8)));
    let mut cursor = trunk.subscribe();
    let writer = trunk.writer().expect("the single writer");

    let first = info(720, AspectRatioIdc::Sar16x11);
    let second = info(720, AspectRatioIdc::Square);
    writer.publish(
        TrackId::new(1),
        RetentionClass::Timed,
        video_sample(1, Some(first.clone())),
    );
    writer.publish(
        TrackId::new(1),
        RetentionClass::Timed,
        video_sample(2, Some(second.clone())),
    );

    let mut seen: Vec<Option<Arc<DisplayInfo>>> = Vec::new();
    for _ in 0..2 {
        let item = cursor.poll().expect("both samples are published");
        let SampleCursorItem::Timed { sample, .. } = item else {
            panic!("a timed video sample: {item:?}");
        };
        seen.push(sample.display.clone());
    }

    let got_first = seen[0].as_ref().expect("the first tag survives");
    assert!(
        Arc::ptr_eq(got_first, &first),
        "the trunk carries the demuxer's Arc, not a copy"
    );
    assert_eq!(
        got_first
            .geometry
            .as_ref()
            .and_then(|g| g.aspect_ratio),
        Some(AspectRatioIdc::Sar16x11)
    );
    let got_second = seen[1].as_ref().expect("the second tag survives");
    assert!(
        Arc::ptr_eq(got_second, &second),
        "and the next sample carries its own, different state"
    );
    assert!(
        !Arc::ptr_eq(got_first, got_second),
        "two geometries are two allocations"
    );
}
