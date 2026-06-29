//! Basic: parse a real `splice_info_section` carrying a `splice_insert()`.
//!
//! Run with: `cargo run -p scte35-splice --example parse_splice_insert`
//!
//! Bytes are a well-known SCTE 35 reference vector (a `splice_insert` with
//! event_id 0x4800008F), inlined so the example is self-contained.

use broadcast_common::Parse;
use scte35_splice::{commands::AnyCommand, SpliceInfoSection};

#[rustfmt::skip]
const SPLICE_INSERT: [u8; 50] = [
    0xFC, 0x30, 0x2F, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0xF0, 0x14, 0x05, 0x48,
    0x00, 0x00, 0x8F, 0x7F, 0xEF, 0xFE, 0x73, 0x69, 0xC0, 0x2E, 0xFE, 0x00, 0x52, 0xCC, 0xF5,
    0x00, 0x00, 0x00, 0x00, 0x00, 0x0A, 0x00, 0x08, 0x43, 0x55, 0x45, 0x49, 0x00, 0x00, 0x01,
    0x35, 0x62, 0xDB, 0xA3, 0x0A,
];

fn main() {
    let s = SpliceInfoSection::parse(&SPLICE_INSERT).expect("CRC + parse");

    println!("protocol_version : {}", s.protocol_version);
    println!("pts_adjustment   : {}", s.pts_adjustment);
    println!("tier             : {:#05X}", s.tier);

    let clear = s.clear.as_ref().expect("an unencrypted (clear) section");
    match &clear.command {
        AnyCommand::SpliceInsert(si) => {
            println!("command          : splice_insert");
            println!("splice_event_id  : {:#010X}", si.splice_event_id);
        }
        other => println!("command          : {other:?}"),
    }
}
