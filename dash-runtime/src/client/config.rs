//! `DashConfig`: the DASH-specific knobs of a client session.
//!
//! `docs/dash-runtime/DESIGN.md` 6.4 (fields and defaults), 3.2-3.4 (clock
//! ladder), 5.2 (refresh floor), 5.4 (early-request cadence), D9 (LL-DASH) and
//! 10.2 R3 (cross-origin restriction). Pure data: nothing here reads a clock.
//! Every span is a [`Duration`]; a deadline built from one is `None` (never
//! fires) past the end of the clock rather than clamped to it.

use core::time::Duration;

use broadcast_common::impl_spec_display;
use thiserror::Error;

/// Default time one `UTCTiming` request may take.
pub const DEFAULT_UTC_TIMEOUT: Duration = Duration::from_secs(2);
/// Default total wait for a `UTCTiming` sample before the clock ladder falls
/// through to the next source.
pub const DEFAULT_UTC_WAIT: Duration = Duration::from_secs(5);
/// Default longest round trip a `UTCTiming` sample may have and still be used.
pub const DEFAULT_UTC_MAX_RTT: Duration = Duration::from_secs(2);
/// Default age after which a same-source clock anchor is replaced.
pub const DEFAULT_UTC_RESYNC: Duration = Duration::from_secs(600);
/// Default error bar reported for a host-supplied clock.
pub const DEFAULT_HOST_CLOCK_UNCERTAINTY: Duration = Duration::from_millis(500);
/// Default margin added to a segment's availability start before requesting it.
pub const DEFAULT_AVAILABILITY_MARGIN: Duration = Duration::ZERO;
/// Default first retry delay for a segment requested before it exists.
pub const DEFAULT_EARLY_RETRY: Duration = Duration::from_millis(250);
/// Default window after a segment's availability start in which a 404, 410 or
/// 425 still means "too early" rather than "missing".
pub const DEFAULT_EARLY_GRACE: Duration = Duration::from_secs(1);
/// Default floor under the MPD refresh interval.
pub const DEFAULT_MIN_MPD_REFRESH: Duration = Duration::from_secs(1);

/// Which clock source the ladder tries first for a dynamic MPD.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum DashClockPolicy {
    /// `UTCTiming`, then the host clock, then the MPD response `Date`.
    #[default]
    Auto,
    /// The host clock first (a host with a trusted NTP clock saves the round
    /// trip), then `UTCTiming`, then the MPD response `Date`.
    PreferHost,
}

impl DashClockPolicy {
    /// Stable snake_case label.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::PreferHost => "prefer_host",
        }
    }
}
impl_spec_display!(DashClockPolicy);

/// A [`DashConfig`] field no client can run with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("invalid dash config: {field}")]
pub struct DashConfigError {
    /// The offending field name.
    pub field: &'static str,
}

/// DASH-specific configuration. The knobs shared with every segment-pull
/// source (redirects, headers, cookies, backoff, in-flight window, body caps,
/// playout, timeline-jump detection) stay on the session's own config.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct DashConfig {
    /// Clock ladder order, see [`DashClockPolicy`].
    pub clock_policy: DashClockPolicy,
    /// Time one `UTCTiming` request may take; must be non-zero.
    pub utc_timeout: Duration,
    /// Total wait for a `UTCTiming` sample before the ladder falls through;
    /// must be non-zero and not shorter than [`DashConfig::utc_timeout`] (a
    /// wait that ends before one request can time out would never let the
    /// first request finish).
    pub utc_wait: Duration,
    /// Longest round trip a `UTCTiming` sample may have; a slower exchange is
    /// discarded. Must be non-zero.
    pub utc_max_rtt: Duration,
    /// Age after which a same-source clock anchor is replaced by any new
    /// sample of that source; must be non-zero. `None` never re-syncs on age.
    pub utc_resync: Option<Duration>,
    /// Error bar reported with a clock taken from the host. `ZERO` claims an
    /// exact host clock.
    pub host_clock_uncertainty: Duration,
    /// Override of the presentation delay a live join keeps behind the live
    /// edge. `None` derives it from the MPD. `Some(ZERO)` joins at the edge.
    pub live_delay: Option<Duration>,
    /// Added to a segment's availability start before it is requested.
    /// `ZERO` requests at the availability start.
    pub availability_margin: Duration,
    /// First retry delay for a segment requested before it exists; must be
    /// non-zero. The delay doubles up to one second.
    pub early_retry: Duration,
    /// Window after a segment's availability start in which a 404, 410 or
    /// 425 still means "too early". `ZERO` treats every such response after
    /// the start as a missing segment.
    pub early_grace: Duration,
    /// Floor under the MPD refresh interval; must be non-zero so
    /// `minimumUpdatePeriod="PT0S"` cannot become a request storm.
    pub min_mpd_refresh: Duration,
    /// Request low-latency segments at `availabilityTimeOffset` earlier than
    /// their complete-segment time. Delivery stays whole-segment.
    pub use_availability_time_offset: bool,
    /// `false` fails any MPD-derived URL (BaseURL, segment, `UTCTiming`)
    /// whose origin differs from the MPD's, for server-side pulls.
    pub cross_origin_urls: bool,
}

impl Default for DashConfig {
    fn default() -> Self {
        Self {
            clock_policy: DashClockPolicy::Auto,
            utc_timeout: DEFAULT_UTC_TIMEOUT,
            utc_wait: DEFAULT_UTC_WAIT,
            utc_max_rtt: DEFAULT_UTC_MAX_RTT,
            utc_resync: Some(DEFAULT_UTC_RESYNC),
            host_clock_uncertainty: DEFAULT_HOST_CLOCK_UNCERTAINTY,
            live_delay: None,
            availability_margin: DEFAULT_AVAILABILITY_MARGIN,
            early_retry: DEFAULT_EARLY_RETRY,
            early_grace: DEFAULT_EARLY_GRACE,
            min_mpd_refresh: DEFAULT_MIN_MPD_REFRESH,
            use_availability_time_offset: false,
            cross_origin_urls: true,
        }
    }
}

impl DashConfig {
    /// Check the configuration.
    ///
    /// # Errors
    /// [`DashConfigError`] naming the first field a client cannot run with.
    pub fn validate(&self) -> Result<(), DashConfigError> {
        let invalid = |field| Err(DashConfigError { field });
        if self.utc_timeout.is_zero() {
            return invalid("utc_timeout");
        }
        if self.utc_wait < self.utc_timeout {
            return invalid("utc_wait");
        }
        if self.utc_max_rtt.is_zero() {
            return invalid("utc_max_rtt");
        }
        if self.utc_resync.is_some_and(|d| d.is_zero()) {
            return invalid("utc_resync");
        }
        if self.early_retry.is_zero() {
            return invalid("early_retry");
        }
        if self.min_mpd_refresh.is_zero() {
            return invalid("min_mpd_refresh");
        }
        Ok(())
    }
}

/// Default bound on one fetched resource body (init, index or segment).
pub const DEFAULT_MAX_SEGMENT_BYTES: usize = 64 * 1024 * 1024;
/// Default number of segment requests kept outstanding at once.
pub const DEFAULT_MAX_INFLIGHT_SEGMENTS: usize = 4;
/// Default consecutive failures of one segment before it is given up on.
pub const DEFAULT_MAX_SEGMENT_FAILURES: u32 = 8;
/// Default consecutive failures of one init or index resource before the
/// presentation is given up on.
pub const DEFAULT_MAX_INIT_FAILURES: u32 = 8;

/// A rejected [`PullLimits`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("invalid PullLimits: {field} must be non-zero")]
pub struct PullLimitsError {
    /// The offending field.
    pub field: &'static str,
}

/// The knobs every segment-pull source shares (DESIGN 6.4, "shared with
/// HLS"): the session owns them in its own config and passes them in; they
/// are deliberately not part of [`DashConfig`].
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct PullLimits {
    /// Representation and AdaptationSet selection policy.
    pub policy: crate::selection::Policy,
    /// MPD parse bounds; `limits.max_bytes` is the MPD body cap.
    pub limits: broadcast_dash::limits::Limits,
    /// Largest accepted init, index or segment body, bytes.
    pub max_segment_bytes: usize,
    /// Segment requests kept outstanding at once, counting bodies that have
    /// arrived but are not yet delivered; `0` behaves as `1`.
    pub max_inflight_segments: usize,
    /// Consecutive failures of one segment after which a static presentation
    /// ends with a fatal error; `0` makes the first failure final and
    /// `u32::MAX` retries forever (the `broadcast-client` rule).
    pub max_segment_failures: u32,
    /// The same for the init and index resources.
    pub max_init_failures: u32,
}

impl Default for PullLimits {
    fn default() -> Self {
        Self {
            policy: crate::selection::Policy::default(),
            limits: broadcast_dash::limits::Limits::default(),
            max_segment_bytes: DEFAULT_MAX_SEGMENT_BYTES,
            max_inflight_segments: DEFAULT_MAX_INFLIGHT_SEGMENTS,
            max_segment_failures: DEFAULT_MAX_SEGMENT_FAILURES,
            max_init_failures: DEFAULT_MAX_INIT_FAILURES,
        }
    }
}

impl PullLimits {
    /// Reject limits that cannot work.
    ///
    /// # Errors
    /// [`PullLimitsError`] naming the first zero size bound.
    pub fn validate(&self) -> Result<(), PullLimitsError> {
        if self.limits.max_bytes == 0 {
            return Err(PullLimitsError {
                field: "limits.max_bytes",
            });
        }
        if self.max_segment_bytes == 0 {
            return Err(PullLimitsError {
                field: "max_segment_bytes",
            });
        }
        Ok(())
    }

    pub(crate) fn window(&self) -> usize {
        self.max_inflight_segments.max(1)
    }
}
