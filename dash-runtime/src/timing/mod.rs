//! Pure time arithmetic: UTC clock, `xs:dateTime`, UTCTiming, availability,
//! live edge and PeriodStart. Nothing in this module reads a clock; every
//! function takes the instants it needs as arguments.

use thiserror::Error;

pub mod clock;
pub mod datetime;
pub mod utc_timing;

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
    /// The time text was not accepted.
    #[error(transparent)]
    DateTime(#[from] datetime::DateTimeError),
}
