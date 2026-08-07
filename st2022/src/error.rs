//! Error types for ST 2022-6/-7 parsing.

/// Errors produced when parsing ST 2022-6/-7 structures.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// Input buffer too short.
    #[error("buffer too short: need {need}, have {have} for {what}")]
    BufferTooShort {
        /// Minimum bytes required.
        need: usize,
        /// Bytes actually available.
        have: usize,
        /// What was being parsed.
        what: &'static str,
    },
}
