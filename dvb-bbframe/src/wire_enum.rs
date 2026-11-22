//! `wire_enum!`: a `#[repr]` spec enum declared together with its checked
//! `TryFrom<repr>` and `From<Self> for repr`, both generated from the one
//! variant table so the conversions cannot drift from the enum.

macro_rules! wire_enum {
    (
        $(#[$meta:meta])*
        $vis:vis enum $name:ident: $repr:ident, invalid($value:ident) => $err:expr,
        {
            $( $(#[$vmeta:meta])* $variant:ident = $disc:literal ),+ $(,)?
        }
    ) => {
        $(#[$meta])*
        #[repr($repr)]
        $vis enum $name {
            $( $(#[$vmeta])* $variant = $disc, )+
        }

        impl TryFrom<$repr> for $name {
            type Error = $crate::error::Error;
            fn try_from($value: $repr) -> core::result::Result<Self, Self::Error> {
                match $value {
                    $( $disc => Ok(Self::$variant), )+
                    _ => Err($err),
                }
            }
        }

        impl From<$name> for $repr {
            fn from(v: $name) -> Self {
                v as $repr
            }
        }
    };
}

pub(crate) use wire_enum;
