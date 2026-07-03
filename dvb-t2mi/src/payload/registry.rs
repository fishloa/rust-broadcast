//! Runtime payload registry — open registration of client private packet types.
//!
//! [`PayloadRegistry`] is a runtime-configurable dispatch engine that allows
//! clients to register their own private T2-MI payload types alongside (or in
//! place of) the built-ins.  Registered custom parsers win over built-in
//! dispatch when [`crate::payload::AnyPayload::dispatch_with`] is used.
//!
//! # Owned types only
//!
//! Registered types must be `'static` (i.e. owned — no borrowed slices).
//! This is required because the parsed value is heap-allocated as a
//! `Box<dyn PayloadObject>` whose concrete type is erased; `dyn Any`
//! downcast demands `'static`.  If your wire layout contains borrowed bytes,
//! copy them into a `Vec<u8>` in the struct.
//!
//! # Example
//!
//! ```rust,no_run
//! use dvb_t2mi::payload::{PayloadRegistry, AnyPayload};
//! use dvb_t2mi::traits::PayloadDef;
//! use broadcast_common::Parse;
//!
//! // A registered type must be `serde::Serialize` only when the `serde`
//! // feature is on (that is what `PayloadObject` requires there).
//! #[derive(Debug)]
//! #[cfg_attr(feature = "serde", derive(serde::Serialize))]
//! struct MyPrivate { x: u8 }
//!
//! impl<'a> Parse<'a> for MyPrivate {
//!     type Error = dvb_t2mi::Error;
//!     fn parse(bytes: &'a [u8]) -> dvb_t2mi::Result<Self> {
//!         if bytes.is_empty() {
//!             return Err(dvb_t2mi::Error::BufferTooShort {
//!                 need: 1, have: 0, what: "MyPrivate",
//!             });
//!         }
//!         Ok(Self { x: bytes[0] })
//!     }
//! }
//!
//! impl<'a> PayloadDef<'a> for MyPrivate {
//!     const PACKET_TYPE: u8 = 0x40;
//!     const NAME: &'static str = "MY_PRIVATE";
//! }
//!
//! let mut reg = PayloadRegistry::new();
//! reg.register::<MyPrivate>();
//!
//! let bytes = [0x42u8];
//! let result = AnyPayload::dispatch_with(
//!     &reg, 0x40, &bytes,
//! ).unwrap().unwrap();
//! if let AnyPayload::Other { packet_type, ref value } = result {
//!     assert_eq!(packet_type, 0x40);
//!     assert_eq!(value.downcast_ref::<MyPrivate>().unwrap().x, 0x42);
//! }
//! ```

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use core::any::Any;

// ---------------------------------------------------------------------------
// PayloadObject trait
// ---------------------------------------------------------------------------

/// Object-safe face of a runtime-registered payload value.
///
/// Registered types must be owned (`'static`) because the `dyn Any` downcast
/// path requires it.  See the [module docs][self] for details.
///
/// Implemented automatically via the blanket impl for any `T` satisfying the
/// supertraits; you do not need to write this by hand.
#[cfg(not(feature = "serde"))]
pub trait PayloadObject: core::fmt::Debug + Any + Send + Sync {
    /// Borrow as `&dyn Any` so the caller can downcast to the concrete type.
    fn as_any(&self) -> &dyn Any;
}

/// Object-safe face of a runtime-registered payload value.
///
/// Registered types must be owned (`'static`) because the `dyn Any` downcast
/// path requires it.  See the [module docs][self] for details.
///
/// Implemented automatically via the blanket impl for any `T` satisfying the
/// supertraits; you do not need to write this by hand.
#[cfg(feature = "serde")]
pub trait PayloadObject: core::fmt::Debug + Any + Send + Sync + erased_serde::Serialize {
    /// Borrow as `&dyn Any` so the caller can downcast to the concrete type.
    fn as_any(&self) -> &dyn Any;
}

// Blanket impl — no-serde arm.
#[cfg(not(feature = "serde"))]
impl<T> PayloadObject for T
where
    T: core::fmt::Debug + Any + Send + Sync,
{
    fn as_any(&self) -> &dyn Any {
        self
    }
}

// Blanket impl — serde arm.
#[cfg(feature = "serde")]
impl<T> PayloadObject for T
where
    T: core::fmt::Debug + Any + Send + Sync + serde::Serialize,
{
    fn as_any(&self) -> &dyn Any {
        self
    }
}

// Downcast helpers ON THE TRAIT OBJECT (not the blanket).
//
// These MUST be inherent methods on `dyn PayloadObject` rather than something
// callable on `Box<dyn PayloadObject>` via the blanket impl. The blanket
// `impl<T> PayloadObject for T` also covers `Box<dyn PayloadObject>` itself
// whenever the box satisfies the bounds (it does under `--no-default-features`,
// where the bound is just `Debug + Any + Send + Sync`). So `the_box.as_any()`
// resolves to the *box's* impl and reports the box's `TypeId`, not the inner
// value's — a silent downcast failure. Calling through `dyn PayloadObject`
// (which `Box` derefs to) always hits the inner value. Always downcast via
// these methods, never `the_box.as_any()`.
impl dyn PayloadObject {
    /// Downcast a registered payload to its concrete type `T`.
    ///
    /// Works for `Box<dyn PayloadObject>` (it derefs to the trait object) under
    /// every feature configuration.
    #[must_use]
    pub fn downcast_ref<T: Any>(&self) -> Option<&T> {
        self.as_any().downcast_ref::<T>()
    }

    /// `true` if the registered payload's concrete type is `T`.
    #[must_use]
    pub fn is<T: Any>(&self) -> bool {
        self.as_any().is::<T>()
    }
}

// ---------------------------------------------------------------------------
// Erased serialisation helper (serde-gated)
// ---------------------------------------------------------------------------

/// `serialize_with` helper used on [`crate::payload::AnyPayload::Other`]'s `value` field.
///
/// Delegates to [`erased_serde::serialize`] so the concrete type's
/// `serde::Serialize` impl is invoked through the trait object.
///
/// The `&Box<T>` is required by serde's `serialize_with` codegen — the field
/// type is `Box<dyn PayloadObject>` so serde passes `&Box<dyn PayloadObject>`.
#[cfg(feature = "serde")]
#[allow(clippy::borrowed_box)]
pub(crate) fn serialize_erased<S: serde::Serializer>(
    v: &Box<dyn PayloadObject>,
    s: S,
) -> Result<S::Ok, S::Error> {
    erased_serde::serialize(&**v, s)
}

// ---------------------------------------------------------------------------
// Internal parse closure type
// ---------------------------------------------------------------------------

/// A heap-allocated parse closure that takes raw payload bytes and returns an
/// owned, type-erased payload value.
pub(crate) type CustomParse =
    Box<dyn for<'a> Fn(&'a [u8]) -> crate::Result<Box<dyn PayloadObject>> + Send + Sync>;

// ---------------------------------------------------------------------------
// PayloadRegistry
// ---------------------------------------------------------------------------

/// Runtime-configurable payload registry.
///
/// By default the registry has no custom parsers.  Use
/// [`register`][Self::register] to add private types, then call
/// [`crate::payload::AnyPayload::dispatch_with`] to dispatch through the registry.
///
/// # Precedence (per entry)
///
/// 1. Custom-registered parser (packet_type in the [`register`][Self::register]
///    map) → [`crate::payload::AnyPayload::Other`]
/// 2. Built-in dispatch (internal [`crate::payload::AnyPayload::dispatch`]) → typed variant
/// 3. Unknown → [`crate::payload::AnyPayload::Unknown`]
#[derive(Default)]
pub struct PayloadRegistry {
    custom: BTreeMap<u8, CustomParse>,
}

impl PayloadRegistry {
    /// Create an empty registry (built-in dispatch only).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register an owned custom payload type for its
    /// [`PayloadDef::PACKET_TYPE`][crate::traits::PayloadDef::PACKET_TYPE].
    ///
    /// # Owned types only
    ///
    /// `T` must be `'static` — no borrowed slices.  The registered value is
    /// type-erased as `Box<dyn PayloadObject>`; `dyn Any` downcast requires
    /// the concrete type to be `'static`.
    ///
    /// Registering a type whose `PACKET_TYPE` is already used by a built-in
    /// **overrides** the built-in for that packet_type (custom wins precedence
    /// in [`crate::payload::AnyPayload::dispatch_with`]).
    ///
    /// Re-registering the same packet_type replaces the prior custom parser
    /// (last wins).  A failing custom parse surfaces the client's
    /// `Parse::Error` unwrapped — embed identifying context (type/ packet_type)
    /// in your error's `what`/`reason` fields.
    pub fn register<T>(&mut self) -> &mut Self
    where
        T: for<'a> crate::traits::PayloadDef<'a> + PayloadObject + 'static,
    {
        // Name PACKET_TYPE without a lifetime — `for<'a> PayloadDef<'a>`
        // guarantees the const is identical for all lifetimes.
        let packet_type = <T as crate::traits::PayloadDef<'static>>::PACKET_TYPE;
        self.custom.insert(
            packet_type,
            Box::new(|b| {
                Ok(Box::new(<T as broadcast_common::Parse>::parse(b)?) as Box<dyn PayloadObject>)
            }),
        );
        self
    }

    /// Look up a custom parser for `packet_type` — `None` if not registered.
    #[must_use]
    pub(crate) fn lookup(&self, packet_type: u8) -> Option<&CustomParse> {
        self.custom.get(&packet_type)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use broadcast_common::Parse;

    // A custom owned payload type for testing, using an unused packet_type.
    #[derive(Debug, PartialEq)]
    #[cfg_attr(feature = "serde", derive(serde::Serialize))]
    struct TestPayload {
        val: u8,
    }

    const TEST_PACKET_TYPE: u8 = 0x40;

    impl<'a> Parse<'a> for TestPayload {
        type Error = crate::Error;

        fn parse(bytes: &'a [u8]) -> crate::Result<Self> {
            if bytes.is_empty() {
                return Err(crate::Error::BufferTooShort {
                    need: 1,
                    have: 0,
                    what: "TestPayload",
                });
            }
            Ok(Self { val: bytes[0] })
        }
    }

    impl crate::traits::PayloadDef<'_> for TestPayload {
        const PACKET_TYPE: u8 = TEST_PACKET_TYPE;
        const NAME: &'static str = "TEST_PAYLOAD";
    }

    #[test]
    fn register_and_dispatch_returns_other() {
        let mut reg = PayloadRegistry::new();
        reg.register::<TestPayload>();

        let bytes = [0x42u8];
        let result = crate::payload::AnyPayload::dispatch_with(&reg, TEST_PACKET_TYPE, &bytes);
        let parsed = result.unwrap().unwrap();
        match parsed {
            crate::payload::AnyPayload::Other {
                packet_type,
                ref value,
            } => {
                assert_eq!(packet_type, TEST_PACKET_TYPE);
                let tp = value.downcast_ref::<TestPayload>().unwrap();
                assert_eq!(tp.val, 0x42);
            }
            _ => panic!("expected Other, got {parsed:?}"),
        }
    }

    #[test]
    fn dispatch_with_falls_back_to_builtin() {
        let reg = PayloadRegistry::new();
        // 0x00 is Bbframe — not in registry, falls back to built-in.
        let bytes = [0x00, 0x00, 0x00]; // minimal valid BBFrame payload
        let result = crate::payload::AnyPayload::dispatch_with(&reg, 0x00, &bytes);
        let parsed = result.unwrap().unwrap();
        assert!(
            matches!(parsed, crate::payload::AnyPayload::Bbframe(_)),
            "expected Bbframe, got {parsed:?}"
        );
    }

    #[test]
    fn custom_overrides_builtin_packet_type() {
        // Use a type that claims the built-in 0x00 Bbframe packet_type.
        #[derive(Debug)]
        #[cfg_attr(feature = "serde", derive(serde::Serialize))]
        struct OverridePayload;

        const OVERRIDE_PT: u8 = 0x00;

        impl<'a> Parse<'a> for OverridePayload {
            type Error = crate::Error;
            fn parse(_bytes: &'a [u8]) -> crate::Result<Self> {
                Ok(Self)
            }
        }

        impl crate::traits::PayloadDef<'_> for OverridePayload {
            const PACKET_TYPE: u8 = OVERRIDE_PT;
            const NAME: &'static str = "OVERRIDE";
        }

        let mut reg = PayloadRegistry::new();
        reg.register::<OverridePayload>();

        let bytes = [0x00, 0x00, 0x00];
        let result = crate::payload::AnyPayload::dispatch_with(&reg, 0x00, &bytes);
        let parsed = result.unwrap().unwrap();
        assert!(
            matches!(
                parsed,
                crate::payload::AnyPayload::Other {
                    packet_type: 0x00,
                    ..
                }
            ),
            "expected Other override for 0x00, got {parsed:?}"
        );
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_other_round_trips_through_json() {
        let mut reg = PayloadRegistry::new();
        reg.register::<TestPayload>();

        let bytes = [0x7Fu8];
        let result = crate::payload::AnyPayload::dispatch_with(&reg, TEST_PACKET_TYPE, &bytes);
        let parsed = result.unwrap().unwrap();

        let json = serde_json::to_value(&parsed).unwrap();
        let obj = json
            .as_object()
            .unwrap()
            .get("other")
            .expect("expected 'other' key");
        assert_eq!(obj["packet_type"], TEST_PACKET_TYPE);
        assert_eq!(obj["value"]["val"], 0x7F);
    }
}
