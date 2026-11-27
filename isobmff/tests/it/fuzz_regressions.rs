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

/// The exact 157-byte entry `isobmff_whole_file` failed on: the AV1 fixture's
/// `av01` entry with its four-CC changed to `sowt`.
const SOWT_ENTRY: [u8; 157] = [
    0, 0, 0, 157, 115, 111, 119, 116, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 1, 64, 0, 240, 0, 72, 0, 0, 0, 72, 0, 0, 0, 0, 0, 0, 0, 1, 23, 76, 97, 118, 99, 54,
    50, 46, 50, 56, 46, 49, 48, 48, 32, 108, 105, 98, 115, 118, 116, 97, 118, 49, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 24, 255, 255, 0, 0, 0, 25, 97, 118, 49, 67, 129, 0, 12, 0, 10, 11, 0, 0, 0, 4, 60,
    255, 188, 2, 248, 0, 64, 0, 0, 0, 10, 102, 105, 101, 108, 1, 0, 0, 0, 0, 16, 112, 97, 115, 112,
    0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 20, 98, 116, 114, 116, 0, 0, 0, 0, 0, 4, 96, 96, 0, 4, 96, 96,
];

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
    let entry: [u8; 157] = SOWT_ENTRY;
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
    let mut entry: [u8; 157] = SOWT_ENTRY;
    entry[..4].fill(0);
    let variant = isobmff::init_segment::sample_entry_variant(&entry)
        .expect("a to-end sample entry is representable");
    let out = variant.try_to_bytes().expect("opaque entry serializes");
    assert_eq!(out, entry, "a to-end entry keeps its header form");
}

#[test]
fn truncated_sample_entry_is_a_typed_error() {
    let mut entry = SOWT_ENTRY;
    entry[..4].copy_from_slice(&0x0001_006Au32.to_be_bytes());
    assert!(
        isobmff::init_segment::sample_entry_variant(&entry).is_err(),
        "an entry shorter than its declared size must not be re-emitted"
    );
}

#[test]
fn udta_to_end_child_keeps_its_header_form() {
    // `isobmff_metadata` crash-2f504d53: a `udta` child declared with
    // `size == 0` (extends to the end of `udta`, ISO/IEC 14496-12:2026 §4.2).
    let wire = [0, 0, 0, 16, 117, 100, 116, 97, 0, 0, 0, 0, 16, 100, 116, 97];
    let udta = isobmff::metadata::Udta::parse(&wire).expect("udta parses");
    let out = udta.try_to_bytes().expect("udta serializes");
    assert_eq!(out, wire, "the to-end child keeps its header form");
}

#[test]
fn udta_to_end_meta_child_keeps_its_header_form() {
    // `isobmff_metadata` crash-af6003: a typed `meta` child declared with
    // `size == 0`. A typed child serialises the compact header, so the to-end
    // child is kept opaque.
    let wire = [
        0, 0, 0, 46, 117, 100, 116, 97, 0, 0, 0, 8, 16, 0, 0, 0, 0, 0, 0, 0, 109, 101, 116, 97,
        254, 255, 0, 0, 0, 0, 0, 10, 109, 97, 8, 16, 20, 0, 0, 0, 0, 0, 0, 0, 0, 200,
    ];
    let udta = isobmff::metadata::Udta::parse(&wire).expect("udta parses");
    let out = udta.try_to_bytes().expect("udta serializes");
    assert_eq!(out, wire, "the to-end meta child keeps its header form");
}

#[test]
fn udta_plain_text_atom_keeps_its_terminator_state() {
    // `isobmff_metadata` crash-580599: a `name` atom with an empty body. A
    // plain text atom is a NUL-terminated string, but this one has no NUL and
    // must not gain one.
    let wire = [
        0, 0, 0, 16, 117, 100, 116, 97, 0, 0, 0, 8, 110, 97, 109, 101,
    ];
    let udta = isobmff::metadata::Udta::parse(&wire).expect("udta parses");
    let out = udta.try_to_bytes().expect("udta serializes");
    assert_eq!(out, wire, "a plain text atom keeps its terminator state");
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

/// The exact 345-byte input `isobmff_item_reader` failed on: a file carrying
/// two top-level `meta` boxes. The reader kept only one body, so a re-emit
/// wrote the second body under both headers — the whole-file round trip now
/// asserted by that target caught it, and a second `meta` is a typed error
/// because only one describes the item model (ISO/IEC 14496-12:2026 §8.11.1).
const TWO_META_FILES: [u8; 345] = [
    0, 0, 0, 17, 4, 42, 0, 4, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 8, 255, 255, 255, 255, 0, 0, 0,
    15, 255, 1, 207, 38, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 17, 4, 42, 0, 4, 0, 0, 0, 0, 0, 66, 239, 1,
    0, 0, 0, 0, 17, 4, 42, 0, 4, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 8, 255, 255, 255, 129, 0, 0,
    0, 15, 255, 1, 207, 38, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 17, 4, 42, 0, 4, 0, 0, 0, 0, 1, 66, 239,
    0, 0, 0, 0, 0, 64, 109, 101, 116, 97, 0, 0, 0, 0, 0, 0, 0, 0, 8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0, 15, 255, 1, 207, 38, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 17, 4, 42, 0, 4, 0,
    0, 0, 0, 0, 66, 239, 1, 0, 0, 0, 0, 17, 4, 42, 0, 4, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 8,
    255, 255, 255, 129, 0, 0, 0, 15, 255, 1, 207, 38, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 17, 4, 42, 0,
    4, 0, 0, 0, 0, 1, 66, 239, 0, 0, 0, 0, 0, 64, 109, 101, 116, 97, 0, 0, 0, 0, 0, 0, 0, 0, 15, 0,
    0, 0, 0, 0, 0, 0, 0, 15, 255, 1, 207, 38, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 17, 4, 42, 0, 4, 0, 0,
    0, 0, 1, 66, 239, 0, 0, 0, 0, 0, 64, 109, 101, 116, 97, 0, 0, 0, 0, 0, 0, 0, 0, 8, 0, 0, 0, 1,
    0, 0, 0,
];

#[test]
fn item_reader_keeps_a_second_meta_box_opaque_and_re_emits_it() {
    use isobmff::image_brands::ImageFinding;
    use isobmff::item_reader::{ItemReader, ItemReaderConfig, ItemStep};

    let data = &TWO_META_FILES;
    let config = ItemReaderConfig::default().with_total_len(Some(data.len() as u64));
    let mut reader = ItemReader::new(config);
    let mut image = None;
    loop {
        match reader.step().expect("step") {
            ItemStep::Need(range) => {
                let start = usize::try_from(range.offset).expect("offset");
                let len = usize::try_from(range.len).expect("length");
                reader
                    .supply(range.offset, &data[start..start + len])
                    .expect("supply");
            }
            ItemStep::Loaded(model) => image = Some(model.clone()),
            ItemStep::Done => break,
            _ => {}
        }
    }
    let image = image.expect("the first meta box describes the model");
    assert!(
        image
            .structure_findings()
            .iter()
            .any(|finding| matches!(finding, ImageFinding::ExtraMetaBox { .. })),
        "the second meta box is reported"
    );
    assert_eq!(
        image.reemit_into(data).expect("re-emit"),
        data,
        "the second meta box is carried byte-exact"
    );
}

/// The exact 100-byte input `isobmff_item_reader` failed on next: a top-level
/// box written with `size == 1` and a `largesize` of 0. It described no bytes,
/// so the re-emit dropped the 55 the input still held; a `largesize` shorter
/// than its own 16-byte header is now a typed error (§4.2).
const ZERO_LARGESIZE_TAIL: [u8; 100] = [
    0, 0, 0, 29, 109, 101, 116, 97, 0, 0, 0, 0, 0, 0, 0, 17, 4, 42, 0, 4, 0, 0, 0, 0, 0, 210, 255,
    1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 49, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    36, 0, 0, 0, 36, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 198, 207, 207, 210, 0, 0,
];

#[test]
fn item_reader_zero_largesize_is_a_typed_error() {
    use isobmff::item_reader::{ItemReader, ItemReaderConfig, ItemStep};

    let data = &ZERO_LARGESIZE_TAIL;
    let config = ItemReaderConfig::default().with_total_len(Some(data.len() as u64));
    let mut reader = ItemReader::new(config);
    loop {
        match reader.step() {
            Ok(ItemStep::Need(range)) => {
                let start = usize::try_from(range.offset).expect("offset");
                let len = usize::try_from(range.len).expect("length");
                if let Err(err) = reader.supply(range.offset, &data[start..start + len]) {
                    assert!(
                        matches!(err, Error::BoxSizeUnderflow { .. }),
                        "expected BoxSizeUnderflow, got {err:?}"
                    );
                    return;
                }
            }
            Ok(ItemStep::Loaded(_)) => panic!("a zero-largesize tail must not load"),
            Ok(ItemStep::Done) => panic!("done without an error"),
            Ok(_) => {}
            Err(err) => {
                assert!(
                    matches!(err, Error::BoxSizeUnderflow { .. }),
                    "expected BoxSizeUnderflow, got {err:?}"
                );
                return;
            }
        }
    }
}
