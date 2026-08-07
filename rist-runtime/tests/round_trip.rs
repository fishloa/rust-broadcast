//! Round-trip tests for RIST wire types: parse -> serialize -> byte-identical,
//! and construct -> serialize -> parse -> equal.

use broadcast_common::{Parse, Serialize};
use rist_runtime::{
    GenericNack, NackFci, PacketRange, RangeNack, RistReceiverCompound, RistSenderCompound,
    RttEcho, RttEchoKind,
};
use rtcp_packet::{ReceiverReport, ReportBlock, SenderReport};

// ---------------------------------------------------------------------------
// GenericNack round-trips
// ---------------------------------------------------------------------------

#[test]
fn generic_nack_single_fci_round_trip() {
    let nack = GenericNack {
        ssrc_sender: 0x0102_0304,
        ssrc_media: 0x0506_0708,
        nacks: vec![NackFci {
            pid: 42,
            blp: 0xABCD,
        }],
    };
    let bytes = nack.to_bytes();
    let parsed = GenericNack::parse(&bytes).unwrap();
    assert_eq!(parsed, nack);
    assert_eq!(parsed.to_bytes(), bytes);
}

#[test]
fn generic_nack_multiple_fci_round_trip() {
    let nack = GenericNack {
        ssrc_sender: 0xDEAD_BEEF,
        ssrc_media: 0xCAFE_BABE,
        nacks: vec![
            NackFci {
                pid: 100,
                blp: 0xFFFC,
            },
            NackFci {
                pid: 117,
                blp: 0x003F,
            },
            NackFci {
                pid: 200,
                blp: 0x0000,
            },
        ],
    };
    let bytes = nack.to_bytes();
    assert_eq!(bytes.len() % 4, 0);
    let parsed = GenericNack::parse(&bytes).unwrap();
    assert_eq!(parsed, nack);
    assert_eq!(parsed.to_bytes(), bytes);
}

#[test]
fn generic_nack_empty_blp_round_trip() {
    let nack = GenericNack {
        ssrc_sender: 1,
        ssrc_media: 2,
        nacks: vec![NackFci { pid: 0, blp: 0 }],
    };
    let bytes = nack.to_bytes();
    let parsed = GenericNack::parse(&bytes).unwrap();
    assert_eq!(parsed, nack);
    assert_eq!(parsed.to_bytes(), bytes);
}

// ---------------------------------------------------------------------------
// RangeNack round-trips
// ---------------------------------------------------------------------------

#[test]
fn range_nack_single_range_round_trip() {
    let rn = RangeNack {
        ssrc_media: 0xAABB_CC00,
        ranges: vec![PacketRange {
            start: 100,
            additional: 0,
        }],
    };
    let bytes = rn.to_bytes();
    assert_eq!(bytes.len() % 4, 0);
    let parsed = RangeNack::parse(&bytes).unwrap();
    assert_eq!(parsed, rn);
    assert_eq!(parsed.to_bytes(), bytes);
}

#[test]
fn range_nack_multiple_ranges_round_trip() {
    let rn = RangeNack {
        ssrc_media: 0x1234_5678,
        ranges: vec![
            PacketRange {
                start: 100,
                additional: 0,
            },
            PacketRange {
                start: 103,
                additional: 19,
            },
            PacketRange {
                start: 500,
                additional: 3,
            },
        ],
    };
    let bytes = rn.to_bytes();
    assert_eq!(bytes.len() % 4, 0);
    let parsed = RangeNack::parse(&bytes).unwrap();
    assert_eq!(parsed, rn);
    assert_eq!(parsed.to_bytes(), bytes);
}

#[test]
fn range_nack_max_entries_round_trip() {
    let rn = RangeNack {
        ssrc_media: 0xFFFF_FFFF,
        ranges: (0..16)
            .map(|i| PacketRange {
                start: i * 100,
                additional: i,
            })
            .collect(),
    };
    let bytes = rn.to_bytes();
    let parsed = RangeNack::parse(&bytes).unwrap();
    assert_eq!(parsed, rn);
    assert_eq!(parsed.to_bytes(), bytes);
}

// ---------------------------------------------------------------------------
// RttEcho round-trips
// ---------------------------------------------------------------------------

#[test]
fn rtt_echo_request_round_trip() {
    let echo = RttEcho {
        kind: RttEchoKind::Request,
        ssrc_media: 0xAABB_CC00,
        timestamp: 0x0102_0304_0506_0708,
        processing_delay_us: 0,
        padding: vec![],
    };
    let bytes = echo.to_bytes();
    assert_eq!(bytes.len(), 24);
    assert_eq!(bytes.len() % 4, 0);
    let parsed = RttEcho::parse(&bytes).unwrap();
    assert_eq!(parsed, echo);
    assert_eq!(parsed.to_bytes(), bytes);
}

#[test]
fn rtt_echo_response_round_trip() {
    let echo = RttEcho {
        kind: RttEchoKind::Response,
        ssrc_media: 0x1234_5678,
        timestamp: 0xDEAD_BEEF_CAFE_BABE,
        processing_delay_us: 42_000,
        padding: vec![],
    };
    let bytes = echo.to_bytes();
    assert_eq!(bytes.len(), 24);
    let parsed = RttEcho::parse(&bytes).unwrap();
    assert_eq!(parsed, echo);
    assert_eq!(parsed.to_bytes(), bytes);
}

#[test]
fn rtt_echo_response_with_padding_round_trip() {
    let echo = RttEcho {
        kind: RttEchoKind::Response,
        ssrc_media: 0x1234_5678,
        timestamp: 0xDEAD_BEEF_CAFE_BABE,
        processing_delay_us: 1_500,
        padding: vec![0u8; 20], // 5 extra words of padding
    };
    let bytes = echo.to_bytes();
    assert_eq!(bytes.len(), 24 + 20);
    assert_eq!(bytes.len() % 4, 0);
    let parsed = RttEcho::parse(&bytes).unwrap();
    assert_eq!(parsed, echo);
    assert_eq!(parsed.to_bytes(), bytes);
}

#[test]
fn rtt_echo_large_timestamp_round_trip() {
    let echo = RttEcho {
        kind: RttEchoKind::Request,
        ssrc_media: 0,
        timestamp: u64::MAX,
        processing_delay_us: 0,
        padding: vec![],
    };
    let bytes = echo.to_bytes();
    let parsed = RttEcho::parse(&bytes).unwrap();
    assert_eq!(parsed.timestamp, u64::MAX);
    assert_eq!(parsed.to_bytes(), bytes);
}

// ---------------------------------------------------------------------------
// Compound packet tests
// ---------------------------------------------------------------------------

#[test]
fn sender_compound_sr_sdes_round_trip() {
    let compound = RistSenderCompound {
        sr: SenderReport {
            ssrc: 0x1122_3344,
            ntp_msw: 0xE0E1_E2E3,
            ntp_lsw: 0x1020_3040,
            rtp_timestamp: 0x0009_0000,
            packet_count: 100,
            octet_count: 50_000,
            report_blocks: vec![],
        },
        cname: "sender@example.com".to_string(),
        rtt_echo: None,
    };
    let bytes = compound.to_bytes();
    assert_eq!(bytes.len() % 4, 0);
    // First packet: SR (PT 200).
    assert_eq!(bytes[1], 200);
}

#[test]
fn sender_compound_with_rtt_echo() {
    let compound = RistSenderCompound {
        sr: SenderReport {
            ssrc: 0x1122_3344,
            ntp_msw: 0xE0E1_E2E3,
            ntp_lsw: 0x1020_3040,
            rtp_timestamp: 0x0009_0000,
            packet_count: 100,
            octet_count: 50_000,
            report_blocks: vec![],
        },
        cname: "s@e.com".to_string(),
        rtt_echo: Some(RttEcho {
            kind: RttEchoKind::Request,
            ssrc_media: 0x1122_3344,
            timestamp: 12345,
            processing_delay_us: 0,
            padding: vec![],
        }),
    };
    let bytes = compound.to_bytes();
    assert_eq!(bytes.len() % 4, 0);
    // Verify the compound contains at least 3 RTCP packets worth of data.
    assert!(bytes.len() > 24 + 12 + 24); // SR + minimal SDES + RTT Echo
}

#[test]
fn receiver_compound_rr_sdes_nack() {
    let compound = RistReceiverCompound {
        rr: ReceiverReport {
            ssrc: 0xAAAA_BBBB,
            report_blocks: vec![ReportBlock {
                ssrc: 0xCCCC_DDDD,
                fraction_lost: 10,
                cumulative_lost: 5,
                ext_highest_seq: 0x0000_1000,
                jitter: 100,
                lsr: 0,
                dlsr: 0,
            }],
        },
        cname: "receiver@example.com".to_string(),
        nacks: vec![GenericNack {
            ssrc_sender: 0xAAAA_BBBB,
            ssrc_media: 0xCCCC_DDDD,
            nacks: vec![NackFci { pid: 500, blp: 0 }],
        }],
        range_nacks: vec![],
        rtt_echo: None,
    };
    let bytes = compound.to_bytes();
    assert_eq!(bytes.len() % 4, 0);
    // First packet: RR (PT 201).
    assert_eq!(bytes[1], 201);
}

#[test]
fn receiver_compound_empty_rr() {
    let compound = RistReceiverCompound {
        rr: ReceiverReport {
            ssrc: 0x0000_0001,
            report_blocks: vec![],
        },
        cname: "r@e.com".to_string(),
        nacks: vec![],
        range_nacks: vec![],
        rtt_echo: None,
    };
    let bytes = compound.to_bytes();
    assert_eq!(bytes.len() % 4, 0);
    assert_eq!(bytes[1], 201);
}

// ---------------------------------------------------------------------------
// Mutation tests — verify parsers actually decode each field
// ---------------------------------------------------------------------------

#[test]
fn generic_nack_mutation_bites_pid() {
    let nack = GenericNack {
        ssrc_sender: 1,
        ssrc_media: 2,
        nacks: vec![NackFci {
            pid: 1000,
            blp: 0xAAAA,
        }],
    };
    let mut bytes = nack.to_bytes();
    // PID is at offset 12-13.
    bytes[12] ^= 0xFF;
    let mutated = GenericNack::parse(&bytes).unwrap();
    assert_ne!(mutated.nacks[0].pid, 1000);
    assert_eq!(mutated.to_bytes(), bytes);
}

#[test]
fn range_nack_mutation_bites_additional() {
    let rn = RangeNack {
        ssrc_media: 1,
        ranges: vec![PacketRange {
            start: 100,
            additional: 10,
        }],
    };
    let mut bytes = rn.to_bytes();
    // additional is at offset 14-15.
    bytes[14] ^= 0xFF;
    let mutated = RangeNack::parse(&bytes).unwrap();
    assert_ne!(mutated.ranges[0].additional, 10);
    assert_eq!(mutated.to_bytes(), bytes);
}

#[test]
fn rtt_echo_mutation_bites_processing_delay() {
    let echo = RttEcho {
        kind: RttEchoKind::Response,
        ssrc_media: 1,
        timestamp: 0,
        processing_delay_us: 42_000,
        padding: vec![],
    };
    let mut bytes = echo.to_bytes();
    // processing_delay_us is at offset 20-23.
    bytes[20] ^= 0xFF;
    let mutated = RttEcho::parse(&bytes).unwrap();
    assert_ne!(mutated.processing_delay_us, 42_000);
    assert_eq!(mutated.to_bytes(), bytes);
}
