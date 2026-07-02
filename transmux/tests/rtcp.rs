//! Integration tests for the RTCP control-packet codec (RFC 3550 §6).
//!
//! Verifies Parse/Serialize symmetry, wire-layout correctness, compound-packet
//! framing + the §6.1 leading-report rule, mutation-bites (field decode, not
//! passthrough), and the RC boundary case.

use broadcast_common::{Parse, Serialize};
use transmux::rtcp::{
    App, Bye, CompoundPacket, ReceiverReport, ReportBlock, RtcpPacket, RtcpPacketType, SdesChunk,
    SdesItem, SdesItemType, SenderReport, SourceDescription, PT_APP, PT_BYE, PT_RECEIVER_REPORT,
    PT_SENDER_REPORT, PT_SOURCE_DESCRIPTION, REPORT_BLOCK_LEN,
};

fn block(ssrc: u32, jitter: u32, cumulative: i32) -> ReportBlock {
    ReportBlock {
        ssrc,
        fraction_lost: 7,
        cumulative_lost: cumulative,
        ext_highest_seq: 0x0000_ABCD,
        jitter,
        lsr: 0x1234_5678,
        dlsr: 0x0000_0100,
    }
}

fn sr_two_blocks() -> SenderReport {
    SenderReport {
        ssrc: 0x1122_3344,
        ntp_msw: 0xE1E2_E3E4,
        ntp_lsw: 0x5060_7080,
        rtp_timestamp: 0x000A_0000,
        packet_count: 12345,
        octet_count: 6_789_012,
        report_blocks: vec![block(0xAAAA_0001, 111, 42), block(0xBBBB_0002, 222, -9)],
    }
}

// --- Gate 1: round-trip each type from fields, both directions. ------------

#[test]
fn round_trip_sr_rr_sdes_bye_app() {
    // SR with 2 report blocks.
    let sr = sr_two_blocks();
    let sr_bytes = sr.to_bytes();
    assert_eq!(SenderReport::parse(&sr_bytes).unwrap(), sr);
    assert_eq!(SenderReport::parse(&sr_bytes).unwrap().to_bytes(), sr_bytes);

    // RR.
    let rr = ReceiverReport {
        ssrc: 0x0A0B_0C0D,
        report_blocks: vec![block(0xCCCC_0003, 33, -1)],
    };
    let rr_bytes = rr.to_bytes();
    assert_eq!(ReceiverReport::parse(&rr_bytes).unwrap(), rr);
    assert_eq!(
        ReceiverReport::parse(&rr_bytes).unwrap().to_bytes(),
        rr_bytes
    );

    // SDES: CNAME + TOOL.
    let sdes = SourceDescription {
        chunks: vec![SdesChunk {
            source: 0x1357_9BDF,
            items: vec![
                SdesItem {
                    item_type: SdesItemType::CName,
                    text: "bob@host.example".to_string(),
                },
                SdesItem {
                    item_type: SdesItemType::Tool,
                    text: "transmux".to_string(),
                },
            ],
        }],
    };
    let sdes_bytes = sdes.to_bytes();
    assert_eq!(SourceDescription::parse(&sdes_bytes).unwrap(), sdes);
    assert_eq!(
        SourceDescription::parse(&sdes_bytes).unwrap().to_bytes(),
        sdes_bytes
    );

    // BYE with a reason.
    let bye = Bye {
        sources: vec![0x1111_2222, 0x3333_4444],
        reason: Some("session over".to_string()),
    };
    let bye_bytes = bye.to_bytes();
    assert_eq!(Bye::parse(&bye_bytes).unwrap(), bye);
    assert_eq!(Bye::parse(&bye_bytes).unwrap().to_bytes(), bye_bytes);

    // APP.
    let app = App {
        subtype: 5,
        ssrc: 0xFEED_FACE,
        name: *b"TEST",
        data: vec![1, 2, 3, 4, 5, 6, 7, 8],
    };
    let app_bytes = app.to_bytes();
    assert_eq!(App::parse(&app_bytes).unwrap(), app);
    assert_eq!(App::parse(&app_bytes).unwrap().to_bytes(), app_bytes);
}

// --- Gate 2: wire-layout correctness + negative cumulative_lost. -----------

#[test]
fn sr_wire_layout() {
    let sr = sr_two_blocks();
    let bytes = sr.to_bytes();
    // V=2 in the top 2 bits.
    assert_eq!(bytes[0] >> 6, 2, "version must be 2");
    // RC=2 in the low 5 bits.
    assert_eq!(bytes[0] & 0x1F, 2, "RC must be 2");
    // PT byte == 200.
    assert_eq!(bytes[1], PT_SENDER_REPORT);
    assert_eq!(bytes[1], 200);
    // length == total 32-bit words − 1.
    let words = bytes.len() / 4;
    assert_eq!(
        u16::from_be_bytes([bytes[2], bytes[3]]) as usize,
        words - 1,
        "length field is words − 1"
    );
}

#[test]
fn report_block_negative_cumulative_lost() {
    for v in [-1_i32, -100, -0x80_0000, 0, 1, 0x7F_FFFF] {
        let b = block(0x1, 0, v);
        let parsed = ReportBlock::parse(&b.to_bytes()).unwrap();
        assert_eq!(
            parsed.cumulative_lost, v,
            "24-bit signed round-trip for {v}"
        );
    }
}

// --- Gate 3: compound packet + §6.1 leading-report rule. -------------------

#[test]
fn compound_sr_then_sdes() {
    let sdes = SourceDescription {
        chunks: vec![SdesChunk {
            source: 0x1122_3344,
            items: vec![SdesItem {
                item_type: SdesItemType::CName,
                text: "x".to_string(),
            }],
        }],
    };
    let cp = CompoundPacket::new(vec![
        RtcpPacket::SenderReport(sr_two_blocks()),
        RtcpPacket::SourceDescription(sdes),
    ])
    .unwrap();
    let bytes = cp.to_bytes();
    let parsed = CompoundPacket::parse(&bytes).unwrap();
    assert_eq!(parsed.packets.len(), 2);
    assert_eq!(parsed, cp);
    assert_eq!(parsed.to_bytes(), bytes);
    assert_eq!(
        parsed.packets[0].packet_type(),
        RtcpPacketType::SenderReport
    );
    assert_eq!(
        parsed.packets[1].packet_type(),
        RtcpPacketType::SourceDescription
    );
}

#[test]
fn compound_not_starting_with_report_rejected() {
    // Construction rejects a non-report leader.
    assert!(CompoundPacket::new(vec![RtcpPacket::App(App {
        subtype: 0,
        ssrc: 1,
        name: *b"NOPE",
        data: vec![],
    })])
    .is_err());

    // Parse rejects it too: serialize a lone BYE and parse as compound.
    let bye_bytes = Bye {
        sources: vec![0xDEAD_BEEF],
        reason: None,
    }
    .to_bytes();
    assert!(CompoundPacket::parse(&bye_bytes).is_err());
}

// --- Gate 4: mutation bites (proves field decode, not passthrough). --------

#[test]
fn mutating_packet_count_bites() {
    let sr = sr_two_blocks();
    let mut bytes = sr.to_bytes();
    let orig = SenderReport::parse(&bytes).unwrap().packet_count;
    // packet_count is at 4 (header) + 16 = offset 20.
    bytes[20] ^= 0xFF;
    let mutated = SenderReport::parse(&bytes).unwrap();
    assert_ne!(mutated.packet_count, orig);
    // The decoded (mutated) value re-serializes to exactly the mutated bytes.
    assert_eq!(mutated.to_bytes(), bytes);
}

#[test]
fn mutating_jitter_bites() {
    let mut sr = sr_two_blocks();
    let before = sr.to_bytes();
    sr.report_blocks[0].jitter = sr.report_blocks[0].jitter.wrapping_add(0x0100_0000);
    let after = sr.to_bytes();
    assert_ne!(before, after, "changing jitter changes the bytes");
    let parsed = SenderReport::parse(&after).unwrap();
    assert_eq!(parsed.report_blocks[0].jitter, sr.report_blocks[0].jitter);
}

// --- Gate 5: boundary — RC=2, two report blocks, length matches size. ------

#[test]
fn sr_rc_two_boundary() {
    let sr = sr_two_blocks();
    assert_eq!(sr.report_blocks.len(), 2);
    let bytes = sr.to_bytes();
    // header(4) + ssrc+sender-info(24) + 2 report blocks(48) = 76 bytes.
    assert_eq!(bytes.len(), 4 + 24 + 2 * REPORT_BLOCK_LEN);
    // RC field is 2.
    assert_eq!(bytes[0] & 0x1F, 2);
    // length field == actual size in words − 1.
    assert_eq!(
        u16::from_be_bytes([bytes[2], bytes[3]]) as usize,
        bytes.len() / 4 - 1
    );
    // Both blocks parse back.
    let parsed = SenderReport::parse(&bytes).unwrap();
    assert_eq!(parsed.report_blocks.len(), 2);
    assert_eq!(parsed.report_blocks[1].cumulative_lost, -9);
}

// --- PT constants sanity. --------------------------------------------------

#[test]
fn pt_constants() {
    assert_eq!(PT_SENDER_REPORT, 200);
    assert_eq!(PT_RECEIVER_REPORT, 201);
    assert_eq!(PT_SOURCE_DESCRIPTION, 202);
    assert_eq!(PT_BYE, 203);
    assert_eq!(PT_APP, 204);
}
