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

fn emsg_header(size: u32, version: u8) -> [u8; 12] {
    let mut header = [0u8; 12];
    header[0..4].copy_from_slice(&size.to_be_bytes());
    header[4..8].copy_from_slice(b"emsg");
    header[8] = version;
    header
}

#[test]
fn declared_size_below_the_fullbox_header_is_invalid() {
    for size in [8u32, 9, 10, 11] {
        assert_eq!(
            EmsgBox::parse(&emsg_header(size, 0)).unwrap_err(),
            Error::InvalidSize {
                size,
                reason: "size smaller than the FullBox header (0/1 large-size forms unsupported)",
            },
            "size {size}"
        );
    }
}

#[test]
fn size_equal_to_the_fullbox_header_is_a_valid_empty_box() {
    // 12 is exactly the FullBox header: a valid box with an empty body. The
    // parse must fail inside the body, never on the size guard.
    assert_eq!(
        EmsgBox::parse(&emsg_header(12, 0)).unwrap_err(),
        Error::InvalidString {
            field: "scheme_id_uri",
            reason: "missing null terminator",
        }
    );
}

#[test]
fn a_short_v1_body_is_buffer_too_short_at_every_length_under_twenty() {
    for len in 0..20usize {
        let mut data = vec![0u8; 12 + len];
        data[..12].copy_from_slice(&emsg_header((12 + len) as u32, 1));
        assert_eq!(
            EmsgBox::parse(&data).unwrap_err(),
            Error::BufferTooShort(broadcast_common::error::BufferTooShort {
                need: 20,
                have: len,
                what: "emsg v1 integer fields",
            }),
            "body length {len}"
        );
    }
}
