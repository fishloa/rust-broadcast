//! Spec-derived test vectors — VSF TR-06-1:2020, Appendix A.
//!
//! Scenario: a RIST receiver at 192.168.1.10:3000 watches a stream with
//! SSRC `0xAABBCC00`. It misses packet 100, receives 101 and 102, then
//! misses the contiguous block 103..=122 (20 packets).
//!
//! Both NACK formats must express this same loss pattern.

use broadcast_common::{Parse, Serialize};
use rist_runtime::{GenericNack, NackFci, PT_RTPFB, PacketRange, RIST_APP_NAME_U32, RangeNack};

/// The scenario's media SSRC.
const SSRC_MEDIA: u32 = 0xAABB_CC00;
/// The receiver's SSRC (arbitrary for this test).
const SSRC_SENDER: u32 = 0x0A00_0001;

// ---------------------------------------------------------------------------
// Bitmask NACK (Generic NACK, PT 205, FMT 1)
// ---------------------------------------------------------------------------
//
// Lost packets: 100, 103..=122.
//
// FCI 1: PID = 100
//   bit 1 (PID+1=101) = 0 (received)
//   bit 2 (PID+2=102) = 0 (received)
//   bit 3 (PID+3=103) = 1 (lost)
//   ...
//   bit 16 (PID+16=116) = 1 (lost)
//   BLP = bits 3..=16 set = 0b1111_1111_1111_1100 = 0xFFFC
//
// FCI 2: PID = 117
//   bit 1 (117+1=118) = 1 (lost)
//   bit 2 (117+2=119) = 1 (lost)
//   bit 3 (117+3=120) = 1 (lost)
//   bit 4 (117+4=121) = 1 (lost)
//   bit 5 (117+5=122) = 1 (lost)
//   bits 6..=16 = 0 (not lost)
//   BLP = 0b0000_0000_0001_1111 = 0x001F

#[test]
fn appendix_a_bitmask_nack() {
    let nack = GenericNack {
        ssrc_sender: SSRC_SENDER,
        ssrc_media: SSRC_MEDIA,
        nacks: vec![
            NackFci {
                pid: 100,
                blp: 0xFFFC,
            },
            NackFci {
                pid: 117,
                blp: 0x001F,
            },
        ],
    };

    let bytes = nack.to_bytes();

    // --- header checks ---
    // V=2, P=0, FMT=1 -> byte 0 = 0b10_0_00001 = 0x81
    assert_eq!(bytes[0], 0x81);
    // PT = 205
    assert_eq!(bytes[1], PT_RTPFB);
    // length = (total_bytes/4) - 1 = (20/4) - 1 = 4
    assert_eq!(u16::from_be_bytes([bytes[2], bytes[3]]), 4);
    // Total packet = 20 bytes (header 4 + SSRC sender 4 + SSRC media 4 + 2*FCI 8)
    assert_eq!(bytes.len(), 20);

    // --- SSRC fields ---
    assert_eq!(
        u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
        SSRC_SENDER
    );
    assert_eq!(
        u32::from_be_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]),
        SSRC_MEDIA
    );

    // --- FCI 1: PID=100, BLP=0xFFFC ---
    assert_eq!(u16::from_be_bytes([bytes[12], bytes[13]]), 100);
    assert_eq!(u16::from_be_bytes([bytes[14], bytes[15]]), 0xFFFC);

    // --- FCI 2: PID=117, BLP=0x001F ---
    assert_eq!(u16::from_be_bytes([bytes[16], bytes[17]]), 117);
    assert_eq!(u16::from_be_bytes([bytes[18], bytes[19]]), 0x001F);

    // --- round-trip ---
    let parsed = GenericNack::parse(&bytes).unwrap();
    assert_eq!(parsed, nack);
    assert_eq!(parsed.to_bytes(), bytes);
}

/// Verify the bitmask NACK covers exactly the right lost sequence numbers.
#[test]
fn appendix_a_bitmask_nack_loss_set() {
    let nack = GenericNack {
        ssrc_sender: SSRC_SENDER,
        ssrc_media: SSRC_MEDIA,
        nacks: vec![
            NackFci {
                pid: 100,
                blp: 0xFFFC,
            },
            NackFci {
                pid: 117,
                blp: 0x001F,
            },
        ],
    };

    // Expand the NACK into a set of lost sequence numbers.
    let mut lost = Vec::new();
    for fci in &nack.nacks {
        lost.push(fci.pid);
        for bit in 0..16u16 {
            if fci.blp & (1 << bit) != 0 {
                lost.push(fci.pid + bit + 1);
            }
        }
    }
    lost.sort();
    lost.dedup();

    // Expected: 100, 103..=122 (21 packets total).
    let mut expected: Vec<u16> = vec![100];
    expected.extend(103..=122);
    assert_eq!(lost, expected);
}

// ---------------------------------------------------------------------------
// Range NACK (RIST APP, PT 204, Subtype 0)
// ---------------------------------------------------------------------------
//
// Same loss pattern expressed as ranges:
// Range 1: Start=100, Additional=0   (just packet 100)
// Range 2: Start=103, Additional=19  (packets 103..=122 inclusive)

#[test]
fn appendix_a_range_nack() {
    let rn = RangeNack {
        ssrc_media: SSRC_MEDIA,
        ranges: vec![
            PacketRange {
                start: 100,
                additional: 0,
            },
            PacketRange {
                start: 103,
                additional: 19,
            },
        ],
    };

    let bytes = rn.to_bytes();

    // --- header checks ---
    // V=2, P=0, Subtype=0 -> byte 0 = 0b10_0_00000 = 0x80
    assert_eq!(bytes[0], 0x80);
    // PT = 204
    assert_eq!(bytes[1], 204);
    // length = (total_bytes/4) - 1 = (20/4) - 1 = 4
    assert_eq!(u16::from_be_bytes([bytes[2], bytes[3]]), 4);
    // Total packet = 20 bytes (header 4 + SSRC 4 + name 4 + 2*range 8)
    assert_eq!(bytes.len(), 20);

    // --- SSRC ---
    assert_eq!(
        u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
        SSRC_MEDIA
    );

    // --- APP name = "RIST" ---
    assert_eq!(
        u32::from_be_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]),
        RIST_APP_NAME_U32
    );
    assert_eq!(&bytes[8..12], b"RIST");

    // --- Range 1: Start=100, Additional=0 ---
    assert_eq!(u16::from_be_bytes([bytes[12], bytes[13]]), 100);
    assert_eq!(u16::from_be_bytes([bytes[14], bytes[15]]), 0);

    // --- Range 2: Start=103, Additional=19 ---
    assert_eq!(u16::from_be_bytes([bytes[16], bytes[17]]), 103);
    assert_eq!(u16::from_be_bytes([bytes[18], bytes[19]]), 19);

    // --- round-trip ---
    let parsed = RangeNack::parse(&bytes).unwrap();
    assert_eq!(parsed, rn);
    assert_eq!(parsed.to_bytes(), bytes);
}

/// Verify the range NACK covers exactly the same loss set as the bitmask NACK.
#[test]
fn appendix_a_range_nack_loss_set() {
    let rn = RangeNack {
        ssrc_media: SSRC_MEDIA,
        ranges: vec![
            PacketRange {
                start: 100,
                additional: 0,
            },
            PacketRange {
                start: 103,
                additional: 19,
            },
        ],
    };

    // Expand ranges into a set of lost sequence numbers.
    let mut lost: Vec<u16> = Vec::new();
    for range in &rn.ranges {
        for i in 0..=range.additional {
            lost.push(range.start + i);
        }
    }
    lost.sort();
    lost.dedup();

    // Expected: 100, 103..=122 (21 packets total).
    let mut expected: Vec<u16> = vec![100];
    expected.extend(103..=122);
    assert_eq!(lost, expected);
}

/// Cross-verify: both NACK formats produce the same loss set for the
/// Appendix A scenario.
#[test]
fn appendix_a_cross_verify_both_formats() {
    // Bitmask NACK loss set.
    let bitmask_nack = GenericNack {
        ssrc_sender: SSRC_SENDER,
        ssrc_media: SSRC_MEDIA,
        nacks: vec![
            NackFci {
                pid: 100,
                blp: 0xFFFC,
            },
            NackFci {
                pid: 117,
                blp: 0x001F,
            },
        ],
    };
    let mut bitmask_lost = Vec::new();
    for fci in &bitmask_nack.nacks {
        bitmask_lost.push(fci.pid);
        for bit in 0..16u16 {
            if fci.blp & (1 << bit) != 0 {
                bitmask_lost.push(fci.pid + bit + 1);
            }
        }
    }
    bitmask_lost.sort();
    bitmask_lost.dedup();

    // Range NACK loss set.
    let range_nack = RangeNack {
        ssrc_media: SSRC_MEDIA,
        ranges: vec![
            PacketRange {
                start: 100,
                additional: 0,
            },
            PacketRange {
                start: 103,
                additional: 19,
            },
        ],
    };
    let mut range_lost: Vec<u16> = Vec::new();
    for range in &range_nack.ranges {
        for i in 0..=range.additional {
            range_lost.push(range.start + i);
        }
    }
    range_lost.sort();
    range_lost.dedup();

    assert_eq!(
        bitmask_lost, range_lost,
        "both NACK formats must express the same loss set"
    );
}

// ---------------------------------------------------------------------------
// Hand-crafted wire bytes — parse direction
// ---------------------------------------------------------------------------

/// Parse a hand-crafted Generic NACK from known bytes.
#[test]
fn parse_handcrafted_generic_nack() {
    #[rustfmt::skip]
    let bytes: &[u8] = &[
        0x81,                   // V=2, P=0, FMT=1
        205,                    // PT=205
        0x00, 0x03,             // length=3 -> 16 bytes total (header + 2*SSRC + 1*FCI)
        0x00, 0x00, 0x00, 0x01, // SSRC sender
        0x00, 0x00, 0x00, 0x02, // SSRC media
        0x00, 0x64,             // PID=100
        0xFF, 0xFC,             // BLP=0xFFFC
    ];
    let nack = GenericNack::parse(bytes).unwrap();
    assert_eq!(nack.ssrc_sender, 1);
    assert_eq!(nack.ssrc_media, 2);
    assert_eq!(nack.nacks.len(), 1);
    assert_eq!(nack.nacks[0].pid, 100);
    assert_eq!(nack.nacks[0].blp, 0xFFFC);
    // Byte-exact round-trip.
    assert_eq!(nack.to_bytes(), bytes);
}

/// Parse a hand-crafted Range NACK from known bytes.
#[test]
fn parse_handcrafted_range_nack() {
    #[rustfmt::skip]
    let bytes: &[u8] = &[
        0x80,                   // V=2, P=0, Subtype=0
        204,                    // PT=204
        0x00, 0x03,             // length=3 -> 16 bytes total
        0xAA, 0xBB, 0xCC, 0x00, // SSRC media
        0x52, 0x49, 0x53, 0x54, // name = "RIST"
        0x00, 0x67,             // start=103
        0x00, 0x13,             // additional=19
    ];
    let rn = RangeNack::parse(bytes).unwrap();
    assert_eq!(rn.ssrc_media, 0xAABB_CC00);
    assert_eq!(rn.ranges.len(), 1);
    assert_eq!(rn.ranges[0].start, 103);
    assert_eq!(rn.ranges[0].additional, 19);
    assert_eq!(rn.to_bytes(), bytes);
}

/// Parse a hand-crafted RTT Echo Request from known bytes.
#[test]
fn parse_handcrafted_rtt_echo_request() {
    #[rustfmt::skip]
    let bytes: &[u8] = &[
        0x82,                   // V=2, P=0, Subtype=2 (Request)
        204,                    // PT=204
        0x00, 0x05,             // length=5 -> 24 bytes total
        0x12, 0x34, 0x56, 0x78, // SSRC media
        0x52, 0x49, 0x53, 0x54, // name = "RIST"
        0x00, 0x00, 0x00, 0x01, // timestamp MSW
        0x00, 0x00, 0x00, 0x02, // timestamp LSW
        0x00, 0x00, 0x00, 0x00, // processing delay = 0
    ];
    let echo = rist_runtime::RttEcho::parse(bytes).unwrap();
    assert_eq!(echo.kind, rist_runtime::RttEchoKind::Request);
    assert_eq!(echo.ssrc_media, 0x1234_5678);
    assert_eq!(echo.timestamp, 0x0000_0001_0000_0002);
    assert_eq!(echo.processing_delay_us, 0);
    assert!(echo.padding.is_empty());
    assert_eq!(echo.to_bytes(), bytes);
}

// ---------------------------------------------------------------------------
// Error path tests
// ---------------------------------------------------------------------------

#[test]
fn generic_nack_rejects_wrong_version() {
    let nack = GenericNack {
        ssrc_sender: 1,
        ssrc_media: 2,
        nacks: vec![NackFci { pid: 1, blp: 0 }],
    };
    let mut bytes = nack.to_bytes();
    // Set V=1 instead of V=2.
    bytes[0] = 0x41; // V=1, FMT=1
    assert!(GenericNack::parse(&bytes).is_err());
}

#[test]
fn range_nack_rejects_wrong_subtype() {
    let rn = RangeNack {
        ssrc_media: 1,
        ranges: vec![PacketRange {
            start: 1,
            additional: 0,
        }],
    };
    let mut bytes = rn.to_bytes();
    // Change subtype from 0 to 5.
    bytes[0] = (2 << 6) | 5;
    assert!(RangeNack::parse(&bytes).is_err());
}
