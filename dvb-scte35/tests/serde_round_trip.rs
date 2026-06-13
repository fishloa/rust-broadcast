//! serde serialization coverage for the Serialize-only path (the workspace
//! posture: no Deserialize). Asserts the JSON shape of a parsed section and
//! that the typed command / descriptors surface under camelCase variant keys.

#![cfg(feature = "serde")]

use base64::{engine::general_purpose::STANDARD, Engine};
use dvb_common::Parse;
use dvb_scte35::SpliceInfoSection;

#[test]
fn splice_insert_section_serializes_to_json() {
    let bytes = STANDARD
        .decode("/DAvAAAAAAAA///wFAVIAACPf+/+c2nALv4AUsz1AAAAAAAKAAhDVUVJAAABNWLbowo=")
        .unwrap();
    let s = SpliceInfoSection::parse(&bytes).unwrap();
    let v = serde_json::to_value(&s).unwrap();

    assert_eq!(v["protocol_version"], 0);
    assert_eq!(v["encrypted_packet"], false);
    assert_eq!(v["pts_adjustment"], 0);
    // The clear payload's command serializes under the camelCase variant key.
    let cmd = &v["clear"]["command"];
    assert!(cmd.get("spliceInsert").is_some(), "got {cmd}");
    assert_eq!(cmd["spliceInsert"]["splice_event_id"], 0x4800_008Fu32);
    assert_eq!(cmd["spliceInsert"]["out_of_network_indicator"], true);
}

#[test]
fn segmentation_section_serializes_descriptors() {
    // ANSI/SCTE 35 2023r1 §14.1 — time_signal + segmentation_descriptor
    // (Placement Opportunity Start). Base64 verbatim from the spec; CRC validates.
    let bytes = STANDARD
        .decode("/DA0AAAAAAAA///wBQb+cr0AUAAeAhxDVUVJSAAAjn/PAAGlmbAICAAAAAAsoKGKNAIAmsnRfg==")
        .unwrap();
    let s = SpliceInfoSection::parse(&bytes).unwrap();

    // The section itself serializes (time_signal command).
    let v = serde_json::to_value(&s).unwrap();
    assert!(v["clear"]["command"].get("timeSignal").is_some());

    // Each typed descriptor from the loop serializes under its variant key.
    let descs: Vec<_> = s.descriptors().map(Result::unwrap).collect();
    let dv = serde_json::to_value(&descs[0]).unwrap();
    assert!(dv.get("segmentation").is_some(), "got {dv}");
    // segmentation_event_id per §14.1 decoded breakdown.
    assert_eq!(dv["segmentation"]["segmentation_event_id"], 0x4800_008Eu32);
    // upid_type per §14.1 = Ti (0x08).
    assert_eq!(dv["segmentation"]["segmentation_upid_type"], "Ti");
}

#[test]
fn enums_serialize_as_names() {
    use dvb_scte35::descriptors::{SegmentationTypeId, SegmentationUpidType};
    assert_eq!(
        serde_json::to_value(SegmentationTypeId::ProviderPlacementOpportunityStart).unwrap(),
        serde_json::json!("ProviderPlacementOpportunityStart")
    );
    assert_eq!(
        serde_json::to_value(SegmentationUpidType::Mpu).unwrap(),
        serde_json::json!("Mpu")
    );
    // Reserved values carry the raw byte.
    assert_eq!(
        serde_json::to_value(SegmentationTypeId::Reserved(0x99)).unwrap(),
        serde_json::json!({ "Reserved": 0x99 })
    );
}
