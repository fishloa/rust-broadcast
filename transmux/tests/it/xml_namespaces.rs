//! Namespace resolution and hostile-input bounds of the MPD and Smooth
//! manifest readers (issue #1708). PlayReady has no reader in transmux (the
//! WRMHEADER is only written), so it has no case here.
#![cfg(feature = "std")]

use broadcast_common::xml_chars::{XmlLimit, XmlLimits};
use transmux::Mpd;
use transmux::dash_parse::DashParseError;
use transmux::dash_parse::default_xml_limits as dash_defaults;
use transmux::smooth_fragment::manifest_declares_protection;
use transmux::smooth_parse::default_xml_limits as smooth_defaults;
use transmux::smooth_parse::{SmoothManifest, SmoothParseError};

const DASH: &str = "urn:mpeg:dash:schema:mpd:2011";

fn mpd(root_attrs: &str, body: &str) -> String {
    format!(r#"<MPD xmlns="{DASH}" xmlns:ext="urn:x" {root_attrs}>{body}</MPD>"#)
}

#[test]
fn foreign_base_url_is_not_the_base_url() {
    let m = Mpd::parse(&mpd(
        r#"profiles="p""#,
        "<ext:BaseURL>http://evil/</ext:BaseURL><BaseURL>http://good/</BaseURL>",
    ))
    .unwrap();
    assert_eq!(m.base_url.as_deref(), Some("http://good/"));
}

#[test]
fn foreign_period_is_not_a_period() {
    let m = Mpd::parse(&mpd(r#"profiles="p""#, "<ext:Period/><Period/>")).unwrap();
    assert_eq!(m.periods.len(), 1);
}

#[test]
fn foreign_attribute_with_a_dash_local_name_is_ignored() {
    let m = Mpd::parse(&mpd(r#"profiles="p" ext:type="dynamic""#, "")).unwrap();
    assert_eq!(m.mpd_type, transmux::MpdType::Static);
    let err = Mpd::parse(&mpd(r#"ext:profiles="p""#, "")).unwrap_err();
    assert_eq!(
        err,
        DashParseError::MissingAttribute {
            element: "MPD",
            attr: "profiles"
        }
    );
}

#[test]
fn prefixed_dash_namespace_parses() {
    let xml = format!(
        r#"<d:MPD xmlns:d="{DASH}" profiles="p"><d:BaseURL>http://b/</d:BaseURL><d:Period id="1"/></d:MPD>"#
    );
    let m = Mpd::parse(&xml).unwrap();
    assert_eq!(m.base_url.as_deref(), Some("http://b/"));
    assert_eq!(m.periods[0].id.as_deref(), Some("1"));
}

#[test]
fn cenc_under_another_prefix_parses() {
    let xml = format!(
        r#"<MPD xmlns="{DASH}" xmlns:c="urn:mpeg:cenc:2013" profiles="p"><Period><AdaptationSet><ContentProtection c:default_KID="x"/></AdaptationSet></Period></MPD>"#
    );
    assert_eq!(Mpd::parse(&xml).unwrap().periods.len(), 1);
}

#[test]
fn wrong_root_namespace_is_rejected() {
    let err = Mpd::parse(r#"<MPD xmlns="urn:evil" profiles="p"/>"#).unwrap_err();
    assert_eq!(
        err,
        DashParseError::UnexpectedElement {
            expected: "MPD",
            found: "{urn:evil}MPD".into()
        }
    );
}

#[test]
fn unbound_prefix_is_rejected() {
    let err = Mpd::parse(&mpd(r#"profiles="p""#, "<zz:BaseURL>x</zz:BaseURL>")).unwrap_err();
    assert!(
        matches!(&err, DashParseError::Xml { message, .. } if message.contains("unknown namespace prefix")),
        "{err:?}"
    );
}

#[test]
fn trailing_content_is_rejected() {
    for tail in ["<junk/>", "junk"] {
        let xml = format!("{}{tail}", mpd(r#"profiles="p""#, ""));
        assert!(
            matches!(Mpd::parse(&xml), Err(DashParseError::Xml { .. })),
            "{tail}"
        );
    }
    Mpd::parse(&format!("{}\n<!-- ok -->\n", mpd(r#"profiles="p""#, ""))).unwrap();
}

#[test]
fn oversized_attribute_is_rejected() {
    let big = "a".repeat(8 * 1024 + 1);
    let err = Mpd::parse(&mpd(&format!(r#"profiles="{big}""#), "")).unwrap_err();
    assert_eq!(
        err,
        DashParseError::TooLarge {
            limit: XmlLimit::AttributeBytes,
            max: 8 * 1024
        }
    );
}

#[test]
fn deep_nesting_is_rejected() {
    let body = format!("{}{}", "<x>".repeat(100), "</x>".repeat(100));
    let err = Mpd::parse(&mpd(r#"profiles="p""#, &body)).unwrap_err();
    assert_eq!(
        err,
        DashParseError::TooLarge {
            limit: XmlLimit::Depth,
            max: 32
        }
    );
}

#[test]
fn huge_element_count_is_rejected() {
    let err = Mpd::parse(&mpd(r#"profiles="p""#, &"<A/>".repeat(100_001))).unwrap_err();
    assert_eq!(
        err,
        DashParseError::TooLarge {
            limit: XmlLimit::Elements,
            max: 100_000
        }
    );
}

#[test]
fn oversized_document_is_rejected() {
    let pad = " ".repeat(4 * 1024 * 1024);
    let err = Mpd::parse(&format!("{}{pad}", mpd(r#"profiles="p""#, ""))).unwrap_err();
    assert!(matches!(
        err,
        DashParseError::TooLarge {
            limit: XmlLimit::InputBytes,
            ..
        }
    ));
}

const SMOOTH: &str = r#"MajorVersion="2" MinorVersion="0" TimeScale="10000000""#;

#[test]
fn smooth_namespaced_look_alike_is_ignored() {
    let xml = format!(
        r#"<SmoothStreamingMedia {SMOOTH} xmlns:x="urn:x"><x:StreamIndex Type="video" Url="u"/><StreamIndex Type="video" Url="u"/></SmoothStreamingMedia>"#
    );
    assert_eq!(SmoothManifest::parse(&xml).unwrap().streams.len(), 1);
}

#[test]
fn smooth_namespaced_root_is_rejected() {
    let xml = format!(r#"<SmoothStreamingMedia xmlns="urn:x" {SMOOTH}/>"#);
    assert!(matches!(
        SmoothManifest::parse(&xml),
        Err(SmoothParseError::UnexpectedElement { .. })
    ));
}

#[test]
fn smooth_unbound_prefix_trailing_and_depth_are_rejected() {
    let unbound = format!("<SmoothStreamingMedia {SMOOTH}><q:StreamIndex/></SmoothStreamingMedia>");
    assert!(matches!(
        SmoothManifest::parse(&unbound),
        Err(SmoothParseError::Xml { .. })
    ));
    let trailing = format!("<SmoothStreamingMedia {SMOOTH}/><junk/>");
    assert!(matches!(
        SmoothManifest::parse(&trailing),
        Err(SmoothParseError::Xml { .. })
    ));
    let deep = format!(
        "<SmoothStreamingMedia {SMOOTH}>{}{}</SmoothStreamingMedia>",
        "<x>".repeat(100),
        "</x>".repeat(100)
    );
    assert!(matches!(
        SmoothManifest::parse(&deep),
        Err(SmoothParseError::TooLarge {
            limit: XmlLimit::Depth,
            ..
        })
    ));
}

#[test]
fn protection_probe_ignores_foreign_namespace_look_alikes() {
    assert!(manifest_declares_protection(&format!(
        "<SmoothStreamingMedia {SMOOTH}><Protection/></SmoothStreamingMedia>"
    )));
    assert!(!manifest_declares_protection(&format!(
        r#"<SmoothStreamingMedia {SMOOTH} xmlns:ms="urn:x"><ms:Protection/></SmoothStreamingMedia>"#
    )));
    assert!(!manifest_declares_protection(&format!(
        r#"<SmoothStreamingMedia {SMOOTH}><Protection xmlns="urn:x"/></SmoothStreamingMedia>"#
    )));
}

#[test]
fn protection_probe_fails_closed_to_false_past_the_bounds() {
    let deep = format!(
        "<SmoothStreamingMedia {SMOOTH}>{}<Protection/>{}</SmoothStreamingMedia>",
        "<x>".repeat(100),
        "</x>".repeat(100)
    );
    assert!(!manifest_declares_protection(&deep));
}

fn tight(mut limits: XmlLimits, change: impl FnOnce(&mut XmlLimits)) -> XmlLimits {
    change(&mut limits);
    limits
}

fn dash_hit(xml: &str, limits: XmlLimits) -> XmlLimit {
    match Mpd::parse_with_limits(xml, limits) {
        Err(DashParseError::TooLarge { limit, .. }) => limit,
        other => panic!("expected TooLarge, got {other:?}"),
    }
}

fn smooth_hit(xml: &str, limits: XmlLimits) -> XmlLimit {
    match SmoothManifest::parse_with_limits(xml, limits) {
        Err(SmoothParseError::TooLarge { limit, .. }) => limit,
        other => panic!("expected TooLarge, got {:?}", other.map(|_| ())),
    }
}

#[test]
fn dash_caller_limits_trip_each_bound_and_can_raise_the_defaults() {
    let xml = mpd(r#"profiles="p""#, "<Period/>");
    let d = dash_defaults();
    assert_eq!(
        dash_hit(&xml, tight(d.clone(), |l| l.max_bytes = xml.len() - 1)),
        XmlLimit::InputBytes
    );
    assert_eq!(
        dash_hit(&xml, tight(d.clone(), |l| l.max_depth = 1)),
        XmlLimit::Depth
    );
    assert_eq!(
        dash_hit(&xml, tight(d.clone(), |l| l.max_elements = 1)),
        XmlLimit::Elements
    );
    assert_eq!(
        dash_hit(&xml, tight(d.clone(), |l| l.max_attribute_bytes = 0)),
        XmlLimit::AttributeBytes
    );
    assert_eq!(
        dash_hit(&xml, tight(d.clone(), |l| l.max_attributes = 0)),
        XmlLimit::Attributes
    );
    let nested = format!("{}{}", "<x>".repeat(100), "</x>".repeat(100));
    let deep = mpd(r#"profiles="p""#, &nested);
    assert!(Mpd::parse(&deep).is_err());
    assert!(Mpd::parse_with_limits(&deep, tight(d, |l| l.max_depth = 200)).is_ok());
}

#[test]
fn smooth_caller_limits_trip_each_bound_and_can_raise_the_defaults() {
    let xml = format!(
        r#"<SmoothStreamingMedia {SMOOTH}><StreamIndex Type="video" Url="u"/></SmoothStreamingMedia>"#
    );
    let d = smooth_defaults();
    assert_eq!(
        smooth_hit(&xml, tight(d.clone(), |l| l.max_bytes = xml.len() - 1)),
        XmlLimit::InputBytes
    );
    assert_eq!(
        smooth_hit(&xml, tight(d.clone(), |l| l.max_depth = 1)),
        XmlLimit::Depth
    );
    assert_eq!(
        smooth_hit(&xml, tight(d.clone(), |l| l.max_elements = 1)),
        XmlLimit::Elements
    );
    assert_eq!(
        smooth_hit(&xml, tight(d.clone(), |l| l.max_attribute_bytes = 0)),
        XmlLimit::AttributeBytes
    );
    assert_eq!(
        smooth_hit(&xml, tight(d.clone(), |l| l.max_attributes = 1)),
        XmlLimit::Attributes
    );
    let deep = format!(
        "<SmoothStreamingMedia {SMOOTH}>{}{}</SmoothStreamingMedia>",
        "<x>".repeat(100),
        "</x>".repeat(100)
    );
    assert!(SmoothManifest::parse(&deep).is_err());
    assert!(SmoothManifest::parse_with_limits(&deep, tight(d, |l| l.max_depth = 200)).is_ok());
}

#[test]
fn largest_real_manifest_parses_with_the_defaults() {
    let xml = test_fixtures::read_to_string("dash/livesim2/segtimelinenr_a.mpd");
    assert!(xml.len() > 2_000);
    transmux::Mpd::parse(&xml).expect("defaults admit the fixture");
}
