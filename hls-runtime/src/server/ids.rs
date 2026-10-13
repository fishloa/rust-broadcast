//! Distinct integer types for the origin's resource-naming arguments
//! (issue #1159).
//!
//! `init_name(track, generation)`, `segment_name(track, seq, ext)` and
//! `part_name(track, seq, idx, ext)` used to take bare, adjacent `u32`s, so a
//! swapped argument still compiled and produced a plausible-looking but wrong
//! URL. Each of these is its own newtype, so a swap is a compile error.
//!
//! `Display` and `Debug` render the bare number: they feed the wire names
//! (`seg-1-5-7.m4s`, `init-1-5-2.mp4`), which must not change.

use core::fmt;

macro_rules! id_newtype {
    ($(#[$meta:meta])* $name:ident) => {
        $(#[$meta])*
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
        pub struct $name(u32);

        impl $name {
            /// Wrap a raw value.
            #[must_use]
            pub const fn new(value: u32) -> Self {
                Self(value)
            }

            /// The raw value.
            #[must_use]
            pub const fn value(self) -> u32 {
                self.0
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::Debug::fmt(&self.0, f)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::Display::fmt(&self.0, f)
            }
        }
    };
}

id_newtype! {
    /// An init-segment generation: the version counter in
    /// `init-{track}-{instance}-{generation}.mp4`, starting at `1`.
    Generation
}

id_newtype! {
    /// An origin segment sequence number: the `Trunk`'s own numbering of closed
    /// segments (before the configured Media Sequence Number offset is added).
    SegmentSeq
}

id_newtype! {
    /// A part's index within its segment: the `{idx}` in
    /// `part-{track}-{instance}-{msn}.{idx}.{ext}`, from `0`.
    PartIndex
}
