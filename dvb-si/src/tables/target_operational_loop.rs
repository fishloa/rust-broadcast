//! Shared target/operational descriptor-loop pair — ETSI EN 301 192 §8.4.4.1
//! Tables 17/18 (INT) and ETSI TS 102 006 §9.4 Table 11 (UNT) (r02-W22).

/// A target/operational descriptor-loop pair — the loop element shared by the
/// INT body (ETSI EN 301 192 §8.4.4.1 Tables 17/18) and the UNT platform loop
/// (ETSI TS 102 006 §9.4 Table 11). Both specs define the identical
/// `target_descriptor_loop` + `operational_descriptor_loop` pair; the INT
/// module used to own a duplicate struct and the UNT module an anonymous
/// tuple (r02-W22).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct TargetOperationalLoop<'a> {
    /// Target descriptor loop — raw descriptor bytes (after the 12-bit length
    /// field).  Serializes as the typed descriptor sequence; `.raw()` yields the
    /// wire bytes.
    pub target_descriptors: crate::descriptors::DescriptorLoop<'a>,
    /// Operational descriptor loop — raw descriptor bytes (after the 12-bit
    /// length field).  Serializes as the typed descriptor sequence; `.raw()`
    /// yields the wire bytes.
    pub operational_descriptors: crate::descriptors::DescriptorLoop<'a>,
}

/// Wire width of the 12-bit descriptor-loop length field.
pub(crate) const DESC_LOOP_LEN_FIELD: usize = 2;

impl TargetOperationalLoop<'_> {
    /// Wire length of the pair: two 12-bit length fields plus both loops.
    pub(crate) fn serialized_len(&self) -> usize {
        DESC_LOOP_LEN_FIELD
            + self.target_descriptors.len()
            + DESC_LOOP_LEN_FIELD
            + self.operational_descriptors.len()
    }
}
