//! SI-specific traits. `Parse` is provided by `dvb_common`
//! and imported directly at call sites.

use dvb_common::Parse;

/// Implemented by every typed descriptor; drives [`crate::descriptors::AnyDescriptor`]
/// dispatch. `TAG` is the wire descriptor_tag this type parses.
pub trait DescriptorDef<'a>: Parse<'a, Error = crate::error::Error> {
    /// Wire descriptor_tag.
    const TAG: u8;
    /// Diagnostic name. Convention (workspace-wide): SCREAMING_SNAKE,
    /// suffix-free — `SHORT_EVENT`, `EXTENSION`, `NETWORK_NAME`
    /// (no `_descriptor` suffix).
    const NAME: &'static str;
}

/// Implemented by every typed table-section parser; drives
/// [`crate::tables::AnyTableSection`] dispatch. `TABLE_ID_RANGES` lists the
/// inclusive `(lo, hi)` table_id ranges this type accepts.
///
/// Third-party types may implement this trait to register private table_ids
/// via [`TableRegistry`][crate::tables::TableRegistry].
pub trait TableDef<'a>: dvb_common::Parse<'a, Error = crate::error::Error> {
    /// Inclusive `(lo, hi)` table_id ranges this type parses.
    ///
    /// Single-id types use a single-element slice `&[(id, id)]`.
    /// Multi-range types (e.g. SDT `[(0x42,0x42),(0x46,0x46)]`) list each
    /// contiguous run separately.
    const TABLE_ID_RANGES: &'static [(u8, u8)];
    /// Spec name for diagnostics. SCREAMING_SNAKE, suffix-free:
    /// `PROGRAM_ASSOCIATION`, `EVENT_INFORMATION`, `SERVICE_DESCRIPTION`.
    ///
    /// Deliberate exceptions: `DSM_CC_SECTION` and `MPE_DATAGRAM_SECTION` keep
    /// `_SECTION` because it is part of the spec entity name, not a type suffix.
    const NAME: &'static str;
}
