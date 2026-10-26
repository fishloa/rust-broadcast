//! [`DashConfig`]: the knobs of [`super::DashClient`].
//!
//! Pure data, like `broadcast-client`'s `Config`; the session maps its own
//! knobs onto it (`docs/dash-runtime/DESIGN.md` section 6.4).

use thiserror::Error;

use crate::limits::Limits;
use crate::selection::Policy;

/// Default bound on one fetched resource body (init, index or segment).
pub const DEFAULT_MAX_SEGMENT_BYTES: usize = 64 * 1024 * 1024;
/// Default number of segment requests kept outstanding at once.
pub const DEFAULT_MAX_INFLIGHT_SEGMENTS: usize = 4;
/// Default consecutive failures of one segment before it is given up on.
pub const DEFAULT_MAX_SEGMENT_FAILURES: u32 = 8;
/// Default consecutive failures of one init or index resource before the
/// presentation is given up on.
pub const DEFAULT_MAX_INIT_FAILURES: u32 = 8;

/// A rejected [`DashConfig`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
#[error("invalid DashConfig: {field} must be non-zero")]
pub struct ConfigError {
    /// The offending field.
    pub field: &'static str,
}

/// Configuration of a [`super::DashClient`].
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct DashConfig {
    /// Representation and AdaptationSet selection policy.
    pub policy: Policy,
    /// MPD parse bounds; `limits.max_bytes` is the MPD body cap.
    pub limits: Limits,
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
    /// `false`: every URL derived from the MPD (BaseURL, init, index,
    /// segment) must have the origin of the MPD itself, else it is refused as
    /// a typed cross-origin error (DESIGN R3, for server-side pulls).
    pub cross_origin_urls: bool,
}

impl Default for DashConfig {
    fn default() -> Self {
        Self {
            policy: Policy::default(),
            limits: Limits::default(),
            max_segment_bytes: DEFAULT_MAX_SEGMENT_BYTES,
            max_inflight_segments: DEFAULT_MAX_INFLIGHT_SEGMENTS,
            max_segment_failures: DEFAULT_MAX_SEGMENT_FAILURES,
            max_init_failures: DEFAULT_MAX_INIT_FAILURES,
            cross_origin_urls: true,
        }
    }
}

impl DashConfig {
    /// Reject a configuration that cannot work.
    ///
    /// # Errors
    /// [`ConfigError`] naming the first zero size bound.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.limits.max_bytes == 0 {
            return Err(ConfigError {
                field: "limits.max_bytes",
            });
        }
        if self.max_segment_bytes == 0 {
            return Err(ConfigError {
                field: "max_segment_bytes",
            });
        }
        Ok(())
    }

    pub(crate) fn window(&self) -> usize {
        self.max_inflight_segments.max(1)
    }
}
