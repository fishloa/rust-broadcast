//! Secret handling (audit umbrella #1142): the publish stream key is the
//! ingest secret and must never reach a `{:?}` / `dbg!` / log line.

use rtmp_runtime::client::ClientConfig;
use rtmp_runtime::server::{ServerConfig, ServerEvent, ServerSession};
use rtmp_runtime::target::RtmpTarget;

const KEY: &str = "sk-live-4f9a1c77-top-secret";

#[test]
fn server_config_and_session_debug_hide_the_expected_stream_key() {
    let cfg = ServerConfig::default().with_expected_stream_key(Some(KEY.to_string()));
    let text = format!("{cfg:?}");
    assert!(!text.contains(KEY), "{text}");
    assert!(text.contains("redacted"), "{text}");
    let session = ServerSession::new(cfg);
    let text = format!("{session:?}");
    assert!(!text.contains(KEY), "{text}");
    // No key configured: nothing to redact, and it says so.
    let text = format!("{:?}", ServerConfig::default());
    assert!(text.contains("expected_stream_key: None"), "{text}");
}

#[test]
fn publish_event_debug_hides_the_stream_key() {
    let event = ServerEvent::Publish {
        app: "live".into(),
        stream_key: KEY.into(),
        stream_id: 1,
    };
    let text = format!("{event:?}");
    assert!(!text.contains(KEY), "{text}");
    assert!(text.contains("live"), "{text}");
    // The value itself is still delivered to the caller.
    assert!(matches!(&event, ServerEvent::Publish { stream_key, .. } if stream_key == KEY));
}

#[test]
fn client_config_and_target_debug_hide_the_stream_key() {
    let mut cfg = ClientConfig::default();
    cfg.stream_key = KEY.to_string();
    let text = format!("{cfg:?}");
    assert!(!text.contains(KEY), "{text}");

    let target = RtmpTarget::parse(&format!("rtmp://ingest.example.com/live/{KEY}")).unwrap();
    assert!(target.stream_key.contains(KEY));
    let text = format!("{target:?}");
    assert!(!text.contains(KEY), "{text}");
    assert!(text.contains("ingest.example.com"), "{text}");
}
