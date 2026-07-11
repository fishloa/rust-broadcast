#![no_main]

use broadcast_common::Parse;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = st291::AncDataPacket::parse(data);
    let _ = st291::AncDataDescriptor::parse(data);
    // RFC 8331 / ST 2110-40 ANC-over-RTP (issue #648): both the bare payload
    // parser (starting at Extended Sequence Number) and the full
    // RtpPacket+AncRtpPayload composition, since the two have independent
    // buffer-slicing logic (payload_type/fixed-header vs. payload-header).
    let _ = st291::AncRtpPayload::parse(data);
    let _ = st291::AncRtpPayload::parse_rtp_packet(data);
});
