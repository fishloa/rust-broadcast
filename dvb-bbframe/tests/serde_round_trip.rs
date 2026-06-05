//! Serde coverage for the dvb_bbframe types.
//!
//! Serde is **Serialize-only** (3.0.1): JSON is a display/export format and
//! parsing FROM JSON is deliberately unsupported — re-parse from wire bytes.
//! Each type is serialized and its shape asserted.
#![cfg(feature = "serde")]

use dvb_bbframe::crc::crc8;
use dvb_bbframe::header::Bbheader;
use dvb_bbframe::issy::Issy;

#[test]
fn bbheader_serializes_to_valid_json() {
    // Build a valid 10-byte Normal-Mode BBHEADER. byte 9 must equal
    // crc8(bytes[0..9]) so mode detection yields NM (crc ^ stored == 0).
    let mut bytes: [u8; 10] = [
        0xC0, // MATYPE-1: TS/GS=TS (0b11), all other flags 0
        0x00, // MATYPE-2: ISI=0
        0x05, 0xC0, // UPL = 1472 bits (188-byte UP)
        0x17, 0x00, // DFL
        0x47, // SYNC byte
        0x00, 0x00, // SYNCD
        0x00, // CRC placeholder, filled below
    ];
    bytes[9] = crc8(&bytes[..9]);

    let parsed = Bbheader::parse(&bytes).expect("parse BBHEADER");
    let v = serde_json::to_value(parsed).expect("serialize Bbheader");
    assert!(v.is_object(), "expected JSON object, got {v}");
    assert_eq!(v["dfl"], 0x1700);
    assert_eq!(v["sync"], 0x47);
}

#[test]
fn issy_serializes_to_valid_json() {
    // Each Issy variant serializes as an externally-tagged enum object.
    let cases = [
        (Issy::IscrShort(0x1234), "IscrShort"),
        (Issy::IscrLong(0x0003_FFFF), "IscrLong"),
        (Issy::Signalling(0x0012_3456), "Signalling"),
    ];
    for (issy, tag) in cases {
        let v = serde_json::to_value(issy).expect("serialize Issy");
        assert!(v.get(tag).is_some(), "missing variant tag {tag} in {v}");
    }
}
