//! A manifest fragment-URL template that renders a reference carrying an ASCII
//! control character is never fetched and never sanitised (#2458): the WHATWG
//! parser behind `url` DELETES a raw CR/LF/TAB, so without the guard
//! `seg\r\n1.m4s` would become a request to a URL the manifest never named.
//! The guarded resolver in `broadcast-common::url_ref` refuses it and the
//! engine reports the fragment URL as dropped, the way it already reports a
//! template that does not resolve.
//!
//! `broadcast_smooth` normalises XML attribute whitespace (XML 1.0 §3.3.3: TAB,
//! LF and CR in an attribute value become spaces), so the engine receives the
//! rendered reference with a space where the manifest wrote a control
//! character. That is still a request to a URL the manifest never named, so the
//! resolver refuses it, which is what these tests pin.

use smooth_runtime::{Dropped, Output, SmoothClientConfig};

use crate::common::{Host, Reply, fixture};

const VIDEO_TEMPLATE: &str = r#"QualityLevels({bitrate})/Fragments(video={start time})"#;

fn manifest_with_video_url(url: &str) -> Vec<u8> {
    let text = fixture("av/Manifest");
    let text = core::str::from_utf8(&text).expect("manifest is UTF-8");
    let (head, tail) = text.split_once(VIDEO_TEMPLATE).expect("template");
    format!(
        "{head}{url}{tail}",
        url = url.replace('&', "&amp;").replace('<', "&lt;")
    )
    .into_bytes()
}

fn av_host(manifest: Vec<u8>) -> Host {
    let mut host = Host::new(SmoothClientConfig::default());
    host.serve_dir("av");
    host.route("Manifest", vec![Reply::Body(manifest)]);
    host
}

fn dropped_fragment_urls(host: &Host) -> Vec<&Output> {
    host.outputs
        .iter()
        .filter(|o| matches!(o, Output::Dropped(Dropped::FragmentUrl { .. })))
        .collect()
}

#[test]
fn a_fragment_reference_with_a_crlf_is_never_fetched() {
    let mut host = av_host(manifest_with_video_url(
        "QualityLevels({bitrate})/seg\r\n1-{start time}.m4s",
    ));
    host.run();
    assert!(
        !host.requested.iter().any(|u| u.contains("/seg")),
        "{:?}",
        host.requested
    );
    assert!(
        !dropped_fragment_urls(&host).is_empty(),
        "the bad reference is reported as a dropped fragment URL"
    );
}

#[test]
fn a_fragment_reference_with_a_tab_is_never_fetched() {
    let mut host = av_host(manifest_with_video_url(
        "QualityLevels({bitrate})/seg\t1-{start time}.m4s",
    ));
    host.run();
    assert!(
        !host.requested.iter().any(|u| u.contains("/seg1-")),
        "{:?}",
        host.requested
    );
    assert!(
        !dropped_fragment_urls(&host).is_empty(),
        "the bad reference is reported as a dropped fragment URL"
    );
}

#[test]
fn a_clean_template_still_requests_the_fragment() {
    let mut host = av_host(manifest_with_video_url(VIDEO_TEMPLATE));
    host.run();
    assert_eq!(host.count_requests("Fragments(video=0)"), 1);
    assert!(dropped_fragment_urls(&host).is_empty());
}

#[test]
fn the_fixture_manifest_is_unchanged() {
    assert_eq!(
        manifest_with_video_url(VIDEO_TEMPLATE),
        fixture("av/Manifest")
    );
}
