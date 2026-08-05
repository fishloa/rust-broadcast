//! Error types for the DVB-CSA crate.
use thiserror::Error;

/// Errors that can occur during DVB-CSA operations.
#[non_exhaustive]
#[derive(Error, Debug)]
pub enum Error {
    /// The provided buffer is too short for the requested operation.
    #[error("buffer too short: need {need} bytes, have {have}")]
    BufferTooShort {
        /// The number of bytes required.
        need: usize,
        /// The number of bytes available.
        have: usize,
    },
}
