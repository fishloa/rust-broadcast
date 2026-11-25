//! The coded spec enumerations this crate decodes (RFC 9559 §5).
//!
//! Each is a view over the integer the wire carries: the raw value stays in
//! the element, so an unknown code is preserved, and
//! [`from_u64`](TrackType::from_u64) returns `None` for it rather than
//! inventing a variant.

use broadcast_common::impl_spec_display;

/// Build a spec enum that is a closed decode view over an integer field.
macro_rules! spec_enum {
    ($ty:ident, $doc:literal, { $( $variant:ident = $value:literal => $label:literal ),* $(,)? }) => {
        #[doc = $doc]
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        #[cfg_attr(feature = "serde", derive(serde::Serialize))]
        #[non_exhaustive]
        pub enum $ty {
            $(
                #[doc = $label]
                $variant,
            )*
        }

        impl $ty {
            /// Decode the wire value, or `None` when it is not a known code.
            #[must_use]
            pub const fn from_u64(value: u64) -> Option<Self> {
                match value {
                    $( $value => Some(Self::$variant), )*
                    _ => None,
                }
            }

            /// The wire value.
            #[must_use]
            pub const fn value(self) -> u64 {
                match self {
                    $( Self::$variant => $value, )*
                }
            }

            /// The code's name.
            #[must_use]
            pub const fn name(&self) -> &'static str {
                match self {
                    $( Self::$variant => $label, )*
                }
            }
        }

        impl_spec_display!($ty);
    };
}

spec_enum!(
    TrackType,
    "`TrackType` (RFC 9559 §5.1.4.1.3).",
    {
        Video = 1 => "video",
        Audio = 2 => "audio",
        Complex = 3 => "complex",
        Logo = 16 => "logo",
        Subtitle = 17 => "subtitle",
        Buttons = 18 => "buttons",
        Control = 32 => "control",
        Metadata = 33 => "metadata",
    }
);

/// The lacing form of a Block, selected by the two-bit field of RFC 9559 §12.
///
/// This is a derived view of the flags octet, not an independent integer: read
/// it from [`BlockHeader::lacing_kind`](crate::matroska::BlockHeader::lacing_kind).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub enum LacingKind {
    /// No lacing.
    None,
    /// Xiph lacing.
    Xiph,
    /// Fixed-size lacing.
    Fixed,
    /// EBML lacing.
    Ebml,
}

impl LacingKind {
    /// The code's name.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Xiph => "xiph",
            Self::Fixed => "fixed",
            Self::Ebml => "ebml",
        }
    }
}

impl_spec_display!(LacingKind);

spec_enum!(
    FlagInterlaced,
    "`FlagInterlaced` (RFC 9559 §5.1.4.1.28.1).",
    {
        Undetermined = 0 => "undetermined",
        Interlaced = 1 => "interlaced",
        Progressive = 2 => "progressive",
    }
);

spec_enum!(
    FieldOrder,
    "`FieldOrder` (RFC 9559 §5.1.4.1.28.2).",
    {
        Progressive = 0 => "progressive",
        Tff = 1 => "tff",
        Undetermined = 2 => "undetermined",
        Bff = 6 => "bff",
        TffSwapped = 9 => "tff-swapped",
        BffSwapped = 14 => "bff-swapped",
    }
);

spec_enum!(
    StereoMode,
    "`StereoMode` (RFC 9559 §5.1.4.1.28.3).",
    {
        Mono = 0 => "mono",
        SideBySideLeftFirst = 1 => "side-by-side-left-first",
        TopBottomRightFirst = 2 => "top-bottom-right-first",
        TopBottomLeftFirst = 3 => "top-bottom-left-first",
        CheckerboardRightFirst = 4 => "checkerboard-right-first",
        CheckerboardLeftFirst = 5 => "checkerboard-left-first",
        RowInterleavedRightFirst = 6 => "row-interleaved-right-first",
        RowInterleavedLeftFirst = 7 => "row-interleaved-left-first",
        ColumnInterleavedRightFirst = 8 => "column-interleaved-right-first",
        ColumnInterleavedLeftFirst = 9 => "column-interleaved-left-first",
        AnaglyphCyanRed = 10 => "anaglyph-cyan-red",
        SideBySideRightFirst = 11 => "side-by-side-right-first",
        AnaglyphGreenMagenta = 12 => "anaglyph-green-magenta",
        LacedLeftFirst = 13 => "laced-left-first",
        LacedRightFirst = 14 => "laced-right-first",
    }
);

spec_enum!(
    DisplayUnit,
    "`DisplayUnit` (RFC 9559 §5.1.4.1.28.14).",
    {
        Pixels = 0 => "pixels",
        Centimetres = 1 => "centimetres",
        Inches = 2 => "inches",
        DisplayAspectRatio = 3 => "display-aspect-ratio",
        Unknown = 4 => "unknown",
    }
);

spec_enum!(
    ColourRange,
    "`Range` (RFC 9559 §5.1.4.1.28.25).",
    {
        Unspecified = 0 => "unspecified",
        Broadcast = 1 => "broadcast",
        Full = 2 => "full",
        DefinedByMatrixCoefficients = 3 => "defined-by-matrix-coefficients",
    }
);

spec_enum!(
    ProjectionType,
    "`ProjectionType` (RFC 9559 §5.1.4.1.28.42).",
    {
        Rectangular = 0 => "rectangular",
        Equirectangular = 1 => "equirectangular",
        Cubemap = 2 => "cubemap",
        Mesh = 3 => "mesh",
    }
);

spec_enum!(
    ContentCompAlgo,
    "`ContentCompAlgo` (RFC 9559 §5.1.4.1.31.6).",
    {
        Zlib = 0 => "zlib",
        Bzlib = 1 => "bzlib",
        Lzo1x = 2 => "lzo1x",
        HeaderStripping = 3 => "header-stripping",
    }
);

spec_enum!(
    ContentEncAlgo,
    "`ContentEncAlgo` (RFC 9559 §5.1.4.1.31.9).",
    {
        NotEncrypted = 0 => "not-encrypted",
        Des = 1 => "des",
        TripleDes = 2 => "3des",
        Twofish = 3 => "twofish",
        Blowfish = 4 => "blowfish",
        Aes = 5 => "aes",
    }
);

spec_enum!(
    AesSettingsCipherMode,
    "`AESSettingsCipherMode` (RFC 9559 §5.1.4.1.31.12).",
    {
        Aes128Ctr = 1 => "aes-128-ctr",
        Aes128Cbc = 2 => "aes-128-cbc",
    }
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn views_decode_and_encode() {
        assert_eq!(TrackType::from_u64(1), Some(TrackType::Video));
        assert_eq!(TrackType::Video.value(), 1);
        assert_eq!(TrackType::from_u64(99), None);
        assert_eq!(LacingKind::Ebml.name(), "ebml");
        assert_eq!(LacingKind::Ebml.name(), "ebml");
        assert_eq!(FieldOrder::from_u64(14), Some(FieldOrder::BffSwapped));
        assert_eq!(
            ProjectionType::from_u64(1).unwrap().to_string(),
            "equirectangular"
        );
    }
}
