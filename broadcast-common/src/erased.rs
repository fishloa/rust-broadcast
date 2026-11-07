//! The type-erased value face behind the runtime registries of `dvb-si`
//! (`DescriptorRegistry`, `TableRegistry`, the extension-descriptor registry)
//! and `dvb-t2mi` (`PayloadRegistry`).
//!
//! A registry holds an owned, `'static` value of a caller-registered type
//! behind `Box<dyn XObject>`. Each registry names its own trait so the
//! `AnyX::Other(Box<dyn XObject>)` variants stay distinct types;
//! [`declare_object_trait!`] generates that trait, its blanket impl, the
//! `downcast_ref` / `is` helpers on the trait object and the
//! `serialize_erased` helper, so the four registries cannot drift apart.
//!
//! The expansion is gated on the *calling* crate's `serde` feature, which must
//! enable `broadcast-common/serde`.

#[cfg(feature = "serde")]
#[doc(hidden)]
pub use {erased_serde, serde};

#[doc(hidden)]
pub use alloc::boxed::Box;

/// Declare an object-safe trait `$Name` for a runtime-registered value, plus
/// its blanket impl, `downcast_ref` / `is` on `dyn $Name`, and (under the
/// calling crate's `serde` feature) a `pub(crate) fn serialize_erased` for a
/// serde `serialize_with` on a `Box<dyn $Name>` field.
///
/// Calling through `dyn $Name` (which `Box` derefs to) always reaches the
/// inner value; `the_box.as_any()` would resolve to the box's own blanket impl
/// under `--no-default-features` and report the box's `TypeId`, so always
/// downcast with the helpers.
#[macro_export]
macro_rules! declare_object_trait {
    ($(#[$meta:meta])* $vis:vis trait $Name:ident, $what:literal) => {
        $(#[$meta])*
        #[cfg(not(feature = "serde"))]
        $vis trait $Name: ::core::fmt::Debug + ::core::any::Any + Send + Sync {
            /// Borrow as `&dyn Any` so the caller can downcast to the concrete type.
            fn as_any(&self) -> &dyn ::core::any::Any;
        }

        $(#[$meta])*
        #[cfg(feature = "serde")]
        $vis trait $Name:
            ::core::fmt::Debug
            + ::core::any::Any
            + Send
            + Sync
            + $crate::erased::erased_serde::Serialize
        {
            /// Borrow as `&dyn Any` so the caller can downcast to the concrete type.
            fn as_any(&self) -> &dyn ::core::any::Any;
        }

        #[cfg(not(feature = "serde"))]
        impl<T> $Name for T
        where
            T: ::core::fmt::Debug + ::core::any::Any + Send + Sync,
        {
            fn as_any(&self) -> &dyn ::core::any::Any {
                self
            }
        }

        #[cfg(feature = "serde")]
        impl<T> $Name for T
        where
            T: ::core::fmt::Debug
                + ::core::any::Any
                + Send
                + Sync
                + $crate::erased::serde::Serialize,
        {
            fn as_any(&self) -> &dyn ::core::any::Any {
                self
            }
        }

        impl dyn $Name {
            #[doc = concat!("Downcast a registered ", $what, " to its concrete type `T`.")]
            ///
            /// Works for a `Box` of the trait object (it derefs to it) under
            /// every feature configuration.
            #[must_use]
            pub fn downcast_ref<T: ::core::any::Any>(&self) -> Option<&T> {
                self.as_any().downcast_ref::<T>()
            }

            #[doc = concat!("`true` if the registered ", $what, "'s concrete type is `T`.")]
            #[must_use]
            pub fn is<T: ::core::any::Any>(&self) -> bool {
                self.as_any().is::<T>()
            }
        }

        /// `serialize_with` helper for a `Box<dyn _>` field: serde passes
        /// `&Box<T>` for such a field, and the concrete value's `Serialize`
        /// is invoked through the trait object.
        #[cfg(feature = "serde")]
        pub(crate) fn serialize_erased<S: $crate::erased::serde::Serializer>(
            v: &$crate::erased::Box<dyn $Name>,
            s: S,
        ) -> ::core::result::Result<S::Ok, S::Error> {
            $crate::erased::erased_serde::serialize(&**v, s)
        }
    };
}
