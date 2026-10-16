//! Shared target/operational descriptor-loop pair — ETSI EN 301 192 §8.4.4.1
//! Tables 17/18 (INT) and ETSI TS 102 006 §9.4 Table 11 (UNT) (r02-W22).

use crate::descriptors::scoped::ScopedDescriptorLoop;

/// A target/operational descriptor-loop pair — the loop element shared by the
/// INT body (ETSI EN 301 192 §8.4.4.1 Tables 17/18) and the UNT platform loop
/// (ETSI TS 102 006 §9.4 Table 11). Both specs define the identical
/// `target_descriptor_loop` + `operational_descriptor_loop` pair, but each in
/// its OWN descriptor-tag namespace (issue #1285), so the pair is generic over
/// the loop type: [`IntLoopEntry`](crate::tables::int::IntLoopEntry) holds
/// [`IntDescriptorLoop`](crate::descriptors::int::IntDescriptorLoop)s and
/// [`UntLoopEntry`](crate::tables::unt::UntLoopEntry) holds
/// [`UntDescriptorLoop`](crate::descriptors::unt::UntDescriptorLoop)s.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct TargetOperationalLoop<'a, L: ScopedDescriptorLoop<'a>> {
    /// Target descriptor loop — raw descriptor bytes (after the 12-bit length
    /// field).  Serializes as the typed descriptor sequence; `.raw()` yields the
    /// wire bytes.
    pub target_descriptors: L,
    /// Operational descriptor loop — raw descriptor bytes (after the 12-bit
    /// length field).  Serializes as the typed descriptor sequence; `.raw()`
    /// yields the wire bytes.
    pub operational_descriptors: L,
    /// `reserved(4)` before `target_descriptor_loop_length` (`1111`);
    /// preserved from the wire.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub reserved_target_length: u8,
    /// `reserved(4)` before `operational_descriptor_loop_length` (`1111`);
    /// preserved from the wire.
    #[cfg_attr(feature = "serde", serde(skip))]
    pub reserved_operational_length: u8,
    #[cfg_attr(feature = "serde", serde(skip))]
    _lt: core::marker::PhantomData<&'a ()>,
}

impl<'a, L: ScopedDescriptorLoop<'a>> TargetOperationalLoop<'a, L> {
    /// Construct a pair with the spec `reserved` fill.
    #[must_use]
    pub fn new(target_descriptors: L, operational_descriptors: L) -> Self {
        Self {
            target_descriptors,
            operational_descriptors,
            reserved_target_length: super::RESERVED_4,
            reserved_operational_length: super::RESERVED_4,
            _lt: core::marker::PhantomData,
        }
    }

    /// Construct a pair from the wire, keeping the parsed `reserved(4)` fills.
    pub(crate) fn from_parts(
        target_descriptors: L,
        operational_descriptors: L,
        reserved_target_length: u8,
        reserved_operational_length: u8,
    ) -> Self {
        Self {
            target_descriptors,
            operational_descriptors,
            reserved_target_length,
            reserved_operational_length,
            _lt: core::marker::PhantomData,
        }
    }

    /// Wire length of the pair: two 12-bit length fields plus both loops.
    pub(crate) fn serialized_len(&self) -> usize {
        DESC_LOOP_LEN_FIELD
            + self.target_descriptors.raw().len()
            + DESC_LOOP_LEN_FIELD
            + self.operational_descriptors.raw().len()
    }
}

/// Wire width of the 12-bit descriptor-loop length field.
pub(crate) const DESC_LOOP_LEN_FIELD: usize = 2;
