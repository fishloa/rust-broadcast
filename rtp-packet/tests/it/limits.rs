//! Maximum-legal-value / maximum-plus-one boundaries for every RTP and RTCP
//! count, width, and padding limit (RFC 3550 §5.1, §5.3.1, §6).
//!
//! Each maximum is a spec constant: the 4-bit `CC` field (15), the 7-bit `PT`
//! field (127), the 16-bit extension word count (65535), the 8-bit padding
//! count (255), and the 5-bit RTCP `RC`/`SC`/subtype fields (31).

use broadcast_core::{Parse, Serialize};
use rtp_packet::rtcp::{
    App, Bye, Error as RtcpError, ReceiverReport, ReportBlock, RtcpPacket, RtcpPacketType,
    RtcpPadding, SdesChunk, SdesItem, SdesItemType, SenderReport, SourceDescription,
};
use rtp_packet::{
    Error, ExtendedSeqNo, HeaderExtension, MAX_CSRC_COUNT, MAX_PADDING_COUNT, MAX_PAYLOAD_TYPE,
    RtpPacket, SeqNo, Ssrc,
};

fn empty_packet() -> RtpPacket<'static> {
    RtpPacket {
        marker: false,
        payload_type: 0,
        sequence_number: SeqNo::new(0),
        timestamp: 0,
        ssrc: Ssrc::new(0),
        csrc: Vec::new(),
        extension: None,
        padding: None,
        payload: &[],
    }
}

fn serialize_rtp(p: &RtpPacket<'_>) -> Vec<u8> {
    let mut out = vec![0u8; p.serialized_len()];
    let written = p.serialize_into(&mut out).unwrap();
    assert_eq!(written, p.serialized_len());
    out
}

fn serialize_rtcp<T: Serialize<Error = RtcpError>>(p: &T) -> Vec<u8> {
    let mut out = vec![0u8; p.serialized_len()];
    let written = p.serialize_into(&mut out).unwrap();
    assert_eq!(written, p.serialized_len());
    out
}

fn block(ssrc: u32) -> ReportBlock {
    ReportBlock {
        ssrc: Ssrc::new(ssrc),
        fraction_lost: 0,
        cumulative_lost: 0,
        ext_highest_seq: ExtendedSeqNo::new(0),
        jitter: 0,
        lsr: 0,
        dlsr: 0,
    }
}

// ── RTP fixed header fields ────────────────────────────────────────────────

#[test]
fn csrc_count_maximum_round_trips_and_one_more_is_rejected() {
    assert_eq!(MAX_CSRC_COUNT, 15);
    let mut p = empty_packet();
    p.csrc = (0..15u32).map(|i| 0x0101_0101 * (i + 1)).collect();
    let out = serialize_rtp(&p);
    assert_eq!(out[0] & 0x0F, 15);
    let back = RtpPacket::parse(&out).unwrap();
    assert_eq!(back.csrc_count(), 15);
    assert_eq!(back, p);

    p.csrc = vec![0; 16];
    let mut out = vec![0u8; p.serialized_len()];
    assert_eq!(
        p.serialize_into(&mut out),
        Err(Error::InvalidValue(broadcast_core::error::InvalidValue {
            field: "csrc.len()",
            value: 16,
            reason: "exceeds the 4-bit CC field maximum (15)",
        }))
    );
}

#[test]
fn payload_type_maximum_round_trips_and_one_more_is_rejected() {
    assert_eq!(MAX_PAYLOAD_TYPE, 127);
    let mut p = empty_packet();
    p.payload_type = 127;
    let out = serialize_rtp(&p);
    assert_eq!(out[1] & 0x7F, 127);
    assert_eq!(RtpPacket::parse(&out).unwrap(), p);

    p.payload_type = 128;
    let mut out = vec![0u8; p.serialized_len()];
    assert_eq!(
        p.serialize_into(&mut out),
        Err(Error::InvalidValue(broadcast_core::error::InvalidValue {
            field: "payload_type",
            value: 128,
            reason: "exceeds the 7-bit PT field maximum (127)",
        }))
    );
}

#[test]
fn extension_word_count_maximum_round_trips_and_one_more_is_rejected() {
    let data = vec![0xA5u8; 65_535 * 4];
    let mut p = empty_packet();
    p.extension = Some(HeaderExtension {
        profile_id: 0xBEDE,
        data: &data,
    });
    let out = serialize_rtp(&p);
    assert_eq!(u16::from_be_bytes([out[14], out[15]]), 65_535);
    let back = RtpPacket::parse(&out).unwrap();
    assert_eq!(back.extension.unwrap().length_words(), 65_535);

    let too_long = vec![0xA5u8; 65_536 * 4];
    p.extension = Some(HeaderExtension {
        profile_id: 0xBEDE,
        data: &too_long,
    });
    let mut out = vec![0u8; p.serialized_len()];
    assert_eq!(
        p.serialize_into(&mut out),
        Err(Error::InvalidValue(broadcast_core::error::InvalidValue {
            field: "extension.data.len()/4",
            value: 65_536,
            reason: "exceeds the 16-bit length field maximum",
        }))
    );
}

#[test]
fn padding_count_maximum_round_trips_and_one_more_is_rejected() {
    assert_eq!(MAX_PADDING_COUNT, 255);
    let mut pad = vec![0u8; 255];
    pad[254] = 255;
    let mut p = empty_packet();
    p.padding = Some(&pad);
    let out = serialize_rtp(&p);
    assert_eq!(out[0] & 0x20, 0x20);
    assert_eq!(*out.last().unwrap(), 255);
    assert_eq!(RtpPacket::parse(&out).unwrap().padding, Some(&pad[..]));

    let mut too_long = vec![0u8; 256];
    too_long[255] = 0;
    p.padding = Some(&too_long);
    let mut out = vec![0u8; p.serialized_len()];
    assert_eq!(
        p.serialize_into(&mut out),
        Err(Error::InvalidValue(broadcast_core::error::InvalidValue {
            field: "padding.len()",
            value: 256,
            reason: "exceeds the 8-bit padding-count field maximum (255)",
        }))
    );
}

// ── RTCP report / source counts ────────────────────────────────────────────

#[test]
fn sr_report_count_maximum_round_trips_and_one_more_is_rejected() {
    let blocks: Vec<ReportBlock> = (0..31).map(|i| block(i + 1)).collect();
    let sr = SenderReport::new(Ssrc::new(9), 1, 2, 3, 4, 5, blocks.clone());
    let out = serialize_rtcp(&sr);
    assert_eq!(out[0] & 0x1F, 31);
    assert_eq!(SenderReport::parse(&out).unwrap(), sr);

    let too_many = SenderReport::new(
        Ssrc::new(9),
        1,
        2,
        3,
        4,
        5,
        (0..32).map(|i| block(i + 1)).collect(),
    );
    let mut out = vec![0u8; too_many.serialized_len()];
    assert_eq!(
        too_many.serialize_into(&mut out),
        Err(RtcpError::InvalidValue(
            broadcast_core::error::InvalidValue {
                field: "rtcp_report_count",
                value: 32,
                reason: "exceeds 5-bit RC field",
            }
        ))
    );
}

#[test]
fn sdes_source_count_maximum_round_trips_and_one_more_is_rejected() {
    let chunks: Vec<SdesChunk> = (0..31)
        .map(|i| SdesChunk {
            source: i + 1,
            items: vec![SdesItem {
                item_type: SdesItemType::CName,
                text: "c".into(),
            }],
        })
        .collect();
    let sdes = SourceDescription {
        chunks,
        padding: None,
    };
    let out = serialize_rtcp(&sdes);
    assert_eq!(out[0] & 0x1F, 31);
    assert_eq!(SourceDescription::parse(&out).unwrap(), sdes);

    let too_many = SourceDescription {
        chunks: (0..32)
            .map(|i| SdesChunk {
                source: i + 1,
                items: Vec::new(),
            })
            .collect(),
        padding: None,
    };
    let mut out = vec![0u8; too_many.serialized_len()];
    assert_eq!(
        too_many.serialize_into(&mut out),
        Err(RtcpError::InvalidValue(
            broadcast_core::error::InvalidValue {
                field: "rtcp_source_count",
                value: 32,
                reason: "exceeds 5-bit SC field",
            }
        ))
    );
}

#[test]
fn sdes_item_text_maximum_round_trips_and_one_more_is_rejected() {
    let text = "a".repeat(255);
    let sdes = SourceDescription {
        chunks: vec![SdesChunk {
            source: 0x1234_5678,
            items: vec![SdesItem {
                item_type: SdesItemType::CName,
                text: text.clone(),
            }],
        }],
        padding: None,
    };
    let out = serialize_rtcp(&sdes);
    assert_eq!(out[9], 255, "the 8-bit item length octet");
    assert_eq!(SourceDescription::parse(&out).unwrap(), sdes);

    let too_long = SourceDescription {
        chunks: vec![SdesChunk {
            source: 0x1234_5678,
            items: vec![SdesItem {
                item_type: SdesItemType::CName,
                text: "a".repeat(256),
            }],
        }],
        padding: None,
    };
    let mut out = vec![0u8; too_long.serialized_len()];
    assert_eq!(
        too_long.serialize_into(&mut out),
        Err(RtcpError::InvalidValue(
            broadcast_core::error::InvalidValue {
                field: "sdes_item_len",
                value: 256,
                reason: "exceeds 8-bit SDES item length",
            }
        ))
    );
}

#[test]
fn bye_source_count_maximum_round_trips_and_one_more_is_rejected() {
    let bye = Bye {
        sources: (1..=31).collect(),
        reason: None,
        padding: None,
    };
    let out = serialize_rtcp(&bye);
    assert_eq!(out[0] & 0x1F, 31);
    assert_eq!(Bye::parse(&out).unwrap(), bye);

    let too_many = Bye {
        sources: (1..=32).collect(),
        reason: None,
        padding: None,
    };
    let mut out = vec![0u8; too_many.serialized_len()];
    assert_eq!(
        too_many.serialize_into(&mut out),
        Err(RtcpError::InvalidValue(
            broadcast_core::error::InvalidValue {
                field: "rtcp_source_count",
                value: 32,
                reason: "exceeds 5-bit SC field",
            }
        ))
    );
}

#[test]
fn bye_reason_maximum_round_trips_and_one_more_is_rejected() {
    let reason = "r".repeat(255);
    let bye = Bye {
        sources: vec![7],
        reason: Some(reason.clone()),
        padding: None,
    };
    let out = serialize_rtcp(&bye);
    assert_eq!(out[8], 255, "the 8-bit reason length octet");
    assert_eq!(Bye::parse(&out).unwrap(), bye);

    let too_long = Bye {
        sources: vec![7],
        reason: Some("r".repeat(256)),
        padding: None,
    };
    let mut out = vec![0u8; too_long.serialized_len()];
    assert_eq!(
        too_long.serialize_into(&mut out),
        Err(RtcpError::InvalidValue(
            broadcast_core::error::InvalidValue {
                field: "bye_reason_len",
                value: 256,
                reason: "exceeds 8-bit reason length",
            }
        ))
    );
}

#[test]
fn app_subtype_maximum_round_trips_and_one_more_is_rejected() {
    let app = App {
        subtype: 31,
        ssrc: Ssrc::new(0x1122_3344),
        name: *b"TEST",
        data: vec![1, 2, 3, 4, 5, 6, 7, 8],
        padding: None,
    };
    let out = serialize_rtcp(&app);
    assert_eq!(out[0] & 0x1F, 31);
    assert_eq!(&out[8..12], b"TEST");
    assert_eq!(&out[12..20], &[1, 2, 3, 4, 5, 6, 7, 8]);
    assert_eq!(App::parse(&out).unwrap(), app);

    let too_high = App { subtype: 32, ..app };
    let mut out = vec![0u8; too_high.serialized_len()];
    assert_eq!(
        too_high.serialize_into(&mut out),
        Err(RtcpError::InvalidValue(
            broadcast_core::error::InvalidValue {
                field: "app_subtype",
                value: 32,
                reason: "exceeds 5-bit subtype field",
            }
        ))
    );
}

#[test]
fn unknown_packet_count_maximum_round_trips_and_one_more_is_rejected() {
    let unknown = RtcpPacket::Unknown {
        packet_type: 205,
        count: 31,
        padding: false,
        payload: vec![0u8; 8],
    };
    let out = serialize_rtcp(&unknown);
    assert_eq!(out[0] & 0x1F, 31);
    assert_eq!(RtcpPacket::parse(&out).unwrap(), unknown);

    let too_high = RtcpPacket::Unknown {
        packet_type: 205,
        count: 32,
        padding: false,
        payload: vec![0u8; 8],
    };
    let mut out = vec![0u8; too_high.serialized_len()];
    assert_eq!(
        too_high.serialize_into(&mut out),
        Err(RtcpError::InvalidValue(
            broadcast_core::error::InvalidValue {
                field: "unknown_count",
                value: 32,
                reason: "exceeds 5-bit RC/SC field",
            }
        ))
    );
}

// ── RTCP accessors and offset arithmetic ───────────────────────────────────

#[test]
fn rtcp_packet_type_pt_is_the_wire_byte() {
    assert_eq!(RtcpPacketType::SenderReport.pt(), 200);
    assert_eq!(RtcpPacketType::ReceiverReport.pt(), 201);
    assert_eq!(RtcpPacketType::SourceDescription.pt(), 202);
    assert_eq!(RtcpPacketType::Bye.pt(), 203);
    assert_eq!(RtcpPacketType::App.pt(), 204);
    assert_eq!(RtcpPacketType::Unknown(207).pt(), 207);
}

#[test]
fn rtcp_padding_count_accessor_is_the_last_octet() {
    let pad = RtcpPadding::new(4).unwrap();
    assert_eq!(pad.count(), 4);
    assert_eq!(pad.octets(), &[0, 0, 0, 4]);
    assert_eq!(RtcpPadding::new(255).unwrap().count(), 255);
    assert!(RtcpPadding::new(0).is_err());
    assert!(RtcpPadding::new(256).is_err());
}

#[test]
fn padding_may_fill_the_whole_rtcp_body() {
    // P=1, PT=BYE, length=1 (8 bytes total), SC=0: the four body octets are
    // entirely padding, so the unpadded body is empty (RFC 3550 §6.6).
    let bytes = [0xA0, 0xCB, 0x00, 0x01, 0x00, 0x00, 0x00, 0x04];
    let bye = Bye::parse(&bytes).expect("padding exactly filling the body is valid");
    assert!(bye.sources.is_empty());
    assert_eq!(bye.reason, None);
    assert_eq!(bye.padding.as_ref().unwrap().count(), 4);

    let mut out = vec![0u8; bye.serialized_len()];
    bye.serialize_into(&mut out).unwrap();
    assert_eq!(out, bytes);
}

#[test]
fn sr_and_rr_write_profile_extensions_before_padding() {
    let sr = SenderReport::new(Ssrc::new(1), 0, 0, 0, 0, 0, Vec::new())
        .with_profile_extensions(vec![0xAA, 0xBB, 0xCC, 0xDD])
        .with_padding(RtcpPadding::new(4).unwrap());
    let out = serialize_rtcp(&sr);
    assert_eq!(&out[28..32], &[0xAA, 0xBB, 0xCC, 0xDD]);
    assert_eq!(&out[32..36], &[0x00, 0x00, 0x00, 0x04]);
    assert_eq!(SenderReport::parse(&out).unwrap(), sr);

    let rr = ReceiverReport::new(Ssrc::new(2), vec![block(3)])
        .with_profile_extensions(vec![0x11, 0x22, 0x33, 0x44])
        .with_padding(RtcpPadding::new(4).unwrap());
    let out = serialize_rtcp(&rr);
    assert_eq!(&out[32..36], &[0x11, 0x22, 0x33, 0x44]);
    assert_eq!(&out[36..40], &[0x00, 0x00, 0x00, 0x04]);
    assert_eq!(ReceiverReport::parse(&out).unwrap(), rr);
}

#[test]
fn bye_writes_its_reason_before_padding() {
    let bye = Bye {
        sources: vec![1, 2, 3],
        reason: Some("bye".into()),
        padding: Some(RtcpPadding::new(4).unwrap()),
    };
    let out = serialize_rtcp(&bye);
    assert_eq!(&out[4..16], &[0, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0, 3]);
    assert_eq!(out[16], 3, "reason length octet");
    assert_eq!(&out[17..20], b"bye");
    assert_eq!(&out[20..24], &[0x00, 0x00, 0x00, 0x04]);
    assert_eq!(Bye::parse(&out).unwrap(), bye);
}

#[test]
fn truncated_rtcp_bodies_report_the_exact_needed_bytes() {
    // RR with an empty body: the SSRC read needs the first four octets.
    assert_eq!(
        ReceiverReport::parse(&[0x80, 0xC9, 0x00, 0x00]),
        Err(RtcpError::BufferTooShort(
            broadcast_core::error::BufferTooShort {
                need: 4,
                have: 0,
                what: "RR ssrc",
            }
        ))
    );

    // SDES chunk with only its source and no item terminator.
    let sdes = [0x81, 0xCA, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01];
    assert_eq!(
        SourceDescription::parse(&sdes),
        Err(RtcpError::BufferTooShort(
            broadcast_core::error::BufferTooShort {
                need: 5,
                have: 4,
                what: "SDES item type",
            }
        ))
    );

    // SDES chunk ending right after a non-terminator item type byte.
    let sdes = [
        0x81, 0xCA, 0x00, 0x02, 0x00, 0x00, 0x00, 0x01, 0x01, 0x01, b'a', 0x01,
    ];
    assert_eq!(
        SourceDescription::parse(&sdes),
        Err(RtcpError::BufferTooShort(
            broadcast_core::error::BufferTooShort {
                need: 9,
                have: 8,
                what: "SDES item length",
            }
        ))
    );
}

#[test]
fn sdes_second_chunk_with_two_items_parses_both_chunks() {
    let sdes = SourceDescription {
        chunks: vec![
            SdesChunk {
                source: 0x0102_0304,
                items: vec![SdesItem {
                    item_type: SdesItemType::CName,
                    text: "host".into(),
                }],
            },
            SdesChunk {
                source: 0x0506_0708,
                items: vec![
                    SdesItem {
                        item_type: SdesItemType::CName,
                        text: "other".into(),
                    },
                    SdesItem {
                        item_type: SdesItemType::Name,
                        text: "user".into(),
                    },
                ],
            },
        ],
        padding: None,
    };
    let out = serialize_rtcp(&sdes);
    let back = SourceDescription::parse(&out).unwrap();
    assert_eq!(back, sdes);
    assert_eq!(back.chunks[0].source, 0x0102_0304);
    assert_eq!(back.chunks[0].items[0].text, "host");
    assert_eq!(back.chunks[1].source, 0x0506_0708);
    assert_eq!(back.chunks[1].items[0].item_type, SdesItemType::CName);
    assert_eq!(back.chunks[1].items[0].text, "other");
    assert_eq!(back.chunks[1].items[1].item_type, SdesItemType::Name);
    assert_eq!(back.chunks[1].items[1].text, "user");
}

#[test]
fn truncated_app_body_needs_the_full_ssrc_and_name() {
    // PT=APP, length=0: only the common header, no SSRC and no 4-byte name.
    assert_eq!(
        App::parse(&[0x80, 0xCC, 0x00, 0x00]),
        Err(RtcpError::BufferTooShort(
            broadcast_core::error::BufferTooShort {
                need: 8,
                have: 0,
                what: "RTCP APP ssrc+name",
            }
        ))
    );
}

#[test]
fn bye_declaring_more_sources_than_the_body_holds_is_rejected() {
    // SC=4 but only two source words in the 8-byte body.
    let bytes = [
        0x84, 0xCB, 0x00, 0x02, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x02,
    ];
    assert_eq!(
        Bye::parse(&bytes),
        Err(RtcpError::BufferTooShort(
            broadcast_core::error::BufferTooShort {
                need: 16,
                have: 8,
                what: "RTCP BYE sources",
            }
        ))
    );
}
