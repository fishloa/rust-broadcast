//! Advanced: parse a `splice_info_section`, walk its descriptor loop, and
//! prove the serializer is byte-exact (the project's round-trip invariant).
//!
//! Run with: `cargo run -p scte35-splice --example round_trip_and_descriptors`

use dvb_common::{Parse, Serialize};
use scte35_splice::SpliceInfoSection;

#[rustfmt::skip]
const SPLICE_INSERT: [u8; 50] = [
    0xFC, 0x30, 0x2F, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0xF0, 0x14, 0x05, 0x48,
    0x00, 0x00, 0x8F, 0x7F, 0xEF, 0xFE, 0x73, 0x69, 0xC0, 0x2E, 0xFE, 0x00, 0x52, 0xCC, 0xF5,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x0A, 0x00, 0x08, 0x43, 0x55, 0x45, 0x49, 0x00, 0x00, 0x01,
    0x35, 0x62, 0xDB, 0xA3, 0x0A,
];

fn main() {
    let s = SpliceInfoSection::parse(&SPLICE_INSERT).expect("CRC + parse");

    let mut n = 0;
    for d in s.descriptors() {
        n += 1;
        match d {
            Ok(desc) => println!("descriptor #{n}: {desc:?}"),
            Err(e) => println!("descriptor #{n}: <malformed: {e}>"),
        }
    }
    println!("{n} splice descriptor(s) in the loop");

    // Round-trip: re-serialize and require byte-identical output (CRC included).
    let bytes = s.to_bytes();
    assert_eq!(bytes.len(), s.serialized_len());
    assert_eq!(
        bytes, SPLICE_INSERT,
        "serialize must reproduce the input exactly"
    );
    println!("round-trip: {} bytes, byte-identical ✔", bytes.len());
}
