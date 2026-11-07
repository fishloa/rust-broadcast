//! `OutputBufferTooSmall { need, have }` for every buffer shorter than a
//! host-control `Tune` APDU.

use broadcast_common::{Parse, Serialize};
use dvb_ci::Error;
use dvb_ci::objects::host_control::Tune;

#[test]
fn tune_rejects_every_short_buffer() {
    let wire = [
        0x9F, 0x84, 0x00, 0x08, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88,
    ];
    let tune = Tune::parse(&wire).expect("Tune");
    test_fixtures::assert_short_buffers_rejected(
        tune.serialized_len(),
        |buf| tune.serialize_into(buf),
        |e| match e {
            Error::OutputBufferTooSmall(broadcast_common::error::OutputBufferTooSmall {
                need,
                have,
            }) => Some((*need, *have)),
            _ => None,
        },
    );
}
