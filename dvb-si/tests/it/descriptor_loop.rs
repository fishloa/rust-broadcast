//! Integration tests for [`dvb_si::descriptors::parse_loop`] and the
//! macro-generated [`dvb_si::descriptors::AnyDescriptor`] dispatcher.

use dvb_si::descriptors::{AnyDescriptor, parse_loop};

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
    assert_eq!(
        it.next().and_then(Result::err),
        Some(dvb_si::Error::BufferTooShort(
            broadcast_core::error::BufferTooShort {
                need: 257,
                have: 3,
                what: "descriptor body in loop"
            }
        ))
    );
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
    assert_eq!(
        items[0].as_ref().err(),
        Some(&dvb_si::Error::InvalidDescriptor {
            tag: 0x69,
            reason: "PDC_descriptor length must be exactly 3"
        })
    );
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

/// `avs_scope` opens only on an AVS private_data_specifier, on both iterator
/// types, and closes again on a foreign specifier.
#[test]
fn avs_scope_tracks_the_opening_private_data_specifier() {
    use dvb_si::descriptors::extension::registry::ExtensionRegistry;
    use dvb_si::descriptors::private_data_specifier::{PDS_AVSA, PDS_AVSV, TAG as PDS_TAG};
    use dvb_si::descriptors::{DescriptorLoop, DescriptorRegistry};

    #[rustfmt::skip]
    let bytes = [
        PDS_TAG, 4, 0x41, 0x56, 0x53, 0x41, // 'AVSA'
        PDS_TAG, 4, 0x41, 0x56, 0x53, 0x56, // 'AVSV'
        PDS_TAG, 4, 0x00, 0x00, 0x00, 0x28, // foreign
    ];
    let desc_reg = DescriptorRegistry::new();
    let ext_reg = ExtensionRegistry::new();
    let loop_ = DescriptorLoop::new(&bytes);

    let mut it = loop_.iter_with(&desc_reg);
    assert_eq!(it.avs_scope(), None);
    it.next().unwrap().unwrap();
    assert_eq!(it.avs_scope(), Some(PDS_AVSA));
    it.next().unwrap().unwrap();
    assert_eq!(it.avs_scope(), Some(PDS_AVSV));
    it.next().unwrap().unwrap();
    assert_eq!(it.avs_scope(), None);
    assert!(it.next().is_none());

    let mut it = loop_.iter_with_extensions(&desc_reg, &ext_reg);
    assert_eq!(it.avs_scope(), None);
    it.next().unwrap().unwrap();
    assert_eq!(it.avs_scope(), Some(PDS_AVSA));
    it.next().unwrap().unwrap();
    assert_eq!(it.avs_scope(), Some(PDS_AVSV));
    it.next().unwrap().unwrap();
    assert_eq!(it.avs_scope(), None);

    // A loop with no private_data_specifier never opens a scope.
    let plain = [0x42u8, 0x01, 0xFF];
    let plain_loop = DescriptorLoop::new(&plain);
    let mut it = plain_loop.iter_with(&desc_reg);
    assert_eq!(it.avs_scope(), None);
    it.next().unwrap().unwrap();
    assert_eq!(it.avs_scope(), None);
    let mut it = plain_loop.iter_with_extensions(&desc_reg, &ext_reg);
    assert_eq!(it.avs_scope(), None);
    it.next().unwrap().unwrap();
    assert_eq!(it.avs_scope(), None);
}

/// The extension registry walk must not index past an empty
/// extension_descriptor body and must not dispatch a non-extension descriptor's
/// first body byte as a tag extension.
#[test]
fn ext_registry_iter_guards_empty_and_non_extension_bodies() {
    use dvb_si::descriptors::extension::registry::ExtensionRegistry;
    use dvb_si::descriptors::extension::{ExtensionBodyDef, TAG as EXT_TAG};
    use dvb_si::descriptors::registry::ExtIterItem;
    use dvb_si::descriptors::{DescriptorLoop, DescriptorRegistry};

    const TEST_EXT: u8 = 0x30;

    #[derive(Debug)]
    #[cfg_attr(feature = "serde", derive(serde::Serialize))]
    struct TestExt {
        payload: Vec<u8>,
    }
    impl ExtensionBodyDef<'_> for TestExt {
        const TAG_EXTENSION: u8 = TEST_EXT;
        const NAME: &'static str = "TEST_EXT";
    }
    impl<'a> broadcast_core::Parse<'a> for TestExt {
        type Error = dvb_si::error::Error;
        fn parse(sel: &'a [u8]) -> Result<Self, Self::Error> {
            Ok(Self {
                payload: sel.to_vec(),
            })
        }
    }

    let desc_reg = DescriptorRegistry::new();
    let mut ext_reg = ExtensionRegistry::new();
    ext_reg.register::<TestExt>();

    // Empty extension_descriptor body: one item, never a custom extension.
    let empty = [EXT_TAG, 0x00];
    let items: Vec<_> = DescriptorLoop::new(&empty)
        .iter_with_extensions(&desc_reg, &ext_reg)
        .collect();
    assert_eq!(items.len(), 1);
    assert!(!matches!(
        &items[0],
        Ok(ExtIterItem::CustomExtension { .. })
    ));

    // A non-extension descriptor whose first body byte is a registered tag
    // extension must stay an ordinary descriptor.
    let not_extension = [0x40u8, 0x01, TEST_EXT];
    let items: Vec<_> = DescriptorLoop::new(&not_extension)
        .iter_with_extensions(&desc_reg, &ext_reg)
        .collect();
    assert_eq!(items.len(), 1);
    assert!(matches!(&items[0], Ok(ExtIterItem::Descriptor(_))));
    assert!(!matches!(
        &items[0],
        Ok(ExtIterItem::CustomExtension { .. })
    ));

    // A one-byte extension body still reaches the custom parser.
    let one_byte = [EXT_TAG, 0x01, TEST_EXT];
    let items: Vec<_> = DescriptorLoop::new(&one_byte)
        .iter_with_extensions(&desc_reg, &ext_reg)
        .collect();
    assert_eq!(items.len(), 1);
    match &items[0] {
        Ok(ExtIterItem::CustomExtension {
            tag_extension,
            value,
        }) => {
            assert_eq!(*tag_extension, TEST_EXT);
            let concrete = value.downcast_ref::<TestExt>().expect("downcast");
            assert!(concrete.payload.is_empty());
        }
        other => panic!("expected CustomExtension, got {other:?}"),
    }
}
