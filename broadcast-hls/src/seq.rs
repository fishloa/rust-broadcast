//! HLS sequence numbers as their own types: one `MediaSequence` (RFC 8216
//! §4.3.3.2 `#EXT-X-MEDIA-SEQUENCE`, 64-bit) and one `PartIndex` (RFC 8216bis
//! §4.4.4.9 `#EXT-X-PART` index within a segment, 32-bit by spec), so a
//! playlist, an LL-HLS segmenter and a client never disagree about the width
//! of either.

use core::fmt;
use core::num::NonZeroU32;

/// A Media Sequence Number (RFC 8216 §4.3.3.2): the number of the first Media
/// Segment the playlist describes, counted from the first segment the stream
/// ever had, so it is 64-bit on the wire.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MediaSequence(u64);

impl MediaSequence {
    /// The first segment of a stream.
    pub const ZERO: MediaSequence = MediaSequence(0);

    /// The sequence numbered `value`.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        MediaSequence(value)
    }

    /// The number itself.
    #[must_use]
    pub const fn value(self) -> u64 {
        self.0
    }

    /// The sequence `offset` segments later, or `None` when that would pass
    /// the 64-bit range.
    #[must_use]
    pub const fn checked_add(self, offset: u64) -> Option<Self> {
        match self.0.checked_add(offset) {
            Some(v) => Some(MediaSequence(v)),
            None => None,
        }
    }

    /// The next sequence number, or `None` at the end of the 64-bit range.
    #[must_use]
    pub const fn checked_next(self) -> Option<Self> {
        self.checked_add(1)
    }

    /// How many segments `self` is after `earlier`, or `None` when it is not
    /// after it.
    #[must_use]
    pub const fn checked_distance_from(self, earlier: MediaSequence) -> Option<u64> {
        self.0.checked_sub(earlier.0)
    }

    /// The sequence `offset` segments earlier, or `None` when that would go
    /// below zero.
    #[must_use]
    pub const fn checked_sub(self, offset: u64) -> Option<Self> {
        match self.0.checked_sub(offset) {
            Some(v) => Some(MediaSequence(v)),
            None => None,
        }
    }
}

impl From<u64> for MediaSequence {
    fn from(value: u64) -> Self {
        MediaSequence(value)
    }
}

impl From<MediaSequence> for u64 {
    fn from(sequence: MediaSequence) -> Self {
        sequence.value()
    }
}

impl fmt::Display for MediaSequence {
    /// The wire spelling: the bare decimal number.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A Part Index (RFC 8216bis §4.4.4.9): a partial segment's position within
/// its parent Media Segment, starting at 0 for each segment.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PartIndex(u32);

impl PartIndex {
    /// The first part of a segment.
    pub const ZERO: PartIndex = PartIndex(0);

    /// The part at `value`.
    #[must_use]
    pub const fn new(value: u32) -> Self {
        PartIndex(value)
    }

    /// The index itself.
    #[must_use]
    pub const fn value(self) -> u32 {
        self.0
    }

    /// The index `offset` parts later, or `None` past the 32-bit range.
    #[must_use]
    pub const fn checked_add(self, offset: u32) -> Option<Self> {
        match self.0.checked_add(offset) {
            Some(v) => Some(PartIndex(v)),
            None => None,
        }
    }

    /// The next part index, or `None` at the end of the 32-bit range.
    #[must_use]
    pub const fn checked_next(self) -> Option<Self> {
        self.checked_add(1)
    }

    /// How many parts `self` is after `earlier`, or `None` when it is not
    /// after it.
    #[must_use]
    pub const fn checked_distance_from(self, earlier: PartIndex) -> Option<u32> {
        self.0.checked_sub(earlier.0)
    }
}

impl PartIndex {
    /// The largest part index the index field can carry.
    pub const MAX: PartIndex = PartIndex(u32::MAX);
}

impl fmt::Display for PartIndex {
    /// The wire spelling: the bare decimal number.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<u32> for PartIndex {
    fn from(value: u32) -> Self {
        PartIndex(value)
    }
}

impl From<PartIndex> for u32 {
    fn from(index: PartIndex) -> Self {
        index.value()
    }
}

impl From<NonZeroU32> for PartIndex {
    fn from(value: NonZeroU32) -> Self {
        PartIndex(value.get())
    }
}

#[cfg(feature = "serde")]
impl serde::Serialize for MediaSequence {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u64(self.0)
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for MediaSequence {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        u64::deserialize(deserializer).map(MediaSequence)
    }
}

#[cfg(feature = "serde")]
impl serde::Serialize for PartIndex {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u32(self.0)
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for PartIndex {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        u32::deserialize(deserializer).map(PartIndex)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wire_spelling_is_the_bare_decimal_number() {
        assert_eq!(MediaSequence::new(266).to_string(), "266");
        assert_eq!(PartIndex::new(3).to_string(), "3");
        assert_eq!(MediaSequence::ZERO.to_string(), "0");
    }

    #[test]
    fn a_sequence_advances_checks_its_range_and_wraps_nothing() {
        let first = MediaSequence::new(10);
        assert_eq!(first.checked_next(), Some(MediaSequence::new(11)));
        assert_eq!(first.checked_add(5), Some(MediaSequence::new(15)));
        assert_eq!(first.checked_sub(10), Some(MediaSequence::ZERO));
        assert_eq!(
            first.checked_sub(11),
            None,
            "before the stream is not a number"
        );
        assert_eq!(MediaSequence::new(u64::MAX).checked_next(), None);
        assert_eq!(MediaSequence::new(15).checked_distance_from(first), Some(5));
        assert_eq!(first.checked_distance_from(MediaSequence::new(15)), None);
    }

    #[test]
    fn a_part_index_advances_within_a_segment() {
        assert_eq!(PartIndex::ZERO.checked_next(), Some(PartIndex::new(1)));
        assert_eq!(PartIndex::new(2).checked_add(3), Some(PartIndex::new(5)));
        assert_eq!(PartIndex::new(u32::MAX).checked_next(), None);
        assert_eq!(
            PartIndex::new(5).checked_distance_from(PartIndex::new(2)),
            Some(3)
        );
    }
}
