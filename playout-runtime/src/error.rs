//! Crate error type.

/// Errors produced by schedule construction, transition planning, and
/// SCTE-35 emission-point construction.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// [`crate::schedule::Schedule::push`] was called with an entry whose
    /// `planned_start` does not strictly follow the schedule's last entry —
    /// entries must be supplied in increasing timeline order.
    #[error(
        "schedule entries must be strictly increasing by planned_start: \
         previous entry starts at {prev}, new entry starts at {next}"
    )]
    OutOfOrder {
        /// `planned_start` of the schedule's current last entry.
        prev: u64,
        /// `planned_start` of the entry that was rejected.
        next: u64,
    },
    /// Splice-point conditioning ([`ssai_runtime::splice::condition_splice_point`],
    /// via [`crate::scte35::build_splice_insert`]) found no candidate
    /// boundary within the caller's tolerance, or was given no candidates at
    /// all.
    #[error(transparent)]
    SpliceConditioning(#[from] ssai_runtime::Error),
}

/// Crate result alias.
pub type Result<T> = core::result::Result<T, Error>;
