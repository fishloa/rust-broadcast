//! `OutputBufferTooSmall { need, have }` for every buffer shorter than the real
//! v0 and v1 SCTE 35 `emsg` fixtures.

use mp4_emsg::{EmsgBox, Error};

#[test]
fn real_emsg_boxes_reject_every_short_buffer() {
    for name in [
        "scte35/scte35-emsg-v0.bin",
        "scte35/emsg-v1-scte35-livesim.bin",
    ] {
        let data = test_fixtures::read(name);
        let b = EmsgBox::parse(&data).expect("real emsg");
        test_fixtures::assert_short_buffers_rejected(
            b.serialized_len(),
            |buf| b.serialize_into(buf),
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

#[test]
fn a_truncated_header_is_a_missing_fullbox_header() {
    assert_eq!(
        EmsgBox::parse(&[0u8; 4]).unwrap_err(),
        Error::BufferTooShort(broadcast_common::error::BufferTooShort {
            need: 12,
            have: 4,
            what: "emsg FullBox header",
        })
    );
}
