//! Every input a `fuzz/fuzz_targets/isobmff_*.rs` target has crashed on, fed
//! back through the same entry point verbatim.
//!
//! Each case is the exact artifact bytes libFuzzer wrote, so the test fails on
//! the historical panic before it can reach the typed error the parser now
//! returns.

use broadcast_core::{Parse, Serialize};
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
fn quicktime_sound_entry_child_size_is_not_rewritten() {
    // `isobmff_whole_file` crash: `fixtures/mp4/frag/av1.frag.mp4` with the
    // `av01` entry's four-CC changed to `sowt`. The `0x00480000` at the sound
    // fixed part's child offset is the visual entry's horizontal resolution,
    // not a child size; the entry ends 121 bytes later, so the child is
    // truncated. The rewrite re-stated the clamped 121-byte length.
    let entry: [u8; 157] = [
        0, 0, 0, 157, 115, 111, 119, 116, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 1, 64, 0, 240, 0, 72, 0, 0, 0, 72, 0, 0, 0, 0, 0, 0, 0, 1, 23, 76, 97, 118,
        99, 54, 50, 46, 50, 56, 46, 49, 48, 48, 32, 108, 105, 98, 115, 118, 116, 97, 118, 49, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 24, 255, 255, 0, 0, 0, 25, 97, 118, 49, 67, 129, 0, 12, 0, 10, 11, 0,
        0, 0, 4, 60, 255, 188, 2, 248, 0, 64, 0, 0, 0, 10, 102, 105, 101, 108, 1, 0, 0, 0, 0, 16,
        112, 97, 115, 112, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 20, 98, 116, 114, 116, 0, 0, 0, 0, 0,
        4, 96, 96, 0, 4, 96, 96,
    ];
    let parsed = isobmff::quicktime::SoundEntry::parse(&entry).expect("sowt entry parses");
    assert!(
        parsed.config_boxes.is_empty(),
        "the overrunning child is not a box"
    );
    assert_eq!(parsed.trailing.len(), entry.len() - 36);
    let out = parsed.try_to_bytes().expect("sowt entry serializes");
    assert_eq!(out, entry, "the sound entry rewrite changed a child size");
}

#[test]
fn sample_entry_with_to_end_header_stays_opaque() {
    let mut entry: [u8; 157] = [
        0, 0, 0, 157, 115, 111, 119, 116, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 1, 64, 0, 240, 0, 72, 0, 0, 0, 72, 0, 0, 0, 0, 0, 0, 0, 1, 23, 76, 97, 118,
        99, 54, 50, 46, 50, 56, 46, 49, 48, 48, 32, 108, 105, 98, 115, 118, 116, 97, 118, 49, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 24, 255, 255, 0, 0, 0, 25, 97, 118, 49, 67, 129, 0, 12, 0, 10, 11, 0,
        0, 0, 4, 60, 255, 188, 2, 248, 0, 64, 0, 0, 0, 10, 102, 105, 101, 108, 1, 0, 0, 0, 0, 16,
        112, 97, 115, 112, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 20, 98, 116, 114, 116, 0, 0, 0, 0, 0,
        4, 96, 96, 0, 4, 96, 96,
    ];
    entry[..4].fill(0);
    let variant = isobmff::init_segment::sample_entry_variant(&entry)
        .expect("a to-end sample entry is representable");
    let out = variant.try_to_bytes().expect("opaque entry serializes");
    assert_eq!(out, entry, "a to-end entry keeps its header form");
}
