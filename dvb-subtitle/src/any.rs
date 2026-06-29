//! Unified segment dispatch: [`AnySegment`] + segment parsing.
//!
//! [`AnySegment`] is generated from a single declarative list
//! (`declare_segments!`) — one line per crate-implemented segment type.
//! The list is the single source of truth: it produces the enum, the
//! `From<T>` conversions, and the segment_type → type dispatcher, and
//! a drift test pins each segment_type literal to the type's
//! [`crate::traits::SegmentDef::SEGMENT_TYPE`].

use broadcast_common::{Parse, Serialize};

/// Declares [`AnySegment`] + its dispatcher from one segment type list.
///
/// Each line is `Variant = 0xSEGMENT_TYPE => module::Type[<'a>]`.
macro_rules! declare_segments {
    (
        $lt:lifetime;
        $( $variant:ident = $seg_type:literal => $($path:ident)::+ $(<$plt:lifetime>)? ),+ $(,)?
    ) => {
        /// Every crate-implemented segment type, plus an `Unknown` fallthrough.
        ///
        /// serde uses external tagging with camelCase variant keys.
        #[derive(Debug, Clone, PartialEq, Eq)]
        #[cfg_attr(feature = "serde", derive(serde::Serialize))]
        #[cfg_attr(feature = "serde", serde(rename_all = "camelCase"))]
        #[non_exhaustive]
        pub enum AnySegment<$lt> {
            $(
                #[allow(missing_docs)]
                $variant($($path)::+ $(<$plt>)?),
            )+
            /// Segment with an unrecognized or unsupported segment_type; `data` is the
            /// segment body sans the 6-byte header (4 bytes of generic segment framing).
            Unknown {
                /// The raw segment_type byte.
                segment_type: u8,
                /// The page_id from the segment header.
                page_id: u16,
                /// The segment body bytes (starting after the 4-byte sync_byte+segment_type+page_id+segment_length header).
                data: &$lt [u8],
            },
        }

        $(
            impl<$lt> From<$($path)::+ $(<$plt>)?> for AnySegment<$lt> {
                fn from(s: $($path)::+ $(<$plt>)?) -> Self {
                    Self::$variant(s)
                }
            }
        )+

        impl<$lt> AnySegment<$lt> {
            /// Every segment_type the generated dispatcher routes.
            pub const DISPATCHED_SEGMENT_TYPES: &'static [u8] = &[$($seg_type),+];

            /// Diagnostic name of the contained segment — the type's
            /// [`SegmentDef::NAME`](crate::traits::SegmentDef::NAME)
            /// (`"PAGE_COMPOSITION"`, `"OBJECT_DATA"`, …); `"UNKNOWN"` for
            /// [`AnySegment::Unknown`].
            #[must_use]
            pub fn name(&self) -> &'static str {
                match self {
                    $(
                        Self::$variant(_) =>
                            <$($path)::+ as crate::traits::SegmentDef>::NAME,
                    )+
                    Self::Unknown { .. } => "UNKNOWN",
                }
            }

            /// Parse one full segment (including 6-byte generic header) by its segment_type.
            ///
            /// `None` means no typed implementation exists for `segment_type` (the
            /// caller turns that into [`AnySegment::Unknown`]). `Some(Err)`
            /// is a typed parse failure for a recognized segment_type.
            pub(crate) fn dispatch(segment_type: u8, full: &$lt [u8]) -> Option<crate::Result<Self>> {
                match segment_type {
                    $(
                        $seg_type => Some(<$($path)::+>::parse(full).map(Self::$variant)),
                    )+
                    _ => None,
                }
            }

            pub(crate) fn serialized_len(&self) -> usize {
                match self {
                    $(
                        Self::$variant(s) => s.serialized_len(),
                    )+
                    Self::Unknown { data, .. } => (6 + data.len()),
                }
            }

            pub(crate) fn serialize_into(&self, buf: &mut [u8]) -> crate::Result<usize> {
                match self {
                    $(
                        Self::$variant(s) => s.serialize_into(buf),
                    )+
                    Self::Unknown { segment_type, page_id, data } => {
                        let len = 6 + data.len();
                        if buf.len() < len {
                            return Err(crate::error::Error::BufferTooShort {
                                need: len,
                                have: buf.len(),
                                what: "Unknown segment serialize",
                            });
                        }
                        buf[0] = 0x0F;
                        buf[1] = *segment_type;
                        buf[2..4].copy_from_slice(&page_id.to_be_bytes());
                        let seg_len = data.len() as u16;
                        buf[4..6].copy_from_slice(&seg_len.to_be_bytes());
                        buf[6..len].copy_from_slice(data);
                        Ok(len)
                    }
                }
            }
        }

        #[cfg(test)]
        mod macro_drift {
            #[test]
            fn segment_type_literals_match_segment_def() {
                use crate::traits::SegmentDef;
                $(
                    assert_eq!(
                        $seg_type,
                        <$($path)::+ as SegmentDef>::SEGMENT_TYPE,
                        concat!("segment_type literal drift for ", stringify!($variant)),
                    );
                    assert!(
                        !<$($path)::+ as SegmentDef>::NAME.is_empty(),
                        concat!("empty NAME for ", stringify!($variant)),
                    );
                )+
            }
        }
    };
}

declare_segments! {'a;
    PageComposition = 0x10 => crate::segments::page_composition::PageCompositionSegment,
    RegionComposition = 0x11 => crate::segments::region_composition::RegionCompositionSegment,
    ClutDefinition = 0x12 => crate::segments::clut_definition::ClutDefinitionSegment,
    ObjectData = 0x13 => crate::segments::object_data::ObjectDataSegment<'a>,
    DisplayDefinition = 0x14 => crate::segments::display_definition::DisplayDefinitionSegment,
    DisparitySignalling = 0x15 => crate::segments::disparity_signalling::DisparitySignallingSegment,
    AlternativeClut = 0x16 => crate::segments::alternative_clut::AlternativeClutSegment,
    EndOfDisplaySet = 0x80 => crate::segments::end_of_display_set::EndOfDisplaySetSegment,
    Stuffing = 0xFF => crate::segments::stuffing::StuffingSegment<'a>,
}
