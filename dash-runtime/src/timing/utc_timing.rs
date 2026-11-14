//! The client-measured clock sample and the `UTCTiming` readers over it.
//!
//! ISO/IEC 23009-1 5.8.5.7 Table 35 names the schemes; the sampling rule
//! (midpoint of the request/response exchange, uncertainty = half the round
//! trip plus the time source's own resolution) is
//! `docs/dash-runtime/DESIGN.md` 3.3. The caller measures `sent` and
//! `received` on its own monotonic clock; nothing here reads a clock.

use core::time::Duration;

pub use broadcast_dash::utc_timing::UtcScheme;
use broadcast_dash::xs_datetime::{XsDateTime, parse_ntp_timestamp, parse_xs_datetime};

use super::TimingError;
use broadcast_common::http_date::date_to_epoch_us;
use broadcast_common::stage::Timestamp;

/// 5.8.5.7: `urn:mpeg:dash:utc:ntp:2014`.
pub const SCHEME_NTP: &str = "urn:mpeg:dash:utc:ntp:2014";
/// 5.8.5.7: `urn:mpeg:dash:utc:sntp:2014`.
pub const SCHEME_SNTP: &str = "urn:mpeg:dash:utc:sntp:2014";
/// 5.8.5.7: `urn:mpeg:dash:utc:http-head:2014`.
pub const SCHEME_HTTP_HEAD: &str = "urn:mpeg:dash:utc:http-head:2014";
/// 5.8.5.7: `urn:mpeg:dash:utc:http-xsdate:2014`.
pub const SCHEME_HTTP_XSDATE: &str = "urn:mpeg:dash:utc:http-xsdate:2014";
/// 5.8.5.7: `urn:mpeg:dash:utc:http-iso:2014`.
pub const SCHEME_HTTP_ISO: &str = "urn:mpeg:dash:utc:http-iso:2014";
/// 5.8.5.7: `urn:mpeg:dash:utc:http-ntp:2014`.
pub const SCHEME_HTTP_NTP: &str = "urn:mpeg:dash:utc:http-ntp:2014";
/// 5.8.5.7: `urn:mpeg:dash:utc:direct:2014`.
pub const SCHEME_DIRECT: &str = "urn:mpeg:dash:utc:direct:2014";

/// Largest UTC response body read (DESIGN risk R3).
pub const UTC_BODY_MAX_BYTES: usize = 4 * 1024;
/// `Date` carries whole seconds (RFC 9110 5.6.7): the sample is taken at the
/// middle of the second and the half second is added to the uncertainty.
pub const DATE_HEADER_HALF_SECOND: Duration = Duration::from_millis(500);
/// Resolution of an NTP timestamp read over HTTP.
pub const HTTP_NTP_RESOLUTION: Duration = Duration::from_micros(1_000);

/// The request/response pair the caller timed on its monotonic clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Exchange {
    /// Monotonic microsecond count the request was emitted at.
    pub sent: Timestamp,
    /// Monotonic instant the first response byte arrived.
    pub received: Timestamp,
}

impl Exchange {
    /// A new exchange.
    #[must_use]
    pub fn new(sent: Timestamp, received: Timestamp) -> Self {
        Self { sent, received }
    }
}

/// One clock measurement: at monotonic `mono_us` the UTC time was `utc_us`
/// (Unix microseconds), give or take `uncertainty_us`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct UtcSample {
    /// Monotonic microseconds the sample refers to.
    pub mono_us: u64,
    /// Unix microseconds at that instant.
    pub utc_us: i64,
    /// Half-width of the error bar.
    pub uncertainty_us: u64,
}

/// Build a sample from a timed exchange and the server time it returned.
///
/// The server stamps the time near response generation, so the sample sits at
/// the exchange midpoint; the error bar is half the round trip plus the
/// source's own `resolution_us`.
///
/// # Errors
/// [`TimingError::ExchangeReversed`] when `received` precedes `sent`;
/// [`TimingError::RttTooLarge`] when the round trip exceeds `max_rtt`.
pub fn sample_from_exchange(
    exchange: Exchange,
    server_utc_us: i64,
    resolution: Duration,
    max_rtt: Duration,
) -> Result<UtcSample, TimingError> {
    if exchange.received < exchange.sent {
        return Err(TimingError::ExchangeReversed);
    }
    let rtt = exchange.received.saturating_sub(exchange.sent);
    if rtt > max_rtt {
        return Err(TimingError::RttTooLarge { rtt, max: max_rtt });
    }
    let half = rtt / 2;
    let mono = Timestamp::from_micros(exchange.sent.as_micros() + micros(half));
    Ok(UtcSample {
        mono_us: mono.as_micros(),
        utc_us: server_utc_us,
        uncertainty_us: micros(half.saturating_add(resolution)),
    })
}

fn micros(d: Duration) -> u64 {
    u64::try_from(d.as_micros()).unwrap_or(u64::MAX)
}

/// Sample from the body of an `http-xsdate`, `http-iso` or `http-ntp`
/// response.
///
/// # Errors
/// [`TimingError`] for an oversized or unreadable body, a scheme that has no
/// body time, or a bad exchange.
pub fn sample_from_body(
    scheme: UtcScheme,
    body: &[u8],
    exchange: Exchange,
    max_rtt: Duration,
) -> Result<UtcSample, TimingError> {
    if body.len() > UTC_BODY_MAX_BYTES {
        return Err(TimingError::BodyTooLarge {
            max: UTC_BODY_MAX_BYTES,
        });
    }
    match scheme {
        UtcScheme::HttpXsdate | UtcScheme::HttpIso => {
            let text = core::str::from_utf8(body).map_err(|_| TimingError::NotUtf8)?;
            sample_from_datetime(parse_xs_datetime(text)?, exchange, max_rtt)
        }
        UtcScheme::HttpNtp => {
            let utc = parse_ntp_timestamp(body)?;
            sample_from_exchange(exchange, utc, HTTP_NTP_RESOLUTION, max_rtt)
        }
        other => Err(TimingError::SchemeNotSampled(other)),
    }
}

/// Sample from the `Date` header of an `http-head` (or any) response.
///
/// # Errors
/// [`TimingError`] when the header is not a date or the exchange is bad.
pub fn sample_from_date_header(
    date: headers::Date,
    exchange: Exchange,
    max_rtt: Duration,
) -> Result<UtcSample, TimingError> {
    let seconds_us = date_to_epoch_us(date)?;
    let half_second = i64::try_from(DATE_HEADER_HALF_SECOND.as_micros()).unwrap_or(i64::MAX);
    sample_from_exchange(
        exchange,
        seconds_us.saturating_add(half_second),
        DATE_HEADER_HALF_SECOND,
        max_rtt,
    )
}

/// Sample from a `direct` scheme `@value`, taken at the midpoint of the MPD
/// response exchange.
///
/// # Errors
/// [`TimingError`] when the value is not an accepted `xs:dateTime` or the
/// exchange is bad.
pub fn sample_from_direct(
    value: &str,
    mpd_exchange: Exchange,
    max_rtt: Duration,
) -> Result<UtcSample, TimingError> {
    sample_from_datetime(parse_xs_datetime(value)?, mpd_exchange, max_rtt)
}

fn sample_from_datetime(
    dt: XsDateTime,
    exchange: Exchange,
    max_rtt: Duration,
) -> Result<UtcSample, TimingError> {
    if dt.fraction_digits > 0 {
        return sample_from_exchange(
            exchange,
            dt.unix_us,
            Duration::from_micros(dt.resolution_us()),
            max_rtt,
        );
    }
    let half_second = i64::try_from(DATE_HEADER_HALF_SECOND.as_micros()).unwrap_or(i64::MAX);
    sample_from_exchange(
        exchange,
        dt.unix_us.saturating_add(half_second),
        DATE_HEADER_HALF_SECOND,
        max_rtt,
    )
}
