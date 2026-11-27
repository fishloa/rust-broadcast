//! The one place a streaming demuxer applies the caller's damage policy.
//!
//! Every best-effort demuxer publishes what it skipped as a
//! [`DemuxEvent::Damage`] (`broadcast_core::damage::Damage`). This module holds
//! the single helper that lets a caller stop on those reports: a demuxer records
//! the length of its event queue before it starts work and hands the events
//! added since to [`admit_new`], which returns the typed
//! [`DamageError`](broadcast_core::damage::DamageError) of the first report the
//! policy refuses.

use alloc::collections::VecDeque;
use broadcast_core::damage::{DamageError, DamagePolicy};

use crate::ir::DemuxEvent;

/// Apply `policy` to every damage report queued at or after `from`.
///
/// The reports stay in the queue: a demuxer that stops returns the error and
/// its events are discarded with it, and one that continues has already queued
/// the report for its caller. `from` is the queue length before the work, so a
/// demuxer never re-admits a report it already published.
///
/// # Errors
/// The first report whose [`DamagePolicy`] decision is `Disposition::Fail`, as
/// the demuxer's own error type (`crate::Error::Damage`, `FlvError::Damage`).
pub(crate) fn admit_new<E: From<DamageError>>(
    events: &VecDeque<DemuxEvent>,
    policy: &DamagePolicy,
    from: usize,
) -> Result<(), E> {
    for event in events.iter().skip(from) {
        if let DemuxEvent::Damage(damage) = event {
            policy.admit(damage.clone())?;
        }
    }
    Ok(())
}
