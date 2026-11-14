//! Secret handling (audit umbrella #1142): `Debug` of the push-transport
//! configs never prints the RTMP stream key or the RTSP username/password.

use multimux::push::rtmp::RtmpTransportConfig;
use multimux::push::rtsp::RtspTransportConfig;

#[test]
fn rtmp_push_config_debug_hides_the_stream_key() {
    let cfg = RtmpTransportConfig::default()
        .with_app("live".into())
        .with_stream_key("sk-live-9d2f-top-secret".into());
    let text = format!("{cfg:?}");
    assert!(!text.contains("sk-live-9d2f"), "{text}");
    assert!(text.contains("live"), "{text}");
    assert!(text.contains("redacted"), "{text}");
}

#[test]
fn rtsp_push_config_debug_hides_the_credentials() {
    let cfg = RtspTransportConfig::default()
        .with_credentials("operator".into(), "hunter2-password".into());
    let text = format!("{cfg:?}");
    assert!(!text.contains("hunter2-password"), "{text}");
    assert!(!text.contains("operator"), "{text}");
    assert!(text.contains("redacted"), "{text}");
    assert!(format!("{:?}", RtspTransportConfig::default()).contains("None"));
}
