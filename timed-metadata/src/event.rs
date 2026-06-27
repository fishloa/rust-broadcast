//! Unified timed-event types produced by the [`crate::Timeline`] session.

/// Kind of timed event (splice-out, splice-in, signal, etc.).
pub enum EventKind {}

/// Duration expressed in 90 kHz ticks.
pub struct MediaDuration(pub u64);

/// Position on the media timeline in 90 kHz ticks.
pub struct MediaTime(pub u64);

/// Original signalling payload in its source encoding.
pub enum SourcePayload {}

/// A decoded timed event ready for downstream use.
pub struct TimedEvent;
