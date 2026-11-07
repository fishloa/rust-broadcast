//! `PesPacket` through `broadcast_common::{Parse, Serialize}`.

use broadcast_common::{Parse, Serialize};
use mpeg_pes::{Error, PesPacket};

const VIDEO_PTS_ONLY: [u8; 16] = [
    0x00, 0x00, 0x01, 0xE0, 0x00, 0x0A, 0x80, 0x80, 0x05, 0x21, 0x00, 0x01, 0x00, 0x01, 0xAA, 0xBB,
];

#[test]
fn pes_packet_round_trips_through_the_traits() {
    let pkt = <PesPacket as Parse>::parse(&VIDEO_PTS_ONLY).unwrap();
    let wire = Serialize::try_to_bytes(&pkt).unwrap();
    assert_eq!(wire, VIDEO_PTS_ONLY);
    assert_eq!(<PesPacket as Parse>::parse(&wire).unwrap(), pkt);
}

#[test]
fn short_output_is_output_buffer_too_small() {
    let pkt = <PesPacket as Parse>::parse(&VIDEO_PTS_ONLY).unwrap();
    assert_eq!(
        Serialize::serialize_into(&pkt, &mut [0u8; 15]),
        Err(Error::OutputBufferTooSmall { need: 16, have: 15 })
    );
}
