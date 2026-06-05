//! SI-specific traits. `Parse` / `Serialize` are provided by `dvb_common`
//! and imported directly at call sites.

use dvb_common::{Parse, Serialize};

/// Contract every DVB descriptor implements.
///
/// Descriptors are length-prefixed payloads inside tables. The
/// `TAG` constant ties a struct to its wire identifier.
pub trait Descriptor<'a>: Parse<'a> + Serialize {
    /// Descriptor tag byte — the wire identifier.
    const TAG: u8;

    /// Length of the payload portion (NOT including the 2 header bytes).
    fn descriptor_length(&self) -> u8;
}

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

/// Contract every section-carried table implements.
pub trait Table<'a>: Parse<'a> + Serialize {
    /// Expected `table_id` for this table.
    ///
    /// Tables that occupy a range of table_ids (e.g. EIT schedule which covers
    /// 0x50..=0x5F) expose a range helper on the type itself rather than a
    /// single `TABLE_ID` constant.
    const TABLE_ID: u8;

    /// PID on which this table is typically carried.
    ///
    /// Some tables (PMT) use per-programme PIDs signalled by PAT; those
    /// return `0x0000` here and the consumer is expected to know better.
    const PID: u16;
}
