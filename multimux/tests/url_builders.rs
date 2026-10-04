//! SP3 (url crate). The IPv6 rows are CHARACTERISATION: main already keeps the
//! brackets via `Url::host_str()` (verified push/rtmp.rs:351-359,
//! push/rtsp.rs:94-96, source/rtsp.rs connect_addr); defect 8 was rtmp-runtime-only
//! and was fixed in W1-R-low-a. Redaction must ALSO keep working on a
//! URL the `url` parser rejects — redact.rs's documented contract.

use multimux::push::srt::parse_srt_url_for_test;
use multimux::redact::{redact_destination, redact_url};

/// CHARACTERISATION (passes on old code too — main already brackets IPv6;
/// pins the bracketing across the `url`/`RtmpTarget` migration).
#[test]
fn an_ipv6_host_is_bracketed_in_the_connect_address_and_tc_url() {
    let tc = multimux::push::rtmp::tc_url_for_test("rtmp://[::1]:1935/live/stream").unwrap();
    assert_eq!(tc, "rtmp://[::1]:1935/live");
    let control = multimux::push::rtsp::control_url_for_test("rtsp://[2001:db8::1]:554/live");
    assert_eq!(control, "rtsp://[2001:db8::1]:554/live/trackID=0");
    assert!(!tc.contains("://::1"));
    assert!(!control.contains("://2001:db8::1"));
}

/// CHARACTERISATION (passes on old code too — pins the behaviour across
/// the url migration).
#[test]
fn redaction_keeps_percent_encoded_credentials_opaque() {
    let url = "rtsp://user:p%40ss@cam.example/live";
    assert_eq!(redact_url(url), "rtsp://***@cam.example/live");
    assert_eq!(redact_destination(url), "rtsp://cam.example/<redacted>");
    assert!(!redact_url(url).contains("p%40ss"));
}

/// BITING (fails on old code — the old fallback kept the host, `cam local`;
/// the NEW masking-only fallback does not). The difference is intended and
/// listed in the CHANGELOG, and the guard test (harness_guard/no_handroll)
/// pins that `redact_unparseable` contains no host extraction.
#[test]
fn redaction_still_masks_a_url_the_parser_rejects() {
    assert_eq!(
        redact_destination("rtsp://user:p@cam local/live"),
        "rtsp://<redacted>/live"
    );
}

#[test]
fn an_srt_query_keeps_the_address_and_streamid_separate() {
    let (addr, overrides) =
        parse_srt_url_for_test("srt://[::1]:9000?streamid=live%2Fcam&latency=120").unwrap();
    assert_eq!(addr, "[::1]:9000");
    assert_eq!(overrides.stream_id.as_deref(), Some("live/cam"));
}
