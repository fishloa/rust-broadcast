//! `OutputBufferTooSmall { need, have }` for every buffer shorter than the real
//! RFC 4326 Appendix B SNDU.

use ule::{Error, Sndu};

#[test]
fn appendix_b_sndu_rejects_every_short_buffer() {
    let data = test_fixtures::read("ule/appendix-b.bin");
    let sndu = Sndu::parse(&data).expect("Appendix B SNDU");
    test_fixtures::assert_short_buffers_rejected(
        sndu.serialized_len(),
        |buf| sndu.serialize_into(buf),
        |e| match e {
            Error::OutputBufferTooSmall(broadcast_common::error::OutputBufferTooSmall {
                need,
                have,
            }) => Some((*need, *have)),
            _ => None,
        },
    );
}
