//! Bug-audit D reproductions (dash-runtime).

use dash_runtime::address::base_index;
use transmux::SegmentIndexBox;
use transmux::timing::SidxReference;

fn reference(size: u32, duration: u32) -> SidxReference {
    SidxReference {
        reference_type: 0,
        referenced_size: size,
        subsegment_duration: duration,
        starts_with_sap: 1,
        sap_type: 0,
        sap_delta_time: 0,
    }
}

/// A zero-size sidx reference still occupies `subsegment_duration` of the
/// timeline: the segment after it starts at earliest_presentation_time + 5.
#[test]
fn sidx_zero_size_reference_still_advances_the_timeline() {
    let sidx = SegmentIndexBox {
        version: 0,
        flags: 0,
        reference_id: 1,
        timescale: 1,
        earliest_presentation_time: 0,
        first_offset: 0,
        references: vec![reference(0, 5), reference(10, 5)],
    };
    let out = base_index::segments(&sidx, 100).unwrap();
    assert_eq!(out.len(), 1);
    assert_eq!(
        out[0].time, 5,
        "the second reference follows 5 ticks of the zero-size one"
    );
}
