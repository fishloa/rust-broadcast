//! `UTCTiming` schemes and the client-measured clock sample.
//!
//! ISO/IEC 23009-1 5.8.5.7 Table 35 names the schemes; the sampling rule
//! (midpoint of the request/response exchange, uncertainty = half the round
//! trip plus the time source's own resolution) is
//! `docs/dash-runtime/DESIGN.md` 3.3. The caller measures `sent_us` and
//! `received_us` on its own monotonic clock; nothing here reads a clock.

use broadcast_common::impl_spec_display;

use super::TimingError;
use super::datetime::{parse_http_date, parse_ntp_timestamp, parse_xs_datetime};

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
pub const DATE_HEADER_HALF_SECOND_US: u64 = 500_000;
/// Resolution of an NTP timestamp read over HTTP, in microseconds.
pub const HTTP_NTP_RESOLUTION_US: u64 = 1_000;
const MICROS_PER_SECOND: i64 = 1_000_000;

/// A `UTCTiming` scheme (Table 35).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum UtcScheme {
    /// NTP servers; not acted on (no UDP in a sans-IO core).
    Ntp,
    /// SNTP servers; not acted on.
    Sntp,
    /// `HEAD`, read the `Date` header.
    HttpHead,
    /// `GET`, body is an `xs:dateTime`.
    HttpXsdate,
    /// `GET`, body is an ISO 8601 time.
    HttpIso,
    /// `GET`, body is an NTP timestamp.
    HttpNtp,
    /// `@value` is the time.
    Direct,
    /// A scheme this core does not know.
    Unknown,
}

impl UtcScheme {
    /// Classify a `schemeIdUri`.
    #[must_use]
    pub fn from_scheme_id_uri(uri: &str) -> Self {
        match uri {
            SCHEME_NTP => Self::Ntp,
            SCHEME_SNTP => Self::Sntp,
            SCHEME_HTTP_HEAD => Self::HttpHead,
            SCHEME_HTTP_XSDATE => Self::HttpXsdate,
            SCHEME_HTTP_ISO => Self::HttpIso,
            SCHEME_HTTP_NTP => Self::HttpNtp,
            SCHEME_DIRECT => Self::Direct,
            _ => Self::Unknown,
        }
    }

    /// Stable snake_case label.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Ntp => "ntp",
            Self::Sntp => "sntp",
            Self::HttpHead => "http_head",
            Self::HttpXsdate => "http_xsdate",
            Self::HttpIso => "http_iso",
            Self::HttpNtp => "http_ntp",
            Self::Direct => "direct",
            Self::Unknown => "unknown",
        }
    }

    /// Whether the client can act on the scheme without UDP.
    #[must_use]
    pub fn is_actionable(&self) -> bool {
        !matches!(self, Self::Ntp | Self::Sntp | Self::Unknown)
    }

    /// Whether the scheme needs an HTTP request (`@value` lists URLs).
    #[must_use]
    pub fn needs_request(&self) -> bool {
        matches!(
            self,
            Self::HttpHead | Self::HttpXsdate | Self::HttpIso | Self::HttpNtp
        )
    }
}
impl_spec_display!(UtcScheme);

/// The request/response pair the caller timed on its monotonic clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Exchange {
    /// Monotonic microseconds when the request was emitted.
    pub sent_us: u64,
    /// Monotonic microseconds when the first response byte arrived.
    pub received_us: u64,
}

impl Exchange {
    /// A new exchange.
    #[must_use]
    pub fn new(sent_us: u64, received_us: u64) -> Self {
        Self {
            sent_us,
            received_us,
        }
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
/// [`TimingError::ExchangeReversed`] when `received_us < sent_us`;
/// [`TimingError::RttTooLarge`] when the round trip exceeds `max_rtt_us`.
pub fn sample_from_exchange(
    exchange: Exchange,
    server_utc_us: i64,
    resolution_us: u64,
    max_rtt_us: u64,
) -> Result<UtcSample, TimingError> {
    let rtt = exchange
        .received_us
        .checked_sub(exchange.sent_us)
        .ok_or(TimingError::ExchangeReversed)?;
    if rtt > max_rtt_us {
        return Err(TimingError::RttTooLarge {
            rtt_us: rtt,
            max_us: max_rtt_us,
        });
    }
    let half = rtt / 2;
    Ok(UtcSample {
        mono_us: exchange.sent_us + half,
        utc_us: server_utc_us,
        uncertainty_us: half.saturating_add(resolution_us),
    })
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
    max_rtt_us: u64,
) -> Result<UtcSample, TimingError> {
    if body.len() > UTC_BODY_MAX_BYTES {
        return Err(TimingError::BodyTooLarge {
            max: UTC_BODY_MAX_BYTES,
        });
    }
    match scheme {
        UtcScheme::HttpXsdate | UtcScheme::HttpIso => {
            let text = core::str::from_utf8(body).map_err(|_| TimingError::NotUtf8)?;
            let dt = parse_xs_datetime(text)?;
            sample_from_exchange(exchange, dt.unix_us, dt.resolution_us(), max_rtt_us)
        }
        UtcScheme::HttpNtp => {
            let utc = parse_ntp_timestamp(body)?;
            sample_from_exchange(exchange, utc, HTTP_NTP_RESOLUTION_US, max_rtt_us)
        }
        other => Err(TimingError::SchemeNotSampled(other)),
    }
}

/// Sample from the `Date` header of an `http-head` (or any) response.
///
/// # Errors
/// [`TimingError`] when the header is not a date or the exchange is bad.
pub fn sample_from_date_header(
    date: &str,
    exchange: Exchange,
    max_rtt_us: u64,
) -> Result<UtcSample, TimingError> {
    let seconds_us = parse_http_date(date)?;
    let half_second = i64::try_from(DATE_HEADER_HALF_SECOND_US).unwrap_or(MICROS_PER_SECOND / 2);
    sample_from_exchange(
        exchange,
        seconds_us.saturating_add(half_second),
        DATE_HEADER_HALF_SECOND_US,
        max_rtt_us,
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
    max_rtt_us: u64,
) -> Result<UtcSample, TimingError> {
    let dt = parse_xs_datetime(value)?;
    sample_from_exchange(mpd_exchange, dt.unix_us, dt.resolution_us(), max_rtt_us)
}
