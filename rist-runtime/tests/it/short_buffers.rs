//! `OutputBufferTooSmall { need, have }` for every buffer shorter than each RIST
//! RTCP message type.

use broadcast_common::Serialize;
use broadcast_common::error::OutputBufferTooSmall;
use rist_runtime::{Error, GenericNack, NackFci, PacketRange, RangeNack, RttEcho, RttEchoKind};

fn too_small(e: &Error) -> Option<(usize, usize)> {
    match e {
        Error::OutputBufferTooSmall(OutputBufferTooSmall { need, have }) => Some((*need, *have)),
        _ => None,
    }
}

#[test]
fn generic_nack_rejects_every_short_buffer() {
    let nack = GenericNack {
        ssrc_sender: 0x0102_0304,
        ssrc_media: 0x0506_0708,
        nacks: vec![
            NackFci {
                pid: 42,
                blp: 0xABCD,
            },
            NackFci { pid: 99, blp: 0 },
        ],
        padding: Vec::new(),
    };
    test_fixtures::assert_short_buffers_rejected(
        nack.serialized_len(),
        |buf| nack.serialize_into(buf),
        too_small,
    );
}

#[test]
fn range_nack_rejects_every_short_buffer() {
    let nack = RangeNack {
        ssrc_media: 0x0506_0708,
        ranges: vec![PacketRange {
            start: 100,
            additional: 7,
        }],
        padding: Vec::new(),
    };
    test_fixtures::assert_short_buffers_rejected(
        nack.serialized_len(),
        |buf| nack.serialize_into(buf),
        too_small,
    );
}

#[test]
fn rtt_echo_rejects_every_short_buffer() {
    let echo = RttEcho {
        kind: RttEchoKind::Request,
        ssrc_media: 0xAABB_CC00,
        timestamp: 0x0102_0304_0506_0708,
        processing_delay_us: 0,
        padding: vec![],
    };
    test_fixtures::assert_short_buffers_rejected(
        echo.serialized_len(),
        |buf| echo.serialize_into(buf),
        too_small,
    );
}
