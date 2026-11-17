//! `name()`/`Display` labels for the SCTE 35 and DVB-TA spec/field enums
//! (the workspace #204 convention): every token is asserted literally, so a
//! label cannot silently become empty or a placeholder.

use scte35_splice::commands::{ProgramSplice, SpliceMode, SpliceInsertComponent};
use scte35_splice::descriptors::segmentation_enums::{
    DeviceRestrictions, SegmentationTypeId, SegmentationUpidType,
};
use scte35_splice::dvb_ta::{
    DasNibbleOrder, EquivalentSegmentationType, PlacementOpportunity, ProfileViolation, TimelineType,
};
use scte35_splice::time::SpliceTime;

#[test]
fn splice_mode_labels_are_the_table_10_tokens() {
    let at = ProgramSplice::At(SpliceTime::immediate());
    assert_eq!(ProgramSplice::Immediate.name(), "immediate");
    assert_eq!(at.name(), "at_splice_time");
    assert_eq!(std::format!("{}", ProgramSplice::Immediate), "immediate");

    assert_eq!(
        SpliceMode::Program(ProgramSplice::Immediate).name(),
        "program_immediate"
    );
    assert_eq!(SpliceMode::Program(at).name(), "program_at");
    assert_eq!(
        SpliceMode::Components {
            immediate: false,
            components: vec![SpliceInsertComponent::new(1, Some(SpliceTime::immediate()))],
        }
        .name(),
        "components_at"
    );
    assert_eq!(
        SpliceMode::Components {
            immediate: true,
            components: vec![SpliceInsertComponent::new(1, None)],
        }
        .name(),
        "components_immediate"
    );
    assert_eq!(
        std::format!("{}", SpliceMode::Program(ProgramSplice::Immediate)),
        "program_immediate"
    );
}

#[test]
fn device_restriction_labels_are_the_table_21_tokens() {
    let cases = [
        (DeviceRestrictions::RestrictGroup0, "restricted for device group 0"),
        (DeviceRestrictions::RestrictGroup1, "restricted for device group 1"),
        (DeviceRestrictions::RestrictGroup2, "restricted for device group 2"),
        (DeviceRestrictions::None, "no device restrictions"),
    ];
    for (value, label) in cases {
        assert_eq!(value.name(), label);
        assert_eq!(std::format!("{value}"), label);
    }
}

#[test]
fn every_known_segmentation_type_id_has_its_table_23_label() {
    for raw in 0..=u8::MAX {
        let id = SegmentationTypeId::from_u8(raw);
        if matches!(id, SegmentationTypeId::Reserved(_)) {
            assert_eq!(id.name(), "reserved");
            continue;
        }
        assert_eq!(id.to_u8(), raw);
        assert!(!id.name().is_empty(), "{id:?}");
        assert_ne!(id.name(), "reserved", "{id:?}");
        assert_eq!(std::format!("{id}"), id.name());
    }

    assert_eq!(
        SegmentationTypeId::from_u8(0x34).name(),
        "Provider Placement Opportunity Start"
    );
    assert_eq!(
        SegmentationTypeId::from_u8(0x00).name(),
        "Not Indicated"
    );
    assert_eq!(SegmentationTypeId::from_u8(0x10).name(), "Program Start");
    assert_eq!(SegmentationTypeId::from_u8(0x11).name(), "Program End");
    assert_eq!(
        SegmentationTypeId::from_u8(0x30).name(),
        "Provider Advertisement Start"
    );
}

#[test]
fn every_known_segmentation_upid_type_has_its_table_22_label() {
    for raw in 0..=u8::MAX {
        let upid = SegmentationUpidType::from_u8(raw);
        if matches!(upid, SegmentationUpidType::Reserved(_)) {
            assert_eq!(upid.name(), "reserved");
            continue;
        }
        assert_eq!(upid.to_u8(), raw);
        assert!(!upid.name().is_empty(), "{upid:?}");
        assert_ne!(upid.name(), "reserved", "{upid:?}");
        assert_eq!(std::format!("{upid}"), upid.name());
    }
    assert_eq!(SegmentationUpidType::from_u8(0x03).name(), "Ad-ID");
    assert_eq!(SegmentationUpidType::from_u8(0x0D).name(), "MID()");
}

#[test]
fn dvb_ta_enum_labels_are_their_spec_tokens() {
    let equivalents = [
        (EquivalentSegmentationType::NoEquivalent, "no equivalent"),
        (
            EquivalentSegmentationType::DistributorPlacementOpportunity,
            "Distributor Placement Opportunity (DPO)",
        ),
        (
            EquivalentSegmentationType::ProviderPlacementOpportunity,
            "Provider Placement Opportunity (PPO)",
        ),
        (
            EquivalentSegmentationType::DistributorAdvertisement,
            "Distributor Advertisement (DA)",
        ),
        (
            EquivalentSegmentationType::ProviderAdvertisement,
            "Provider Advertisement (PA)",
        ),
        (EquivalentSegmentationType::Reserved(0xEE), "reserved"),
    ];
    for (value, label) in equivalents {
        assert_eq!(value.name(), label);
    }
    assert_eq!(
        std::format!("{}", EquivalentSegmentationType::Reserved(0xEE)),
        "reserved(0xEE)"
    );
    assert_eq!(
        std::format!("{}", EquivalentSegmentationType::NoEquivalent),
        "no equivalent"
    );

    assert_eq!(
        DasNibbleOrder::TypeHigh.name(),
        "type high / reserved low (Table 8)"
    );
    assert_eq!(
        DasNibbleOrder::TypeLow.name(),
        "reserved high / type low (Table 1)"
    );
    assert_eq!(
        std::format!("{}", DasNibbleOrder::TypeLow),
        "reserved high / type low (Table 1)"
    );

    let placements = [
        (
            PlacementOpportunity::ProviderStart,
            "Provider Placement Opportunity Start",
        ),
        (
            PlacementOpportunity::ProviderEnd,
            "Provider Placement Opportunity End",
        ),
        (
            PlacementOpportunity::DistributorStart,
            "Distributor Placement Opportunity Start",
        ),
        (
            PlacementOpportunity::DistributorEnd,
            "Distributor Placement Opportunity End",
        ),
    ];
    for (value, label) in placements {
        assert_eq!(value.name(), label);
        assert_eq!(std::format!("{value}"), label);
    }

    let violations = [
        (ProfileViolation::Ok, "ok"),
        (
            ProfileViolation::EventCancelled,
            "segmentation_event_cancel_indicator set (§5.3.5.3)",
        ),
        (
            ProfileViolation::DurationMissing,
            "segmentation_duration_flag clear on a Start (§5.3.5.5)",
        ),
        (
            ProfileViolation::UpidTypeNotUri,
            "segmentation_upid_type is not 0x0F URI (§5.3.5.10)",
        ),
    ];
    for (value, label) in violations {
        assert_eq!(value.name(), label);
        assert_eq!(std::format!("{value}"), label);
    }

    let timelines = [
        (TimelineType::None, "no timeline used"),
        (TimelineType::VideoPts, "PTS references video PTS"),
        (TimelineType::Temi, "PTS references TEMI timeline"),
        (TimelineType::Reserved(0x0E), "reserved"),
    ];
    for (value, label) in timelines {
        assert_eq!(value.name(), label);
    }
    assert_eq!(
        std::format!("{}", TimelineType::Reserved(0x0E)),
        "reserved(0x0E)"
    );
    assert_eq!(std::format!("{}", TimelineType::VideoPts), "PTS references video PTS");
}
