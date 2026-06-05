//! Integration tests for [`dvb_si::descriptors::parse_loop`] and the
//! macro-generated [`dvb_si::descriptors::AnyDescriptor`] dispatcher.

use dvb_si::descriptors::{parse_loop, AnyDescriptor};

/// Completeness: every dispatched README-matrix tag routes to a typed parser
/// (never falls through to `Unknown`).
#[test]
fn every_dispatched_tag_is_not_unknown() {
    for &tag in AnyDescriptor::DISPATCHED_TAGS {
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
        if AnyDescriptor::DISPATCHED_TAGS.contains(&tag) {
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

/// `AnyDescriptor::name()` reflects `DescriptorDef::NAME`; UNKNOWN for unknowns.
#[test]
fn name_maps_variant_to_descriptordef_name() {
    // stuffing descriptor: tag 0x42, len 1, one byte.
    let bytes = [0x42, 0x01, 0xFF, 0xA7, 0x01, 0x00];
    let items: Vec<_> = parse_loop(&bytes).collect();
    assert_eq!(items[0].as_ref().unwrap().name(), "STUFFING");
    assert_eq!(items[1].as_ref().unwrap().name(), "UNKNOWN");
}
