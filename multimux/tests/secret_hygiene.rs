//! Secret handling (audit umbrella #1142): `Debug` of the push-transport
//! configs never prints the RTMP stream key or the RTSP username/password.

use multimux::push::rtmp::RtmpTransportConfig;
use multimux::push::rtsp::RtspTransportConfig;

#[test]
fn rtmp_push_config_debug_hides_the_stream_key() {
    let cfg = RtmpTransportConfig {
        app: "live".into(),
        stream_key: "sk-live-9d2f-top-secret".into(),
        ..Default::default()
    };
    let text = format!("{cfg:?}");
    assert!(!text.contains("sk-live-9d2f"), "{text}");
    assert!(text.contains("live"), "{text}");
    assert!(text.contains("redacted"), "{text}");
}

#[test]
fn rtsp_push_config_debug_hides_the_credentials() {
    let cfg = RtspTransportConfig {
        credentials: Some(("operator".into(), "hunter2-password".into())),
        ..Default::default()
    };
    let text = format!("{cfg:?}");
    assert!(!text.contains("hunter2-password"), "{text}");
    assert!(!text.contains("operator"), "{text}");
    assert!(text.contains("redacted"), "{text}");
    assert!(format!("{:?}", RtspTransportConfig::default()).contains("None"));
}
