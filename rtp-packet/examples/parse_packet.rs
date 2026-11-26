//! Parse the committed real-capture fixture (`../fixtures/rtp/rtp-simple.bin`)
//! and print its decoded RTP fields, then round-trip it back to bytes and
//! confirm the output is byte-identical — RFC 3550 §5.1.
//!
//! Run with `cargo run -p rtp-packet --example parse_packet`.

use broadcast_common::bounded::{Limit, read_file_bounded};
use broadcast_core::{Parse, Serialize};
use rtp_packet::RtpPacket;

/// One RTP packet: its 16-bit length field caps the datagram under 64 KiB.
const RTP_PACKET_LIMIT: Limit = Limit::new("RTP packet", 64 * 1024);

fn main() {
    let path = test_fixtures::path("rtp/rtp-simple.bin");
    let bytes = read_file_bounded(&path, RTP_PACKET_LIMIT).expect("fixture must exist");

    let pkt = RtpPacket::parse(&bytes).expect("parse RTP fixed header");
    println!("marker:          {}", pkt.marker);
    println!("payload_type:    {}", pkt.payload_type);
    println!("sequence_number: {}", pkt.sequence_number.get());
    println!("timestamp:       {}", pkt.timestamp);
    println!("ssrc:            {}", pkt.ssrc);
    println!("csrc count:      {}", pkt.csrc_count());
    println!("extension:       {}", pkt.extension.is_some());
    println!("padding:         {}", pkt.padding.is_some());
    println!("payload len:     {}", pkt.payload.len());

    let mut out = vec![0u8; pkt.serialized_len()];
    pkt.serialize_into(&mut out).expect("serialize");
    assert_eq!(out, bytes, "byte-identical round trip");
    println!("round trip byte-identical: OK ({} bytes)", out.len());
}
