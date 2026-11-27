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
