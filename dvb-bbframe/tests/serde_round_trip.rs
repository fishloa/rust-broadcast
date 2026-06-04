//! Serde coverage for the dvb_bbframe types.
//!
//! Both [`Bbheader`] and [`Issy`] are owned (`Copy`) types with no borrowed
//! fields, so each is exercised with a full `to_string` -> `from_str` JSON
//! round-trip and `assert_eq!`.
#![cfg(feature = "serde")]

use dvb_bbframe::crc::crc8;
use dvb_bbframe::header::Bbheader;
use dvb_bbframe::issy::Issy;

#[test]
fn bbheader_round_trips_via_json() {
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
    let j = serde_json::to_string(&parsed).expect("serialize Bbheader");
    let back: Bbheader = serde_json::from_str(&j).expect("deserialize Bbheader");
    assert_eq!(parsed, back);
}

#[test]
fn issy_round_trips_via_json() {
    for issy in [
        Issy::IscrShort(0x1234),
        Issy::IscrLong(0x0003_FFFF),
        Issy::Signalling(0x0012_3456),
    ] {
        let j = serde_json::to_string(&issy).expect("serialize Issy");
        let back: Issy = serde_json::from_str(&j).expect("deserialize Issy");
        assert_eq!(issy, back);
    }
}
