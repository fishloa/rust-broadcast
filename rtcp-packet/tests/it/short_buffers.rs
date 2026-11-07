//! `OutputBufferTooSmall { need, have }` for every buffer shorter than a Sender
//! Report, a Receiver Report and a compound packet (RFC 3550 §6).

use broadcast_common::{Parse, Serialize};
use rtcp_packet::{CompoundPacket, Error, ReceiverReport, SenderReport};

fn too_small(e: &Error) -> Option<(usize, usize)> {
    match e {
        Error::OutputBufferTooSmall(broadcast_common::error::OutputBufferTooSmall {
            need,
            have,
        }) => Some((*need, *have)),
        _ => None,
    }
}

#[rustfmt::skip]
const SR: [u8; 28] = [
    0x80, 0xC8, 0x00, 0x06,
    0x11, 0x22, 0x33, 0x44,
    0xAA, 0xBB, 0xCC, 0xDD,
    0x10, 0x20, 0x30, 0x40,
    0x00, 0x05, 0x00, 0x00,
    0x00, 0x00, 0x00, 0x64,
    0x00, 0x00, 0x13, 0x88,
];

const RR: [u8; 8] = [0x80, 0xC9, 0x00, 0x01, 0x55, 0x66, 0x77, 0x88];

#[test]
fn reports_reject_every_short_buffer() {
    let sr = SenderReport::parse(&SR).expect("SR");
    test_fixtures::assert_short_buffers_rejected(
        sr.serialized_len(),
        |buf| sr.serialize_into(buf),
        too_small,
    );
    let rr = ReceiverReport::parse(&RR).expect("RR");
    test_fixtures::assert_short_buffers_rejected(
        rr.serialized_len(),
        |buf| rr.serialize_into(buf),
        too_small,
    );
}

#[test]
fn compound_packet_rejects_every_short_buffer() {
    let wire = [SR.as_slice(), RR.as_slice()].concat();
    let compound = CompoundPacket::parse(&wire).expect("compound");
    test_fixtures::assert_short_buffers_rejected(
        compound.serialized_len(),
        |buf| compound.serialize_into(buf),
        too_small,
    );
}
