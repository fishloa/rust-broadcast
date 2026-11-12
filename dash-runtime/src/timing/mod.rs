//! Pure time arithmetic: UTC clock, HTTP `Date`, availability, live edge and
//! PeriodStart. Nothing in this module reads a clock; every function takes the
//! instants it needs as arguments. The `xs:dateTime`, NTP and `UTCTiming`
//! readers live in [`broadcast_dash::xs_datetime`] and
//! [`broadcast_dash::utc_timing`] and are re-exported here.

use thiserror::Error;

pub mod availability;
pub mod clock;
pub use broadcast_common::http_date;
pub mod live_edge;
pub mod period;
pub mod utc_timing;

pub use broadcast_dash::xs_datetime as datetime;

/// Why a clock sample could not be built.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum TimingError {
    /// The response arrived before the request was sent.
    #[error("response timestamp precedes the request timestamp")]
    ExchangeReversed,
    /// The round trip is too long to trust.
    #[error("round trip of {rtt_us} us exceeds the {max_us} us limit")]
    RttTooLarge {
        /// Measured round trip.
        rtt_us: u64,
        /// Limit.
        max_us: u64,
    },
    /// A UTC response body above the cap.
    #[error("UTC response body exceeds {max} bytes")]
    BodyTooLarge {
        /// The cap.
        max: usize,
    },
    /// A UTC response body that is not UTF-8.
    #[error("UTC response body is not UTF-8")]
    NotUtf8,
    /// The scheme has no body time this core can read.
    #[error("UTCTiming scheme {0} is not sampled from a response body")]
    SchemeNotSampled(utc_timing::UtcScheme),
    /// An `xs:dateTime` was not accepted.
    #[error(transparent)]
    DateTime(#[from] datetime::DateTimeError),
    /// An HTTP `Date` value was not accepted.
    #[error(transparent)]
    HttpDate(#[from] http_date::HttpDateError),
}
