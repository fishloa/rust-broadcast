//! Round-trip tests for RIST wire types: parse -> serialize -> byte-identical,
//! and construct -> serialize -> parse -> equal.

use broadcast_core::error::OutputBufferTooSmall;
use broadcast_core::{Parse, Serialize};
use rist_runtime::{
    Error, GenericNack, NackFci, PacketRange, RangeNack, ReportPart, RistReceiverCompound,
    RistSenderCompound, RttEcho, RttEchoKind,
};
use rtp_packet::rtcp::{ReceiverReport, ReportBlock, SenderReport};
use rtp_packet::{ExtendedSeqNo, Ssrc};

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
        padding: Vec::new(),
    };
    let bytes = nack.try_to_bytes().unwrap();
    let parsed = GenericNack::parse(&bytes).unwrap();
    assert_eq!(parsed, nack);
    assert_eq!(parsed.try_to_bytes().unwrap(), bytes);
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
        padding: Vec::new(),
    };
    let bytes = nack.try_to_bytes().unwrap();
    assert_eq!(bytes.len() % 4, 0);
    let parsed = GenericNack::parse(&bytes).unwrap();
    assert_eq!(parsed, nack);
    assert_eq!(parsed.try_to_bytes().unwrap(), bytes);
}

#[test]
fn generic_nack_empty_blp_round_trip() {
    let nack = GenericNack {
        ssrc_sender: 1,
        ssrc_media: 2,
        nacks: vec![NackFci { pid: 0, blp: 0 }],
        padding: Vec::new(),
    };
    let bytes = nack.try_to_bytes().unwrap();
    let parsed = GenericNack::parse(&bytes).unwrap();
    assert_eq!(parsed, nack);
    assert_eq!(parsed.try_to_bytes().unwrap(), bytes);
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
        padding: Vec::new(),
    };
    let bytes = rn.try_to_bytes().unwrap();
    assert_eq!(bytes.len() % 4, 0);
    let parsed = RangeNack::parse(&bytes).unwrap();
    assert_eq!(parsed, rn);
    assert_eq!(parsed.try_to_bytes().unwrap(), bytes);
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
        padding: Vec::new(),
    };
    let bytes = rn.try_to_bytes().unwrap();
    assert_eq!(bytes.len() % 4, 0);
    let parsed = RangeNack::parse(&bytes).unwrap();
    assert_eq!(parsed, rn);
    assert_eq!(parsed.try_to_bytes().unwrap(), bytes);
}

/// Neither NACK format requires an entry: an empty list rounds-trips as an
/// empty FCI/ranges list (the serializer emits exactly that), padded or not.
#[test]
fn zero_entry_nacks_round_trip() {
    let nack = GenericNack {
        ssrc_sender: 0x0102_0304,
        ssrc_media: 0x0506_0708,
        nacks: vec![],
        padding: Vec::new(),
    };
    let bytes = nack.try_to_bytes().unwrap();
    assert_eq!(bytes.len(), 12);
    assert_eq!(bytes[0..4], [0x81, 205, 0, 2]);
    assert_eq!(GenericNack::parse(&bytes).unwrap(), nack);
    assert_eq!(
        GenericNack::parse(&bytes).unwrap().try_to_bytes().unwrap(),
        bytes
    );

    let padded_nack = GenericNack {
        padding: vec![0, 0, 0],
        ..nack
    };
    let bytes = padded_nack.try_to_bytes().unwrap();
    assert_eq!(bytes.len(), 16);
    assert_eq!(bytes[2..4], [0, 3], "length field counts the padding word");
    assert_eq!(*bytes.last().unwrap(), 4, "count byte covers itself");
    assert_eq!(GenericNack::parse(&bytes).unwrap(), padded_nack);
    assert_eq!(
        GenericNack::parse(&bytes).unwrap().try_to_bytes().unwrap(),
        bytes
    );

    let range_nack = RangeNack {
        ssrc_media: 0x0506_0708,
        ranges: vec![],
        padding: Vec::new(),
    };
    let bytes = range_nack.try_to_bytes().unwrap();
    assert_eq!(bytes.len(), 12);
    assert_eq!(bytes[0..4], [0x80, 204, 0, 2]);
    assert_eq!(RangeNack::parse(&bytes).unwrap(), range_nack);
    assert_eq!(
        RangeNack::parse(&bytes).unwrap().try_to_bytes().unwrap(),
        bytes
    );

    let padded_range_nack = RangeNack {
        padding: vec![0, 0, 0],
        ..range_nack
    };
    let bytes = padded_range_nack.try_to_bytes().unwrap();
    assert_eq!(bytes.len(), 16);
    assert_eq!(bytes[2..4], [0, 3]);
    assert_eq!(*bytes.last().unwrap(), 4);
    assert_eq!(RangeNack::parse(&bytes).unwrap(), padded_range_nack);
    assert_eq!(
        RangeNack::parse(&bytes).unwrap().try_to_bytes().unwrap(),
        bytes
    );
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
        padding: Vec::new(),
    };
    let bytes = rn.try_to_bytes().unwrap();
    let parsed = RangeNack::parse(&bytes).unwrap();
    assert_eq!(parsed, rn);
    assert_eq!(parsed.try_to_bytes().unwrap(), bytes);
}

// ---------------------------------------------------------------------------
// RttEcho round-trips
// ---------------------------------------------------------------------------

/// TR-06-1 §5.2.6 padding is an opaque path-MTU probe field, so it must be
/// written even into a caller's dirty output buffer.
#[test]
fn rtt_echo_padding_reaches_a_dirty_output_buffer() {
    let echo = RttEcho {
        kind: RttEchoKind::Response,
        ssrc_media: 0x1234_5678,
        timestamp: 0xDEAD_BEEF_CAFE_BABE,
        processing_delay_us: 1_500,
        padding: vec![0xA5; 8],
    };
    let mut out = vec![0xFFu8; echo.serialized_len()];
    assert_eq!(echo.serialize_into(&mut out).unwrap(), 32);
    assert_eq!(out[24..32], [0xA5; 8]);
    assert_eq!(RttEcho::parse(&out).unwrap(), echo);
}

#[test]
fn rtt_echo_request_round_trip() {
    let echo = RttEcho {
        kind: RttEchoKind::Request,
        ssrc_media: 0xAABB_CC00,
        timestamp: 0x0102_0304_0506_0708,
        processing_delay_us: 0,
        padding: vec![],
    };
    let bytes = echo.try_to_bytes().unwrap();
    assert_eq!(bytes.len(), 24);
    assert_eq!(bytes.len() % 4, 0);
    let parsed = RttEcho::parse(&bytes).unwrap();
    assert_eq!(parsed, echo);
    assert_eq!(parsed.try_to_bytes().unwrap(), bytes);
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
    let bytes = echo.try_to_bytes().unwrap();
    assert_eq!(bytes.len(), 24);
    let parsed = RttEcho::parse(&bytes).unwrap();
    assert_eq!(parsed, echo);
    assert_eq!(parsed.try_to_bytes().unwrap(), bytes);
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
    let bytes = echo.try_to_bytes().unwrap();
    assert_eq!(bytes.len(), 24 + 20);
    assert_eq!(bytes.len() % 4, 0);
    let parsed = RttEcho::parse(&bytes).unwrap();
    assert_eq!(parsed, echo);
    assert_eq!(parsed.try_to_bytes().unwrap(), bytes);
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
    let bytes = echo.try_to_bytes().unwrap();
    let parsed = RttEcho::parse(&bytes).unwrap();
    assert_eq!(parsed.timestamp, u64::MAX);
    assert_eq!(parsed.try_to_bytes().unwrap(), bytes);
}

// ---------------------------------------------------------------------------
// Compound packet tests
// ---------------------------------------------------------------------------

#[test]
fn sender_compound_sr_sdes_round_trip() {
    let compound = RistSenderCompound {
        report: ReportPart::Sr(SenderReport::new(
            Ssrc::new(0x1122_3344),
            0xE0E1_E2E3,
            0x1020_3040,
            0x0009_0000,
            100,
            50_000,
            vec![],
        )),
        report_padding: vec![],
        cname: "sender@example.com".to_string(),
        rtt_echo: None,
        unknown: vec![],
    };
    let bytes = compound.try_to_bytes().unwrap();
    assert_eq!(bytes.len() % 4, 0);
    // First packet: SR (PT 200).
    assert_eq!(bytes[1], 200);
    let parsed = RistSenderCompound::parse(&bytes).unwrap();
    assert_eq!(parsed, compound);
    assert_eq!(parsed.try_to_bytes().unwrap(), bytes);
}

#[test]
fn sender_compound_with_rtt_echo_round_trip() {
    let compound = RistSenderCompound {
        report: ReportPart::Sr(SenderReport::new(
            Ssrc::new(0x1122_3344),
            0xE0E1_E2E3,
            0x1020_3040,
            0x0009_0000,
            100,
            50_000,
            vec![],
        )),
        report_padding: vec![],
        cname: "s@e.com".to_string(),
        rtt_echo: Some(RttEcho {
            kind: RttEchoKind::Request,
            ssrc_media: 0x1122_3344,
            timestamp: 12345,
            processing_delay_us: 0,
            padding: vec![],
        }),
        unknown: vec![],
    };
    let bytes = compound.try_to_bytes().unwrap();
    assert_eq!(bytes.len() % 4, 0);
    // Verify the compound contains at least 3 RTCP packets worth of data.
    assert!(bytes.len() > 24 + 12 + 24); // SR + minimal SDES + RTT Echo
    let parsed = RistSenderCompound::parse(&bytes).unwrap();
    assert_eq!(parsed, compound);
    assert_eq!(parsed.try_to_bytes().unwrap(), bytes);
}

#[test]
fn receiver_compound_rr_sdes_nack_round_trip() {
    let compound = RistReceiverCompound {
        report: ReportPart::Rr(ReceiverReport::new(
            Ssrc::new(0xAAAA_BBBB),
            vec![ReportBlock {
                ssrc: Ssrc::new(0xCCCC_DDDD),
                fraction_lost: 10,
                cumulative_lost: 5,
                ext_highest_seq: ExtendedSeqNo::new(0x0000_1000),
                jitter: 100,
                lsr: 0,
                dlsr: 0,
            }],
        )),
        report_padding: vec![],
        cname: "receiver@example.com".to_string(),
        nacks: vec![GenericNack {
            ssrc_sender: 0xAAAA_BBBB,
            ssrc_media: 0xCCCC_DDDD,
            nacks: vec![NackFci { pid: 500, blp: 0 }],
            padding: Vec::new(),
        }],
        range_nacks: vec![],
        rtt_echo: None,
        unknown: vec![],
    };
    let bytes = compound.try_to_bytes().unwrap();
    assert_eq!(bytes.len() % 4, 0);
    // First packet: RR (PT 201).
    assert_eq!(bytes[1], 201);
    let parsed = RistReceiverCompound::parse(&bytes).unwrap();
    assert_eq!(parsed, compound);
    assert_eq!(parsed.try_to_bytes().unwrap(), bytes);
}

#[test]
fn receiver_compound_empty_rr_round_trip() {
    let compound = RistReceiverCompound {
        report: ReportPart::Rr(ReceiverReport::new(Ssrc::new(0x0000_0001), vec![])),
        report_padding: vec![],
        cname: "r@e.com".to_string(),
        nacks: vec![],
        range_nacks: vec![],
        rtt_echo: None,
        unknown: vec![],
    };
    let bytes = compound.try_to_bytes().unwrap();
    assert_eq!(bytes.len() % 4, 0);
    assert_eq!(bytes[1], 201);
    let parsed = RistReceiverCompound::parse(&bytes).unwrap();
    assert_eq!(parsed, compound);
    assert_eq!(parsed.try_to_bytes().unwrap(), bytes);
}

#[test]
fn receiver_compound_full_round_trip() {
    // Exercises every optional slot at once: multiple Generic NACKs,
    // multiple Range NACKs, and a trailing RTT Echo — the parser must
    // classify and consume each in wire order.
    let compound = RistReceiverCompound {
        report: ReportPart::Rr(ReceiverReport::new(
            Ssrc::new(0xAAAA_BBBB),
            vec![ReportBlock {
                ssrc: Ssrc::new(0xCCCC_DDDD),
                fraction_lost: 10,
                cumulative_lost: 5,
                ext_highest_seq: ExtendedSeqNo::new(0x0000_1000),
                jitter: 100,
                lsr: 0,
                dlsr: 0,
            }],
        )),
        report_padding: vec![],
        cname: "full@example.com".to_string(),
        nacks: vec![
            GenericNack {
                ssrc_sender: 0xAAAA_BBBB,
                ssrc_media: 0xCCCC_DDDD,
                nacks: vec![NackFci { pid: 500, blp: 0 }],
                padding: Vec::new(),
            },
            GenericNack {
                ssrc_sender: 0xAAAA_BBBB,
                ssrc_media: 0xCCCC_DDDD,
                nacks: vec![NackFci {
                    pid: 700,
                    blp: 0x00FF,
                }],
                padding: Vec::new(),
            },
        ],
        range_nacks: vec![
            RangeNack {
                ssrc_media: 0xCCCC_DDDD,
                ranges: vec![PacketRange {
                    start: 100,
                    additional: 3,
                }],
                padding: Vec::new(),
            },
            RangeNack {
                ssrc_media: 0xCCCC_DDDD,
                ranges: vec![
                    PacketRange {
                        start: 200,
                        additional: 0,
                    },
                    PacketRange {
                        start: 250,
                        additional: 5,
                    },
                ],
                padding: Vec::new(),
            },
        ],
        rtt_echo: Some(RttEcho {
            kind: RttEchoKind::Response,
            ssrc_media: 0xCCCC_DDDD,
            timestamp: 0xDEAD_BEEF_CAFE_BABE,
            processing_delay_us: 1_234,
            padding: vec![0u8; 4],
        }),
        unknown: vec![],
    };
    let bytes = compound.try_to_bytes().unwrap();
    assert_eq!(bytes.len() % 4, 0);
    let parsed = RistReceiverCompound::parse(&bytes).unwrap();
    assert_eq!(parsed, compound);
    assert_eq!(parsed.nacks.len(), 2);
    assert_eq!(parsed.range_nacks.len(), 2);
    assert!(parsed.rtt_echo.is_some());
    assert_eq!(parsed.try_to_bytes().unwrap(), bytes);
}

/// `serialized_len()` must be exactly the number of bytes `serialize_into`
/// writes: a caller sizing a datagram from it sends a right-length packet,
/// and a buffer one byte shorter is rejected with the exact need/have pair.
fn assert_serialized_len_matches<T: Serialize<Error = rist_runtime::Error>>(
    value: &T,
    bytes: &[u8],
) {
    let len = value.serialized_len();
    assert_eq!(len, bytes.len());
    let mut buf = vec![0u8; len];
    assert_eq!(value.serialize_into(&mut buf).unwrap(), len);
    assert_eq!(&buf[..len], bytes);

    let mut short = vec![0u8; len - 1];
    assert_eq!(
        value.serialize_into(&mut short).unwrap_err(),
        Error::OutputBufferTooSmall(OutputBufferTooSmall {
            need: len,
            have: len - 1,
        })
    );
}

#[test]
fn compound_serialized_len_is_the_bytes_actually_written() {
    let sender = RistSenderCompound {
        report: ReportPart::Sr(SenderReport::new(
            Ssrc::new(0x1122_3344),
            0xE0E1_E2E3,
            0x1020_3040,
            0x0009_0000,
            100,
            50_000,
            vec![],
        )),
        report_padding: vec![],
        cname: "s@e.com".to_string(),
        rtt_echo: Some(RttEcho {
            kind: RttEchoKind::Request,
            ssrc_media: 0x1122_3344,
            timestamp: 12345,
            processing_delay_us: 0,
            padding: vec![],
        }),
        unknown: vec![],
    };
    let bytes = sender.try_to_bytes().unwrap();
    // SR 28 + SDES 20 + RTT Echo 24.
    assert_eq!(bytes.len(), 72);
    assert_serialized_len_matches(&sender, &bytes);

    let sender_with_report_padding = RistSenderCompound {
        report: ReportPart::Rr(ReceiverReport::new(Ssrc::new(0x1122_3344), vec![])),
        report_padding: vec![0, 0, 0],
        ..sender.clone()
    };
    let bytes = sender_with_report_padding.try_to_bytes().unwrap();
    // RR 8 + 4-byte padding region + SDES 20 + RTT Echo 24.
    assert_eq!(bytes.len(), 56);
    assert_serialized_len_matches(&sender_with_report_padding, &bytes);

    let receiver = RistReceiverCompound {
        report: ReportPart::Rr(ReceiverReport::new(
            Ssrc::new(0xAAAA_BBBB),
            vec![ReportBlock {
                ssrc: Ssrc::new(0xCCCC_DDDD),
                fraction_lost: 10,
                cumulative_lost: 5,
                ext_highest_seq: ExtendedSeqNo::new(0x0000_1000),
                jitter: 100,
                lsr: 0,
                dlsr: 0,
            }],
        )),
        report_padding: vec![],
        cname: "receiver@example.com".to_string(),
        nacks: vec![GenericNack {
            ssrc_sender: 0xAAAA_BBBB,
            ssrc_media: 0xCCCC_DDDD,
            nacks: vec![NackFci { pid: 500, blp: 0 }],
            padding: Vec::new(),
        }],
        range_nacks: vec![RangeNack {
            ssrc_media: 0xCCCC_DDDD,
            ranges: vec![PacketRange {
                start: 100,
                additional: 3,
            }],
            padding: Vec::new(),
        }],
        rtt_echo: Some(RttEcho {
            kind: RttEchoKind::Response,
            ssrc_media: 0xCCCC_DDDD,
            timestamp: 0xDEAD_BEEF_CAFE_BABE,
            processing_delay_us: 1_234,
            padding: vec![0u8; 4],
        }),
        unknown: vec![],
    };
    let bytes = receiver.try_to_bytes().unwrap();
    assert_serialized_len_matches(&receiver, &bytes);
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
        padding: Vec::new(),
    };
    let mut bytes = nack.try_to_bytes().unwrap();
    // PID is at offset 12-13.
    bytes[12] ^= 0xFF;
    let mutated = GenericNack::parse(&bytes).unwrap();
    assert_ne!(mutated.nacks[0].pid, 1000);
    assert_eq!(mutated.try_to_bytes().unwrap(), bytes);
}

#[test]
fn range_nack_mutation_bites_additional() {
    let rn = RangeNack {
        ssrc_media: 1,
        ranges: vec![PacketRange {
            start: 100,
            additional: 10,
        }],
        padding: Vec::new(),
    };
    let mut bytes = rn.try_to_bytes().unwrap();
    // additional is at offset 14-15.
    bytes[14] ^= 0xFF;
    let mutated = RangeNack::parse(&bytes).unwrap();
    assert_ne!(mutated.ranges[0].additional, 10);
    assert_eq!(mutated.try_to_bytes().unwrap(), bytes);
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
    let mut bytes = echo.try_to_bytes().unwrap();
    // processing_delay_us is at offset 20-23.
    bytes[20] ^= 0xFF;
    let mutated = RttEcho::parse(&bytes).unwrap();
    assert_ne!(mutated.processing_delay_us, 42_000);
    assert_eq!(mutated.try_to_bytes().unwrap(), bytes);
}
