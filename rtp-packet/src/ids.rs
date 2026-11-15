//! The RTP/RTCP wire identities — RFC 3550 §5.1 `SSRC`/`sequence number` and
//! §6.4.1 `extended highest sequence number`.
//!
//! Each is a fixed-width wire field with no invalid bit pattern, so every
//! bit pattern round-trips byte-exactly; the type exists so a sequence number
//! cannot be passed where an SSRC is expected and so the wrap arithmetic of
//! the 16-bit sequence space lives in one place.

use crate::seq::RTP_SEQ_MOD;

/// `SSRC` — the 32-bit synchronization source identifier (RFC 3550 §5.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[repr(transparent)]
pub struct Ssrc(u32);

impl Ssrc {
    /// Wrap a raw 32-bit identifier from the wire or a key table.
    #[must_use]
    pub const fn new(raw: u32) -> Self {
        Self(raw)
    }

    /// The raw 32-bit identifier.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

impl From<u32> for Ssrc {
    fn from(raw: u32) -> Self {
        Self(raw)
    }
}

impl From<Ssrc> for u32 {
    fn from(ssrc: Ssrc) -> Self {
        ssrc.0
    }
}

impl core::fmt::Display for Ssrc {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "0x{:08x}", self.0)
    }
}

/// RTP `sequence number` — 16 bits, incremented by one per packet and
/// wrapping modulo 2^16 (RFC 3550 §5.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[repr(transparent)]
pub struct SeqNo(u16);

impl SeqNo {
    /// Wrap a raw 16-bit sequence number from the wire.
    #[must_use]
    pub const fn new(raw: u16) -> Self {
        Self(raw)
    }

    /// The raw 16-bit sequence number.
    #[must_use]
    pub const fn get(self) -> u16 {
        self.0
    }

    /// The next sequence number, wrapping modulo 2^16.
    #[must_use]
    pub const fn wrapping_next(self) -> Self {
        Self(self.0.wrapping_add(1))
    }

    /// `self - earlier` modulo 2^16: the forward distance from `earlier`
    /// (0..=65535), the same quantity RFC 3550 §A.1 computes as `udelta`.
    #[must_use]
    pub const fn forward_distance(self, earlier: Self) -> u16 {
        self.0.wrapping_sub(earlier.0)
    }
}

impl From<u16> for SeqNo {
    fn from(raw: u16) -> Self {
        Self(raw)
    }
}

impl From<SeqNo> for u16 {
    fn from(seq: SeqNo) -> Self {
        seq.0
    }
}

/// `extended highest sequence number received` — the 16-bit sequence number
/// combined with a 16-bit cycle count into one 32-bit value (RFC 3550 §6.4.1,
/// `extended_highest_sequence_number`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[repr(transparent)]
pub struct ExtendedSeqNo(u32);

impl ExtendedSeqNo {
    /// Wrap a raw 32-bit extended sequence number.
    #[must_use]
    pub const fn new(raw: u32) -> Self {
        Self(raw)
    }

    /// The raw 32-bit value.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }

    /// Packets from `self` down to `earlier`, resolving `earlier` to the
    /// nearest 16-bit cycle at or below this value. The subtraction is exact
    /// only while the distance is at most half the 16-bit space, which is
    /// the assumption RFC 3550 §A.3's loss estimate already makes.
    #[must_use]
    pub const fn forward_distance(self, earlier: SeqNo) -> u32 {
        let ext_earlier = (self.0 & !(RTP_SEQ_MOD - 1)) | earlier.0 as u32;
        let ext_earlier = if ext_earlier > self.0 {
            ext_earlier - RTP_SEQ_MOD
        } else {
            ext_earlier
        };
        self.0.wrapping_sub(ext_earlier)
    }
}

impl From<u32> for ExtendedSeqNo {
    fn from(raw: u32) -> Self {
        Self(raw)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sequence_numbers_wrap_modulo_64k() {
        assert_eq!(SeqNo::new(u16::MAX).wrapping_next(), SeqNo::new(0));
        assert_eq!(SeqNo::new(65535).wrapping_next(), SeqNo::new(0));
        assert_eq!(SeqNo::new(1).wrapping_next(), SeqNo::new(2));
    }

    #[test]
    fn forward_distance_is_modular() {
        assert_eq!(SeqNo::new(5).forward_distance(SeqNo::new(3)), 2);
        assert_eq!(SeqNo::new(0).forward_distance(SeqNo::new(u16::MAX)), 1);
    }

    #[test]
    fn extended_distance_across_a_cycle_is_counted() {
        let ext = ExtendedSeqNo::new(RTP_SEQ_MOD + 5);
        assert_eq!(ext.forward_distance(SeqNo::new(3)), 2);
        assert_eq!(ExtendedSeqNo::new(5).forward_distance(SeqNo::new(3)), 2);
    }

    #[test]
    fn id_wrappers_round_trip_the_wire_value() {
        assert_eq!(u32::from(Ssrc::from(0xDEAD_BEEF)), 0xDEAD_BEEF);
        assert_eq!(u16::from(SeqNo::from(4321)), 4321);
        assert_eq!(ExtendedSeqNo::new(7).get(), 7);
        assert_eq!(Ssrc::new(1).to_string(), "0x00000001");
    }
}
