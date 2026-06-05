//! Integration tests for [`dvb_si::descriptors::parse_loop`] and the
//! macro-generated [`dvb_si::descriptors::AnyDescriptor`] dispatcher.

use dvb_si::descriptors::{parse_loop, AnyDescriptor};

/// Every tag with a dispatcher entry (the full README matrix except the
/// private 0x83 logical_channel). The completeness probe below feeds each of
/// these into `parse_loop` and asserts the result is NOT `Unknown` — proving
/// the dispatcher routes the tag to a typed parser. (Whether that parser then
/// succeeds or returns a structured `Err` is irrelevant to coverage; both
/// prove the tag is dispatched, neither is `Unknown`.)
const DISPATCHED_TAGS: &[u8] = &[
    0x05, 0x06, 0x09, 0x0A, 0x0F, // MPEG-2 subset
    0x40, 0x41, 0x42, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x49, 0x4A, 0x4B, 0x4C, 0x4D, 0x4E, 0x4F,
    0x50, 0x51, 0x52, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5A, 0x5B, 0x5C, 0x5D, 0x5E, 0x5F,
    0x60, 0x61, 0x62, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69, 0x6A, 0x6B, 0x6C, 0x6D, 0x6E, 0x6F,
    0x70, 0x71, 0x72, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79, 0x7A, 0x7B, 0x7C, 0x7D, 0x7E, 0x7F,
];

/// Completeness: every dispatched README-matrix tag routes to a typed parser
/// (never falls through to `Unknown`).
#[test]
fn every_dispatched_tag_is_not_unknown() {
    for &tag in DISPATCHED_TAGS {
        // A generous but well-formed-length single descriptor: 8 zero body
        // bytes. The dispatcher routes on the tag regardless of body content;
        // a body the typed parser rejects yields `Err`, never `Unknown`.
        let mut bytes = vec![tag, 0x08];
        bytes.extend_from_slice(&[0u8; 8]);
        let items: Vec<_> = parse_loop(&bytes).collect();
        assert_eq!(items.len(), 1, "tag {tag:#04x}: expected one item");
        let parsed = items.into_iter().next().unwrap();
        // Either Ok(typed) or Err(parse) — but never Unknown.
        if let Ok(AnyDescriptor::Unknown { .. }) = &parsed {
            panic!("tag {tag:#04x} dispatched to Unknown — missing dispatcher entry");
        }
    }
}

/// The full 256-tag space: only the dispatched set (plus 0x83, which has a
/// variant but is intentionally not auto-dispatched) is recognised; every
/// other tag must produce `Unknown`.
#[test]
fn undispatched_tags_yield_unknown() {
    for tag in 0u8..=0xFF {
        if DISPATCHED_TAGS.contains(&tag) {
            continue;
        }
        let bytes = [tag, 0x01, 0x00];
        let parsed = parse_loop(&bytes).next().unwrap();
        assert!(
            matches!(parsed, Ok(AnyDescriptor::Unknown { tag: t, .. }) if t == tag),
            "tag {tag:#04x}: expected Unknown, got {parsed:?}",
        );
    }
}

/// EIT-style mixed loop: short_event + parental_rating + an unknown 0xA7 tag →
/// three items: ShortEvent, ParentalRating, Unknown.
#[test]
fn eit_style_mixed_loop() {
    let mut loop_bytes = Vec::new();
    // short_event: tag 0x4D, lang "eng", name "Hi", text "".
    loop_bytes.extend_from_slice(&[0x4D, 0x07, b'e', b'n', b'g', 0x02, b'H', b'i', 0x00]);
    // parental_rating: tag 0x55, one entry (country "GBR" + rating 0x03).
    loop_bytes.extend_from_slice(&[0x55, 0x04, b'G', b'B', b'R', 0x03]);
    // unknown tag 0xA7, body [0xCA, 0xFE].
    loop_bytes.extend_from_slice(&[0xA7, 0x02, 0xCA, 0xFE]);

    let items: Vec<_> = parse_loop(&loop_bytes).collect::<Result<_, _>>().unwrap();
    assert_eq!(items.len(), 3);
    assert!(matches!(items[0], AnyDescriptor::ShortEvent(_)));
    assert!(matches!(items[1], AnyDescriptor::ParentalRating(_)));
    assert!(matches!(
        items[2],
        AnyDescriptor::Unknown {
            tag: 0xA7,
            body: [0xCA, 0xFE]
        }
    ));
}

/// Truncated tail: a valid short_event followed by a truncated header/body
/// (`[0x4D, 0xFF, 0x01]` claims length 0xFF but only 1 body byte) → one `Ok`,
/// then one `Err`, then `None` forever (fused).
#[test]
fn truncated_tail_yields_ok_then_err_then_fuses() {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&[0x4D, 0x07, b'e', b'n', b'g', 0x02, b'H', b'i', 0x00]);
    bytes.extend_from_slice(&[0x4D, 0xFF, 0x01]); // claims 255 body bytes, has 1

    let mut it = parse_loop(&bytes);
    assert!(matches!(it.next(), Some(Ok(AnyDescriptor::ShortEvent(_)))));
    assert!(matches!(it.next(), Some(Err(_))));
    assert!(it.next().is_none());
    assert!(it.next().is_none(), "iterator must stay fused");
}

/// Per-descriptor parse error continues: a malformed PDC (length 2, but PDC
/// requires a 3-byte body) followed by a valid stuffing descriptor →
/// `Err`, then `Ok(Stuffing)`, then `None`. The bad entry does not stop the
/// walk because its length field still bounds it.
#[test]
fn per_descriptor_error_continues() {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&[0x69, 0x02, 0x00, 0x00]); // PDC, wrong body length
    bytes.extend_from_slice(&[0x42, 0x02, 0xFF, 0xFF]); // valid stuffing

    let items: Vec<_> = parse_loop(&bytes).collect();
    assert_eq!(items.len(), 2);
    assert!(items[0].is_err(), "malformed PDC should be Err");
    assert!(matches!(items[1], Ok(AnyDescriptor::Stuffing(_))));
}

/// Issue #16 acceptance: a parsed loop serializes to externally-tagged
/// camelCase JSON with decoded string fields.
#[cfg(feature = "serde")]
#[test]
fn loop_serializes_decoded_json() {
    let loop_bytes = [
        0x4D, 0x0C, b'f', b'r', b'e', 0x07, b'J', b'o', b'u', b'r', b'n', b'a', b'l', 0x00,
    ];
    let items: Vec<_> = parse_loop(&loop_bytes).collect::<Result<_, _>>().unwrap();
    let json = serde_json::to_value(&items).unwrap();
    assert_eq!(json[0]["shortEvent"]["event_name"], "Journal");
    assert_eq!(json[0]["shortEvent"]["language_code"], "fre");
}
