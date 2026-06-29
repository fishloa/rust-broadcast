//! Runtime table registry — open registration of client private table_ids.
//!
//! [`TableRegistry`] allows clients to register their own private table types.
//! Registered custom parsers win over built-in dispatch; an unregistered
//! table_id falls through to [`crate::tables::AnyTableSection::Unknown`].
//!
//! # Owned types only
//!
//! Registered types must be `'static` (i.e. owned — no borrowed slices).
//! This is required because the parsed value is heap-allocated as a
//! `Box<dyn TableObject>` whose concrete type is erased; `dyn Any`
//! downcast demands `'static`.  If your wire layout contains borrowed bytes,
//! copy them into a `Vec<u8>` in the struct.
//!
//! # Example
//!
//! ```rust,no_run
//! use dvb_si::tables::{TableRegistry, AnyTableSection};
//! use dvb_si::traits::TableDef;
//! use broadcast_common::Parse;
//!
//! // A registered type must be `serde::Serialize` only when the `serde`
//! // feature is on (that is what `TableObject` requires there).
//! #[derive(Debug)]
//! #[cfg_attr(feature = "serde", derive(serde::Serialize))]
//! struct MyPrivate { x: u8 }
//!
//! impl<'a> Parse<'a> for MyPrivate {
//!     type Error = dvb_si::Error;
//!     fn parse(bytes: &'a [u8]) -> dvb_si::Result<Self> {
//!         if bytes.len() < 2 {
//!             return Err(dvb_si::Error::BufferTooShort {
//!                 need: 2, have: bytes.len(), what: "MyPrivate",
//!             });
//!         }
//!         Ok(Self { x: bytes[1] })
//!     }
//! }
//!
//! impl<'a> TableDef<'a> for MyPrivate {
//!     const TABLE_ID_RANGES: &'static [(u8, u8)] = &[(0x90, 0x90)];
//!     const NAME: &'static str = "MY_PRIVATE";
//! }
//!
//! let mut reg = TableRegistry::new();
//! reg.register::<MyPrivate>();
//!
//! let bytes = [0x90, 0x00, 0x42u8];
//! match AnyTableSection::parse_with(&reg, &bytes).unwrap() {
//!     AnyTableSection::Other { table_id, ref value } => {
//!         assert_eq!(table_id, 0x90);
//!         assert_eq!(value.downcast_ref::<MyPrivate>().unwrap().x, 0x42);
//!     }
//!     other => panic!("expected Other, got {other:?}"),
//! }
//! ```

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use core::any::Any;

// ---------------------------------------------------------------------------
// TableObject trait
// ---------------------------------------------------------------------------

/// Object-safe face of a runtime-registered table-section value.
///
/// Registered types must be owned (`'static`) because the `dyn Any` downcast
/// path requires it.  See the [module docs][self] for details.
///
/// Implemented automatically via the blanket impl for any `T` satisfying the
/// supertraits; you do not need to write this by hand.
#[cfg(not(feature = "serde"))]
pub trait TableObject: core::fmt::Debug + Any + Send + Sync {
    /// Borrow as `&dyn Any` so the caller can downcast to the concrete type.
    fn as_any(&self) -> &dyn Any;
}

/// Object-safe face of a runtime-registered table-section value.
///
/// Registered types must be owned (`'static`) because the `dyn Any` downcast
/// path requires it.  See the [module docs][self] for details.
///
/// Implemented automatically via the blanket impl for any `T` satisfying the
/// supertraits; you do not need to write this by hand.
#[cfg(feature = "serde")]
pub trait TableObject: core::fmt::Debug + Any + Send + Sync + erased_serde::Serialize {
    /// Borrow as `&dyn Any` so the caller can downcast to the concrete type.
    fn as_any(&self) -> &dyn Any;
}

// Blanket impl — no-serde arm.
#[cfg(not(feature = "serde"))]
impl<T> TableObject for T
where
    T: core::fmt::Debug + Any + Send + Sync,
{
    fn as_any(&self) -> &dyn Any {
        self
    }
}

// Blanket impl — serde arm.
#[cfg(feature = "serde")]
impl<T> TableObject for T
where
    T: core::fmt::Debug + Any + Send + Sync + serde::Serialize,
{
    fn as_any(&self) -> &dyn Any {
        self
    }
}

// Downcast helpers ON THE TRAIT OBJECT (not the blanket).
//
// The blanket `impl<T> TableObject for T` also covers `Box<dyn TableObject>`
// itself whenever the box satisfies the bounds — it does under
// `--no-default-features`, where the bound is just `Debug + Any + Send + Sync`.
// So `the_box.as_any()` resolves to the *box's* impl and reports the box's
// `TypeId`, not the inner value's — a silent downcast failure. (Under `serde`
// the extra `serde::Serialize` bound excludes the box, which is why the footgun
// only bites without default features.) Calling through `dyn TableObject`
// (which `Box` derefs to) always hits the inner value, so always downcast via
// these methods rather than `the_box.as_any()`.
impl dyn TableObject {
    /// Downcast a registered table section to its concrete type `T`.
    ///
    /// Works for `Box<dyn TableObject>` (it derefs to the trait object) under
    /// every feature configuration.
    #[must_use]
    pub fn downcast_ref<T: Any>(&self) -> Option<&T> {
        self.as_any().downcast_ref::<T>()
    }

    /// `true` if the registered table section's concrete type is `T`.
    #[must_use]
    pub fn is<T: Any>(&self) -> bool {
        self.as_any().is::<T>()
    }
}

// ---------------------------------------------------------------------------
// Erased serialisation helper (serde-gated)
// ---------------------------------------------------------------------------

/// `serialize_with` helper used on [`AnyTableSection::Other`]'s `value` field.
///
/// Delegates to [`erased_serde::serialize`] so the concrete type's
/// `serde::Serialize` impl is invoked through the trait object.
///
/// The `&Box<T>` is required by serde's `serialize_with` codegen — the field
/// type is `Box<dyn TableObject>` so serde passes `&Box<dyn TableObject>`.
#[cfg(feature = "serde")]
#[allow(clippy::borrowed_box)]
pub(crate) fn serialize_erased<S: serde::Serializer>(
    v: &Box<dyn TableObject>,
    s: S,
) -> Result<S::Ok, S::Error> {
    erased_serde::serialize(&**v, s)
}

// ---------------------------------------------------------------------------
// Internal parse closure type
// ---------------------------------------------------------------------------

/// A heap-allocated parse closure that takes a full section (header + body)
/// and returns an owned, type-erased table-section value.
pub(crate) type CustomParse =
    Box<dyn for<'a> Fn(&'a [u8]) -> crate::Result<Box<dyn TableObject>> + Send + Sync>;

// ---------------------------------------------------------------------------
// TableRegistry
// ---------------------------------------------------------------------------

/// Runtime-configurable table registry.
///
/// By default the registry has no custom parsers.  Use
/// [`register`][Self::register] to add one.
///
/// # Precedence (per table_id)
///
/// 1. Custom-registered parser (table_id in the [`custom`][Self::register] map) →
///    [`crate::tables::AnyTableSection::Other`]
/// 2. Built-in dispatch ([`crate::tables::AnyTableSection::parse`]) → typed variant
/// 3. Unknown → [`crate::tables::AnyTableSection::Unknown`]
#[derive(Default)]
pub struct TableRegistry {
    custom: BTreeMap<u8, CustomParse>,
}

impl TableRegistry {
    /// Create an empty registry (built-in dispatch only).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Look up a custom parser for the given `table_id`.
    #[must_use]
    pub(crate) fn lookup(&self, table_id: u8) -> Option<&CustomParse> {
        self.custom.get(&table_id)
    }

    /// Register an owned custom table-section type for every table_id in its
    /// [`TableDef::TABLE_ID_RANGES`][crate::traits::TableDef::TABLE_ID_RANGES].
    ///
    /// # Owned types only
    ///
    /// `T` must be `'static` — no borrowed slices.  The registered value is
    /// type-erased as `Box<dyn TableObject>`; `dyn Any` downcast requires
    /// the concrete type to be `'static`.
    ///
    /// # Multi-range registration
    ///
    /// A type may cover multiple table_id ranges (e.g.
    /// `&[(0x90, 0x90), (0x92, 0x93)]`).  Every table_id in every range is
    /// registered under the same parse closure.
    ///
    /// Registering a type whose table_id is already used by a built-in **overrides**
    /// the built-in for that table_id.
    ///
    /// Re-registering the same table_id replaces the prior custom parser (last wins).
    /// A failing custom parse surfaces the client's `Parse::Error` unwrapped —
    /// embed identifying context (type/table_id) in your error's `what`/`reason` fields.
    pub fn register<T>(&mut self) -> &mut Self
    where
        T: for<'a> crate::traits::TableDef<'a> + TableObject + 'static,
    {
        let ranges = <T as crate::traits::TableDef<'static>>::TABLE_ID_RANGES;
        for &(lo, hi) in ranges {
            for id in lo..=hi {
                self.custom.insert(
                    id,
                    Box::new(|b| {
                        Ok(Box::new(<T as broadcast_common::Parse>::parse(b)?) as Box<dyn TableObject>)
                    }),
                );
            }
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tables::AnyTableSection;
    use crate::traits::TableDef;
    use broadcast_common::Parse;

    // -- A trivial custom table for a private table_id -------------------------
    const CUSTOM_TABLE_ID: u8 = 0x90;

    #[derive(Debug, PartialEq)]
    #[cfg_attr(feature = "serde", derive(serde::Serialize))]
    struct CustomTable {
        table_id: u8,
        payload: Vec<u8>,
    }

    impl<'a> Parse<'a> for CustomTable {
        type Error = crate::Error;
        fn parse(bytes: &'a [u8]) -> crate::Result<Self> {
            if bytes.is_empty() {
                return Err(crate::Error::BufferTooShort {
                    need: 1,
                    have: 0,
                    what: "CustomTable",
                });
            }
            Ok(Self {
                table_id: bytes[0],
                payload: bytes[1..].to_vec(),
            })
        }
    }

    impl<'a> TableDef<'a> for CustomTable {
        const TABLE_ID_RANGES: &'static [(u8, u8)] = &[(CUSTOM_TABLE_ID, CUSTOM_TABLE_ID)];
        const NAME: &'static str = "CUSTOM_TABLE";
    }

    // -- A multi-range custom table --------------------------------------------
    const MULTI_LO: u8 = 0x90;
    const MULTI_MID: u8 = 0x92;
    const MULTI_HI: u8 = 0x93;

    #[derive(Debug, PartialEq)]
    #[cfg_attr(feature = "serde", derive(serde::Serialize))]
    struct MultiRangeTable {
        table_id: u8,
    }

    impl<'a> Parse<'a> for MultiRangeTable {
        type Error = crate::Error;
        fn parse(bytes: &'a [u8]) -> crate::Result<Self> {
            if bytes.is_empty() {
                return Err(crate::Error::BufferTooShort {
                    need: 1,
                    have: 0,
                    what: "MultiRangeTable",
                });
            }
            Ok(Self { table_id: bytes[0] })
        }
    }

    impl<'a> TableDef<'a> for MultiRangeTable {
        const TABLE_ID_RANGES: &'static [(u8, u8)] = &[(MULTI_LO, MULTI_LO), (MULTI_MID, MULTI_HI)];
        const NAME: &'static str = "MULTI_RANGE_TABLE";
    }

    // -- A custom table that overrides a built-in (PAT = 0x00) -----------------
    #[derive(Debug, PartialEq)]
    #[cfg_attr(feature = "serde", derive(serde::Serialize))]
    struct OverridePat {
        table_id: u8,
    }

    impl<'a> Parse<'a> for OverridePat {
        type Error = crate::Error;
        fn parse(bytes: &'a [u8]) -> crate::Result<Self> {
            if bytes.is_empty() {
                return Err(crate::Error::BufferTooShort {
                    need: 1,
                    have: 0,
                    what: "OverridePat",
                });
            }
            Ok(Self { table_id: bytes[0] })
        }
    }

    impl<'a> TableDef<'a> for OverridePat {
        const TABLE_ID_RANGES: &'static [(u8, u8)] = &[(0x00, 0x00)];
        const NAME: &'static str = "OVERRIDE_PAT";
    }

    #[test]
    fn custom_table_dispatches_to_other() {
        let mut reg = TableRegistry::new();
        reg.register::<CustomTable>();

        let bytes = [CUSTOM_TABLE_ID, 0xAA, 0xBB];
        let result = AnyTableSection::parse_with(&reg, &bytes).unwrap();
        match result {
            AnyTableSection::Other {
                table_id,
                ref value,
            } => {
                assert_eq!(table_id, CUSTOM_TABLE_ID);
                let ct = value.downcast_ref::<CustomTable>().unwrap();
                assert_eq!(ct.table_id, CUSTOM_TABLE_ID);
                assert_eq!(ct.payload, &[0xAA, 0xBB]);
            }
            other => panic!("expected Other, got {other:?}"),
        }
    }

    #[test]
    fn multi_range_registers_all_ids() {
        let mut reg = TableRegistry::new();
        reg.register::<MultiRangeTable>();

        assert!(reg.lookup(MULTI_LO).is_some());
        assert!(reg.lookup(MULTI_MID).is_some());
        assert!(reg.lookup(MULTI_MID + 1).is_some());
        assert!(reg.lookup(MULTI_HI).is_some());
        // 0x91 is NOT in any range
        assert!(reg.lookup(MULTI_LO + 1).is_none());

        // Each registered id dispatches to Other
        for id in [MULTI_LO, MULTI_MID, MULTI_MID + 1, MULTI_HI] {
            let bytes = [id, 0x00];
            let result = AnyTableSection::parse_with(&reg, &bytes).unwrap();
            match result {
                AnyTableSection::Other { table_id, .. } => assert_eq!(table_id, id),
                other => panic!("id {id:#04x}: expected Other, got {other:?}"),
            }
        }

        // 0x91 falls to Unknown
        let bytes = [MULTI_LO + 1, 0x00];
        let result = AnyTableSection::parse_with(&reg, &bytes).unwrap();
        assert!(
            matches!(result, AnyTableSection::Unknown { .. }),
            "expected Unknown for unregistered id 0x91, got {result:?}",
        );
    }

    #[test]
    fn fallback_to_builtin_for_unregistered() {
        let reg = TableRegistry::new();
        // Build a minimal PAT (table_id 0x00)
        use crate::tables::pat::{PatEntry, PatSection};
        use broadcast_common::Serialize;
        let pat = PatSection {
            transport_stream_id: 1,
            version_number: 0,
            current_next_indicator: true,
            section_number: 0,
            last_section_number: 0,
            entries: vec![PatEntry {
                program_number: 1,
                pid: 0x0100,
            }],
        };
        let mut buf = vec![0u8; pat.serialized_len()];
        pat.serialize_into(&mut buf).unwrap();

        let result = AnyTableSection::parse_with(&reg, &buf).unwrap();
        match result {
            AnyTableSection::PatSection(p) => assert_eq!(p.entries.len(), 1),
            other => panic!("expected PatSection, got {other:?}"),
        }
    }

    #[test]
    fn override_builtin_yields_other() {
        let mut reg = TableRegistry::new();
        reg.register::<OverridePat>();

        // Minimal PAT bytes (just table_id + enough to not error in OverridePat)
        let bytes = [0x00u8, 0x01, 0x02];
        let result = AnyTableSection::parse_with(&reg, &bytes).unwrap();
        match result {
            AnyTableSection::Other {
                table_id,
                ref value,
            } => {
                assert_eq!(table_id, 0x00);
                let op = value.downcast_ref::<OverridePat>().unwrap();
                assert_eq!(op.table_id, 0x00);
            }
            other => panic!("expected Other (override), got {other:?}"),
        }
    }

    #[test]
    fn empty_bytes_returns_buffer_too_short() {
        let reg = TableRegistry::new();
        let result = AnyTableSection::parse_with(&reg, &[]);
        assert!(matches!(
            result,
            Err(crate::Error::BufferTooShort {
                need: 1,
                have: 0,
                ..
            })
        ));
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_other_variant() {
        let mut reg = TableRegistry::new();
        reg.register::<CustomTable>();

        let bytes = [CUSTOM_TABLE_ID, 0xAA, 0xBB];
        let result = AnyTableSection::parse_with(&reg, &bytes).unwrap();

        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("\"other\""), "unexpected JSON: {json}");
    }
}
