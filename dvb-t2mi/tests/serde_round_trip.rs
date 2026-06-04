//! Serde coverage for the dvb_t2mi types.
//!
//! Owned types (no `&'a [u8]` fields) are exercised with a full
//! `to_string` -> `from_str` JSON round-trip and `assert_eq!`.
//!
//! Borrowed types cannot do a true JSON round-trip: JSON encodes `&[u8]`
//! as an array of integers, which is not contiguous in the wire form and
//! therefore cannot be borrowed back during deserialize. For those, this
//! file constructs the struct directly, calls `serde_json::to_string`,
//! and verifies the output is valid JSON containing the wire-relevant
//! top-level fields. Round-trip via a borrowing-friendly format
//! (postcard, bincode) is a separate concern.
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

// --- Owned types: full JSON round-trip ------------------------------------

#[test]
fn header_round_trips_via_json() {
    let h = Header {
        packet_type: PacketType::BasebandFrame,
        packet_count: 0x12,
        superframe_idx: 3,
        t2mi_stream_id: 5,
        payload_len_bits: 1024,
    };
    let j = serde_json::to_string(&h).expect("serialize Header");
    let back: Header = serde_json::from_str(&j).expect("deserialize Header");
    assert_eq!(h, back);
}

#[test]
fn t2_timestamp_payload_round_trips_via_json() {
    let ts = T2TimestampPayload {
        bw: Bandwidth::Mhz8,
        seconds_since_2000: 0x01_02_03_04_05,
        subseconds: 0x0007_FFFF,
        utco: 0x0123,
    };
    let j = serde_json::to_string(&ts).expect("serialize T2TimestampPayload");
    let back: T2TimestampPayload =
        serde_json::from_str(&j).expect("deserialize T2TimestampPayload");
    assert_eq!(ts, back);
}

// --- Borrowed types: serialize emits valid JSON ---------------------------

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
