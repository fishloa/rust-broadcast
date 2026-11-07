//! `OutputBufferTooSmall { need, have }` for every buffer shorter than each real
//! ATSC 3.0 ROUTE capture (`fixtures/atsc3/route-*.bin`).

use atsc3_route::{Error, RoutePacket};
use broadcast_common::{Parse, Serialize};

const FIXTURES: [&str; 3] = [
    "route-fdt-instance-2020-11-05.bin",
    "route-media-video-fragment-2020-11-05.bin",
    "route-media-audio-fragment-2020-11-05.bin",
];

#[test]
fn real_route_packets_reject_every_short_buffer() {
    for name in FIXTURES {
        let bytes = test_fixtures::read(&format!("atsc3/{name}"));
        let pkt = RoutePacket::parse(&bytes).expect("real ROUTE packet");
        test_fixtures::assert_short_buffers_rejected(
            pkt.serialized_len(),
            |buf| pkt.serialize_into(buf),
            |e| match e {
                Error::OutputBufferTooSmall(broadcast_common::error::OutputBufferTooSmall {
                    need,
                    have,
                }) => Some((*need, *have)),
                _ => None,
            },
        );
    }
}
