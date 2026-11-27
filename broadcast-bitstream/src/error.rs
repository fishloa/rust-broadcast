//! Error type returned by every parser and converter in this crate.

use broadcast_core::error::{BufferTooShort, FieldOverflow, InvalidValue, OutputBufferTooSmall};

/// Crate-wide result alias.
pub type Result<T> = core::result::Result<T, Error>;

/// Errors a bitstream parser or stream-format converter can return.
///
/// The variants carry the workspace's `what`/`need`/`have` shape: a short
/// buffer names the field it was reading and both sizes, an over-wide field
/// names the field, its value and the reason.
// Not `Eq` (only `PartialEq`): the optional `isobmff` glue wraps
// `isobmff::Error`, which carries no `Eq` (the box layer keeps the same
// derive).
#[derive(Debug, thiserror::Error, Clone, PartialEq)]
#[non_exhaustive]
pub enum Error {
    /// The input ran out before the field was read.
    #[error(transparent)]
    BufferTooShort(#[from] BufferTooShort),

    /// A field had an invalid or reserved value.
    #[error(transparent)]
    InvalidValue(#[from] InvalidValue),

    /// A length or count did not fit the wire field it is written to.
    #[error(transparent)]
    FieldOverflow(#[from] FieldOverflow),

    /// The caller's output buffer was too small for the serialized form.
    #[error(transparent)]
    OutputBufferTooSmall(#[from] OutputBufferTooSmall),

    /// The `isobmff` box codec refused a value (the `isobmff` glue).
    #[cfg(feature = "isobmff")]
    #[error(transparent)]
    Isobmff(#[from] isobmff::Error),

    /// A caller-supplied argument violated a documented precondition.
    #[error("invalid input: {0}")]
    InvalidInput(&'static str),
}
