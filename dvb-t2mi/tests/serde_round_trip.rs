//! Serde coverage for the dvb_t2mi types.
//!
//! Serde is **Serialize-only** (3.0.1): JSON is a display/export format and
//! parsing FROM JSON is deliberately unsupported — re-parse from wire bytes.
//! Each type is serialized and its shape asserted (valid JSON with the
//! wire-relevant fields).
#![cfg(feature = "serde")]

use dvb_t2mi::packet::{Header, PacketType};
use dvb_t2mi::payload::{
    Bandwidth, BbframePayload, FefSubPartPayload, IndividualAddressingPayload, SubpartVariety,
    T2TimestampPayload,
};

fn assert_valid_json_with_keys(j: &str, keys: &[&str]) {
    let v: serde_json::Value = serde_json::from_str(j).expect("emitted JSON parses");
    assert!(v.is_object(), "expected JSON object, got {j}");
    for k in keys {
        assert!(v.get(k).is_some(), "missing key {k} in {j}");
    }
}

// --- Types: serialize emits valid JSON ------------------------------------

#[test]
fn header_serializes_to_valid_json() {
    let h = Header {
        packet_type: PacketType::BasebandFrame,
        packet_count: 0x12,
        superframe_idx: 3,
        t2mi_stream_id: 5,
        payload_len_bits: 1024,
    };
    let v = serde_json::to_value(h).expect("serialize Header");
    assert!(v.is_object(), "expected JSON object, got {v}");
    assert_eq!(v["packet_count"], 0x12);
    assert_eq!(v["payload_len_bits"], 1024);
}

#[test]
fn t2_timestamp_payload_serializes_to_valid_json() {
    let ts = T2TimestampPayload {
        bw: Bandwidth::Mhz8,
        seconds_since_2000: 0x01_02_03_04_05,
        subseconds: 0x0007_FFFF,
        utco: 0x0123,
    };
    let j = serde_json::to_string(&ts).expect("serialize T2TimestampPayload");
    assert_valid_json_with_keys(&j, &["bw", "seconds_since_2000", "subseconds", "utco"]);
}

#[test]
fn bbframe_payload_serializes_to_valid_json() {
    let bb = BbframePayload {
        frame_idx: 1,
        plp_id: 2,
        intl_frame_start: true,
        bbframe: &[0x01, 0x02, 0x03],
    };
    let j = serde_json::to_string(&bb).expect("serialize BbframePayload");
    assert_valid_json_with_keys(&j, &["frame_idx", "plp_id", "bbframe"]);
}

#[test]
fn individual_addressing_payload_serializes_to_valid_json() {
    let ia = IndividualAddressingPayload {
        rfu: 0,
        individual_addressing_data: &[0x10, 0x20, 0x30],
    };
    let j = serde_json::to_string(&ia).expect("serialize IndividualAddressingPayload");
    assert_valid_json_with_keys(&j, &["rfu", "individual_addressing_data"]);
}

#[test]
fn fef_subpart_payload_serializes_to_valid_json() {
    let fef = FefSubPartPayload {
        fef_idx: 1,
        tx_identifier: 0x0001,
        subpart_idx: 0x0002,
        subpart_variety: SubpartVariety::Iq,
        subpart_length: 32,
        subpart_data: &[0xAA, 0xBB],
    };
    let j = serde_json::to_string(&fef).expect("serialize FefSubPartPayload");
    assert_valid_json_with_keys(&j, &["fef_idx", "subpart_variety", "subpart_data"]);
}
