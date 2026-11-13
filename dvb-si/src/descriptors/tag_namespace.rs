//! One macro for the closed (non-scoped) descriptor namespaces: AIT and the
//! data-carousel layer. Each is its own tag space with no fall-through to the
//! EN 300 468 dispatch (the scoped INT/UNT/RNT tables use
//! [`declare_scoped_namespace!`](crate::descriptors::scoped), which has one).

/// Declares a namespace dispatch enum + dispatcher from one tag list.
///
/// Mirrors the `declare_descriptors!` macro in `crate::descriptors::any`. Each
/// line is `Variant = 0xTAG => crate::descriptors::<ns>::module::Type[<'a>]`.
macro_rules! declare_tag_namespace {
    (
        $lt:lifetime;
        enum $Enum:ident, space $space:literal, spec $spec:literal, names $names:literal;
        $( $variant:ident = $tag:literal => $($path:ident)::+ $(<$plt:lifetime>)? ),+ $(,)?
    ) => {
        #[doc = concat!("Every crate-implemented ", $space, " descriptor, plus an `Unknown` fallthrough.")]
        ///
        #[doc = concat!(
            $space, " descriptor tags belong to their own tag space (", $spec,
            "), separate from the EN 300 468 SI tags. The same numeric tag value may map to a different type here."
        )]
        #[derive(Debug)]
        #[cfg_attr(feature = "serde", derive(serde::Serialize))]
        #[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
        #[non_exhaustive]
        pub enum $Enum<$lt> {
            $(
                #[doc = concat!("The `", stringify!($variant), "` descriptor.")]
            $variant($($path)::+ $(<$plt>)?),
            )+
            /// Tag with no typed implementation; `body` is the payload sans
            /// the 2-byte (tag, length) header.
            Unknown {
                /// The raw descriptor_tag byte.
                tag: u8,
                /// The raw payload bytes (descriptor_length bytes).
                body: &$lt [u8],
            },
        }

        $(
            impl<$lt> From<$($path)::+ $(<$plt>)?> for $Enum<$lt> {
                fn from(d: $($path)::+ $(<$plt>)?) -> Self {
                    Self::$variant(d)
                }
            }
        )+

        impl<$lt> $Enum<$lt> {
            /// Every tag the generated dispatcher routes (excludes `Unknown`).
            pub const DISPATCHED_TAGS: &'static [u8] = &[$($tag),+];

            #[doc = concat!(
                "Diagnostic name of the contained ", $space, " descriptor: the type's `DescriptorDef::NAME` (",
                $names, "); `\"UNKNOWN\"` for `Unknown`."
            )]
            #[must_use]
            pub fn name(&self) -> &'static str {
                match self {
                    $(
                        Self::$variant(_) =>
                            <$($path)::+ as crate::traits::DescriptorDef>::NAME,
                    )+
                    Self::Unknown { .. } => "UNKNOWN",
                }
            }

            /// Parse one full descriptor (2-byte header included) by its tag.
            ///
            /// `None` means no typed implementation exists for `tag` (the
            /// caller turns that into `Unknown`). `Some(Err)` is a typed
            /// parse failure for a recognised tag.
            pub(crate) fn dispatch(tag: u8, full: &$lt [u8]) -> Option<crate::error::Result<Self>> {
                use broadcast_common::Parse;
                match tag {
                    $(
                        $tag => Some(<$($path)::+>::parse(full).map(Self::$variant)),
                    )+
                    _ => None,
                }
            }
        }

        #[cfg(test)]
        mod macro_drift {
            #[test]
            fn tag_literals_match_descriptor_def() {
                use crate::traits::DescriptorDef;
                $(
                    assert_eq!(
                        $tag,
                        <$($path)::+ as DescriptorDef>::TAG,
                        concat!("tag literal drift for ", stringify!($variant)),
                    );
                    assert!(
                        !<$($path)::+ as DescriptorDef>::NAME.is_empty(),
                        concat!("empty NAME for ", stringify!($variant)),
                    );
                )+
            }
        }
    };
}

pub(crate) use declare_tag_namespace;
