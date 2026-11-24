//! The one box walker: strict vs clamped mode, and child iteration.

use isobmff::{BoxIter, Error};

#[test]
fn strict_rejects_a_box_that_overruns_its_parent() {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&32u32.to_be_bytes());
    bytes.extend_from_slice(b"free");
    bytes.extend_from_slice(&[1, 2, 3, 4]);

    let err = BoxIter::new(&bytes)
        .next()
        .expect("one box")
        .expect_err("a declared size past the buffer is a short read");
    let Error::BufferTooShort(short) = err else {
        panic!("expected BufferTooShort, got {err:?}");
    };
    assert_eq!(short.need, 32);
    assert_eq!(short.have, bytes.len());
    assert_eq!(short.what, "BoxRef body");
}

#[test]
fn clamped_shortens_a_truncated_box_to_the_buffer_end() {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&8u32.to_be_bytes());
    bytes.extend_from_slice(b"free");
    bytes.extend_from_slice(&100u32.to_be_bytes());
    bytes.extend_from_slice(b"moov");
    bytes.extend_from_slice(&[0xAA; 12]);

    let items: Vec<_> = BoxIter::clamped(&bytes).map(|b| b.unwrap()).collect();
    assert_eq!(items.len(), 2);

    assert_eq!(items[0].four_cc(), *b"free");
    assert_eq!((items[0].start, items[0].end), (0, 8));
    assert!(!items[0].is_truncated());

    assert_eq!(items[1].four_cc(), *b"moov");
    assert_eq!((items[1].start, items[1].end), (8, bytes.len()));
    assert!(items[1].is_truncated());
    assert_eq!(items[1].body.len(), 12);
    assert_eq!(items[1].payload_start(), 16);
}

#[test]
fn clamped_keeps_every_intact_leading_box_of_a_real_fixture() {
    let mut data = test_fixtures::read("mp4/h264-aac-frag.mp4");
    data.truncate(data.len() - 5);

    let items: Vec<_> = BoxIter::clamped(&data).map(|b| b.unwrap()).collect();
    assert!(items.len() >= 2, "the fixture starts with ftyp + moov");
    for item in &items[..items.len() - 1] {
        assert!(!item.is_truncated(), "{} is intact", item.header.box_type);
    }
    assert!(items.last().expect("a last box").is_truncated());
}

#[test]
fn moov_children_visit_the_recorded_wire_sequence() {
    let data = test_fixtures::read("mp4/h264-aac-frag.mp4");
    let moov = BoxIter::new(&data)
        .filter_map(|b| b.ok())
        .find(|b| b.header.box_type.is(b"moov"))
        .expect("moov");

    let visited: Vec<([u8; 4], usize, usize)> = moov
        .children()
        .map(|b| {
            let b = b.expect("a well-formed moov child");
            (b.four_cc(), b.start, b.end - b.start)
        })
        .collect();

    let expected: &[([u8; 4], usize, usize)] = &[
        (*b"mvhd", 0, 108),
        (*b"trak", 108, 497),
        (*b"trak", 605, 447),
        (*b"mvex", 1052, 72),
        (*b"udta", 1124, 98),
    ];
    assert_eq!(visited, expected);
}
