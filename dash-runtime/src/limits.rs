//! Hostile-input bounds the player adds on top of the manifest reader's.
//!
//! The document and element bounds are the manifest reader's
//! ([`broadcast_dash::limits::Limits`]); this module owns the ones that limit
//! segment addressing and the BaseURL alternates a plan combines.

/// Most Media Segments one `SegmentTimeline` may describe once `@r` is
/// expanded. The expansion is lazy (run table, never one entry per segment),
/// so this bounds the arithmetic, not the memory.
pub const MAX_TIMELINE_SEGMENTS: u64 = 1_000_000;
/// Most candidate base locations kept when `BaseURL` alternates are combined
/// across levels.
pub const MAX_BASE_CANDIDATES: usize = 64;

/// The manifest reader's hostile-input bounds, re-exported so a player's
/// configuration names one type.
pub use broadcast_dash::limits::Limits;
