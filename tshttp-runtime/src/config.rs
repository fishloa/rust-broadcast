use alloc::vec;

/// Session configuration (defaults per DESIGN.md).
///
/// [`Config::validate`] reports an inconsistent configuration;
/// [`TsHttpClient::new`](crate::TsHttpClient::new) never rejects one but
/// replaces each offending field with the nearest valid value (see
/// [`Config::clamped`]).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Config {
    /// Start timeout in milliseconds, measured from `Open` (T6, T20).
    pub start_timeout_ms: u64,
    /// Stall notice threshold in milliseconds (T11, T14, T15).
    pub stall_notice_ms: u64,
    /// Stall reconnect threshold in milliseconds (T12); must exceed
    /// `stall_notice_ms`.
    pub stall_reconnect_ms: u64,
    /// Backoff schedule in milliseconds (T9); must not be empty.
    pub backoff_ms: alloc::vec::Vec<u64>,
    /// Backoff cap in milliseconds (T9).
    pub backoff_cap_ms: u64,
    /// Minimum spacing between two consecutive `Open`s after a connection
    /// that played and then ended (T8); a floor only when that connection
    /// lasted less than this interval, so a long-lived stream still
    /// reconnects with no delay.
    pub min_reconnect_interval_ms: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            start_timeout_ms: 15_000,
            stall_notice_ms: 5_000,
            stall_reconnect_ms: 30_000,
            backoff_ms: vec![2_000, 4_000, 8_000],
            backoff_cap_ms: 10_000,
            min_reconnect_interval_ms: 1_000,
        }
    }
}

/// Reason a [`Config`] is rejected by [`Config::validate`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum ConfigError {
    /// `start_timeout_ms` is zero.
    ZeroStartTimeout,
    /// `stall_notice_ms` is zero.
    ZeroStallNotice,
    /// `stall_reconnect_ms` is zero.
    ZeroStallReconnect,
    /// `stall_reconnect_ms` is not greater than `stall_notice_ms`.
    StallReconnectNotAfterNotice,
    /// `backoff_ms` is empty.
    EmptyBackoff,
}

impl ConfigError {
    /// Label for this error.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::ZeroStartTimeout => "start_timeout_ms is zero",
            Self::ZeroStallNotice => "stall_notice_ms is zero",
            Self::ZeroStallReconnect => "stall_reconnect_ms is zero",
            Self::StallReconnectNotAfterNotice => {
                "stall_reconnect_ms is not greater than stall_notice_ms"
            }
            Self::EmptyBackoff => "backoff_ms is empty",
        }
    }
}

broadcast_common::impl_spec_display!(ConfigError);

impl core::error::Error for ConfigError {}

impl Config {
    /// Check the configuration for values that would stall or spin the
    /// session.
    ///
    /// # Errors
    /// The first violated rule, in field order.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.start_timeout_ms == 0 {
            return Err(ConfigError::ZeroStartTimeout);
        }
        if self.stall_notice_ms == 0 {
            return Err(ConfigError::ZeroStallNotice);
        }
        if self.stall_reconnect_ms == 0 {
            return Err(ConfigError::ZeroStallReconnect);
        }
        if self.stall_reconnect_ms <= self.stall_notice_ms {
            return Err(ConfigError::StallReconnectNotAfterNotice);
        }
        if self.backoff_ms.is_empty() {
            return Err(ConfigError::EmptyBackoff);
        }
        Ok(())
    }

    /// The nearest configuration that passes [`Config::validate`]: zero
    /// timeouts become 1 ms, a `stall_reconnect_ms` not after the notice is
    /// moved to one millisecond after it, an empty `backoff_ms` becomes the
    /// single entry `backoff_cap_ms` (at least 1 ms). A valid configuration
    /// is returned unchanged.
    #[must_use]
    pub fn clamped(&self) -> Self {
        let mut c = self.clone();
        c.start_timeout_ms = c.start_timeout_ms.max(1);
        c.stall_notice_ms = c.stall_notice_ms.max(1);
        c.stall_reconnect_ms = c
            .stall_reconnect_ms
            .max(c.stall_notice_ms.saturating_add(1));
        if c.backoff_ms.is_empty() {
            c.backoff_ms = vec![c.backoff_cap_ms.max(1)];
        }
        c
    }
}
