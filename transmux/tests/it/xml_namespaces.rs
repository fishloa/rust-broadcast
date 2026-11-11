//! Namespace resolution and hostile-input bounds of the Smooth manifest reader
//! (issue #1708). The MPD reader is dash-runtime's (tested in that crate);
//! PlayReady has no reader in transmux (the WRMHEADER is only written), so it
//! has no case here.
#![cfg(feature = "std")]

use broadcast_common::xml_chars::{XmlLimit, XmlLimits};
use transmux::smooth_fragment::manifest_declares_protection;
use transmux::smooth_parse::default_limits as smooth_defaults;
use transmux::smooth_parse::{SmoothManifest, SmoothParseError};

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

fn smooth_hit(xml: &str, limits: XmlLimits) -> XmlLimit {
    match SmoothManifest::parse_with_limits(xml, limits) {
        Err(SmoothParseError::TooLarge { limit, .. }) => limit,
        other => panic!("expected TooLarge, got {:?}", other.map(|_| ())),
    }
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
