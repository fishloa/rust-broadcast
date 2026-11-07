//! Every `Error` variant `RoutePacket::parse` can return, produced from a real
//! ROUTE capture truncated at the two framing boundaries.

use atsc3_route::rmt_flute::LctHeader;
use atsc3_route::{Error, ROUTE_FEC_PAYLOAD_ID_LEN, RoutePacket};
use broadcast_common::Parse;
use broadcast_common::error::BufferTooShort;

const FIXTURE: &str = "atsc3/route-media-video-fragment-2020-11-05.bin";

#[test]
fn truncation_after_the_lct_header_is_a_missing_fec_payload_id() {
    let bytes = test_fixtures::read(FIXTURE);
    let (_, used) = LctHeader::parse(&bytes).expect("real LCT header");
    assert_eq!(
        RoutePacket::parse(&bytes[..used]).unwrap_err(),
        Error::BufferTooShort(BufferTooShort {
            need: ROUTE_FEC_PAYLOAD_ID_LEN,
            have: 0,
            what: "ROUTE FEC Payload ID",
        })
    );
}

#[test]
fn truncation_inside_the_lct_header_is_an_lct_framing_error() {
    let bytes = test_fixtures::read(FIXTURE);
    let (_, used) = LctHeader::parse(&bytes).expect("real LCT header");
    let expected = LctHeader::parse(&bytes[..used - 1]).unwrap_err();
    assert_eq!(
        RoutePacket::parse(&bytes[..used - 1]).unwrap_err(),
        Error::Lct(expected)
    );
}
