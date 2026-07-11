//! Build an RTCP Sender Report with two reception report blocks from typed
//! fields and serialize it to wire bytes — RFC 3550 §6.4.1.
//!
//! Run with `cargo run -p rtcp-packet --example build_sender_report`.

use broadcast_common::Serialize;
use rtcp_packet::{ReportBlock, SenderReport};

fn main() {
    let sr = SenderReport {
        ssrc: 0x1122_3344,
        ntp_msw: 0xE0E1_E2E3,
        ntp_lsw: 0x1020_3040,
        rtp_timestamp: 0x0009_0000,
        packet_count: 4321,
        octet_count: 999_999,
        report_blocks: vec![
            ReportBlock {
                ssrc: 0xAAAA_AAAA,
                fraction_lost: 12,
                cumulative_lost: 17,
                ext_highest_seq: 0x0001_2345,
                jitter: 500,
                lsr: 0xAABB_CCDD,
                dlsr: 0x0000_1000,
            },
            ReportBlock {
                ssrc: 0xBBBB_BBBB,
                fraction_lost: 0,
                cumulative_lost: -3, // negative: duplicates exceeded losses (§6.4.1)
                ext_highest_seq: 0x0002_0000,
                jitter: 750,
                lsr: 0,
                dlsr: 0,
            },
        ],
    };

    let mut bytes = vec![0u8; sr.serialized_len()];
    sr.serialize_into(&mut bytes).expect("serialize");

    println!("serialized {} bytes:", bytes.len());
    println!(
        "{}",
        bytes
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<Vec<_>>()
            .join(" ")
    );
    println!(
        "V={} P={} RC={} PT={}",
        bytes[0] >> 6,
        (bytes[0] >> 5) & 1,
        bytes[0] & 0x1F,
        bytes[1]
    );
}
