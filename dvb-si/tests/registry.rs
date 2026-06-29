//! Tests for [`dvb_si::descriptors::DescriptorRegistry`] — runtime registration
//! of client private descriptor types with dyn escape hatch.

use dvb_si::descriptors::{parse_loop, AnyDescriptor, DescriptorRegistry};
use dvb_si::traits::DescriptorDef;

// ---------------------------------------------------------------------------
// Test-local owned custom descriptor (tag 0xA7)
// ---------------------------------------------------------------------------

/// A private descriptor: wire layout is [tag, len, x_byte].
/// Must be 'static (no borrowed slices) to be registerable.
#[derive(Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
struct MyPrivate {
    x: u8,
}

impl<'a> broadcast_common::Parse<'a> for MyPrivate {
    type Error = dvb_si::Error;

    fn parse(bytes: &'a [u8]) -> dvb_si::Result<Self> {
        // Full descriptor: [tag, len, x_byte]
        if bytes.len() < 3 {
            return Err(dvb_si::Error::BufferTooShort {
                need: 3,
                have: bytes.len(),
                what: "MyPrivate",
            });
        }
        if bytes[0] != MY_PRIVATE_TAG {
            return Err(dvb_si::Error::InvalidDescriptor {
                tag: bytes[0],
                reason: "unexpected tag for MyPrivate",
            });
        }
        Ok(Self { x: bytes[2] })
    }
}

const MY_PRIVATE_TAG: u8 = 0xA7;

impl<'a> DescriptorDef<'a> for MyPrivate {
    const TAG: u8 = MY_PRIVATE_TAG;
    const NAME: &'static str = "MY_PRIVATE";
}

/// A second private descriptor sharing the same tag (0xA7) for re-register testing.
/// Wire layout: [tag, len, y_byte].
#[derive(Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
struct MyPrivate2 {
    y: u8,
}

impl<'a> broadcast_common::Parse<'a> for MyPrivate2 {
    type Error = dvb_si::Error;

    fn parse(bytes: &'a [u8]) -> dvb_si::Result<Self> {
        if bytes.len() < 3 {
            return Err(dvb_si::Error::BufferTooShort {
                need: 3,
                have: bytes.len(),
                what: "MyPrivate2",
            });
        }
        if bytes[0] != MY_PRIVATE_TAG {
            return Err(dvb_si::Error::InvalidDescriptor {
                tag: bytes[0],
                reason: "unexpected tag for MyPrivate2",
            });
        }
        Ok(Self { y: bytes[2] })
    }
}

impl<'a> DescriptorDef<'a> for MyPrivate2 {
    const TAG: u8 = MY_PRIVATE_TAG; // same tag as MyPrivate
    const NAME: &'static str = "MY_PRIVATE2";
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build a minimal valid MyPrivate descriptor bytes [tag, 1, x].
fn my_private_bytes(x: u8) -> [u8; 3] {
    [MY_PRIVATE_TAG, 0x01, x]
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// A registered type is dispatched to `AnyDescriptor::Other` and its value
/// can be downcast back to the concrete type.
#[test]
fn registered_type_yields_other_with_downcast() {
    let mut reg = DescriptorRegistry::new();
    reg.register::<MyPrivate>();

    let bytes = my_private_bytes(0x42);
    let items: Vec<_> = reg.parse_loop(&bytes).collect::<Result<_, _>>().unwrap();
    assert_eq!(items.len(), 1);

    match &items[0] {
        AnyDescriptor::Other { tag, value } => {
            assert_eq!(*tag, MY_PRIVATE_TAG);
            let concrete = value
                .as_any()
                .downcast_ref::<MyPrivate>()
                .expect("downcast to MyPrivate should succeed");
            assert_eq!(concrete.x, 0x42);
        }
        other => panic!("expected Other, got {other:?}"),
    }
}

/// Without registration, the same tag yields `Unknown` (no side-effects from
/// a different registry instance).
#[test]
fn unregistered_tag_yields_unknown() {
    let reg = DescriptorRegistry::new();
    let bytes = my_private_bytes(0x42);
    let items: Vec<_> = reg.parse_loop(&bytes).collect::<Result<_, _>>().unwrap();
    assert_eq!(items.len(), 1);
    assert!(
        matches!(
            items[0],
            AnyDescriptor::Unknown {
                tag: MY_PRIVATE_TAG,
                ..
            }
        ),
        "expected Unknown for unregistered tag, got {:?}",
        items[0]
    );
}

/// Custom registration on a built-in tag (0x4D = short_event) OVERRIDES the
/// built-in: the loop yields `Other`, not `ShortEvent`.
#[test]
fn custom_registration_overrides_builtin() {
    /// Minimal owned type for the override; wire: [tag, len, ...].
    #[derive(Debug)]
    #[cfg_attr(feature = "serde", derive(serde::Serialize))]
    struct MyShortEventOverride {
        #[allow(dead_code)]
        raw_len: u8,
    }

    impl<'a> broadcast_common::Parse<'a> for MyShortEventOverride {
        type Error = dvb_si::Error;
        fn parse(bytes: &'a [u8]) -> dvb_si::Result<Self> {
            if bytes.len() < 2 {
                return Err(dvb_si::Error::BufferTooShort {
                    need: 2,
                    have: bytes.len(),
                    what: "MyShortEventOverride",
                });
            }
            Ok(Self { raw_len: bytes[1] })
        }
    }

    impl<'a> DescriptorDef<'a> for MyShortEventOverride {
        const TAG: u8 = 0x4D; // same as ShortEvent
        const NAME: &'static str = "MY_SHORT_EVENT_OVERRIDE";
    }

    let mut reg = DescriptorRegistry::new();
    reg.register::<MyShortEventOverride>();

    // A valid short_event descriptor bytes.
    let bytes = [0x4Du8, 0x07, b'e', b'n', b'g', 0x02, b'H', b'i', 0x00];
    let items: Vec<_> = reg.parse_loop(&bytes).collect::<Result<_, _>>().unwrap();
    assert_eq!(items.len(), 1);
    assert!(
        matches!(items[0], AnyDescriptor::Other { tag: 0x4D, .. }),
        "custom registration should override built-in 0x4D; got {:?}",
        items[0]
    );
}

/// 0x83 (logical_channel) yields `Unknown` by default; after
/// `with_logical_channel()` it yields `LogicalChannel`.
#[test]
fn logical_channel_opt_in() {
    let bytes = [0x83u8, 0x04, 0x00, 0x01, 0xFC, 0x05];

    // Default: 0x83 → Unknown.
    let reg = DescriptorRegistry::new();
    let items: Vec<_> = reg.parse_loop(&bytes).collect::<Result<_, _>>().unwrap();
    assert_eq!(items.len(), 1);
    assert!(
        matches!(items[0], AnyDescriptor::Unknown { tag: 0x83, .. }),
        "0x83 should be Unknown by default; got {:?}",
        items[0]
    );

    // After opt-in: 0x83 → LogicalChannel.
    let mut reg = DescriptorRegistry::new();
    reg.with_logical_channel();
    let items: Vec<_> = reg.parse_loop(&bytes).collect::<Result<_, _>>().unwrap();
    assert_eq!(items.len(), 1);
    assert!(
        matches!(items[0], AnyDescriptor::LogicalChannel(_)),
        "0x83 should be LogicalChannel after with_logical_channel(); got {:?}",
        items[0]
    );
}

/// `registry.parse_loop` semantics match the free `parse_loop` on a loop with
/// no custom tags: same typed outputs, same truncation/fuse behaviour.
#[test]
fn registry_parse_loop_matches_free_parse_loop_no_custom_tags() {
    let mut loop_bytes = Vec::new();
    // short_event: tag 0x4D.
    loop_bytes.extend_from_slice(&[0x4D, 0x07, b'e', b'n', b'g', 0x02, b'H', b'i', 0x00]);
    // stuffing: tag 0x42.
    loop_bytes.extend_from_slice(&[0x42, 0x02, 0xFF, 0xFF]);
    // Unknown 0xA7.
    loop_bytes.extend_from_slice(&[0xA7, 0x02, 0xCA, 0xFE]);

    let free: Vec<_> = parse_loop(&loop_bytes).collect();
    let reg_items: Vec<_> = DescriptorRegistry::new().parse_loop(&loop_bytes).collect();

    assert_eq!(free.len(), reg_items.len(), "item counts must match");

    // Compare structural shape (not byte-for-byte equality on parsed structs,
    // just the variant discriminants and Unknown payloads).
    for (f, r) in free.iter().zip(reg_items.iter()) {
        match (f, r) {
            (Ok(AnyDescriptor::ShortEvent(_)), Ok(AnyDescriptor::ShortEvent(_))) => {}
            (Ok(AnyDescriptor::Stuffing(_)), Ok(AnyDescriptor::Stuffing(_))) => {}
            (
                Ok(AnyDescriptor::Unknown { tag: ft, body: fb }),
                Ok(AnyDescriptor::Unknown { tag: rt, body: rb }),
            ) => {
                assert_eq!(ft, rt);
                assert_eq!(fb, rb);
            }
            (Err(_), Err(_)) => {}
            (l, r) => panic!("variant mismatch between free and registry iter: {l:?} vs {r:?}"),
        }
    }
}

/// Truncation semantics are preserved: valid item then truncated entry → Ok
/// then Err then None (fused).
#[test]
fn registry_truncation_semantics_match() {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&[0x4D, 0x07, b'e', b'n', b'g', 0x02, b'H', b'i', 0x00]);
    bytes.extend_from_slice(&[0x4D, 0xFF, 0x01]); // claims 255 body bytes, has 1

    let reg = DescriptorRegistry::new();
    let mut it = reg.parse_loop(&bytes);
    assert!(matches!(it.next(), Some(Ok(AnyDescriptor::ShortEvent(_)))));
    assert!(matches!(it.next(), Some(Err(_))));
    assert!(it.next().is_none());
    assert!(it.next().is_none(), "must stay fused");
}

/// Per-descriptor error continues: a parse failure on a custom tag does NOT
/// stop the walk (the length field still advances the position).
#[test]
fn registry_custom_parse_error_continues() {
    let mut reg = DescriptorRegistry::new();
    reg.register::<MyPrivate>(); // expects [tag, len, x] — so len must be ≥ 1

    let mut bytes = Vec::new();
    // MyPrivate with empty body (len=0) — parse will fail (body too short)
    bytes.extend_from_slice(&[MY_PRIVATE_TAG, 0x00]);
    // valid stuffing after
    bytes.extend_from_slice(&[0x42, 0x02, 0xFF, 0xFF]);

    let items: Vec<_> = reg.parse_loop(&bytes).collect();
    assert_eq!(items.len(), 2, "both entries should appear");
    assert!(items[0].is_err(), "malformed custom should be Err");
    assert!(matches!(items[1], Ok(AnyDescriptor::Stuffing(_))));
}

/// Under `serde`, `AnyDescriptor::Other` serializes the custom value through
/// erased serde; the JSON shows the decoded custom struct fields.
#[cfg(feature = "serde")]
#[test]
fn other_variant_serializes_via_erased_serde() {
    let mut reg = DescriptorRegistry::new();
    reg.register::<MyPrivate>();

    let bytes = my_private_bytes(0x7F);
    let items: Vec<_> = reg.parse_loop(&bytes).collect::<Result<_, _>>().unwrap();
    let json = serde_json::to_value(&items).unwrap();

    // Outer tagging: "other" key (camelCase, from the macro).
    let entry = &json[0];
    assert!(
        entry.get("other").is_some(),
        "expected 'other' key in JSON, got {entry}"
    );
    // Inner value: MyPrivate { x: 0x7F } → {"tag": 167, "value": {"x": 127}}
    // The outer tag is at the "other" object level:
    assert_eq!(entry["other"]["tag"], 0xA7);
    // The value is the erased serialization of MyPrivate.
    assert_eq!(entry["other"]["value"]["x"], 0x7F);
}

/// Under `serde`, a mixed loop with a custom tag and a built-in tag serializes
/// correctly: each item gets its own camelCase key.
#[cfg(feature = "serde")]
#[test]
fn mixed_loop_with_custom_serializes_correctly() {
    let mut reg = DescriptorRegistry::new();
    reg.register::<MyPrivate>();

    let mut loop_bytes = Vec::new();
    // short_event: tag 0x4D, lang "fre", name "Journal", text "".
    loop_bytes.extend_from_slice(&[
        0x4D, 0x0C, b'f', b'r', b'e', 0x07, b'J', b'o', b'u', b'r', b'n', b'a', b'l', 0x00,
    ]);
    // custom MyPrivate: tag 0xA7, x = 5.
    loop_bytes.extend_from_slice(&[MY_PRIVATE_TAG, 0x01, 0x05]);

    let items: Vec<_> = reg
        .parse_loop(&loop_bytes)
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(items.len(), 2);

    let json = serde_json::to_value(&items).unwrap();
    // First item: shortEvent.
    assert_eq!(json[0]["shortEvent"]["event_name"], "Journal");
    // Second item: other.
    assert_eq!(json[1]["other"]["tag"], MY_PRIVATE_TAG as u64);
    assert_eq!(json[1]["other"]["value"]["x"], 5);
}

/// Re-registering the same tag on a registry replaces the prior parser.
/// When we register MyPrivate then MyPrivate2 (both at 0xA7), the second
/// registration wins: parsing a 0xA7 loop entry downcasts to MyPrivate2,
/// not MyPrivate.
#[test]
fn re_registering_same_tag_last_wins() {
    let mut reg = DescriptorRegistry::new();
    reg.register::<MyPrivate>();
    reg.register::<MyPrivate2>(); // overwrites MyPrivate's parser

    let bytes = my_private_bytes(0x99);
    let items: Vec<_> = reg.parse_loop(&bytes).collect::<Result<_, _>>().unwrap();
    assert_eq!(items.len(), 1);

    match &items[0] {
        AnyDescriptor::Other { tag, value } => {
            assert_eq!(*tag, MY_PRIVATE_TAG);
            // Should downcast to MyPrivate2 (last registered)
            let concrete2 = value
                .as_any()
                .downcast_ref::<MyPrivate2>()
                .expect("downcast to MyPrivate2 should succeed");
            assert_eq!(concrete2.y, 0x99);
            // MyPrivate downcast should fail
            assert!(
                value.as_any().downcast_ref::<MyPrivate>().is_none(),
                "downcast to MyPrivate should fail after re-registration"
            );
        }
        other => panic!("expected Other, got {other:?}"),
    }
}
