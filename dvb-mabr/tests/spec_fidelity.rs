//! Regression tests for the W7 audit findings (issue #1121): namespace
//! matching, the per-`schemaVersion` output namespace, non-finite/
//! out-of-range `proportion`, and text truncated at a comment/CDATA
//! boundary. Hand-built minimal documents (not the committed fixtures,
//! which are all `schemaVersion="2"` and have no foreign-namespace content)
//! — this is a text/XML config format, not a binary wire format, so a
//! hand-built document citing the exact spec clause is this crate's normal
//! testing approach (see `tests/round_trip.rs`'s own doc comment).

use dvb_mabr::{Error, MulticastGatewayConfiguration};

const NS_2024: &str = "urn:dvb:metadata:MulticastSessionConfiguration:2024";
const NS_2019: &str = "urn:dvb:metadata:MulticastSessionConfiguration:2019";

/// MABR-W1 (audit issue #1121): a `MulticastGatewaySessionReporting` element
/// in a foreign namespace, reusing the baseline local name, must be skipped
/// (Annex A.1) — not matched as the real element. Observed pre-fix: `child`/
/// `children` matched by local name alone, so `parsed.reporting` was `Some`
/// with the foreign element's own (also wrongly-matched) `ReportingLocator`
/// child.
#[test]
fn foreign_namespace_element_reusing_a_baseline_name_is_skipped() {
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<MulticastGatewayConfiguration xmlns="{NS_2024}" schemaVersion="2">
  <foo:MulticastGatewaySessionReporting xmlns:foo="urn:example:vendor">
    <foo:ReportingLocator period="PT0S" randomDelay="0">https://example.com</foo:ReportingLocator>
  </foo:MulticastGatewaySessionReporting>
</MulticastGatewayConfiguration>
"#
    );
    let parsed = MulticastGatewayConfiguration::parse_str(&xml).expect("parses");
    assert!(
        parsed.reporting.is_none(),
        "a MulticastGatewaySessionReporting in a foreign namespace must be skipped, not matched"
    );
}

/// MABR-W1 (audit issue #1121): the root element's namespace was never
/// checked, only its local name. Observed pre-fix: a root with the right
/// local name but a bogus namespace still parsed `Ok`.
#[test]
fn root_element_in_the_wrong_namespace_is_rejected() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<MulticastGatewayConfiguration xmlns="urn:example:not-a-real-namespace" schemaVersion="2"/>
"#;
    let err = MulticastGatewayConfiguration::parse_str(xml).unwrap_err();
    assert!(matches!(err, Error::UnexpectedRoot(_)));
}

/// A root with a recognized baseline namespace (2019, not just 2024) is
/// still accepted.
#[test]
fn root_element_in_the_2019_namespace_is_accepted() {
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<MulticastGatewayConfiguration xmlns="{NS_2019}" schemaVersion="1"/>
"#
    );
    let parsed = MulticastGatewayConfiguration::parse_str(&xml).expect("parses");
    assert_eq!(parsed.schema_version, 1);
}

/// MABR-W2 (audit issue #1121): `to_xml` always emitted the 2024 namespace
/// regardless of `schema_version`. Observed pre-fix: parsing a
/// `schemaVersion="1"` (2019-namespace) document and re-emitting it produced
/// a contradictory `xmlns="...:2024"` with `schemaVersion="1"`.
#[test]
fn schema_version_1_round_trips_with_the_2019_namespace() {
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<MulticastGatewayConfiguration xmlns="{NS_2019}" schemaVersion="1"/>
"#
    );
    let parsed = MulticastGatewayConfiguration::parse_str(&xml).expect("parses");
    let xml2 = parsed.to_xml();
    assert!(
        xml2.contains(&format!("xmlns=\"{NS_2019}\"")),
        "schemaVersion=1 must re-emit the 2019 namespace, got: {xml2}"
    );
    assert!(!xml2.contains(NS_2024));

    // And schemaVersion=2 still gets the 2024 namespace.
    let parsed2 = MulticastGatewayConfiguration::parse_str(&format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<MulticastGatewayConfiguration xmlns="{NS_2024}" schemaVersion="2"/>
"#
    ))
    .expect("parses");
    assert!(parsed2.to_xml().contains(&format!("xmlns=\"{NS_2024}\"")));
}

/// MABR-W4 (audit issue #1121): element text is truncated at the first
/// comment/CDATA boundary. Observed pre-fix: the `ReportingLocator` URI
/// (element content, read via `own_text`) was `"https://a.example/"`,
/// silently dropping everything after the comment.
#[test]
fn element_text_is_not_truncated_at_a_comment() {
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<MulticastGatewayConfiguration xmlns="{NS_2024}" schemaVersion="2">
  <MulticastGatewaySessionReporting>
    <ReportingLocator period="PT0S" randomDelay="0">https://a.example/<!-- x -->path</ReportingLocator>
  </MulticastGatewaySessionReporting>
</MulticastGatewayConfiguration>
"#
    );
    let parsed = MulticastGatewayConfiguration::parse_str(&xml).expect("parses");
    let uri = &parsed.reporting.expect("reporting present").locators[0].uri;
    assert_eq!(uri, "https://a.example/path");
}

/// MABR-W3 (audit issue #1121): `proportion="NaN"` parsed successfully
/// pre-fix, breaking the documented `parse -> to_xml -> parse` round trip
/// (`NaN != NaN`) and producing an invalid `xs:decimal` lexical form on
/// re-serialize.
#[test]
fn nan_proportion_is_rejected() {
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<MulticastGatewayConfiguration xmlns="{NS_2024}" schemaVersion="2">
  <MulticastGatewaySessionReporting>
    <ReportingLocator period="PT0S" randomDelay="0" proportion="NaN">https://example.com</ReportingLocator>
  </MulticastGatewaySessionReporting>
</MulticastGatewayConfiguration>
"#
    );
    let err = MulticastGatewayConfiguration::parse_str(&xml).unwrap_err();
    assert!(matches!(err, Error::InvalidAttribute { .. }));
}

/// MABR-W3 (audit issue #1121): `proportion` is documented as `(0.0, 1.0]`
/// but was never range-checked.
#[test]
fn out_of_range_proportion_is_rejected() {
    for bad in ["0.0", "1.5", "-0.1"] {
        let xml = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<MulticastGatewayConfiguration xmlns="{NS_2024}" schemaVersion="2">
  <MulticastGatewaySessionReporting>
    <ReportingLocator period="PT0S" randomDelay="0" proportion="{bad}">https://example.com</ReportingLocator>
  </MulticastGatewaySessionReporting>
</MulticastGatewayConfiguration>
"#
        );
        let err = MulticastGatewayConfiguration::parse_str(&xml).unwrap_err();
        assert!(
            matches!(err, Error::InvalidAttribute { .. }),
            "proportion={bad} must be rejected"
        );
    }

    // In-range values still work.
    for good in ["0.1", "1.0"] {
        let xml = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<MulticastGatewayConfiguration xmlns="{NS_2024}" schemaVersion="2">
  <MulticastGatewaySessionReporting>
    <ReportingLocator period="PT0S" randomDelay="0" proportion="{good}">https://example.com</ReportingLocator>
  </MulticastGatewaySessionReporting>
</MulticastGatewayConfiguration>
"#
        );
        MulticastGatewayConfiguration::parse_str(&xml)
            .unwrap_or_else(|e| panic!("proportion={good} must be accepted, got {e}"));
    }
}

/// Release audit: a `schemaVersion="2"` document that declares the 2019
/// namespace must re-serialize under the 2019 namespace (the namespace is
/// preserved, not re-derived from `schemaVersion`).
#[test]
fn schema_version_2_with_the_2019_namespace_is_byte_stable() {
    let xml = format!(r#"<MulticastGatewayConfiguration xmlns="{NS_2019}" schemaVersion="2"/>"#);
    let cfg = MulticastGatewayConfiguration::parse_str(&xml).unwrap();
    assert_eq!(cfg.namespace, dvb_mabr::BaselineNamespace::V2019);
    let xml2 = cfg.to_xml();
    assert!(
        xml2.contains(&format!("xmlns=\"{NS_2019}\"")),
        "2019 namespace must survive re-serialization, got: {xml2}"
    );
    assert!(!xml2.contains(NS_2024), "must not rewrite to 2024: {xml2}");
    assert_eq!(
        MulticastGatewayConfiguration::parse_str(&xml2).unwrap(),
        cfg
    );
}
