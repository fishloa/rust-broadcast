//! Tunables: [`SmoothClientConfig`] and the video-quality choice
//! [`QualitySelector`].

use std::fmt;
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use url::Url;

use super::output::QualityInfo;
use broadcast_common::backoff::BackoffSteps;
use broadcast_common::timescale::Timescale;

/// Fragments before the live edge a live join starts at.
pub const DEFAULT_LIVE_JOIN_FRAGMENTS: usize = 2;
/// Fragment fetch failures tolerated before a fragment is skipped (live) or the
/// session ends (static).
pub const DEFAULT_MAX_FRAGMENT_FAILURES: u32 = 5;
/// Manifest failures tolerated before the session ends.
pub const DEFAULT_MAX_MANIFEST_FAILURES: u32 = 5;
/// Consecutive refreshes lacking a stream before that stream is abandoned.
pub const DEFAULT_MAX_REFRESH_MISSES: u32 = 5;
/// Backoff after the first failures.
pub const DEFAULT_BACKOFF_STEPS: [Duration; 4] = [
    Duration::from_millis(250),
    Duration::from_millis(500),
    Duration::from_millis(1_000),
    Duration::from_millis(2_000),
];
/// Backoff once the list is exhausted.
pub const DEFAULT_BACKOFF_CAP: Duration = Duration::from_millis(4_000);
/// Default [`SmoothClientConfig::jitter_seed`]; a host that runs more than one client sets its own.
pub const DEFAULT_JITTER_SEED: u64 = 0;
/// Shortest live manifest refresh wait.
pub const DEFAULT_REFRESH_MIN: Duration = Duration::from_millis(500);
/// Longest live manifest refresh wait.
pub const DEFAULT_REFRESH_MAX: Duration = Duration::from_secs(5);
/// `mvhd` timescale of each synthesized init segment (ISO/IEC 14496-12:2026 §8.2.2).
pub const SYNTHETIC_MOVIE_TIMESCALE: Timescale = transmux::VIDEO_CLOCK_TIMESCALE;
/// [`SmoothClientConfig::default`]'s placeholder manifest URL: a default has no
/// real manifest, so a client built from one is only useful after
/// [`SmoothClientConfig::new`] or `with_manifest_url`.
static DEFAULT_MANIFEST_URL: LazyLock<Url> = LazyLock::new(|| {
    if let Ok(url) = Url::parse("http://localhost/Manifest") {
        url
    } else {
        std::process::abort()
    }
});

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

/// Engine tunables and the manifest to play: build it with
/// [`SmoothClientConfig::new`] (the manifest URL) and set what differs with the
/// `with_*` setters. Every engine field has a documented default.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct SmoothClientConfig {
    /// The manifest URL the first [`Action`](super::Action) fetches.
    pub manifest_url: Url,
    /// Live join point, in fragments behind the newest.
    pub live_join_fragments: usize,
    /// Consecutive failures of one fragment before it is skipped (live) or the session ends (static); default 5.
    pub max_fragment_failures: u32,
    /// Consecutive manifest failures before the session ends; default 5.
    pub max_manifest_failures: u32,
    /// Consecutive refreshes lacking a stream before that stream is abandoned; default 5.
    pub max_refresh_misses: u32,
    /// Retry delays by attempt, then the cap.
    pub backoff: BackoffSteps,
    /// Seed of the retry jitter ([`broadcast_common::backoff::RetrySchedule`]): the first retry of
    /// a fragment or manifest waits exactly its step, every later one a term of `[5/6, 1.0)` of its
    /// step drawn from this seed, so a fleet of clients that fail together does not retry in
    /// lockstep. The engine reads no clock and no RNG, so a host gives each client its own entropy;
    /// two clients with the same seed retry identically (which is what makes a replay reproducible).
    pub jitter_seed: u64,
    /// Lower clamp of the live refresh wait.
    pub refresh_min: Duration,
    /// Upper clamp of the live refresh wait.
    pub refresh_max: Duration,
    /// Video quality choice.
    pub video_quality: QualitySelector,
}

impl SmoothClientConfig {
    /// A client for the manifest at `manifest_url` (an absolute URL, e.g.
    /// `http://origin/live/Manifest`), with every engine tunable at its
    /// default. A string that does not parse leaves
    /// [`SmoothClientConfig::manifest_url`] at [`SmoothClientConfig::default`]'s
    /// placeholder, which no origin serves; [`SmoothClientConfig::with_manifest_url`]
    /// takes a `Url` already parsed.
    pub fn new(manifest_url: impl Into<String>) -> Self {
        let mut config = SmoothClientConfig::default();
        let Ok(url) = Url::parse(&manifest_url.into()) else {
            return config;
        };
        config.manifest_url = url;
        config
    }

    /// Replace [`SmoothClientConfig::manifest_url`].
    #[must_use]
    pub fn with_manifest_url(mut self, manifest_url: Url) -> Self {
        self.manifest_url = manifest_url;
        self
    }
}

impl Default for SmoothClientConfig {
    fn default() -> Self {
        Self {
            manifest_url: DEFAULT_MANIFEST_URL.clone(),
            live_join_fragments: DEFAULT_LIVE_JOIN_FRAGMENTS,
            max_fragment_failures: DEFAULT_MAX_FRAGMENT_FAILURES,
            max_manifest_failures: DEFAULT_MAX_MANIFEST_FAILURES,
            max_refresh_misses: DEFAULT_MAX_REFRESH_MISSES,
            backoff: BackoffSteps {
                steps: DEFAULT_BACKOFF_STEPS.to_vec(),
                cap: DEFAULT_BACKOFF_CAP,
            },
            jitter_seed: DEFAULT_JITTER_SEED,
            refresh_min: DEFAULT_REFRESH_MIN,
            refresh_max: DEFAULT_REFRESH_MAX,
            video_quality: QualitySelector::first(),
        }
    }
}

impl SmoothClientConfig {
    /// Set [`Self::live_join_fragments`].
    #[must_use]
    pub fn with_live_join_fragments(mut self, live_join_fragments: usize) -> Self {
        self.live_join_fragments = live_join_fragments;
        self
    }

    /// Set [`Self::max_fragment_failures`].
    #[must_use]
    pub fn with_max_fragment_failures(mut self, max_fragment_failures: u32) -> Self {
        self.max_fragment_failures = max_fragment_failures;
        self
    }

    /// Set [`Self::max_manifest_failures`].
    #[must_use]
    pub fn with_max_manifest_failures(mut self, max_manifest_failures: u32) -> Self {
        self.max_manifest_failures = max_manifest_failures;
        self
    }

    /// Set [`Self::max_refresh_misses`].
    #[must_use]
    pub fn with_max_refresh_misses(mut self, max_refresh_misses: u32) -> Self {
        self.max_refresh_misses = max_refresh_misses;
        self
    }

    /// Set [`Self::backoff`].
    #[must_use]
    pub fn with_backoff(mut self, backoff: BackoffSteps) -> Self {
        self.backoff = backoff;
        self
    }

    /// Set [`Self::jitter_seed`].
    #[must_use]
    pub fn with_jitter_seed(mut self, jitter_seed: u64) -> Self {
        self.jitter_seed = jitter_seed;
        self
    }

    /// Set [`Self::refresh_min`].
    #[must_use]
    pub fn with_refresh_min(mut self, refresh_min: Duration) -> Self {
        self.refresh_min = refresh_min;
        self
    }

    /// Set [`Self::refresh_max`].
    #[must_use]
    pub fn with_refresh_max(mut self, refresh_max: Duration) -> Self {
        self.refresh_max = refresh_max;
        self
    }

    /// Set [`Self::video_quality`].
    #[must_use]
    pub fn with_video_quality(mut self, video_quality: QualitySelector) -> Self {
        self.video_quality = video_quality;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_carries_the_manifest_url_bytes_for_bytes() {
        let config = SmoothClientConfig::new("http://origin.example/live/Manifest?token=a%20b");
        assert_eq!(
            config.manifest_url.as_str(),
            "http://origin.example/live/Manifest?token=a%20b"
        );
    }

    #[test]
    fn with_manifest_url_replaces_the_url_and_keeps_the_tunables() {
        let config = SmoothClientConfig::new("http://a.example/Manifest")
            .with_live_join_fragments(7)
            .with_manifest_url(Url::parse("http://b.example/Manifest").expect("URL"));
        assert_eq!(config.manifest_url.as_str(), "http://b.example/Manifest");
        assert_eq!(config.live_join_fragments, 7);
    }

    #[test]
    fn the_default_carries_a_parsed_placeholder_manifest() {
        assert!(SmoothClientConfig::default().manifest_url.has_host());
    }
}
