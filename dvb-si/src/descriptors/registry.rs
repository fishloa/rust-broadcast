//! Runtime descriptor registry — open registration of client private tags.
//!
//! [`DescriptorRegistry`] is a runtime-configurable walk engine that mirrors
//! the semantics of the free [`crate::descriptors::parse_loop`] but allows
//! clients to register their own private descriptor types.  Registered custom
//! parsers win over built-in dispatch; the 0x83 logical_channel built-in is
//! opt-in via [`DescriptorRegistry::with_logical_channel`].
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
//! use dvb_si::descriptors::{DescriptorRegistry, DescriptorObject, AnyDescriptor};
//! use dvb_si::traits::DescriptorDef;
//! use dvb_common::Parse;
//!
//! #[derive(Debug, serde::Serialize)]
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
//!     assert_eq!(value.as_any().downcast_ref::<MyPrivate>().unwrap().x, 0x42);
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
/// 1. Custom-registered parser (tag in the [`custom`][Self::register] map) →
///    [`AnyDescriptor::Other`]
/// 2. Logical-channel opt-in (tag 0x83 + [`with_logical_channel`][Self::with_logical_channel]
///    enabled) → [`AnyDescriptor::LogicalChannel`]
/// 3. Built-in dispatch (internal `AnyDescriptor::dispatch`) → typed variant
/// 4. Unknown → [`AnyDescriptor::Unknown`]
#[derive(Default)]
pub struct DescriptorRegistry {
    custom: HashMap<u8, CustomParse>,
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
    pub fn register<T>(&mut self) -> &mut Self
    where
        T: for<'a> crate::traits::DescriptorDef<'a> + DescriptorObject + 'static,
    {
        // We need to name the TAG without a lifetime — use the 'static elision.
        // `for<'a> DescriptorDef<'a>` guarantees the const is the same for all
        // lifetimes, so calling it with 'static here is fine.
        let tag = <T as crate::traits::DescriptorDef<'static>>::TAG;
        self.custom.insert(
            tag,
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
    #[must_use]
    pub fn parse_loop<'r, 'a>(&'r self, bytes: &'a [u8]) -> RegistryIter<'r, 'a> {
        RegistryIter {
            registry: self,
            bytes,
            pos: 0,
            fused: false,
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
}

impl<'r, 'a> Iterator for RegistryIter<'r, 'a> {
    type Item = crate::Result<AnyDescriptor<'a>>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.fused || self.pos >= self.bytes.len() {
            return None;
        }
        let rem = &self.bytes[self.pos..];
        // --- shared loop-walk arithmetic (mirrors DescriptorIter::next) ---
        if rem.len() < 2 {
            self.fused = true;
            return Some(Err(crate::Error::BufferTooShort {
                need: 2,
                have: rem.len(),
                what: "descriptor header in loop",
            }));
        }
        let tag = rem[0];
        let len = rem[1] as usize;
        let total = 2 + len;
        if rem.len() < total {
            self.fused = true;
            return Some(Err(crate::Error::BufferTooShort {
                need: total,
                have: rem.len(),
                what: "descriptor body in loop",
            }));
        }
        let full = &rem[..total];
        self.pos += total;
        // --- precedence ---
        // 1. Custom-registered parser
        if let Some(parse_fn) = self.registry.custom.get(&tag) {
            return Some(match parse_fn(full) {
                Ok(value) => Ok(AnyDescriptor::Other { tag, value }),
                Err(e) => Err(e),
            });
        }
        // 2. Logical-channel opt-in (0x83)
        if self.registry.logical_channel && tag == crate::descriptors::logical_channel::TAG {
            use dvb_common::Parse;
            return Some(
                crate::descriptors::logical_channel::LogicalChannelDescriptor::parse(full)
                    .map(AnyDescriptor::LogicalChannel),
            );
        }
        // 3. Built-in dispatch
        if let Some(res) = AnyDescriptor::dispatch(tag, full) {
            return Some(res);
        }
        // 4. Unknown
        Some(Ok(AnyDescriptor::Unknown {
            tag,
            body: &full[2..],
        }))
    }
}

impl std::iter::FusedIterator for RegistryIter<'_, '_> {}
