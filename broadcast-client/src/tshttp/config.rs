use alloc::vec;
use core::num::NonZeroUsize;
use core::time::Duration;

use broadcast_common::backoff::{BackoffSteps, MAX_FLOOR};
use broadcast_common::timeouts::Timeouts;

use crate::events::ReadAhead;

/// Session configuration (defaults per DESIGN.md).
///
/// [`Config::validate`] reports an inconsistent configuration;
/// The session (`crate::Session::open`) never rejects one but
/// replaces each offending field with the nearest valid value (see
/// [`Config::clamped`]).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub struct Config {
    /// Start timeout, measured from `Open` (T6, T20).
    pub start_timeout: Duration,
    /// Stall notice threshold (T11, T14, T15).
    pub stall_notice: Duration,
    /// Stall reconnect threshold (T12); must exceed
    /// `stall_notice`.
    pub stall_reconnect: Duration,
    /// Backoff schedule (T9); the step table must not be empty.
    pub backoff: BackoffSteps,
    /// Minimum spacing between two consecutive `Open`s after a connection
    /// that played and then ended (T8); a floor only when that connection
    /// lasted less than this interval, so a long-lived stream still
    /// reconnects with no delay.
    pub min_reconnect_interval: Duration,
    /// Optional read-ahead limit, shared by every pull source (TS-over-HTTP,
    /// HLS, DASH, Smooth, RTSP over interleaved TCP): how far media may run
    /// ahead of the playback position before the session stops pulling it. The
    /// default is [`ReadAhead::Unlimited`], which never paces; the library
    /// chooses no numbers. It lives here because this configuration carries the
    /// knobs every pull source shares.
    pub read_ahead: ReadAhead,
    /// Caller-supplied ceiling on one PID's in-progress PES buffer, applied to
    /// every TS-carrying source (TS-over-HTTP, SRT, UDP) through the TS demux.
    /// `None` (the default) is unlimited: the library picks no bound. A caller
    /// whose input is a network stream supplies one and gets the demux's
    /// `DiscontinuityKind::BudgetExceeded` event for an access unit above it.
    /// It lives here because this configuration carries the knobs every pull
    /// source shares.
    pub max_pes_unit_bytes: Option<NonZeroUsize>,
    /// Failed attempts in a row, with no media played in between, after which the session gives up
    /// with one fatal `Event::Error` (`ErrorKind::ReconnectAttemptsExhausted`) instead of retrying.
    /// `None` (the default) retries forever, which is what a live stream wants: the backoff bounds
    /// the rate, never the number of attempts. A connection that played before it ended does not
    /// count, so a healthy stream that reconnects is never given up on.
    pub max_reconnect_attempts: Option<u32>,
}

/// Smallest start timeout [`Config::with_timeouts`] produces.
/// Default start timeout: how long a pull waits for the first byte.
const DEFAULT_START_TIMEOUT: Duration = Duration::from_millis(15_000);
/// Default stall notice: how long without progress before the stall event.
const DEFAULT_STALL_NOTICE: Duration = Duration::from_millis(5_000);
/// Default stall reconnect: how long a stalled pull waits before reconnecting.
const DEFAULT_STALL_RECONNECT: Duration = Duration::from_secs(30);
/// Default backoff steps.
const DEFAULT_BACKOFF_STEPS: [Duration; 3] = [
    Duration::from_millis(2_000),
    Duration::from_millis(4_000),
    Duration::from_millis(8_000),
];
/// Default backoff cap.
const DEFAULT_BACKOFF_CAP: Duration = Duration::from_millis(10_000);
/// Default minimum interval between reconnect attempts.
const DEFAULT_MIN_RECONNECT_INTERVAL: Duration = Duration::from_millis(1_000);

const MIN_START_TIMEOUT: Duration = Duration::from_millis(1);
/// Smallest stall-reconnect threshold [`Config::with_timeouts`] produces; the
/// notice threshold is half of it, so the two stay ordered.
const MIN_STALL_RECONNECT: Duration = Duration::from_millis(2);
const STALL_NOTICE_DIVISOR: u32 = 2;

impl Config {
    /// Take the start timeout from `timeouts.connect` (the whole first-byte
    /// budget, bounding a connection that sends bytes but no media) and the
    /// stall-reconnect threshold from `timeouts.read_idle`, with the notice
    /// threshold at half of it. Floors: 1 ms start, 2 ms reconnect. The backoff
    /// fields are left as they are.
    #[must_use]
    pub fn with_timeouts(mut self, timeouts: Timeouts) -> Self {
        let reconnect = timeouts.read_idle.max(MIN_STALL_RECONNECT);
        self.start_timeout = timeouts.connect.max(MIN_START_TIMEOUT);
        self.stall_notice = reconnect / STALL_NOTICE_DIVISOR;
        self.stall_reconnect = reconnect;
        self
    }

    /// Set the start timeout (`T6`, `T20`).
    #[must_use]
    pub fn with_start_timeout(mut self, start_timeout: Duration) -> Self {
        self.start_timeout = start_timeout;
        self
    }

    /// Set the stall notice threshold (`T11`, `T14`, `T15`).
    #[must_use]
    pub fn with_stall_notice(mut self, stall_notice: Duration) -> Self {
        self.stall_notice = stall_notice;
        self
    }

    /// Set the stall reconnect threshold (`T12`).
    #[must_use]
    pub fn with_stall_reconnect(mut self, stall_reconnect: Duration) -> Self {
        self.stall_reconnect = stall_reconnect;
        self
    }

    /// Set the backoff schedule (`T9`).
    #[must_use]
    pub fn with_backoff(mut self, backoff: BackoffSteps) -> Self {
        self.backoff = backoff;
        self
    }

    /// Set the minimum reconnect interval (`T8`).
    #[must_use]
    pub fn with_min_reconnect_interval(mut self, min_reconnect_interval: Duration) -> Self {
        self.min_reconnect_interval = min_reconnect_interval;
        self
    }

    /// Set the read-ahead limit ([`ReadAhead`]), which every pull source
    /// honours. `ReadAhead::Unlimited` (the default) never paces a source.
    #[must_use]
    pub fn with_read_ahead(mut self, read_ahead: ReadAhead) -> Self {
        self.read_ahead = read_ahead;
        self
    }

    /// Set the give-up ceiling on failed attempts in a row.
    #[must_use]
    pub fn with_max_reconnect_attempts(mut self, max_reconnect_attempts: Option<u32>) -> Self {
        self.max_reconnect_attempts = max_reconnect_attempts;
        self
    }

    /// Set the caller-supplied PES unit ceiling; `None` (the default) is
    /// unlimited.
    #[must_use]
    pub fn with_max_pes_unit_bytes(mut self, max_pes_unit_bytes: Option<NonZeroUsize>) -> Self {
        self.max_pes_unit_bytes = max_pes_unit_bytes;
        self
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            start_timeout: DEFAULT_START_TIMEOUT,
            stall_notice: DEFAULT_STALL_NOTICE,
            stall_reconnect: DEFAULT_STALL_RECONNECT,
            backoff: BackoffSteps {
                steps: DEFAULT_BACKOFF_STEPS.to_vec(),
                cap: DEFAULT_BACKOFF_CAP,
            },
            min_reconnect_interval: DEFAULT_MIN_RECONNECT_INTERVAL,
            read_ahead: ReadAhead::Unlimited,
            max_pes_unit_bytes: None,
            max_reconnect_attempts: None,
        }
    }
}

/// Reason a [`Config`] is rejected by [`Config::validate`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum ConfigError {
    /// `start_timeout` is zero.
    ZeroStartTimeout,
    /// `stall_notice` is zero.
    ZeroStallNotice,
    /// `stall_reconnect` is zero.
    ZeroStallReconnect,
    /// `stall_reconnect` is not greater than `stall_notice`.
    StallReconnectNotAfterNotice,
    /// `backoff.steps` is empty.
    EmptyBackoff,
    /// A `backoff.steps` entry is zero.
    ZeroBackoffEntry,
}

impl ConfigError {
    /// Label for this error.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::ZeroStartTimeout => "start_timeout is zero",
            Self::ZeroStallNotice => "stall_notice is zero",
            Self::ZeroStallReconnect => "stall_reconnect is zero",
            Self::StallReconnectNotAfterNotice => {
                "stall_reconnect is not greater than stall_notice"
            }
            Self::EmptyBackoff => "backoff.steps is empty",
            Self::ZeroBackoffEntry => "backoff.steps has a zero entry",
        }
    }
}

broadcast_core::impl_spec_display!(ConfigError);

impl core::error::Error for ConfigError {}

impl Config {
    /// Check the configuration for values that would stall or spin the
    /// session.
    ///
    /// # Errors
    /// The first violated rule, in field order.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.start_timeout.is_zero() {
            return Err(ConfigError::ZeroStartTimeout);
        }
        if self.stall_notice.is_zero() {
            return Err(ConfigError::ZeroStallNotice);
        }
        if self.stall_reconnect.is_zero() {
            return Err(ConfigError::ZeroStallReconnect);
        }
        if self.stall_reconnect <= self.stall_notice {
            return Err(ConfigError::StallReconnectNotAfterNotice);
        }
        if self.backoff.steps.is_empty() {
            return Err(ConfigError::EmptyBackoff);
        }
        if self.backoff.steps.contains(&Duration::ZERO) {
            return Err(ConfigError::ZeroBackoffEntry);
        }
        Ok(())
    }

    /// The nearest configuration that passes [`Config::validate`]: a
    /// `backoff.cap` above [`MAX_FLOOR`] (the largest retry delay) becomes it,
    /// zero timeouts become 1 ms, a `stall_reconnect` not after the notice is
    /// moved to one millisecond after it, an empty `backoff.steps` becomes the
    /// single entry `backoff.cap` (at least 1 ms), a zero `backoff.steps`
    /// entry becomes 1 ms. A valid configuration is returned unchanged.
    #[must_use]
    pub fn clamped(&self) -> Self {
        let mut c = self.clone();
        c.backoff.cap = c.backoff.cap.min(MAX_FLOOR);
        c.start_timeout = c.start_timeout.max(MIN_START_TIMEOUT);
        c.stall_notice = c.stall_notice.max(MIN_START_TIMEOUT);
        c.stall_reconnect = c
            .stall_reconnect
            .max(c.stall_notice.saturating_add(MIN_START_TIMEOUT));
        if c.backoff.steps.is_empty() {
            c.backoff.steps = vec![c.backoff.cap.max(MIN_START_TIMEOUT)];
        }
        for entry in &mut c.backoff.steps {
            *entry = (*entry).max(MIN_START_TIMEOUT);
        }
        c
    }
}
