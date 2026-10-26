//! `DashConfig`: the DASH-specific knobs of a client session.
//!
//! `docs/dash-runtime/DESIGN.md` 6.4 (fields and defaults), 3.2-3.4 (clock
//! ladder), 5.2 (refresh floor), 5.4 (early-request cadence), D9 (LL-DASH) and
//! 10.2 R3 (cross-origin restriction). Pure data: nothing here reads a clock.
//! Every duration is in microseconds. A value of `u64::MAX` is accepted
//! wherever zero is not: consumers combine these with saturating arithmetic,
//! so `MAX` reads as "never" or "unbounded" rather than overflowing.

use broadcast_common::impl_spec_display;
use thiserror::Error;

const MICROS_PER_MILLI: u64 = 1_000;
const MICROS_PER_SECOND: u64 = 1_000 * MICROS_PER_MILLI;

/// Default time one `UTCTiming` request may take.
pub const DEFAULT_UTC_TIMEOUT_US: u64 = 2 * MICROS_PER_SECOND;
/// Default total wait for a `UTCTiming` sample before the clock ladder falls
/// through to the next source.
pub const DEFAULT_UTC_WAIT_US: u64 = 5 * MICROS_PER_SECOND;
/// Default longest round trip a `UTCTiming` sample may have and still be used.
pub const DEFAULT_UTC_MAX_RTT_US: u64 = 2 * MICROS_PER_SECOND;
/// Default age after which a same-source clock anchor is replaced.
pub const DEFAULT_UTC_RESYNC_US: u64 = 600 * MICROS_PER_SECOND;
/// Default error bar reported for a host-supplied clock.
pub const DEFAULT_HOST_CLOCK_UNCERTAINTY_US: u64 = 500 * MICROS_PER_MILLI;
/// Default margin added to a segment's availability start before requesting it.
pub const DEFAULT_AVAILABILITY_MARGIN_US: u64 = 0;
/// Default first retry delay for a segment requested before it exists.
pub const DEFAULT_EARLY_RETRY_US: u64 = 250 * MICROS_PER_MILLI;
/// Default window after a segment's availability start in which a 404, 410 or
/// 425 still means "too early" rather than "missing".
pub const DEFAULT_EARLY_GRACE_US: u64 = MICROS_PER_SECOND;
/// Default floor under the MPD refresh interval.
pub const DEFAULT_MIN_MPD_REFRESH_US: u64 = MICROS_PER_SECOND;

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
    /// Time one `UTCTiming` request may take, microseconds; must be non-zero.
    pub utc_timeout_us: u64,
    /// Total wait for a `UTCTiming` sample before the ladder falls through,
    /// microseconds; must be non-zero and not shorter than
    /// [`DashConfig::utc_timeout_us`] (a wait that ends before one request can
    /// time out would never let the first request finish).
    pub utc_wait_us: u64,
    /// Longest round trip a `UTCTiming` sample may have, microseconds; a
    /// slower exchange is discarded. Must be non-zero.
    pub utc_max_rtt_us: u64,
    /// Age after which a same-source clock anchor is replaced by any new
    /// sample of that source, microseconds; must be non-zero. `u64::MAX`
    /// never re-syncs on age.
    pub utc_resync_us: u64,
    /// Error bar reported with a clock taken from the host, microseconds.
    /// `0` claims an exact host clock.
    pub host_clock_uncertainty_us: u64,
    /// Override of the presentation delay a live join keeps behind the live
    /// edge, microseconds. `None` derives it from the MPD. `Some(0)` joins at
    /// the edge.
    pub live_delay_us: Option<u64>,
    /// Added to a segment's availability start before it is requested,
    /// microseconds. `0` requests at the availability start.
    pub availability_margin_us: u64,
    /// First retry delay for a segment requested before it exists,
    /// microseconds; must be non-zero. The delay doubles up to one second.
    pub early_retry_us: u64,
    /// Window after a segment's availability start in which a 404, 410 or
    /// 425 still means "too early", microseconds. `0` treats every such
    /// response after the start as a missing segment.
    pub early_grace_us: u64,
    /// Floor under the MPD refresh interval, microseconds; must be non-zero
    /// so `minimumUpdatePeriod="PT0S"` cannot become a request storm.
    pub min_mpd_refresh_us: u64,
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
            utc_timeout_us: DEFAULT_UTC_TIMEOUT_US,
            utc_wait_us: DEFAULT_UTC_WAIT_US,
            utc_max_rtt_us: DEFAULT_UTC_MAX_RTT_US,
            utc_resync_us: DEFAULT_UTC_RESYNC_US,
            host_clock_uncertainty_us: DEFAULT_HOST_CLOCK_UNCERTAINTY_US,
            live_delay_us: None,
            availability_margin_us: DEFAULT_AVAILABILITY_MARGIN_US,
            early_retry_us: DEFAULT_EARLY_RETRY_US,
            early_grace_us: DEFAULT_EARLY_GRACE_US,
            min_mpd_refresh_us: DEFAULT_MIN_MPD_REFRESH_US,
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
        if self.utc_timeout_us == 0 {
            return invalid("utc_timeout_us");
        }
        if self.utc_wait_us == 0 || self.utc_wait_us < self.utc_timeout_us {
            return invalid("utc_wait_us");
        }
        if self.utc_max_rtt_us == 0 {
            return invalid("utc_max_rtt_us");
        }
        if self.utc_resync_us == 0 {
            return invalid("utc_resync_us");
        }
        if self.early_retry_us == 0 {
            return invalid("early_retry_us");
        }
        if self.min_mpd_refresh_us == 0 {
            return invalid("min_mpd_refresh_us");
        }
        Ok(())
    }
}
