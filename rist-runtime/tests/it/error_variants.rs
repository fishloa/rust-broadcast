use broadcast_common::error::BufferTooShort;
use broadcast_common::{Parse, Serialize};
use rist_runtime::{Error, GenericNack, ReportPart, RistSenderCompound, RttEcho, RttEchoKind};
use rtcp_packet::SenderReport;

fn echo() -> RttEcho {
    RttEcho {
        kind: RttEchoKind::Request,
        ssrc_media: 0x1122_3344,
        timestamp: 12345,
        processing_delay_us: 0,
        padding: vec![],
    }
}

fn compound(rtt_echo: Option<RttEcho>, cname: &str) -> RistSenderCompound {
    RistSenderCompound {
        report: ReportPart::Sr(SenderReport::new(
            0x1122_3344,
            0xE0E1_E2E3,
            0x1020_3040,
            0x0009_0000,
            100,
            50_000,
            vec![],
        )),
        report_padding: vec![],
        cname: cname.to_string(),
        rtt_echo,
        unknown: vec![],
    }
}

#[test]
fn a_two_byte_nack_is_a_missing_common_header() {
    assert_eq!(
        GenericNack::parse(&[0x81, 0xCD]).unwrap_err(),
        Error::BufferTooShort(BufferTooShort {
            need: 4,
            have: 2,
            what: "RTCP common header",
        })
    );
}

#[test]
fn a_nack_with_a_receiver_report_packet_type_is_an_invalid_packet_type() {
    assert_eq!(
        GenericNack::parse(&[0x81, 0xC9, 0x00, 0x03]).unwrap_err(),
        Error::InvalidPacketType {
            expected: 205,
            got: 0xC9,
        }
    );
}

#[test]
fn a_compound_opening_with_an_app_packet_is_an_unexpected_packet_type() {
    let mut bytes = compound(None, "s@e.com").try_to_bytes().unwrap();
    bytes[1] = 204;
    assert_eq!(
        RistSenderCompound::parse(&bytes).unwrap_err(),
        Error::UnexpectedPacketType(204)
    );
}

#[test]
fn a_compound_whose_sdes_has_no_cname_item_is_missing_the_cname() {
    let mut bytes = compound(None, "s@e.com").try_to_bytes().unwrap();
    let sr_len = 28;
    let first_item_type = sr_len + 4 + 4;
    assert_eq!(bytes[first_item_type], 1, "CNAME item type");
    bytes[first_item_type] = 2;
    assert_eq!(
        RistSenderCompound::parse(&bytes).unwrap_err(),
        Error::MissingCname
    );
}

#[test]
fn a_second_rtt_echo_is_a_duplicate() {
    let mut bytes = compound(Some(echo()), "s@e.com").try_to_bytes().unwrap();
    bytes.extend_from_slice(&echo().try_to_bytes().unwrap());
    assert_eq!(
        RistSenderCompound::parse(&bytes).unwrap_err(),
        Error::DuplicateRttEcho
    );
}

#[test]
fn an_oversized_cname_surfaces_the_rtcp_serializer_error() {
    let c = compound(None, &"x".repeat(300));
    let mut out = vec![0u8; c.serialized_len()];
    match c.serialize_into(&mut out).unwrap_err() {
        Error::Rtcp(e) => assert_eq!(
            e,
            rtcp_packet::Error::InvalidValue(broadcast_common::error::InvalidValue {
                field: "sdes_item_len",
                value: 300,
                reason: "exceeds 8-bit SDES item length",
            })
        ),
        other => panic!("expected Error::Rtcp, got {other:?}"),
    }
}
