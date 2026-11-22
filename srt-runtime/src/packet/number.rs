//! The typed SRT wire numbers — `draft-sharabayko-srt-01` §3 (header words)
//! and Appendix A (loss-list coding).
//!
//! Three quantities name a packet or a socket on the wire and were all bare
//! `u32` before: the 31-bit Packet Sequence Number (§3.1, Figure 3), the
//! 26-bit Message Number (§3.1), and the 32-bit Destination/Source Socket ID
//! (§3, Figure 2). Each now has its own type, so the units and the wrap
//! behaviour travel with the value instead of living in a comment, and the
//! node-budget/parse sites convert once at the wire edge.
//!
//! [`SeqNo`] is the one sequence-number type in the crate: the ACK's Last
//! Acknowledged Packet Sequence Number and the NAK/DROPREQ loss-list endpoints
//! are the same 31-bit sequence space, so all of them are a [`SeqNo`]. The wrap
//! arithmetic is [`broadcast_common::seq`] (31 bits), never `wrapping_add` or
//! `abs_diff` on a raw integer.
//!
//! [`AckNumber`] is deliberately *not* a [`SeqNo`]: the ACK/ACKACK
//! Acknowledgement Number lives in a whole 32-bit header word (§3.2.4,
//! §3.2.8), so every `u32` value is a legal acknowledgement number.

use broadcast_common::seq::{MAX_SEQ_BITS, SeqSpace};

/// The SRT packet sequence-number space: 31 bits (`draft-sharabayko-srt-01`
/// §3.1, Figure 3 — `Packet Sequence Number (31)`; the top bit is the header
/// `F` packet-type flag, reused as the NAK loss-list range marker, Appendix A).
const SEQ_SPACE_BITS: u32 = 31;
/// The 31-bit space, proven in range at compile time.
const SEQ_SPACE: SeqSpace = broadcast_common::seq_space!(SEQ_SPACE_BITS);

const _: () = assert!(SEQ_SPACE_BITS == MAX_SEQ_BITS);

/// Bit width of the Message Number field (`draft-sharabayko-srt-01` §3.1,
/// Figure 3: `Message Number (26)`).
pub const MESSAGE_NUMBER_BITS: u32 = 26;
/// The 26-bit Message Number mask (§3.1).
pub const MESSAGE_NUMBER_MASK: u32 = (1 << MESSAGE_NUMBER_BITS) - 1;

/// A 31-bit SRT packet sequence number (`draft-sharabayko-srt-01` §3.1,
/// Figure 3).
///
/// The value always fits the 31-bit field. Construction never masks a
/// caller-supplied value: [`SeqNo::try_new`] rejects an out-of-range one and
/// [`SeqNo::from_wire_masked`] is the explicit parse-site constructor. It is
/// *not* `Ord`: the sequence space is circular, so ordering is
/// [`SeqNo::lt`]/[`SeqNo::gt`]/... (shorter-direction, RFC 1982-style), never a
/// plain numeric compare that breaks at the wrap.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct SeqNo(u32);

impl SeqNo {
    /// The largest sequence number in the 31-bit space (`0x7FFF_FFFF`).
    pub const MAX: SeqNo = SeqNo(SEQ_SPACE.mask());

    /// Construct a `SeqNo`, rejecting a value that does not fit the 31-bit
    /// field.
    ///
    /// # Errors
    /// [`SeqOutOfRange`] when `value > 0x7FFF_FFFF`.
    pub const fn try_new(value: u32) -> Result<Self, SeqOutOfRange> {
        if value > SEQ_SPACE.mask() {
            Err(SeqOutOfRange { value })
        } else {
            Ok(Self(value))
        }
    }

    /// Construct a `SeqNo` from the low 31 bits of `value`, **deliberately**
    /// discarding the rest: the explicit name for extracting the sequence
    /// field out of a wider wire word (e.g. the header word whose bit 31 is
    /// the `F` flag), where masking is the specified behaviour. Use this only
    /// at parse sites; use [`SeqNo::try_new`] for a caller-supplied value.
    #[must_use]
    pub const fn from_wire_masked(value: u32) -> Self {
        Self(value & SEQ_SPACE.mask())
    }

    /// The raw 31-bit value.
    #[must_use]
    pub const fn value(self) -> u32 {
        self.0
    }

    /// `self + n`, wrapping at the 31-bit boundary.
    #[must_use]
    pub const fn add(self, n: u32) -> Self {
        Self(SEQ_SPACE.add(self.0, n))
    }

    /// The next sequence number after `self` (wraps `0x7FFF_FFFF` -> `0`).
    #[must_use]
    pub const fn next(self) -> Self {
        self.add(1)
    }

    /// Signed circular distance `self - other`, in `(-2^30, 2^30]`. Positive
    /// means `self` is ahead of `other`.
    #[must_use]
    pub const fn diff(self, other: Self) -> i32 {
        SEQ_SPACE.diff(self.0, other.0)
    }

    /// `self` precedes `other` in circular sequence order.
    #[must_use]
    pub const fn lt(self, other: Self) -> bool {
        SEQ_SPACE.lt(self.0, other.0)
    }

    /// `self` precedes or equals `other`.
    #[must_use]
    pub const fn leq(self, other: Self) -> bool {
        SEQ_SPACE.leq(self.0, other.0)
    }

    /// `self` follows `other` in circular sequence order.
    #[must_use]
    pub const fn gt(self, other: Self) -> bool {
        SEQ_SPACE.gt(self.0, other.0)
    }

    /// `self` follows or equals `other`.
    #[must_use]
    pub const fn geq(self, other: Self) -> bool {
        SEQ_SPACE.geq(self.0, other.0)
    }

    /// Whether `self` lies within the inclusive circular range `first..=last`,
    /// walking forward from `first`. A `last` that precedes `first` in circular
    /// order is an empty (malformed) range and never matches.
    #[must_use]
    pub const fn in_closed_range(self, first: Self, last: Self) -> bool {
        SEQ_SPACE.in_closed_range(self.0, first.0, last.0)
    }
}

/// A sequence number that does not fit the 31-bit field
/// (`draft-sharabayko-srt-01` §3.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("sequence number {value:#010X} exceeds the 31-bit range")]
#[non_exhaustive]
pub struct SeqOutOfRange {
    /// The rejected value.
    pub value: u32,
}

impl TryFrom<u32> for SeqNo {
    type Error = SeqOutOfRange;
    fn try_from(value: u32) -> Result<Self, Self::Error> {
        Self::try_new(value)
    }
}

impl From<SeqNo> for u32 {
    fn from(seq: SeqNo) -> Self {
        seq.0
    }
}

impl core::fmt::Debug for SeqNo {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "SeqNo({:#010X})", self.0)
    }
}

impl core::fmt::Display for SeqNo {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// An SRT Socket ID — the 32-bit Destination Socket ID / Source Socket ID of
/// `draft-sharabayko-srt-01` §3, Figure 2.
///
/// Every value fits the 32-bit wire field, so there is no range rejection;
/// the type exists so a socket id cannot be confused with a sequence number, a
/// timestamp or an arbitrary count. `0` is meaningful — on a handshake induction
/// packet it is the connection request sentinel (§4.3.1.1).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct SocketId(u32);

impl SocketId {
    /// The connection-request sentinel: a Destination Socket ID of `0` on a
    /// handshake induction packet (§4.3.1.1).
    pub const CONNECTION_REQUEST: SocketId = SocketId(0);

    /// Construct a `SocketId` from its 32-bit wire value.
    #[must_use]
    pub const fn from_wire(value: u32) -> Self {
        Self(value)
    }

    /// The raw 32-bit wire value.
    #[must_use]
    pub const fn value(self) -> u32 {
        self.0
    }

    /// Whether this is the connection-request sentinel ([`Self::CONNECTION_REQUEST`]).
    #[must_use]
    pub const fn is_connection_request(self) -> bool {
        self.0 == 0
    }
}

impl From<u32> for SocketId {
    fn from(value: u32) -> Self {
        SocketId(value)
    }
}

impl From<SocketId> for u32 {
    fn from(id: SocketId) -> Self {
        id.0
    }
}

impl core::fmt::Debug for SocketId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "SocketId({})", self.0)
    }
}

impl core::fmt::Display for SocketId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// The sequential number of a Full ACK control packet — the
/// `draft-sharabayko-srt-01` §3.2.4 (Figure 13) `Acknowledgement Number`,
/// carried in the control header's `Type-specific Information` word, and
/// echoed back by the ACKACK that acknowledges it (§3.2.8, Figure 17).
///
/// Unlike [`SeqNo`], whose 31-bit space exists because the top bit of a *data*
/// packet's first header word is the `F` flag, this field occupies a whole
/// 32-bit word: no bit in it is reserved and no value is unrepresentable, so
/// there is no range rejection. The type keeps an ACK's numbering (an
/// incrementing counter, §4.8.1) from being confused with a sequence number, a
/// socket id, a timestamp or a plain count.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct AckNumber(u32);

impl AckNumber {
    /// The zero acknowledgement number — what a Small/Light ACK carries
    /// (§3.2.4: their `Type-specific Information` "should be set to 0").
    pub const ZERO: AckNumber = AckNumber(0);

    /// Construct an `AckNumber` from its 32-bit wire value.
    #[must_use]
    pub const fn from_wire(value: u32) -> Self {
        Self(value)
    }

    /// The raw 32-bit wire value.
    #[must_use]
    pub const fn value(self) -> u32 {
        self.0
    }
}

impl From<u32> for AckNumber {
    fn from(value: u32) -> Self {
        AckNumber(value)
    }
}

impl From<AckNumber> for u32 {
    fn from(n: AckNumber) -> Self {
        n.0
    }
}

impl core::fmt::Debug for AckNumber {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "AckNumber({})", self.0)
    }
}

impl core::fmt::Display for AckNumber {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A 26-bit SRT Message Number (`draft-sharabayko-srt-01` §3.1, Figure 3:
/// `Message Number (26)`).
///
/// The value always fits the 26-bit field. [`MessageNumber::next`] wraps at
/// the 26-bit boundary back to `1`, not `0`: libsrt's public header reserves
/// message number `0` (`SRT_MSGNO_CONTROL`) as a packet-filter control
/// sentinel, so a data packet must never carry it (issue #1062).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct MessageNumber(u32);

impl MessageNumber {
    /// Construct a `MessageNumber`, rejecting a value that does not fit the
    /// 26-bit field.
    ///
    /// # Errors
    /// [`MessageNumberOutOfRange`] when `value` exceeds [`MESSAGE_NUMBER_MASK`].
    pub const fn try_new(value: u32) -> Result<Self, MessageNumberOutOfRange> {
        if value > MESSAGE_NUMBER_MASK {
            Err(MessageNumberOutOfRange { value })
        } else {
            Ok(Self(value))
        }
    }

    /// Construct a `MessageNumber` from the low 26 bits of `value`,
    /// **deliberately** discarding the rest — the parse-site constructor for
    /// the `word1` Message Number field (§3.1).
    #[must_use]
    pub const fn from_wire_masked(value: u32) -> Self {
        Self(value & MESSAGE_NUMBER_MASK)
    }

    /// The raw 26-bit value.
    #[must_use]
    pub const fn value(self) -> u32 {
        self.0
    }

    /// The next Message Number after `self`, wrapping at the 26-bit boundary
    /// to `1` (never `0` — see the type doc).
    #[must_use]
    pub const fn next(self) -> Self {
        match ((self.0 & MESSAGE_NUMBER_MASK) + 1) & MESSAGE_NUMBER_MASK {
            0 => Self(1),
            next => Self(next),
        }
    }
}

/// A message number that does not fit the 26-bit field
/// (`draft-sharabayko-srt-01` §3.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("message number {value:#010X} exceeds the 26-bit range")]
#[non_exhaustive]
pub struct MessageNumberOutOfRange {
    /// The rejected value.
    pub value: u32,
}

impl TryFrom<u32> for MessageNumber {
    type Error = MessageNumberOutOfRange;
    fn try_from(value: u32) -> Result<Self, Self::Error> {
        Self::try_new(value)
    }
}

impl From<MessageNumber> for u32 {
    fn from(n: MessageNumber) -> Self {
        n.0
    }
}

impl core::fmt::Debug for MessageNumber {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "MessageNumber({})", self.0)
    }
}

impl core::fmt::Display for MessageNumber {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// An SRT header Timestamp — the 32-bit field of `draft-sharabayko-srt-01` §3
/// (Figure 2), carried by every data and control packet.
///
/// On a data packet it is a media timestamp in microseconds relative to
/// connection establishment (§3.1), wrapping every `2^32` µs (~71.58 min,
/// rule 15 of `private/specs-verified/rules/srt-tsbpd.md`); on a control packet
/// it is the sender's current time for the packet's purpose (§3.2). It always
/// fits the 32-bit field, so there is no range rejection — the type keeps a
/// header timestamp from being confused with a sequence number, a socket id or
/// a plain count.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct SrtTimestamp(u32);

impl SrtTimestamp {
    /// The zero timestamp.
    pub const ZERO: SrtTimestamp = SrtTimestamp(0);

    /// Construct an `SrtTimestamp` from its 32-bit wire value.
    #[must_use]
    pub const fn from_wire(value: u32) -> Self {
        Self(value)
    }

    /// The raw 32-bit wire value.
    #[must_use]
    pub const fn value(self) -> u32 {
        self.0
    }
}

impl From<u32> for SrtTimestamp {
    fn from(value: u32) -> Self {
        SrtTimestamp(value)
    }
}

impl From<SrtTimestamp> for u32 {
    fn from(ts: SrtTimestamp) -> Self {
        ts.0
    }
}

impl core::fmt::Debug for SrtTimestamp {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "SrtTimestamp({})", self.0)
    }
}

impl core::fmt::Display for SrtTimestamp {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An ACK/ACKACK Acknowledgement Number is a whole 32-bit word
    /// (§3.2.4/§3.2.8): no value is out of range, unlike the 31-bit
    /// [`SeqNo`], so the type never rejects or masks one.
    #[test]
    fn ack_number_keeps_every_32_bit_value() {
        for v in [0, 1, 0x7FFF_FFFF, 0x8000_0000, 0xFFFF_FFFF] {
            let n = AckNumber::from_wire(v);
            assert_eq!(n.value(), v);
            assert_eq!(u32::from(AckNumber::from(v)), v);
            assert_eq!(alloc::format!("{n:?}"), alloc::format!("AckNumber({v})"));
            assert_eq!(alloc::format!("{n}"), alloc::format!("{v}"));
        }
        assert_eq!(AckNumber::ZERO.value(), 0);
    }

    #[test]
    fn seq_space_matches_the_wire_mask() {
        assert_eq!(SEQ_SPACE.mask().count_ones(), SEQ_SPACE_BITS);
        assert_eq!(SeqNo::MAX.value().count_ones(), 31);
        assert_eq!(SeqNo::from_wire_masked(0x8000_0005).value(), 5);
    }

    #[test]
    fn seq_try_new_rejects_an_out_of_range_value() {
        assert_eq!(
            SeqNo::try_new(0x8000_0000),
            Err(SeqOutOfRange { value: 0x8000_0000 })
        );
        assert_eq!(
            SeqNo::try_new(SeqNo::MAX.value()).map(SeqNo::value),
            Ok(0x7FFF_FFFF)
        );
    }

    #[test]
    fn seq_wraps_and_orders_circularly() {
        assert_eq!(SeqNo::MAX.next(), SeqNo::from_wire_masked(0));
        assert!(SeqNo::MAX.lt(SeqNo::from_wire_masked(0)));
        assert_eq!(SeqNo::from_wire_masked(0).diff(SeqNo::MAX), 1);
        assert!(SeqNo::from_wire_masked(10).lt(SeqNo::from_wire_masked(20)));
        assert!(SeqNo::from_wire_masked(20).gt(SeqNo::from_wire_masked(10)));
    }

    #[test]
    fn seq_closed_range_walks_across_the_wrap() {
        let lo = SeqNo::MAX.add(SEQ_SPACE.mask()); // (MAX + MAX) mod 2^31 = MAX - 1
        assert_eq!(lo.value(), constant_max_minus_one());
        assert!(SeqNo::from_wire_masked(0).in_closed_range(lo, SeqNo::from_wire_masked(1)));
        assert!(
            SeqNo::from_wire_masked(SeqNo::MAX.value())
                .in_closed_range(lo, SeqNo::from_wire_masked(1))
        );
        assert!(!SeqNo::from_wire_masked(2).in_closed_range(lo, SeqNo::from_wire_masked(1)));
    }

    fn constant_max_minus_one() -> u32 {
        SeqNo::MAX.value() - 1
    }

    #[test]
    fn socket_id_round_trips_and_names_the_connection_request() {
        assert!(SocketId::CONNECTION_REQUEST.is_connection_request());
        assert!(!SocketId::from_wire(7).is_connection_request());
        assert_eq!(u32::from(SocketId::from_wire(42)), 42);
    }

    #[test]
    fn message_number_wraps_to_one_and_rejects_overwide() {
        assert_eq!(MessageNumber::from_wire_masked(0).next().value(), 1);
        assert_eq!(
            MessageNumber::from_wire_masked(MESSAGE_NUMBER_MASK)
                .next()
                .value(),
            1
        );
        assert_eq!(
            MessageNumber::try_new(1 << MESSAGE_NUMBER_BITS),
            Err(MessageNumberOutOfRange {
                value: 1 << MESSAGE_NUMBER_BITS
            })
        );
    }

    #[test]
    fn message_number_masks_the_wire_word() {
        assert_eq!(MessageNumber::from_wire_masked(0xFC00_0005).value(), 5);
    }
}
