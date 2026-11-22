//! `wire_enum!`: a `#[repr]` spec enum declared together with its checked
//! `TryFrom<repr>` and `From<Self> for repr`, both generated from the one
//! variant table so the conversions cannot drift from the enum.
//!
//! Every crate's conversions fail into that crate's own `Error`, so the error
//! type is a macro parameter rather than a fixed path.

/// Declare a `#[repr]` spec enum together with its checked `TryFrom<repr>` and
/// `From<Self> for repr`, all generated from one variant table.
///
/// ```
/// use core::convert::TryFrom;
///
/// #[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// pub struct NotAFruit(u8);
///
/// broadcast_common::wire_enum! {
///     #[derive(Debug, Clone, Copy, PartialEq, Eq)]
///     pub enum Fruit: u8, error(NotAFruit), invalid(value) => NotAFruit(value),
///     {
///         Apple = 1,
///         Pear = 2,
///     }
/// }
///
/// assert_eq!(Fruit::try_from(2), Ok(Fruit::Pear));
/// assert_eq!(Fruit::try_from(3), Err(NotAFruit(3)));
/// assert_eq!(u8::from(Fruit::Apple), 1);
/// ```
#[macro_export]
macro_rules! wire_enum {
    (
        $(#[$meta:meta])*
        $vis:vis enum $name:ident: $repr:ident, error($error:ty), invalid($value:ident) => $err:expr,
        {
            $( $(#[$vmeta:meta])* $variant:ident = $disc:literal ),+ $(,)?
        }
    ) => {
        $(#[$meta])*
        #[repr($repr)]
        $vis enum $name {
            $( $(#[$vmeta])* $variant = $disc, )+
        }

        impl core::convert::TryFrom<$repr> for $name {
            type Error = $error;

            fn try_from($value: $repr) -> core::result::Result<Self, Self::Error> {
                match $value {
                    $( $disc => Ok(Self::$variant), )+
                    _ => Err($err),
                }
            }
        }

        impl core::convert::From<$name> for $repr {
            fn from(v: $name) -> Self {
                v as $repr
            }
        }
    };
}
