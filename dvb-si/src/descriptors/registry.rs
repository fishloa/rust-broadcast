//! Runtime descriptor registry — open registration of client private tags.
//!
//! [`DescriptorRegistry`] is a runtime-configurable walk engine that mirrors
//! the semantics of the free [`crate::descriptors::parse_loop`] but allows
//! clients to register their own private descriptor types.  Registered custom
//! parsers win over built-in dispatch; the 0x83 logical_channel built-in is
//! opt-in via [`DescriptorRegistry::with_logical_channel`].
//!
//! # PDS-scoped registration
//!
//! DVB private descriptor tags (`0x80..=0xFE`) are ambiguous without a
//! preceding [`private_data_specifier_descriptor`][crate::descriptors::private_data_specifier]
//! (tag `0x5F`) that scopes them.  Use [`register_for_pds`][DescriptorRegistry::register_for_pds]
//! to register a type that is only dispatched when the active
//! `private_data_specifier` matches.  A PDS-scoped registration takes precedence
//! over a PDS-agnostic [`register`][DescriptorRegistry::register] of the same tag when that
//! PDS is active.
//!
//! # Owned types only
//!
//! Registered types must be `'static` (i.e. owned — no borrowed slices).
//! This is required because the parsed value is heap-allocated as a
//! `Box<dyn DescriptorObject>` whose concrete type is erased; `dyn Any`
//! downcast demands `'static`.  If your wire layout contains borrowed bytes,
//! copy them into a `Vec<u8>` in the struct.
//!
//! # Example
//!
//! ```rust,no_run
//! use dvb_si::descriptors::{DescriptorRegistry, AnyDescriptor};
//! use dvb_si::traits::DescriptorDef;
//! use dvb_common::Parse;
//!
//! // A registered type must be `serde::Serialize` only when the `serde`
//! // feature is on (that is what `DescriptorObject` requires there).
//! #[derive(Debug)]
//! #[cfg_attr(feature = "serde", derive(serde::Serialize))]
//! struct MyPrivate { x: u8 }
//!
//! impl<'a> Parse<'a> for MyPrivate {
//!     type Error = dvb_si::Error;
//!     fn parse(bytes: &'a [u8]) -> dvb_si::Result<Self> {
//!         if bytes.len() < 3 {
//!             return Err(dvb_si::Error::BufferTooShort {
//!                 need: 3, have: bytes.len(), what: "MyPrivate",
//!             });
//!         }
//!         Ok(Self { x: bytes[2] })
//!     }
//! }
//!
//! impl<'a> DescriptorDef<'a> for MyPrivate {
//!     const TAG: u8 = 0xA7;
//!     const NAME: &'static str = "MY_PRIVATE";
//! }
//!
//! let mut reg = DescriptorRegistry::new();
//! reg.register::<MyPrivate>().with_logical_channel();
//!
//! let bytes = [0xA7, 0x01, 0x42u8];
//! let items: Vec<_> = reg.parse_loop(&bytes).collect::<Result<_, _>>().unwrap();
//! if let AnyDescriptor::Other { tag, ref value } = items[0] {
//!     assert_eq!(tag, 0xA7);
//!     assert_eq!(value.downcast_ref::<MyPrivate>().unwrap().x, 0x42);
//! }
//! ```

use std::any::Any;
use std::collections::HashMap;

use crate::descriptors::any::AnyDescriptor;

// ---------------------------------------------------------------------------
// DescriptorObject trait
// ---------------------------------------------------------------------------

/// Object-safe face of a runtime-registered descriptor value.
///
/// Registered types must be owned (`'static`) because the `dyn Any` downcast
/// path requires it.  See the [module docs][self] for details.
///
/// Implemented automatically via the blanket impl for any `T` satisfying the
/// supertraits; you do not need to write this by hand.
#[cfg(not(feature = "serde"))]
pub trait DescriptorObject: std::fmt::Debug + Any + Send + Sync {
    /// Borrow as `&dyn Any` so the caller can downcast to the concrete type.
    fn as_any(&self) -> &dyn Any;
}

/// Object-safe face of a runtime-registered descriptor value.
///
/// Registered types must be owned (`'static`) because the `dyn Any` downcast
/// path requires it.  See the [module docs][self] for details.
///
/// Implemented automatically via the blanket impl for any `T` satisfying the
/// supertraits; you do not need to write this by hand.
#[cfg(feature = "serde")]
pub trait DescriptorObject: std::fmt::Debug + Any + Send + Sync + erased_serde::Serialize {
    /// Borrow as `&dyn Any` so the caller can downcast to the concrete type.
    fn as_any(&self) -> &dyn Any;
}

// Blanket impl — no-serde arm.
#[cfg(not(feature = "serde"))]
impl<T> DescriptorObject for T
where
    T: std::fmt::Debug + Any + Send + Sync,
{
    fn as_any(&self) -> &dyn Any {
        self
    }
}

// Blanket impl — serde arm.
#[cfg(feature = "serde")]
impl<T> DescriptorObject for T
where
    T: std::fmt::Debug + Any + Send + Sync + serde::Serialize,
{
    fn as_any(&self) -> &dyn Any {
        self
    }
}

// Downcast helpers ON THE TRAIT OBJECT (not the blanket).
//
// The blanket `impl<T> DescriptorObject for T` also covers `Box<dyn
// DescriptorObject>` itself whenever the box satisfies the bounds — it does
// under `--no-default-features`, where the bound is just `Debug + Any + Send +
// Sync`. So `the_box.as_any()` resolves to the *box's* impl and reports the
// box's `TypeId`, not the inner value's — a silent downcast failure. (Under
// `serde` the extra `serde::Serialize` bound excludes the box, which is why the
// footgun only bites without default features.) Calling through `dyn
// DescriptorObject` (which `Box` derefs to) always hits the inner value, so
// always downcast via these methods rather than `the_box.as_any()`.
impl dyn DescriptorObject {
    /// Downcast a registered descriptor to its concrete type `T`.
    ///
    /// Works for `Box<dyn DescriptorObject>` (it derefs to the trait object)
    /// under every feature configuration.
    #[must_use]
    pub fn downcast_ref<T: Any>(&self) -> Option<&T> {
        self.as_any().downcast_ref::<T>()
    }

    /// `true` if the registered descriptor's concrete type is `T`.
    #[must_use]
    pub fn is<T: Any>(&self) -> bool {
        self.as_any().is::<T>()
    }
}

// ---------------------------------------------------------------------------
// Erased serialisation helper (serde-gated)
// ---------------------------------------------------------------------------

/// `serialize_with` helper used on [`AnyDescriptor::Other`]'s `value` field.
///
/// Delegates to [`erased_serde::serialize`] so the concrete type's
/// `serde::Serialize` impl is invoked through the trait object.
///
/// The `&Box<T>` is required by serde's `serialize_with` codegen — the field
/// type is `Box<dyn DescriptorObject>` so serde passes `&Box<dyn DescriptorObject>`.
#[cfg(feature = "serde")]
#[allow(clippy::borrowed_box)]
pub(crate) fn serialize_erased<S: serde::Serializer>(
    v: &Box<dyn DescriptorObject>,
    s: S,
) -> Result<S::Ok, S::Error> {
    erased_serde::serialize(&**v, s)
}

// ---------------------------------------------------------------------------
// Internal parse closure type
// ---------------------------------------------------------------------------

/// A heap-allocated parse closure that takes a full descriptor (header + body)
/// and returns an owned, type-erased descriptor value.
pub(crate) type CustomParse =
    Box<dyn for<'a> Fn(&'a [u8]) -> crate::Result<Box<dyn DescriptorObject>> + Send + Sync>;

// ---------------------------------------------------------------------------
// DescriptorRegistry
// ---------------------------------------------------------------------------

/// Runtime-configurable descriptor registry.
///
/// By default the registry has no custom parsers and 0x83 logical_channel is
/// disabled (it is a private tag that requires `private_data_specifier`
/// context).  Use [`register`][Self::register] and
/// [`with_logical_channel`][Self::with_logical_channel] to opt in.
///
/// Walk a byte slice with [`parse_loop`][Self::parse_loop]; it returns a lazy
/// [`RegistryIter`] with identical truncation/fuse/error-continue semantics to
/// the free [`crate::descriptors::parse_loop`].
///
/// # Precedence (per entry)
///
/// 1. PDS-scoped custom parser (if the current `private_data_specifier` matches
///    a [`register_for_pds`][Self::register_for_pds] entry) →
///    [`AnyDescriptor::Other`]
/// 2. PDS-agnostic custom-registered parser (tag in the [`custom`][Self::register]
///    map) → [`AnyDescriptor::Other`]
/// 3. Logical-channel opt-in (tag 0x83 + [`with_logical_channel`][Self::with_logical_channel]
///    enabled) → [`AnyDescriptor::LogicalChannel`]
/// 4. Built-in dispatch (internal `AnyDescriptor::dispatch`) → typed variant
/// 5. Unknown → [`AnyDescriptor::Unknown`]
#[derive(Default)]
pub struct DescriptorRegistry {
    custom: HashMap<(Option<u32>, u8), CustomParse>,
    logical_channel: bool,
}

impl DescriptorRegistry {
    /// Create an empty registry (built-in dispatch only; 0x83 disabled).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register an owned custom descriptor type for its
    /// [`DescriptorDef::TAG`][crate::traits::DescriptorDef::TAG].
    ///
    /// # Owned types only
    ///
    /// `T` must be `'static` — no borrowed slices.  The registered value is
    /// type-erased as `Box<dyn DescriptorObject>`; `dyn Any` downcast requires
    /// the concrete type to be `'static`.
    ///
    /// Registering a type whose `TAG` is already used by a built-in **overrides**
    /// the built-in for that tag.
    ///
    /// Re-registering the same tag replaces the prior custom parser (last wins).
    /// A failing custom parse surfaces the client's `Parse::Error` unwrapped —
    /// embed identifying context (type/tag) in your error's `what`/`reason` fields.
    ///
    /// The registration is PDS-agnostic: it matches the tag regardless of which
    /// `private_data_specifier` (if any) is active in the loop.  A
    /// PDS-scoped registration via [`register_for_pds`][Self::register_for_pds]
    /// takes precedence over this when the matching PDS is active.
    pub fn register<T>(&mut self) -> &mut Self
    where
        T: for<'a> crate::traits::DescriptorDef<'a> + DescriptorObject + 'static,
    {
        let tag = <T as crate::traits::DescriptorDef<'static>>::TAG;
        self.custom.insert(
            (None, tag),
            Box::new(|b| {
                Ok(Box::new(<T as dvb_common::Parse>::parse(b)?) as Box<dyn DescriptorObject>)
            }),
        );
        self
    }

    /// Register an owned custom descriptor type scoped to a specific
    /// `private_data_specifier` value.
    ///
    /// The type is only dispatched when a preceding
    /// [`private_data_specifier_descriptor`][crate::descriptors::private_data_specifier]
    /// (tag `0x5F`) in the same descriptor loop has set the active PDS to
    /// `pds`.  A PDS-scoped registration takes precedence over a PDS-agnostic
    /// [`register`][Self::register] of the same tag when that PDS is active.
    ///
    /// Re-registering the same `(pds, tag)` pair replaces the prior custom
    /// parser (last wins).
    pub fn register_for_pds<T>(&mut self, pds: u32) -> &mut Self
    where
        T: for<'a> crate::traits::DescriptorDef<'a> + DescriptorObject + 'static,
    {
        let tag = <T as crate::traits::DescriptorDef<'static>>::TAG;
        self.custom.insert(
            (Some(pds), tag),
            Box::new(|b| {
                Ok(Box::new(<T as dvb_common::Parse>::parse(b)?) as Box<dyn DescriptorObject>)
            }),
        );
        self
    }

    /// Enable the 0x83 logical_channel built-in.
    ///
    /// By default 0x83 is not auto-dispatched because it is a private tag
    /// whose semantics depend on a `private_data_specifier` context.  Call
    /// this when you know the loop is from an EACEM/NorDig/D-Book stream.
    pub fn with_logical_channel(&mut self) -> &mut Self {
        self.logical_channel = true;
        self
    }

    /// Lazily walk a raw descriptor loop using this registry's configuration.
    ///
    /// Semantics mirror [`crate::descriptors::parse_loop`]: per-descriptor
    /// parse errors yield `Err` and iteration continues; a truncated final
    /// header or body yields one `Err` then fuses.
    ///
    /// A `private_data_specifier_descriptor` (tag `0x5F`) in the loop
    /// automatically updates the iterator's PDS context, scoping subsequent
    /// private-tag dispatch.
    #[must_use]
    pub fn parse_loop<'r, 'a>(&'r self, bytes: &'a [u8]) -> RegistryIter<'r, 'a> {
        RegistryIter {
            registry: self,
            bytes,
            pos: 0,
            fused: false,
            current_pds: None,
        }
    }
}

// ---------------------------------------------------------------------------
// RegistryIter
// ---------------------------------------------------------------------------

/// Lazy iterator over a raw descriptor loop, driven by a [`DescriptorRegistry`].
///
/// Returned by [`DescriptorRegistry::parse_loop`].
pub struct RegistryIter<'r, 'a> {
    registry: &'r DescriptorRegistry,
    bytes: &'a [u8],
    pos: usize,
    fused: bool,
    current_pds: Option<u32>,
}

/// Shared precedence ladder for both [`RegistryIter`] and [`ExtRegistryIter`].
///
/// Returns the [`AnyDescriptor`] that results from applying the 4-step
/// precedence: PDS-scoped custom → PDS-agnostic custom → logical-channel
/// opt-in → built-in dispatch → Unknown.
pub(crate) fn dispatch_entry<'a>(
    registry: &DescriptorRegistry,
    current_pds: Option<u32>,
    tag: u8,
    full: &'a [u8],
) -> crate::Result<AnyDescriptor<'a>> {
    if let Some(pds) = current_pds {
        if let Some(parse_fn) = registry.custom.get(&(Some(pds), tag)) {
            return parse_fn(full).map(|value| AnyDescriptor::Other { tag, value });
        }
    }
    if let Some(parse_fn) = registry.custom.get(&(None, tag)) {
        return parse_fn(full).map(|value| AnyDescriptor::Other { tag, value });
    }
    if registry.logical_channel && tag == crate::descriptors::logical_channel::TAG {
        use dvb_common::Parse;
        return crate::descriptors::logical_channel::LogicalChannelDescriptor::parse(full)
            .map(AnyDescriptor::LogicalChannel);
    }
    if let Some(res) = AnyDescriptor::dispatch(tag, full) {
        return res;
    }
    Ok(AnyDescriptor::Unknown {
        tag,
        body: &full[2..],
    })
}

impl<'r, 'a> Iterator for RegistryIter<'r, 'a> {
    type Item = crate::Result<AnyDescriptor<'a>>;

    fn next(&mut self) -> Option<Self::Item> {
        let (tag, full) = match crate::descriptors::any::next_loop_entry(
            self.bytes,
            &mut self.pos,
            &mut self.fused,
        )? {
            Ok(v) => v,
            Err(e) => return Some(Err(e)),
        };

        if tag == crate::descriptors::private_data_specifier::TAG {
            use dvb_common::Parse;
            if let Ok(pds) =
                crate::descriptors::private_data_specifier::PrivateDataSpecifierDescriptor::parse(
                    full,
                )
            {
                self.current_pds = Some(pds.private_data_specifier);
            }
        }

        Some(dispatch_entry(self.registry, self.current_pds, tag, full))
    }
}

impl std::iter::FusedIterator for RegistryIter<'_, '_> {}

// ---------------------------------------------------------------------------
// ExtIterItem — item type for iter_with_extensions
// ---------------------------------------------------------------------------

/// Item produced by [`DescriptorLoop::iter_with_extensions`](super::any::DescriptorLoop::iter_with_extensions).
///
/// Extends [`AnyDescriptor`] with a third arm for custom-registered extension
/// bodies whose `descriptor_tag_extension` is recognised by an
/// [`ExtensionRegistry`](super::extension::registry::ExtensionRegistry).
/// The `value` field can be downcast to the concrete type via
/// `downcast_ref` on the value (see [`ExtensionObject`](super::extension::registry::ExtensionObject))
#[derive(Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
#[non_exhaustive]
pub enum ExtIterItem<'a> {
    /// A regular descriptor (including built-in extension descriptors).
    Descriptor(AnyDescriptor<'a>),
    /// A custom-registered extension body (tag `0x7F`, known `descriptor_tag_extension`).
    CustomExtension {
        /// The `descriptor_tag_extension` byte.
        tag_extension: u8,
        /// The parsed, type-erased extension body value. Call `downcast_ref`
        /// on it (see [`ExtensionObject`](super::extension::registry::ExtensionObject)) to recover the concrete type.
        #[cfg_attr(
            feature = "serde",
            serde(serialize_with = "super::extension::registry::serialize_erased")
        )]
        value: Box<dyn super::extension::registry::ExtensionObject>,
    },
}

// ---------------------------------------------------------------------------
// ExtRegistryIter — iterator for iter_with_extensions
// ---------------------------------------------------------------------------

/// Lazy iterator over a raw descriptor loop, driven by both a
/// [`DescriptorRegistry`] and an
/// [`ExtensionRegistry`](super::extension::registry::ExtensionRegistry).
///
/// Returned by [`DescriptorLoop::iter_with_extensions`](super::any::DescriptorLoop::iter_with_extensions).
pub struct ExtRegistryIter<'r, 'a> {
    desc_reg: &'r DescriptorRegistry,
    ext_reg: &'r super::extension::registry::ExtensionRegistry,
    bytes: &'a [u8],
    pos: usize,
    fused: bool,
    current_pds: Option<u32>,
}

impl<'r, 'a> ExtRegistryIter<'r, 'a> {
    pub(crate) fn new(
        desc_reg: &'r DescriptorRegistry,
        ext_reg: &'r super::extension::registry::ExtensionRegistry,
        bytes: &'a [u8],
    ) -> Self {
        Self {
            desc_reg,
            ext_reg,
            bytes,
            pos: 0,
            fused: false,
            current_pds: None,
        }
    }
}

impl<'r, 'a> Iterator for ExtRegistryIter<'r, 'a> {
    type Item = crate::Result<ExtIterItem<'a>>;

    fn next(&mut self) -> Option<Self::Item> {
        let (tag, full) = match crate::descriptors::any::next_loop_entry(
            self.bytes,
            &mut self.pos,
            &mut self.fused,
        )? {
            Ok(v) => v,
            Err(e) => return Some(Err(e)),
        };

        if tag == crate::descriptors::private_data_specifier::TAG {
            use dvb_common::Parse;
            if let Ok(pds) =
                crate::descriptors::private_data_specifier::PrivateDataSpecifierDescriptor::parse(
                    full,
                )
            {
                self.current_pds = Some(pds.private_data_specifier);
            }
        }

        let len = full.len() - 2;
        if tag == crate::descriptors::extension::TAG && len >= 1 {
            let tag_extension = full[2];
            if self.ext_reg.has_custom(tag_extension) {
                return Some(
                    match self.ext_reg.parse_body(tag_extension, &full[3..2 + len]) {
                        Ok(super::extension::registry::RegisteredExtension::Custom {
                            tag_extension,
                            value,
                        }) => Ok(ExtIterItem::CustomExtension {
                            tag_extension,
                            value,
                        }),
                        Ok(super::extension::registry::RegisteredExtension::Builtin(d)) => {
                            Ok(ExtIterItem::Descriptor(AnyDescriptor::Extension(d)))
                        }
                        Err(e) => Err(e),
                    },
                );
            }
        }

        Some(
            dispatch_entry(self.desc_reg, self.current_pds, tag, full).map(ExtIterItem::Descriptor),
        )
    }
}

impl std::iter::FusedIterator for ExtRegistryIter<'_, '_> {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::descriptors::private_data_specifier;
    use crate::traits::DescriptorDef;

    const PDS_EACEM: u32 = 0x0000_0028;
    const PDS_NORDIG: u32 = 0x0000_0031;

    #[derive(Debug, PartialEq, Eq)]
    #[cfg_attr(feature = "serde", derive(serde::Serialize))]
    struct PdsEacem {
        v: u8,
    }

    impl<'a> dvb_common::Parse<'a> for PdsEacem {
        type Error = crate::error::Error;
        fn parse(bytes: &'a [u8]) -> crate::Result<Self> {
            if bytes.len() < 3 {
                return Err(crate::error::Error::BufferTooShort {
                    need: 3,
                    have: bytes.len(),
                    what: "PdsEacem",
                });
            }
            Ok(Self { v: bytes[2] })
        }
    }

    impl<'a> DescriptorDef<'a> for PdsEacem {
        const TAG: u8 = 0x83;
        const NAME: &'static str = "PDS_EACEM";
    }

    #[derive(Debug, PartialEq, Eq)]
    #[cfg_attr(feature = "serde", derive(serde::Serialize))]
    struct PdsNordig {
        w: u8,
    }

    impl<'a> dvb_common::Parse<'a> for PdsNordig {
        type Error = crate::error::Error;
        fn parse(bytes: &'a [u8]) -> crate::Result<Self> {
            if bytes.len() < 3 {
                return Err(crate::error::Error::BufferTooShort {
                    need: 3,
                    have: bytes.len(),
                    what: "PdsNordig",
                });
            }
            Ok(Self { w: bytes[2] })
        }
    }

    impl<'a> DescriptorDef<'a> for PdsNordig {
        const TAG: u8 = 0x83;
        const NAME: &'static str = "PDS_NORDIG";
    }

    #[derive(Debug, PartialEq, Eq)]
    #[cfg_attr(feature = "serde", derive(serde::Serialize))]
    struct PdsAgnostic {
        z: u8,
    }

    impl<'a> dvb_common::Parse<'a> for PdsAgnostic {
        type Error = crate::error::Error;
        fn parse(bytes: &'a [u8]) -> crate::Result<Self> {
            if bytes.len() < 3 {
                return Err(crate::error::Error::BufferTooShort {
                    need: 3,
                    have: bytes.len(),
                    what: "PdsAgnostic",
                });
            }
            Ok(Self { z: bytes[2] })
        }
    }

    impl<'a> DescriptorDef<'a> for PdsAgnostic {
        const TAG: u8 = 0x84;
        const NAME: &'static str = "PDS_AGNOSTIC";
    }

    fn pds_descriptor(pds: u32) -> Vec<u8> {
        let mut v = vec![private_data_specifier::TAG, 4];
        v.extend_from_slice(&pds.to_be_bytes());
        v
    }

    #[test]
    fn pds_scoped_same_tag_resolves_by_pds() {
        let mut reg = DescriptorRegistry::new();
        reg.register_for_pds::<PdsEacem>(PDS_EACEM);
        reg.register_for_pds::<PdsNordig>(PDS_NORDIG);

        let mut bytes = Vec::new();
        bytes.extend_from_slice(&pds_descriptor(PDS_EACEM));
        bytes.extend_from_slice(&[0x83, 0x01, 0xAA]);

        let items: Vec<_> = reg.parse_loop(&bytes).collect::<Result<_, _>>().unwrap();
        assert_eq!(items.len(), 2);
        assert!(matches!(items[0], AnyDescriptor::PrivateDataSpecifier(_)));
        match &items[1] {
            AnyDescriptor::Other { tag, value } => {
                assert_eq!(*tag, 0x83);
                let c = value.downcast_ref::<PdsEacem>().unwrap();
                assert_eq!(c.v, 0xAA);
            }
            other => panic!("expected Other (PdsEacem), got {other:?}"),
        }

        let mut bytes2 = Vec::new();
        bytes2.extend_from_slice(&pds_descriptor(PDS_NORDIG));
        bytes2.extend_from_slice(&[0x83, 0x01, 0xBB]);

        let items2: Vec<_> = reg.parse_loop(&bytes2).collect::<Result<_, _>>().unwrap();
        match &items2[1] {
            AnyDescriptor::Other { tag, value } => {
                assert_eq!(*tag, 0x83);
                let c = value.downcast_ref::<PdsNordig>().unwrap();
                assert_eq!(c.w, 0xBB);
            }
            other => panic!("expected Other (PdsNordig), got {other:?}"),
        }
    }

    #[test]
    fn pds_scoped_does_not_match_wrong_pds() {
        let mut reg = DescriptorRegistry::new();
        reg.register_for_pds::<PdsEacem>(PDS_EACEM);

        let mut bytes = Vec::new();
        bytes.extend_from_slice(&pds_descriptor(PDS_NORDIG));
        bytes.extend_from_slice(&[0x83, 0x01, 0xCC]);

        let items: Vec<_> = reg.parse_loop(&bytes).collect::<Result<_, _>>().unwrap();
        assert_eq!(items.len(), 2);
        assert!(matches!(items[0], AnyDescriptor::PrivateDataSpecifier(_)));
        match &items[1] {
            AnyDescriptor::Unknown { tag, .. } => assert_eq!(*tag, 0x83),
            other => panic!("expected Unknown (wrong PDS), got {other:?}"),
        }
    }

    #[test]
    fn pds_agnostic_matches_without_pds() {
        let mut reg = DescriptorRegistry::new();
        reg.register::<PdsAgnostic>();

        let bytes = [0x84, 0x01, 0xDD];
        let items: Vec<_> = reg.parse_loop(&bytes).collect::<Result<_, _>>().unwrap();
        assert_eq!(items.len(), 1);
        match &items[0] {
            AnyDescriptor::Other { tag, value } => {
                assert_eq!(*tag, 0x84);
                let c = value.downcast_ref::<PdsAgnostic>().unwrap();
                assert_eq!(c.z, 0xDD);
            }
            other => panic!("expected Other, got {other:?}"),
        }
    }

    #[test]
    fn pds_scoped_takes_precedence_over_agnostic() {
        // Two parsers compete for the SAME tag 0x83: one PDS-agnostic, one
        // scoped to EACEM. With no PDS active the agnostic one wins; once an
        // EACEM private_data_specifier appears, the scoped one takes over.
        #[derive(Debug, PartialEq, Eq)]
        #[cfg_attr(feature = "serde", derive(serde::Serialize))]
        struct Agnostic83 {
            a: u8,
        }

        impl<'a> dvb_common::Parse<'a> for Agnostic83 {
            type Error = crate::error::Error;
            fn parse(bytes: &'a [u8]) -> crate::Result<Self> {
                if bytes.len() < 3 {
                    return Err(crate::error::Error::BufferTooShort {
                        need: 3,
                        have: bytes.len(),
                        what: "Agnostic83",
                    });
                }
                Ok(Self { a: bytes[2] })
            }
        }

        impl<'a> DescriptorDef<'a> for Agnostic83 {
            const TAG: u8 = 0x83;
            const NAME: &'static str = "AGNOSTIC_83";
        }

        let mut reg = DescriptorRegistry::new();
        reg.register::<Agnostic83>();
        reg.register_for_pds::<PdsEacem>(PDS_EACEM);

        // No PDS → agnostic wins
        let items: Vec<_> = reg
            .parse_loop(&[0x83, 0x01, 0xEE])
            .collect::<Result<_, _>>()
            .unwrap();
        match &items[0] {
            AnyDescriptor::Other { value, .. } => {
                assert!(value.downcast_ref::<Agnostic83>().is_some());
                assert!(value.downcast_ref::<PdsEacem>().is_none());
            }
            other => panic!("expected Other, got {other:?}"),
        }

        // With PDS EACEM → PDS-scoped wins
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&pds_descriptor(PDS_EACEM));
        bytes.extend_from_slice(&[0x83, 0x01, 0xFF]);
        let items: Vec<_> = reg.parse_loop(&bytes).collect::<Result<_, _>>().unwrap();
        match &items[1] {
            AnyDescriptor::Other { value, .. } => {
                assert!(value.downcast_ref::<PdsEacem>().is_some());
                assert!(value.downcast_ref::<Agnostic83>().is_none());
            }
            other => panic!("expected Other, got {other:?}"),
        }
    }

    #[test]
    fn iter_with_extensions_surfaces_custom_extension() {
        use crate::descriptors::any::{AnyDescriptor, DescriptorLoop};
        use crate::descriptors::extension::registry::ExtensionRegistry;

        #[derive(Debug, PartialEq, Eq)]
        #[cfg_attr(feature = "serde", derive(serde::Serialize))]
        struct MyCustomExt {
            payload: Vec<u8>,
        }

        impl<'a> dvb_common::Parse<'a> for MyCustomExt {
            type Error = crate::error::Error;
            fn parse(sel: &'a [u8]) -> crate::Result<Self> {
                Ok(Self {
                    payload: sel.to_vec(),
                })
            }
        }

        impl<'a> crate::descriptors::extension::ExtensionBodyDef<'a> for MyCustomExt {
            const TAG_EXTENSION: u8 = 0x42;
            const NAME: &'static str = "MY_CUSTOM_EXT";
        }

        let mut ext_reg = ExtensionRegistry::new();
        ext_reg.register::<MyCustomExt>();

        let desc_reg = DescriptorRegistry::new();

        // Build a descriptor loop with a short_event + a 0x7F extension with tag_extension 0x42
        let mut loop_bytes = vec![
            0x4D, 0x07, b'e', b'n', b'g', 0x02, b'H', b'i', 0x00, // short_event
        ];
        // extension: tag=0x7F, length=3, tag_extension=0x42, selector=[0xAB, 0xCD]
        loop_bytes.extend_from_slice(&[0x7F, 0x03, 0x42, 0xAB, 0xCD]);

        let dl = DescriptorLoop::new(&loop_bytes);
        let items: Vec<_> = dl
            .iter_with_extensions(&desc_reg, &ext_reg)
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(items.len(), 2);
        // First item is a regular descriptor
        assert!(matches!(
            &items[0],
            ExtIterItem::Descriptor(AnyDescriptor::ShortEvent(_))
        ));
        // Second item surfaces the custom extension body (not Raw!)
        match &items[1] {
            ExtIterItem::CustomExtension {
                tag_extension,
                value,
            } => {
                assert_eq!(*tag_extension, 0x42);
                let concrete = value.downcast_ref::<MyCustomExt>().unwrap();
                assert_eq!(concrete.payload, &[0xAB, 0xCD]);
            }
            other => panic!("expected CustomExtension, got {other:?}"),
        }
    }
}
