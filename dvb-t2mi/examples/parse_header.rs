//! Basic: parse a T2-MI packet header from raw bytes.
//!
//! Run with: `cargo run -p dvb-t2mi --example parse_header`

use dvb_common::Parse;
use dvb_t2mi::packet::Header;

fn main() {
    // A 6-byte T2-MI packet header: packet_type, packet_count, superframe_idx,
    // reserved, and a 16-bit payload length in bits (here: 0).
    let bytes = [0x00u8, 0x00, 0x00, 0x00, 0x00, 0x00];

    let hdr = Header::parse(&bytes).expect("valid T2-MI header");

    println!("packet_type     : {:?}", hdr.packet_type);
    println!("packet_count    : {}", hdr.packet_count);
    println!("superframe_idx  : {}", hdr.superframe_idx);
    println!("payload_len_bits: {}", hdr.payload_len_bits);
}
