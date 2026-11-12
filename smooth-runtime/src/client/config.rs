//! Tunables: [`SmoothConfig`] and the video-quality choice [`QualitySelector`].

use std::fmt;
use std::sync::Arc;

use super::output::QualityInfo;

/// Fragments before the live edge a live join starts at.
pub const DEFAULT_LIVE_JOIN_FRAGMENTS: usize = 2;
/// Fragment fetch failures tolerated before a fragment is skipped (live) or the
/// session ends (static).
pub const DEFAULT_MAX_FRAGMENT_FAILURES: u32 = 5;
/// Manifest failures tolerated before the session ends.
pub const DEFAULT_MAX_MANIFEST_FAILURES: u32 = 5;
/// Consecutive refreshes lacking a stream before that stream is abandoned.
pub const DEFAULT_MAX_REFRESH_MISSES: u32 = 5;
/// Backoff after the first failures, in milliseconds.
pub const DEFAULT_BACKOFF_MS: [u64; 4] = [250, 500, 1_000, 2_000];
/// Backoff once the list is exhausted, in milliseconds.
pub const DEFAULT_BACKOFF_CAP_MS: u64 = 4_000;
/// Shortest live manifest refresh wait, in microseconds.
pub const DEFAULT_REFRESH_MIN_US: u64 = 500_000;
/// Longest live manifest refresh wait, in microseconds.
pub const DEFAULT_REFRESH_MAX_US: u64 = 5_000_000;
/// `mvhd` timescale of each synthesized init segment (ISO/IEC 14496-12:2026 §8.2.2).
pub const SYNTHETIC_MOVIE_TIMESCALE: u32 = 90_000;

type SelectFn = dyn Fn(&[QualityInfo]) -> usize + Send + Sync;

/// Chooses which `QualityLevel` of the first video stream to play.
#[derive(Clone)]
pub struct QualitySelector(Arc<SelectFn>);

impl QualitySelector {
    /// A selector backed by `f`, which returns an index into the offered list.
    pub fn new(f: impl Fn(&[QualityInfo]) -> usize + Send + Sync + 'static) -> Self {
        Self(Arc::new(f))
    }

    /// Always the first `QualityLevel`.
    pub fn first() -> Self {
        Self::new(|_| 0)
    }

    pub(crate) fn choose(&self, qualities: &[QualityInfo]) -> usize {
        (self.0)(qualities).min(qualities.len().saturating_sub(1))
    }
}

impl fmt::Debug for QualitySelector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("QualitySelector(..)")
    }
}

/// Engine tunables.
#[derive(Debug, Clone)]
pub struct SmoothConfig {
    /// Live join point, in fragments behind the newest.
    pub live_join_fragments: usize,
    /// Consecutive failures of one fragment before it is skipped (live) or the session ends (static); default 5.
    pub max_fragment_failures: u32,
    /// Consecutive manifest failures before the session ends; default 5.
    pub max_manifest_failures: u32,
    /// Consecutive refreshes lacking a stream before that stream is abandoned; default 5.
    pub max_refresh_misses: u32,
    /// Retry delays by attempt, in milliseconds.
    pub backoff_ms: Vec<u64>,
    /// Retry delay past the end of [`Self::backoff_ms`], in milliseconds.
    pub backoff_cap_ms: u64,
    /// Lower clamp of the live refresh wait, in microseconds.
    pub refresh_min_us: u64,
    /// Upper clamp of the live refresh wait, in microseconds.
    pub refresh_max_us: u64,
    /// Video quality choice.
    pub video_quality: QualitySelector,
}

impl Default for SmoothConfig {
    fn default() -> Self {
        Self {
            live_join_fragments: DEFAULT_LIVE_JOIN_FRAGMENTS,
            max_fragment_failures: DEFAULT_MAX_FRAGMENT_FAILURES,
            max_manifest_failures: DEFAULT_MAX_MANIFEST_FAILURES,
            max_refresh_misses: DEFAULT_MAX_REFRESH_MISSES,
            backoff_ms: DEFAULT_BACKOFF_MS.to_vec(),
            backoff_cap_ms: DEFAULT_BACKOFF_CAP_MS,
            refresh_min_us: DEFAULT_REFRESH_MIN_US,
            refresh_max_us: DEFAULT_REFRESH_MAX_US,
            video_quality: QualitySelector::first(),
        }
    }
}
