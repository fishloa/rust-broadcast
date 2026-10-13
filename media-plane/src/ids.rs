//! Distinct integer types for a trunk's segment/part numbering (issue #1188).
//!
//! `SegmentEntry::sequence_number`, `PartEntry::segment_number` and
//! `PartEntry::part_index` used to be bare, adjacent `u32`s, so a swapped
//! argument (`PartEntry::new(bytes, part_index, segment_number, ..)`) still
//! compiled. Each is now its own newtype, so a swap is a compile error. The
//! same types name the origin's resources in `hls-runtime` (which re-exports
//! them), so there is one canonical definition below both crates.
//!
//! `Display` and `Debug` render the bare number (they feed wire names such as
//! `seg-1-5-7.m4s`). Arithmetic is explicit: [`SegmentSeq::checked_next`]
//! returns `None` at `u32::MAX`, [`SegmentSeq::wrapping_next`] wraps to `0`;
//! there is deliberately no `Add`/`Sub` impl that could overflow silently.

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

            /// The next value, or `None` if this is `u32::MAX`.
            #[must_use]
            pub const fn checked_next(self) -> Option<Self> {
                match self.0.checked_add(1) {
                    Some(v) => Some(Self(v)),
                    None => None,
                }
            }

            /// The next value, wrapping `u32::MAX` to `0`.
            #[must_use]
            pub const fn wrapping_next(self) -> Self {
                Self(self.0.wrapping_add(1))
            }

            /// The next value, saturating at `u32::MAX`.
            #[must_use]
            pub const fn saturating_next(self) -> Self {
                Self(self.0.saturating_add(1))
            }

            /// `self + n`, or `None` on overflow.
            #[must_use]
            pub const fn checked_add(self, n: u32) -> Option<Self> {
                match self.0.checked_add(n) {
                    Some(v) => Some(Self(v)),
                    None => None,
                }
            }

            /// `self - n`, or `None` on underflow.
            #[must_use]
            pub const fn checked_sub(self, n: u32) -> Option<Self> {
                match self.0.checked_sub(n) {
                    Some(v) => Some(Self(v)),
                    None => None,
                }
            }

            /// The distance `self - earlier`, or `None` if `earlier > self`.
            #[must_use]
            pub const fn checked_distance_from(self, earlier: Self) -> Option<u32> {
                self.0.checked_sub(earlier.0)
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
    /// A segment sequence number: the `Trunk`'s own numbering of closed
    /// segments (the `moof`/`mfhd` number, before any Media Sequence Number
    /// offset is added). Also the parent-segment number of a part.
    SegmentSeq
}

id_newtype! {
    /// A part's 0-based index within its parent segment.
    PartIndex
}

#[cfg(test)]
mod tests {
    extern crate alloc;
    use super::*;
    use alloc::format;

    #[test]
    fn checked_next_stops_at_max_wrapping_wraps() {
        let max = SegmentSeq::new(u32::MAX);
        assert_eq!(max.checked_next(), None);
        assert_eq!(max.wrapping_next(), SegmentSeq::new(0));
        assert_eq!(max.saturating_next(), max);
        assert_eq!(SegmentSeq::new(6).checked_next(), Some(SegmentSeq::new(7)));
        assert_eq!(PartIndex::new(u32::MAX).checked_next(), None);
    }

    #[test]
    fn checked_add_sub_distance() {
        let s = SegmentSeq::new(5);
        assert_eq!(s.checked_add(u32::MAX), None);
        assert_eq!(s.checked_sub(6), None);
        assert_eq!(s.checked_sub(5), Some(SegmentSeq::new(0)));
        assert_eq!(s.checked_distance_from(SegmentSeq::new(2)), Some(3));
        assert_eq!(s.checked_distance_from(SegmentSeq::new(9)), None);
    }

    #[test]
    fn display_and_debug_are_bare() {
        assert_eq!(
            format!("{} {:?}", PartIndex::new(7), SegmentSeq::new(9)),
            "7 9"
        );
    }
}
