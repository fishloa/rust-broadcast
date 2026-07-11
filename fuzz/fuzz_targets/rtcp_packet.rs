#![no_main]

use broadcast_common::Parse;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Every RFC 3550 §6 packet type has independent buffer-slicing logic
    // (sender-info length, report-block count, SDES chunk/item walking, BYE
    // reason length, APP data), plus the dispatch enum and the compound
    // packet's multi-packet framing loop.
    let _ = rtcp_packet::RtcpPacket::parse(data);
    let _ = rtcp_packet::SenderReport::parse(data);
    let _ = rtcp_packet::ReceiverReport::parse(data);
    let _ = rtcp_packet::SourceDescription::parse(data);
    let _ = rtcp_packet::Bye::parse(data);
    let _ = rtcp_packet::App::parse(data);
    let _ = rtcp_packet::CompoundPacket::parse(data);
});
