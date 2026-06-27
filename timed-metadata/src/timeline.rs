//! Stateful session: anchors PTS to wall-clock and unrolls 33-bit wrap.

/// Stateful conversion session that maintains a [`crate::TimeAnchor`] and
/// handles 33-bit PTS wrap-around for long-running streams.
pub struct Timeline;
