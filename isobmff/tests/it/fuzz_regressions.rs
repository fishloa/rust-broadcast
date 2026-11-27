//! Every input a `fuzz/fuzz_targets/isobmff_*.rs` target has crashed on, fed
//! back through the same entry point verbatim.
//!
//! Each case is the exact artifact bytes libFuzzer wrote, so the test fails on
//! the historical panic before it can reach the typed error the parser now
//! returns.

use broadcast_core::Parse;
use isobmff::Error;
use isobmff::audio_elements::{
    AudioElementBox, AudioElementPosInteractivityPolarBox, AudioElementProminenceInteractivityBox,
    AudioElementSelectionBox,
};

fn boxed(fourcc: [u8; 4], body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 + body.len());
    out.extend_from_slice(&((8 + body.len()) as u32).to_be_bytes());
    out.extend_from_slice(&fourcc);
    out.extend_from_slice(body);
    out
}

#[test]
fn tier34_boxes_crash_9562f956_is_a_typed_error() {
    // `isobmff_tier34_boxes` crash-9562f956b504ba425d7a3788f9123182222b82f0.
    let artifact = [187u8, 255, 16, 255, 255, 79];
    let (selector, body) = artifact.split_first().expect("artifact has a selector");
    assert_eq!(selector % 27, 25);

    assert!(AudioElementBox::parse(&boxed(*b"aelm", body)).is_err());
    assert!(AudioElementSelectionBox::parse(&boxed(*b"aesb", body)).is_err());
    assert!(AudioElementProminenceInteractivityBox::parse(&boxed(*b"aepr", body)).is_err());

    match AudioElementPosInteractivityPolarBox::parse(&boxed(*b"aepp", body)) {
        Err(Error::BufferTooShort(short)) => {
            assert_eq!(short.what, "aepp azimuth");
            assert_eq!(short.have, body.len());
        }
        other => panic!("expected a typed short-buffer error, got {other:?}"),
    }
}

#[test]
fn item_boxes_crash_4ef54280_is_a_typed_error() {
    // `isobmff_item_boxes` crash-4ef54280608affdbcae0f17c8c870e0d88614ba5: an
    // `md5i` body that ends right after `input_4cc` (`sgpd`) left no bytes for
    // the grouping type, and the parser indexed past the body.
    let artifact = [
        0u8, 0, 0, 15, 0, 0, 0, 0, 0, 0, 0, 194, 40, 1, 0, 0, 0, 0, 32, 109, 100, 53, 105, 100, 0,
        2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 50, 177, 0, 0, 0, 0, 0, 115, 103, 112, 100, 184, 40, 25,
        25, 99, 109, 105, 110, 25, 25, 25, 128, 0, 114, 1, 0, 0, 0, 0, 32, 109, 100, 53, 105, 100,
        0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 50, 177, 0, 0, 0, 0, 0, 184, 40, 1, 0, 0, 0, 0, 32,
        109, 100, 53, 105, 100, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 184, 100, 0, 0, 0, 0, 0, 0,
        0, 1, 0, 0, 0, 0, 0, 0, 184, 100,
    ];
    assert!(isobmff::integrity::IntegrityItem::parse(&artifact).is_err());
}
