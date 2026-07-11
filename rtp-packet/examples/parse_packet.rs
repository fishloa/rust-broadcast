//! Parse the committed real-capture fixture (`tests/fixtures/rtp_simple.bin`)
//! and print its decoded RTP fields, then round-trip it back to bytes and
//! confirm the output is byte-identical — RFC 3550 §5.1.
//!
//! Run with `cargo run -p rtp-packet --example parse_packet`.

use broadcast_common::{Parse, Serialize};
use rtp_packet::RtpPacket;

fn main() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/rtp_simple.bin");
    let bytes = std::fs::read(path).expect("fixture must exist");

    let pkt = RtpPacket::parse(&bytes).expect("parse RTP fixed header");
    println!("marker:          {}", pkt.marker);
    println!("payload_type:    {}", pkt.payload_type);
    println!("sequence_number: {}", pkt.sequence_number);
    println!("timestamp:       {}", pkt.timestamp);
    println!("ssrc:            {:#010x}", pkt.ssrc);
    println!("csrc count:      {}", pkt.csrc_count());
    println!("extension:       {}", pkt.extension.is_some());
    println!("padding:         {}", pkt.padding.is_some());
    println!("payload len:     {}", pkt.payload.len());

    let mut out = vec![0u8; pkt.serialized_len()];
    pkt.serialize_into(&mut out).expect("serialize");
    assert_eq!(out, bytes, "byte-identical round trip");
    println!("round trip byte-identical: OK ({} bytes)", out.len());
}
