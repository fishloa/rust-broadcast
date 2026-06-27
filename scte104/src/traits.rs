//! SCTE 104 dispatch traits, mirroring scte35-splice's `CommandDef`.
//!
//! Each typed operation implements `OperationDef`, supplying its `op_id`
//! discriminant and diagnostic `NAME`. The `declare_operations!` macro in
//! `any.rs` pins the byte literal to this trait const via a drift test.

use dvb_common::Parse;

/// Implemented by every typed SCTE 104 operation; drives
/// [`crate::operations::AnyOperation`] dispatch.
pub trait OperationDef<'a>: Parse<'a, Error = crate::error::Error> {
    /// Wire `opID` (Tables 8-3/8-4).
    const OP_ID: u16;
    /// Diagnostic name, SCREAMING_SNAKE: `SPLICE_REQUEST`, `TIME_SIGNAL_REQUEST`, …
    const NAME: &'static str;
}

/// Implemented by every typed SCTE 104 operation data structure that belongs
/// in a `single_operation_message` (basic request/response).
pub trait SingleOpDef<'a>: OperationDef<'a> {}

/// Implemented by every typed SCTE 104 operation data structure that belongs
/// in a `multiple_operation_message` (Normal/Supplemental/Control).
pub trait MultiOpDef<'a>: OperationDef<'a> {}
