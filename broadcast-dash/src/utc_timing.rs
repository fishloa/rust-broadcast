//! `UTCTiming` scheme identity (ISO/IEC 23009-1 5.8.5.7 Table 35).
//!
//! The manifest states how a player may learn wall-clock time; acting on it is
//! the client's business, so only the scheme vocabulary lives here.

use broadcast_common::impl_spec_display;

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schemes_round_trip_through_their_uri() {
        for (uri, scheme) in [
            (SCHEME_NTP, UtcScheme::Ntp),
            (SCHEME_SNTP, UtcScheme::Sntp),
            (SCHEME_HTTP_HEAD, UtcScheme::HttpHead),
            (SCHEME_HTTP_XSDATE, UtcScheme::HttpXsdate),
            (SCHEME_HTTP_ISO, UtcScheme::HttpIso),
            (SCHEME_HTTP_NTP, UtcScheme::HttpNtp),
            (SCHEME_DIRECT, UtcScheme::Direct),
            ("urn:example:utc", UtcScheme::Unknown),
        ] {
            assert_eq!(UtcScheme::from_scheme_id_uri(uri), scheme, "{uri}");
        }
    }
}
