//! `UTCTiming` scheme policy (ISO/IEC 23009-1 5.8.5.7 Table 35): which
//! schemes a sans-IO client may act on, which need an HTTP request, their
//! labels, and the `@value` list.

use broadcast_dash::model::UtcTiming;
use broadcast_dash::utc_timing::{
    SCHEME_DIRECT, SCHEME_HTTP_HEAD, SCHEME_HTTP_ISO, SCHEME_HTTP_NTP, SCHEME_HTTP_XSDATE,
    SCHEME_NTP, SCHEME_SNTP, UtcScheme,
};

fn timing(value: &str) -> UtcTiming {
    let mut t = UtcTiming::default();
    t.value = value.to_string();
    t
}

#[test]
fn every_table_35_scheme_states_what_a_client_may_do() {
    let table = [
        (SCHEME_NTP, UtcScheme::Ntp, false, false, "ntp"),
        (SCHEME_SNTP, UtcScheme::Sntp, false, false, "sntp"),
        (
            SCHEME_HTTP_HEAD,
            UtcScheme::HttpHead,
            true,
            true,
            "http_head",
        ),
        (
            SCHEME_HTTP_XSDATE,
            UtcScheme::HttpXsdate,
            true,
            true,
            "http_xsdate",
        ),
        (SCHEME_HTTP_ISO, UtcScheme::HttpIso, true, true, "http_iso"),
        (SCHEME_HTTP_NTP, UtcScheme::HttpNtp, true, true, "http_ntp"),
        (SCHEME_DIRECT, UtcScheme::Direct, true, false, "direct"),
        (
            "urn:example:utc",
            UtcScheme::Unknown,
            false,
            false,
            "unknown",
        ),
    ];
    assert_eq!(table.len(), 8);
    for (uri, scheme, actionable, request, name) in table {
        assert_eq!(UtcScheme::from_scheme_id_uri(uri), scheme, "{uri}");
        assert_eq!(scheme.is_actionable(), actionable, "{name}");
        assert_eq!(scheme.needs_request(), request, "{name}");
        assert_eq!(scheme.name(), name);
        assert_eq!(scheme.to_string(), name);
    }
}

#[test]
fn utc_value_lists_its_whitespace_separated_entries() {
    assert_eq!(
        timing("a b  c").values().collect::<Vec<_>>(),
        vec!["a", "b", "c"]
    );
    assert_eq!(timing("").values().count(), 0);
    assert_eq!(timing("   \t ").values().count(), 0);
    assert_eq!(
        timing("http://a/1\t\nhttp://b/2")
            .values()
            .collect::<Vec<_>>(),
        vec!["http://a/1", "http://b/2"]
    );
    assert_eq!(
        timing("2024-01-01T00:00:00Z").values().collect::<Vec<_>>(),
        vec!["2024-01-01T00:00:00Z"]
    );
}
