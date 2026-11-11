//! Namespace resolution of the `Protection` probe a Smooth puller runs over a
//! fetched client Manifest before handing fragments to the demuxer (issue
//! #1708; the probe itself stays in `transmux` with the fragment layer).
#![cfg(feature = "std")]

use transmux::smooth_fragment::manifest_declares_protection;

const SMOOTH: &str = r#"MajorVersion="2" MinorVersion="0" TimeScale="10000000""#;

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
