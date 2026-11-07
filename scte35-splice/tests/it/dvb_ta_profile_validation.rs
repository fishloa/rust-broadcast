//! `validate_po_segmentation` (ETSI TS 103 752-1 §5.3.5): one descriptor per
//! `ProfileViolation` outcome, checked in the order the function evaluates them.

use scte35_splice::descriptors::{SegmentationDescriptor, SegmentationTypeId, SegmentationUpidType};
use scte35_splice::dvb_ta::{ProfileViolation, validate_po_segmentation};

fn po(type_id: SegmentationTypeId) -> SegmentationDescriptor<'static> {
    SegmentationDescriptor {
        segmentation_upid_type: SegmentationUpidType::Uri,
        segmentation_upid: b"urn:example:po",
        segmentation_type_id: type_id,
        segmentation_duration: Some(90_000 * 30),
        ..Default::default()
    }
}

#[test]
fn a_conforming_start_and_end_pass() {
    for id in [
        SegmentationTypeId::ProviderPlacementOpportunityStart,
        SegmentationTypeId::DistributorPlacementOpportunityStart,
    ] {
        assert_eq!(validate_po_segmentation(&po(id)), ProfileViolation::Ok);
    }
    let mut end = po(SegmentationTypeId::ProviderPlacementOpportunityEnd);
    end.segmentation_duration = None;
    assert_eq!(validate_po_segmentation(&end), ProfileViolation::Ok);
}

#[test]
fn a_cancelled_event_is_reported_first() {
    let mut d = po(SegmentationTypeId::ProviderPlacementOpportunityStart);
    d.segmentation_event_cancel_indicator = true;
    d.segmentation_upid_type = SegmentationUpidType::AdId;
    d.segmentation_duration = None;
    assert_eq!(
        validate_po_segmentation(&d),
        ProfileViolation::EventCancelled
    );

    let mut other = SegmentationDescriptor {
        segmentation_type_id: SegmentationTypeId::ProgramStart,
        ..Default::default()
    };
    other.segmentation_event_cancel_indicator = true;
    assert_eq!(
        validate_po_segmentation(&other),
        ProfileViolation::EventCancelled
    );
}

#[test]
fn a_po_with_a_non_uri_upid_type_is_reported_before_a_missing_duration() {
    let mut d = po(SegmentationTypeId::DistributorPlacementOpportunityStart);
    d.segmentation_upid_type = SegmentationUpidType::AdId;
    d.segmentation_duration = None;
    assert_eq!(
        validate_po_segmentation(&d),
        ProfileViolation::UpidTypeNotUri
    );
}

#[test]
fn a_start_without_a_duration_is_reported() {
    for id in [
        SegmentationTypeId::ProviderPlacementOpportunityStart,
        SegmentationTypeId::DistributorPlacementOpportunityStart,
    ] {
        let mut d = po(id);
        d.segmentation_duration = None;
        assert_eq!(
            validate_po_segmentation(&d),
            ProfileViolation::DurationMissing,
            "{id:?}"
        );
    }
}

#[test]
fn non_po_types_are_only_checked_for_cancellation() {
    let d = SegmentationDescriptor {
        segmentation_type_id: SegmentationTypeId::ProgramStart,
        segmentation_upid_type: SegmentationUpidType::AdId,
        segmentation_duration: None,
        ..Default::default()
    };
    assert_eq!(validate_po_segmentation(&d), ProfileViolation::Ok);
}
