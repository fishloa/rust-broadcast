//! Source-agnostic fetch bookkeeping shared by the segment-pull adapters: small passive state
//! machines with no knowledge of what is being fetched. Each adapter keeps its own failure routing.

pub(crate) mod body;
pub(crate) mod conn;
pub(crate) mod deadlines;
pub(crate) mod failures;
pub(crate) mod inactivity;
pub(crate) mod inflight;
pub(crate) mod resume;
pub(crate) mod retry;
pub(crate) mod wall;
pub(crate) mod window;

use core::time::Duration;

use broadcast_common::stage::Timestamp;

/// `now + span` as the deadline a timer arms, or `None` when that instant does not fit
/// in the clock — a deadline that can never be reached, which is a timer that never
/// fires (`docs/API-CONVENTIONS.md`, Time). Never `saturating_add` a deadline: one that
/// saturates to the last instant reads as "due now" on every poll.
pub(crate) fn deadline(now: Timestamp, span: Duration) -> Option<Timestamp> {
    now.checked_add(span)
}
