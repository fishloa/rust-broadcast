//! Runtime extension-descriptor registry — open registration of client
//! `descriptor_tag_extension` values.
//!
//! [`ExtensionRegistry`] mirrors [`DescriptorRegistry`][super::super::registry::DescriptorRegistry]
//! for the second-level dispatch inside an extension_descriptor (tag `0x7F`):
//! a registered custom `descriptor_tag_extension` is parsed into a client-supplied
//! owned type and surfaced as [`RegisteredExtension::Custom`]; unregistered
//! extensions fall through to the built-in [`ExtensionDescriptor`] via
//! [`RegisteredExtension::Builtin`].
//!
//! # Owned types only
//!
//! Registered types must be `'static` (i.e. owned — no borrowed slices).
//! This is required because the parsed value is heap-allocated as a
//! `Box<dyn ExtensionObject>` whose concrete type is erased; `dyn Any`
//! downcast demands `'static`.  If your wire layout contains borrowed bytes,
//! copy them into a `Vec<u8>` in the struct.
//!
//! [`DescriptorRegistry`]: super::super::registry::DescriptorRegistry

use std::any::Any;
use std::collections::HashMap;

use super::{validate_and_split, ExtensionBodyDef, ExtensionDescriptor};
use crate::error::Result;

// ---------------------------------------------------------------------------
// ExtensionObject trait
// ---------------------------------------------------------------------------

/// Object-safe face of a runtime-registered extension body value.
///
/// Registered types must be owned (`'static`) because the `dyn Any` downcast
/// path requires it.  See the [module docs][self] for details.
///
/// Implemented automatically via the blanket impl for any `T` satisfying the
/// supertraits; you do not need to write this by hand.
#[cfg(not(feature = "serde"))]
pub trait ExtensionObject: std::fmt::Debug + Any + Send + Sync {
    /// Borrow as `&dyn Any` so the caller can downcast to the concrete type.
    fn as_any(&self) -> &dyn Any;
}

/// Object-safe face of a runtime-registered extension body value.
///
/// Registered types must be owned (`'static`) because the `dyn Any` downcast
/// path requires it.  See the [module docs][self] for details.
///
/// Implemented automatically via the blanket impl for any `T` satisfying the
/// supertraits; you do not need to write this by hand.
#[cfg(feature = "serde")]
pub trait ExtensionObject: std::fmt::Debug + Any + Send + Sync + erased_serde::Serialize {
    /// Borrow as `&dyn Any` so the caller can downcast to the concrete type.
    fn as_any(&self) -> &dyn Any;
}

// Blanket impl — no-serde arm.
#[cfg(not(feature = "serde"))]
impl<T> ExtensionObject for T
where
    T: std::fmt::Debug + Any + Send + Sync,
{
    fn as_any(&self) -> &dyn Any {
        self
    }
}

// Blanket impl — serde arm.
#[cfg(feature = "serde")]
impl<T> ExtensionObject for T
where
    T: std::fmt::Debug + Any + Send + Sync + serde::Serialize,
{
    fn as_any(&self) -> &dyn Any {
        self
    }
}

// Downcast helpers ON THE TRAIT OBJECT (not the blanket).
//
// The blanket `impl<T> ExtensionObject for T` also covers `Box<dyn
// ExtensionObject>` itself whenever the box satisfies the bounds — it does
// under `--no-default-features`, where the bound is just `Debug + Any + Send +
// Sync`. So `the_box.as_any()` resolves to the *box's* impl and reports the
// box's `TypeId`, not the inner value's — a silent downcast failure. (Under
// `serde` the extra `serde::Serialize` bound excludes the box, which is why the
// footgun only bites without default features.) Calling through `dyn
// ExtensionObject` (which `Box` derefs to) always hits the inner value, so
// always downcast via these methods rather than `the_box.as_any()`.
impl dyn ExtensionObject {
    /// Downcast a registered extension body to its concrete type `T`.
    ///
    /// Works for `Box<dyn ExtensionObject>` (it derefs to the trait object)
    /// under every feature configuration.
    #[must_use]
    pub fn downcast_ref<T: Any>(&self) -> Option<&T> {
        self.as_any().downcast_ref::<T>()
    }

    /// `true` if the registered extension body's concrete type is `T`.
    #[must_use]
    pub fn is<T: Any>(&self) -> bool {
        self.as_any().is::<T>()
    }
}

// ---------------------------------------------------------------------------
// Erased serialisation helper (serde-gated)
// ---------------------------------------------------------------------------

/// `serialize_with` helper used on [`RegisteredExtension::Custom`]'s `value`
/// field.
///
/// Delegates to [`erased_serde::serialize`] so the concrete type's
/// `serde::Serialize` impl is invoked through the trait object.
///
/// The `&Box<T>` is required by serde's `serialize_with` codegen — the field
/// type is `Box<dyn ExtensionObject>` so serde passes `&Box<dyn ExtensionObject>`.
#[cfg(feature = "serde")]
#[allow(clippy::borrowed_box)]
pub(crate) fn serialize_erased<S: serde::Serializer>(
    v: &Box<dyn ExtensionObject>,
    s: S,
) -> std::result::Result<S::Ok, S::Error> {
    erased_serde::serialize(&**v, s)
}

// ---------------------------------------------------------------------------
// Internal parse closure type
// ---------------------------------------------------------------------------

/// A heap-allocated parse closure that takes the selector bytes (everything
/// after `descriptor_tag_extension`) and returns an owned, type-erased
/// extension body value.
pub(crate) type CustomParse =
    Box<dyn for<'a> Fn(&'a [u8]) -> Result<Box<dyn ExtensionObject>> + Send + Sync>;

// ---------------------------------------------------------------------------
// ExtensionRegistry
// ---------------------------------------------------------------------------

/// Runtime-configurable extension-descriptor registry.
///
/// By default the registry has no custom parsers; all `descriptor_tag_extension`
/// values fall through to the built-in [`ExtensionDescriptor`]/[`ExtensionBody`]
/// dispatch.  Use [`register`][Self::register] to add a custom type.
///
/// Call [`parse`][Self::parse] on a full extension_descriptor (tag `0x7F`)
/// byte slice; it returns a [`RegisteredExtension`].
///
/// [`ExtensionBody`]: super::ExtensionBody
#[derive(Default)]
pub struct ExtensionRegistry {
    custom: HashMap<u8, CustomParse>,
}

impl ExtensionRegistry {
    /// Create an empty registry (built-in dispatch only).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns `true` if a custom parser is registered for the given
    /// `descriptor_tag_extension`.
    #[must_use]
    pub fn has_custom(&self, tag_extension: u8) -> bool {
        self.custom.contains_key(&tag_extension)
    }

    /// Register an owned custom extension body type for its
    /// [`ExtensionBodyDef::TAG_EXTENSION`].
    ///
    /// # Owned types only
    ///
    /// `T` must be `'static` — no borrowed slices.  The registered value is
    /// type-erased as `Box<dyn ExtensionObject>`; `dyn Any` downcast requires
    /// the concrete type to be `'static`.
    ///
    /// Re-registering the same `TAG_EXTENSION` replaces the prior custom
    /// parser (last wins).
    pub fn register<T>(&mut self) -> &mut Self
    where
        T: for<'a> ExtensionBodyDef<'a> + ExtensionObject + 'static,
    {
        let tag_ext = T::TAG_EXTENSION;
        self.custom.insert(
            tag_ext,
            Box::new(|sel| {
                Ok(Box::new(<T as dvb_common::Parse>::parse(sel)?) as Box<dyn ExtensionObject>)
            }),
        );
        self
    }

    /// Parse the already-split (tag_extension, selector) pair into a
    /// [`RegisteredExtension`], checking for a registered custom parser first
    /// and falling back to the built-in dispatch.
    pub fn parse_body<'a>(
        &self,
        tag_extension: u8,
        selector: &'a [u8],
    ) -> Result<RegisteredExtension<'a>> {
        if let Some(parse_fn) = self.custom.get(&tag_extension) {
            let value = parse_fn(selector)?;
            Ok(RegisteredExtension::Custom {
                tag_extension,
                value,
            })
        } else {
            let body = super::parse_body(tag_extension, selector)?;
            Ok(RegisteredExtension::Builtin(ExtensionDescriptor {
                tag_extension,
                body,
            }))
        }
    }

    /// Parse a full extension_descriptor (tag `0x7F`) byte slice.
    ///
    /// Validates the tag, length, and minimum body size (same checks as
    /// `ExtensionDescriptor::parse`).  If the `descriptor_tag_extension`
    /// has a registered custom parser, returns [`RegisteredExtension::Custom`];
    /// otherwise returns [`RegisteredExtension::Builtin`] with the standard
    /// built-in dispatch.
    pub fn parse<'a>(&self, bytes: &'a [u8]) -> Result<RegisteredExtension<'a>> {
        let (tag_extension, sel) = validate_and_split(bytes)?;
        self.parse_body(tag_extension, sel)
    }
}

// ---------------------------------------------------------------------------
// RegisteredExtension
// ---------------------------------------------------------------------------

/// Output of [`ExtensionRegistry::parse`]: built-in or custom extension.
#[derive(Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
#[non_exhaustive]
pub enum RegisteredExtension<'a> {
    /// Built-in [`ExtensionDescriptor`] (no custom parser registered for this
    /// `descriptor_tag_extension`).
    Builtin(super::ExtensionDescriptor<'a>),
    /// Custom-registered extension body.
    Custom {
        /// The `descriptor_tag_extension` byte.
        tag_extension: u8,
        /// The parsed, type-erased value. Call `downcast_ref` on it (see
        /// [`ExtensionObject`]) to recover the concrete type.
        #[cfg_attr(feature = "serde", serde(serialize_with = "serialize_erased"))]
        value: Box<dyn ExtensionObject>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::descriptors::extension::{ExtensionBodyDef, TAG, TAG_EXTENSION_LEN};
    use crate::error::Error;

    const TEST_TAG_EXTENSION: u8 = 0x40;

    #[derive(Debug, PartialEq, Eq)]
    #[cfg_attr(feature = "serde", derive(serde::Serialize))]
    struct MyExtBody {
        payload: Vec<u8>,
    }

    impl<'a> ExtensionBodyDef<'a> for MyExtBody {
        const TAG_EXTENSION: u8 = TEST_TAG_EXTENSION;
        const NAME: &'static str = "MY_EXT_BODY";
    }

    impl<'a> dvb_common::Parse<'a> for MyExtBody {
        type Error = crate::error::Error;
        fn parse(sel: &'a [u8]) -> Result<Self> {
            Ok(Self {
                payload: sel.to_vec(),
            })
        }
    }

    fn wrap_ext(tag_ext: u8, sel: &[u8]) -> Vec<u8> {
        let mut v = vec![TAG, (sel.len() + TAG_EXTENSION_LEN) as u8, tag_ext];
        v.extend_from_slice(sel);
        v
    }

    #[test]
    fn custom_extension_parsed_and_downcastable() {
        let mut reg = ExtensionRegistry::new();
        reg.register::<MyExtBody>();

        let sel = [0xDE, 0xAD, 0xBE];
        let bytes = wrap_ext(TEST_TAG_EXTENSION, &sel);
        let re = reg.parse(&bytes).unwrap();
        match re {
            RegisteredExtension::Custom {
                tag_extension,
                value,
            } => {
                assert_eq!(tag_extension, TEST_TAG_EXTENSION);
                let concrete = value
                    .downcast_ref::<MyExtBody>()
                    .expect("downcast should succeed");
                assert_eq!(concrete.payload, sel);
            }
            other => panic!("expected Custom, got {other:?}"),
        }
    }

    #[test]
    fn unregistered_tag_extension_yields_builtin() {
        use crate::descriptors::extension::ExtensionBody;
        let reg = ExtensionRegistry::new();
        // service_relocated (0x0B) has a fixed 6-byte selector, which is simple.
        let d = crate::descriptors::extension::ExtensionDescriptor {
            tag_extension: 0x0B,
            body: ExtensionBody::ServiceRelocated(
                crate::descriptors::extension::ServiceRelocated {
                    old_original_network_id: 1,
                    old_transport_stream_id: 2,
                    old_service_id: 3,
                },
            ),
        };
        let mut buf = vec![0u8; d.serialized_len()];
        use dvb_common::Serialize;
        d.serialize_into(&mut buf).unwrap();

        let re = reg.parse(&buf).unwrap();
        match re {
            RegisteredExtension::Builtin(d) => {
                assert_eq!(d.tag_extension, 0x0B);
                assert!(matches!(d.body, ExtensionBody::ServiceRelocated(_)));
            }
            other => panic!("expected Builtin, got {other:?}"),
        }
    }

    #[test]
    fn unknown_tag_extension_yields_builtin_raw() {
        use crate::descriptors::extension::ExtensionBody;
        let reg = ExtensionRegistry::new();
        let sel = [0xAA, 0xBB];
        let bytes = wrap_ext(0xFE, &sel);
        let re = reg.parse(&bytes).unwrap();
        match re {
            RegisteredExtension::Builtin(d) => {
                assert_eq!(d.tag_extension, 0xFE);
                assert!(matches!(d.body, ExtensionBody::Raw(b) if b == sel));
            }
            other => panic!("expected Builtin, got {other:?}"),
        }
    }

    #[test]
    fn parse_rejects_wrong_tag() {
        let reg = ExtensionRegistry::new();
        let raw = [0x43, 1, 0x04];
        assert!(matches!(
            reg.parse(&raw).unwrap_err(),
            Error::InvalidDescriptor { tag: 0x43, .. }
        ));
    }

    #[test]
    fn parse_rejects_short_buffer() {
        let reg = ExtensionRegistry::new();
        let raw = [TAG];
        assert!(matches!(
            reg.parse(&raw).unwrap_err(),
            Error::BufferTooShort { .. }
        ));
    }

    #[test]
    fn parse_rejects_empty_body() {
        let reg = ExtensionRegistry::new();
        let raw = [TAG, 0];
        assert!(matches!(
            reg.parse(&raw).unwrap_err(),
            Error::InvalidDescriptor { tag: TAG, .. }
        ));
    }

    #[cfg(feature = "serde")]
    #[test]
    fn custom_variant_serializes_via_erased_serde() {
        let mut reg = ExtensionRegistry::new();
        reg.register::<MyExtBody>();

        let bytes = wrap_ext(TEST_TAG_EXTENSION, &[0x01, 0x02]);
        let re = reg.parse(&bytes).unwrap();
        let json = serde_json::to_value(&re).unwrap();
        let custom = json.get("custom").expect("expected 'custom' key");
        assert_eq!(custom["tag_extension"], TEST_TAG_EXTENSION as u64);
        assert_eq!(custom["value"]["payload"], serde_json::json!([1, 2]));
    }
}
