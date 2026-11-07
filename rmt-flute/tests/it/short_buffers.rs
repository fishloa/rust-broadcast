//! `OutputBufferTooSmall { need, have }` for every buffer shorter than the real
//! FLUTE FDT ALC packet (`fixtures/flute/flute-fdt.bin`).

use rmt_flute::{AlcPacket, Error, FEC_PAYLOAD_ID_128_LEN};

#[test]
fn real_alc_packet_rejects_every_short_buffer() {
    let data = test_fixtures::read("flute/flute-fdt.bin");
    let pkt = AlcPacket::parse(&data, FEC_PAYLOAD_ID_128_LEN).expect("real FLUTE packet");
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
