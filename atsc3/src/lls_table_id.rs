//! LLS Table ID — ATSC A/331:2025-06 §6.2 Table 6.1.
//!
//! Identifies which LLS table body follows the common 4-byte envelope (see
//! [`crate::lls::LlsEnvelope`]). `0x01`-`0x05` are transcribed verbatim in
//! `docs/a331-signalling.md` Table 6.1. `0x06` (CDT, A/360 CertificationData)
//! and `0x07` (DRCT, Dedicated Return Channel Table) are additional
//! cross-spec registrations — Table 6.1's own prose notes "Additional LLS
//! table IDs exist outside this list (e.g. A/360's CertificationData...)".
//!
//! `0x07` was previously modelled as CAP (Common Alerting Protocol). That was
//! wrong: ATSC A/323:2026-04 §7 (p. 66) states "The DRCT is allocated the
//! LLS_table_id 0x07 (see Code Point Registry)", and §5 (p. 18) repeats that
//! the Dedicated Return Channel Table "is defined as case 0x07". CAP messages
//! are carried inside the AEAT (A/331 §6.5) and have no LLS_table_id of their
//! own. Verified against the A/323:2026-04 PDF fetched from atsc.org; A/323 is
//! not vendored here, so that fetch is the provenance.
//!
//! `0x06` (CDT) remains sourced from A/360 and is not independently verified
//! against a vendored PDF in this pass.
//!
//! Table 6.1 as transcribed also names `0xFE` as `SignedMultiTable` (§6.7,
//! not otherwise modeled here) rather than reserved; this enum folds it into
//! the `0x08`-`0xFE` reserved range for this first pass, since
//! `SignedMultiTable` framing (an outer wrapper around other `LLS_table()`
//! instances) is out of scope until it is implemented.

/// The 8-bit `LLS_table_id` field (A/331 §6.2 Table 6.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum LlsTableId {
    /// `0x01` — Service List Table (§6.3).
    Slt,
    /// `0x02` — Rating Region Table (Annex F, not transcribed).
    Rrt,
    /// `0x03` — System Time (§6.4, not transcribed).
    SystemTime,
    /// `0x04` — Advanced Emergency Alerting Table (§6.5, not transcribed).
    Aeat,
    /// `0x05` — Onscreen Message Notification (§6.6, not transcribed).
    OnscreenMessageNotification,
    /// `0x06` — Certification Data Table (A/360 CertificationData; see
    /// module doc for sourcing caveat).
    Cdt,
    /// `0x07` — Dedicated Return Channel Table (ATSC A/323:2026-04 §7,
    /// p. 66: "The DRCT is allocated the LLS_table_id 0x07").
    Drct,
    /// `0x00`, `0x08`-`0xFE` — ATSC/Industry Reserved (see module doc:
    /// includes `0xFE` `SignedMultiTable`, not modeled separately yet).
    Reserved(u8),
    /// `0xFF` — UserDefined (§6.8, not transcribed).
    UserDefined,
}

impl LlsTableId {
    /// The spec token for this value ("reserved" for the reserved arm) —
    /// see the workspace's #204 label convention.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Slt => "SLT",
            Self::Rrt => "RRT",
            Self::SystemTime => "SystemTime",
            Self::Aeat => "AEAT",
            Self::OnscreenMessageNotification => "OnscreenMessageNotification",
            Self::Cdt => "CDT",
            Self::Drct => "DRCT",
            Self::Reserved(_) => "reserved",
            Self::UserDefined => "UserDefined",
        }
    }

    /// Decode from the wire `LLS_table_id` byte.
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v {
            0x01 => Self::Slt,
            0x02 => Self::Rrt,
            0x03 => Self::SystemTime,
            0x04 => Self::Aeat,
            0x05 => Self::OnscreenMessageNotification,
            0x06 => Self::Cdt,
            0x07 => Self::Drct,
            0xFF => Self::UserDefined,
            other => Self::Reserved(other),
        }
    }

    /// Encode back to the wire `LLS_table_id` byte.
    #[must_use]
    pub fn to_u8(&self) -> u8 {
        match self {
            Self::Slt => 0x01,
            Self::Rrt => 0x02,
            Self::SystemTime => 0x03,
            Self::Aeat => 0x04,
            Self::OnscreenMessageNotification => 0x05,
            Self::Cdt => 0x06,
            Self::Drct => 0x07,
            Self::UserDefined => 0xFF,
            Self::Reserved(v) => *v,
        }
    }
}

broadcast_common::impl_spec_display!(LlsTableId, Reserved);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_values_round_trip() {
        for (id, expected) in [
            (LlsTableId::Slt, 0x01),
            (LlsTableId::Rrt, 0x02),
            (LlsTableId::SystemTime, 0x03),
            (LlsTableId::Aeat, 0x04),
            (LlsTableId::OnscreenMessageNotification, 0x05),
            (LlsTableId::Cdt, 0x06),
            (LlsTableId::Drct, 0x07),
            (LlsTableId::UserDefined, 0xFF),
        ] {
            assert_eq!(id.to_u8(), expected);
            assert_eq!(LlsTableId::from_u8(expected), id);
        }
    }

    #[test]
    fn all_byte_values_round_trip() {
        for v in 0u8..=0xFF {
            let id = LlsTableId::from_u8(v);
            assert_eq!(id.to_u8(), v);
        }
    }

    #[test]
    fn reserved_values_are_reserved() {
        for v in [0x00, 0x08, 0x80, 0xFE] {
            assert_eq!(LlsTableId::from_u8(v), LlsTableId::Reserved(v));
            assert_eq!(LlsTableId::from_u8(v).name(), "reserved");
        }
    }

    #[test]
    fn display_matches_name() {
        assert_eq!(LlsTableId::Slt.to_string(), "SLT");
        assert_eq!(LlsTableId::Reserved(0x08).to_string(), "reserved(0x08)");
    }
}
