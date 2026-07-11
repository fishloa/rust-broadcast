//! Build a compound RTCP packet (SR followed by an SDES CNAME chunk, the
//! canonical §6.1 "session report" shape), parse it back, and confirm a
//! byte-exact round trip — RFC 3550 §6.1.
//!
//! Run with `cargo run -p rtcp-packet --example parse_compound_packet`.

use broadcast_common::{Parse, Serialize};
use rtcp_packet::{
    CompoundPacket, ReportBlock, RtcpPacket, SdesChunk, SdesItem, SdesItemType, SenderReport,
    SourceDescription,
};

fn main() {
    let sr = SenderReport {
        ssrc: 0x1122_3344,
        ntp_msw: 0xE1E2_E3E4,
        ntp_lsw: 0x5060_7080,
        rtp_timestamp: 0x000A_0000,
        packet_count: 12345,
        octet_count: 6_789_012,
        report_blocks: vec![ReportBlock {
            ssrc: 0xAAAA_0001,
            fraction_lost: 0,
            cumulative_lost: 0,
            ext_highest_seq: 0x0000_ABCD,
            jitter: 111,
            lsr: 0x1234_5678,
            dlsr: 0x0000_0100,
        }],
    };
    let sdes = SourceDescription {
        chunks: vec![SdesChunk {
            source: 0x1122_3344, // same SSRC as the SR: one participant, two reports
            items: vec![SdesItem {
                item_type: SdesItemType::CName,
                text: "alice@example.com".to_string(),
            }],
        }],
    };

    let compound = CompoundPacket::new(vec![
        RtcpPacket::SenderReport(sr),
        RtcpPacket::SourceDescription(sdes),
    ])
    .expect("SR-first compound packet");

    let mut bytes = vec![0u8; compound.serialized_len()];
    compound.serialize_into(&mut bytes).expect("serialize");
    println!("serialized {} bytes (SR + SDES)", bytes.len());

    let parsed = CompoundPacket::parse(&bytes).expect("parse");
    for (i, pkt) in parsed.packets.iter().enumerate() {
        println!("  packet {i}: {} ({:?})", pkt.name(), pkt.packet_type());
    }

    let mut out = vec![0u8; parsed.serialized_len()];
    parsed.serialize_into(&mut out).expect("re-serialize");
    assert_eq!(out, bytes, "byte-identical round trip");
    println!("round trip byte-identical: OK ({} bytes)", out.len());
}
