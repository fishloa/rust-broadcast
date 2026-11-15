//! The RTMP wire identities — §5.1.2 `chunk stream id`, §5.1.3 `message
//! stream id` and `timestamp`, §5.3.1.1's basic-header encodability rule
//! (reference `private/specs-verified/rtmp/rtmp-runtime/rtmp.md`).
//!
//! Each is a fixed-width wire field with no invalid bit pattern, so every bit
//! pattern round-trips byte-exactly; the types exist so a chunk stream id
//! cannot be passed where a message stream id is expected (the RTMP trap: both
//! are bare integers in the spec) and so the basic-header range lives with the
//! id it constrains.

use core::time::Duration;

/// Lowest chunk stream id a basic header can carry; 0 and 1 are the 2-/3-byte
/// form markers, not ids (§5.3.1.1).
pub const MIN_CHUNK_STREAM_ID: u32 = 2;

/// Highest chunk stream id a basic header can carry: the 3-byte form encodes
/// `(byte2 * 256) + byte1 + 64` (§5.3.1.1).
pub const MAX_CHUNK_STREAM_ID: u32 = 65_599;

/// Chunk stream id (csid) — the per-chunk-stream multiplexing identity
/// (§5.1.2), carried by the basic header in the range 2..=65599.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[repr(transparent)]
pub struct ChunkStreamId(u32);

impl ChunkStreamId {
    /// Wrap a chunk stream id as read from, or written to, the wire. Every
    /// `u32` is representable as a value; [`basic_header_encodable`] says
    /// whether a basic header can carry it.
    ///
    /// [`basic_header_encodable`]: Self::basic_header_encodable
    #[must_use]
    pub const fn new(raw: u32) -> Self {
        Self(raw)
    }

    /// The raw id.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }

    /// Whether a basic header can carry this id (2..=65599, §5.3.1.1).
    #[must_use]
    pub const fn basic_header_encodable(self) -> bool {
        self.0 >= MIN_CHUNK_STREAM_ID && self.0 <= MAX_CHUNK_STREAM_ID
    }
}

impl From<u32> for ChunkStreamId {
    fn from(raw: u32) -> Self {
        Self(raw)
    }
}

impl From<ChunkStreamId> for u32 {
    fn from(id: ChunkStreamId) -> Self {
        id.0
    }
}

impl core::fmt::Display for ChunkStreamId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Message stream id (msid) — the application-level stream a message belongs
/// to (§5.1.3), allocated by `createStream` and carried little-endian in a
/// Type 0 message header.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[repr(transparent)]
pub struct MessageStreamId(u32);

impl MessageStreamId {
    /// Wrap a raw message stream id from the wire or a `createStream` result.
    #[must_use]
    pub const fn new(raw: u32) -> Self {
        Self(raw)
    }

    /// The raw id.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

impl From<u32> for MessageStreamId {
    fn from(raw: u32) -> Self {
        Self(raw)
    }
}

impl From<MessageStreamId> for u32 {
    fn from(id: MessageStreamId) -> Self {
        id.0
    }
}

impl core::fmt::Display for MessageStreamId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// An RTMP acknowledgement sequence number (§5.4.3): the total number of
/// bytes received so far, carried as a 32-bit field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[repr(transparent)]
pub struct SequenceNumber(u32);

impl SequenceNumber {
    /// Wrap a raw byte count from the wire.
    #[must_use]
    pub const fn new(raw: u32) -> Self {
        Self(raw)
    }

    /// The raw byte count.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

impl From<u32> for SequenceNumber {
    fn from(raw: u32) -> Self {
        Self(raw)
    }
}

impl From<SequenceNumber> for u32 {
    fn from(seq: SequenceNumber) -> Self {
        seq.0
    }
}

impl core::fmt::Display for SequenceNumber {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// An RTMP `timestamp` / `timestamp delta` (§5.3.1.2) — a whole number of
/// milliseconds on the wire, held as a span so a caller never passes a raw
/// tick count.
///
/// The wire is a 24-bit field plus the 4-byte Extended Timestamp (§5.3.1.3),
/// so any `Duration` below 2^32 ms converts exactly; a coarser `Duration`
/// (sub-millisecond units) is rounded down to the millisecond the wire
/// carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Timestamp(Duration);

impl Timestamp {
    /// The zero timestamp (`Duration::ZERO`).
    pub const ZERO: Self = Self(Duration::ZERO);

    /// Build from a span, rejecting one the 32-bit wire field cannot hold
    /// (2^32 ms is about 49.7 days).
    ///
    /// # Errors
    /// [`TimestampOutOfRange`] when the span is 2^32 ms or longer.
    pub fn try_new(span: Duration) -> Result<Self, TimestampOutOfRange> {
        u32::try_from(span.as_millis())
            .map(|_| Self(span))
            .map_err(|_| TimestampOutOfRange)
    }

    /// The span this timestamp stands for.
    #[must_use]
    pub const fn span(self) -> Duration {
        self.0
    }

    /// The whole milliseconds the wire carries.
    #[must_use]
    pub fn millis(self) -> u32 {
        u32::try_from(self.0.as_millis()).unwrap_or(u32::MAX)
    }

    /// Read a value straight off the wire (always representable).
    #[must_use]
    pub const fn from_millis(millis: u32) -> Self {
        Self(Duration::from_millis(millis as u64))
    }

    /// `self + delta`, or `None` when that leaves the 32-bit field.
    #[must_use]
    pub fn checked_add(self, delta: Duration) -> Option<Self> {
        self.0
            .checked_add(delta)
            .and_then(|s| Self::try_new(s).ok())
    }
}

impl From<Duration> for Timestamp {
    fn from(span: Duration) -> Self {
        Self(span)
    }
}

impl From<Timestamp> for Duration {
    fn from(ts: Timestamp) -> Self {
        ts.0
    }
}

impl core::fmt::Display for Timestamp {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{} ms", self.millis())
    }
}

/// A `Timestamp` the 32-bit wire field cannot hold (a span of 2^32 ms or more,
/// §5.3.1.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimestampOutOfRange;

impl core::fmt::Display for TimestampOutOfRange {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("RTMP timestamp does not fit the 32-bit wire field (max 2^32-1 ms)")
    }
}

impl core::error::Error for TimestampOutOfRange {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_stream_ids_mark_which_are_encodable() {
        assert!(!ChunkStreamId::new(1).basic_header_encodable());
        assert!(ChunkStreamId::new(MIN_CHUNK_STREAM_ID).basic_header_encodable());
        assert!(ChunkStreamId::new(MAX_CHUNK_STREAM_ID).basic_header_encodable());
        assert!(!ChunkStreamId::new(MAX_CHUNK_STREAM_ID + 1).basic_header_encodable());
    }

    #[test]
    fn timestamps_round_trip_through_milliseconds() {
        let ts = Timestamp::try_new(Duration::from_millis(3000)).unwrap();
        assert_eq!(ts.millis(), 3000);
        assert_eq!(ts.span(), Duration::from_millis(3000));
        assert_eq!(Timestamp::from_millis(3000), ts);
        assert_eq!(Duration::from(ts), Duration::from_millis(3000));
    }

    #[test]
    fn a_timestamp_past_the_wire_width_is_rejected() {
        assert_eq!(
            Timestamp::try_new(Duration::from_millis(u64::from(u32::MAX) + 1)),
            Err(TimestampOutOfRange)
        );
        assert!(Timestamp::try_new(Duration::from_millis(u64::from(u32::MAX))).is_ok());
    }

    #[test]
    fn adding_a_delta_past_the_wire_width_is_none() {
        let near_end = Timestamp::from_millis(u32::MAX);
        assert_eq!(near_end.checked_add(Duration::from_millis(1)), None);
        assert_eq!(
            Timestamp::from_millis(1).checked_add(Duration::from_millis(1)),
            Some(Timestamp::from_millis(2))
        );
    }

    #[test]
    fn ids_carry_their_raw_value() {
        assert_eq!(ChunkStreamId::from(4).get(), 4);
        assert_eq!(u32::from(MessageStreamId::new(0)), 0);
        assert_eq!(ChunkStreamId::new(2).to_string(), "2");
    }
}
