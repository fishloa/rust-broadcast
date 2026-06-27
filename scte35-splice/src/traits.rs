//! SCTE-35-specific dispatch traits. `Parse` / `Serialize` come from
//! `dvb_common` and are imported directly at call sites.
//!
//! These mirror dvb-si's `DescriptorDef` / `TableDef`: each typed wire entity
//! declares its discriminant byte and a SCREAMING_SNAKE diagnostic `NAME`, and
//! the `declare_*!` dispatch macros pin the byte literal in the dispatch list
//! to the trait const via a drift test, so the list can never silently drift
//! from the implemented set.

use dvb_common::Parse;

/// Implemented by every typed splice command; drives
/// [`crate::commands::AnyCommand`] dispatch. `COMMAND_TYPE` is the
/// `splice_command_type` byte (§9.6.1, Table 7) this type parses.
pub trait CommandDef<'a>: Parse<'a, Error = crate::error::Error> {
    /// Wire `splice_command_type` (§9.6.1, Table 7).
    const COMMAND_TYPE: u8;
    /// Diagnostic name, SCREAMING_SNAKE, suffix-free: `SPLICE_INSERT`,
    /// `TIME_SIGNAL`, `BANDWIDTH_RESERVATION`.
    const NAME: &'static str;
}

/// Implemented by every typed splice descriptor; drives
/// [`crate::descriptors::AnySpliceDescriptor`] dispatch. `TAG` is the
/// `splice_descriptor_tag` byte (§10.1, Table 16) this type parses.
pub trait SpliceDescriptorDef<'a>: Parse<'a, Error = crate::error::Error> {
    /// Wire `splice_descriptor_tag` (§10.1, Table 16).
    const TAG: u8;
    /// Diagnostic name, SCREAMING_SNAKE, suffix-free: `AVAIL`, `DTMF`,
    /// `SEGMENTATION`, `TIME`, `AUDIO`.
    const NAME: &'static str;
}
