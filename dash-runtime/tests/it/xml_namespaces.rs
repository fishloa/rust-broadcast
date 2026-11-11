//! XML namespace resolution and the hostile-input bounds of the MPD reader
//! (issue #1708). Ported from transmux's reader tests (issue #2340): the MPD
//! reader moved to `dash_runtime::mpd`, and the DASH half of that suite came
//! with it. The Smooth and PlayReady-probe halves stay in `transmux`.

use dash_runtime::error::{Limit, MpdError};
use dash_runtime::limits::Limits;
use dash_runtime::mpd::Mpd;

const DASH: &str = "urn:mpeg:dash:schema:mpd:2011";

fn mpd(root_attrs: &str, body: &str) -> String {
    format!(r#"<MPD xmlns="{DASH}" xmlns:ext="urn:x" {root_attrs}>{body}</MPD>"#)
}

fn parse(xml: &str) -> Result<Mpd, MpdError> {
    Mpd::parse(xml.as_bytes())
}

fn too_large(r: Result<Mpd, MpdError>) -> Limit {
    match r {
        Err(MpdError::TooLarge { limit, .. }) => limit,
        other => panic!("expected TooLarge, got {other:?}"),
    }
}

#[test]
fn foreign_base_url_is_not_the_base_url() {
    let m = parse(&mpd(
        r#"profiles="p""#,
        "<ext:BaseURL>http://evil/</ext:BaseURL><BaseURL>http://good/</BaseURL>",
    ))
    .unwrap();
    assert_eq!(m.base_urls.len(), 1);
    assert_eq!(m.base_urls[0].url, "http://good/");
}

#[test]
fn foreign_period_is_not_a_period() {
    let m = parse(&mpd(r#"profiles="p""#, "<ext:Period/><Period/>")).unwrap();
    assert_eq!(m.periods.len(), 1);
}

#[test]
fn prefixed_dash_namespace_parses() {
    let xml = format!(
        r#"<d:MPD xmlns:d="{DASH}" profiles="p"><d:BaseURL>http://b/</d:BaseURL><d:Period id="1"/></d:MPD>"#
    );
    let m = parse(&xml).unwrap();
    assert_eq!(m.base_urls.len(), 1);
    assert_eq!(m.base_urls[0].url, "http://b/");
    assert_eq!(m.periods[0].id.as_deref(), Some("1"));
}

#[test]
fn cenc_under_another_prefix_parses() {
    let xml = format!(
        r#"<MPD xmlns="{DASH}" xmlns:c="urn:mpeg:cenc:2013" xmlns:mspr="urn:microsoft:playready" profiles="p"><Period><AdaptationSet><ContentProtection schemeIdUri="urn:mpeg:dash:mp4protection:2011" value="cenc" c:default_KID="x"><c:pssh>UMFZ</c:pssh></ContentProtection><ContentProtection schemeIdUri="urn:microsoft:playready"><mspr:pro>UExB</mspr:pro></ContentProtection></AdaptationSet></Period></MPD>"#
    );
    assert_eq!(parse(&xml).unwrap().periods.len(), 1);
}

#[test]
fn wrong_root_namespace_is_rejected() {
    let err = parse(r#"<MPD xmlns="urn:evil" profiles="p"/>"#).unwrap_err();
    assert_eq!(
        err,
        MpdError::NotMpd {
            found: "MPD".to_owned()
        }
    );
}

#[test]
fn unbound_prefix_is_rejected() {
    let err = parse(&mpd(r#"profiles="p""#, "<zz:BaseURL>x</zz:BaseURL>")).unwrap_err();
    assert!(
        matches!(&err, MpdError::Xml { message, .. } if message.contains("unknown namespace prefix")),
        "{err:?}"
    );
}

#[test]
fn trailing_content_is_rejected() {
    for tail in ["<junk/>", "junk"] {
        let xml = format!("{}{tail}", mpd(r#"profiles="p""#, ""));
        assert!(matches!(parse(&xml), Err(MpdError::Xml { .. })), "{tail}");
    }
    parse(&format!("{}\n<!-- ok -->\n", mpd(r#"profiles="p""#, "")))
        .expect("a trailing comment is not content");
}

#[test]
fn oversized_attribute_is_rejected() {
    let big = "a".repeat(8 * 1024 + 1);
    let xml = mpd(&format!(r#"profiles="{big}""#), "");
    assert_eq!(too_large(parse(&xml)), Limit::AttributeBytes);
}

#[test]
fn deep_nesting_is_rejected() {
    let body = format!("{}{}", "<x>".repeat(100), "</x>".repeat(100));
    assert_eq!(
        too_large(parse(&mpd(r#"profiles="p""#, &body))),
        Limit::Depth
    );
}

#[test]
fn huge_element_count_is_rejected() {
    let body = "<A/>".repeat(100_001);
    assert_eq!(
        too_large(parse(&mpd(r#"profiles="p""#, &body))),
        Limit::Elements
    );
}

#[test]
fn oversized_document_is_rejected() {
    let pad = " ".repeat(4 * 1024 * 1024);
    assert_eq!(
        too_large(parse(&format!("{}{pad}", mpd(r#"profiles="p""#, "")))),
        Limit::InputBytes
    );
}

fn tight(change: impl FnOnce(&mut Limits)) -> Limits {
    let mut limits = Limits::default();
    change(&mut limits);
    limits
}

fn hit(xml: &str, limits: &Limits) -> Limit {
    match Mpd::parse_with(xml.as_bytes(), limits) {
        Err(MpdError::TooLarge { limit, .. }) => limit,
        other => panic!("expected TooLarge, got {:?}", other.map(|_| ())),
    }
}

#[test]
fn caller_limits_trip_each_bound_and_can_raise_the_defaults() {
    let xml = mpd(r#"profiles="p""#, "<Period/>");
    assert_eq!(
        hit(&xml, &tight(|l| l.max_bytes = xml.len() - 1)),
        Limit::InputBytes
    );
    assert_eq!(hit(&xml, &tight(|l| l.max_depth = 1)), Limit::Depth);
    assert_eq!(hit(&xml, &tight(|l| l.max_elements = 1)), Limit::Elements);
    assert_eq!(
        hit(&xml, &tight(|l| l.max_attribute_bytes = 0)),
        Limit::AttributeBytes
    );
    assert_eq!(
        hit(&xml, &tight(|l| l.max_attributes = 0)),
        Limit::Attributes
    );
    let nested = format!("{}{}", "<x>".repeat(100), "</x>".repeat(100));
    let deep = mpd(r#"profiles="p""#, &nested);
    assert!(parse(&deep).is_err());
    assert!(Mpd::parse_with(deep.as_bytes(), &tight(|l| l.max_depth = 200)).is_ok());
}
