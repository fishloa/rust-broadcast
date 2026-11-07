//! `OutputBufferTooSmall { need, have }` for every buffer shorter than the
//! CW_provision fixture.

use broadcast_common::Serialize;
use dvb_simulcrypt::{Interface, SimulcryptMessage};

#[test]
fn cw_provision_rejects_every_short_buffer() {
    let data = test_fixtures::read("simulcrypt/cw-provision.bin");
    let msg = SimulcryptMessage::parse_on(Interface::EcmgScs, &data).expect("fixture");
    test_fixtures::assert_short_buffers_rejected(
        msg.serialized_len(),
        |buf| msg.serialize_into(buf),
        |e| match e {
            dvb_simulcrypt::Error::OutputBufferTooSmall(
                broadcast_common::error::OutputBufferTooSmall { need, have },
            ) => Some((*need, *have)),
            _ => None,
        },
    );
}
