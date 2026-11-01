//! Byte-for-byte golden of the RTMP publish client's wire output (W1-R-low-a).
//! Client and server are both the sans-IO cores, pumped in memory. The
//! handshake random fill is deterministic (`default_random_fill`), so the whole
//! transcript is stable. Regenerate only with `GOLDEN_UPDATE=1` on `main`.

use rtmp_runtime::amf0::Amf0Value;
use rtmp_runtime::client::{ClientConfig, ClientSession};
use rtmp_runtime::server::ServerSession;

fn hex(label: &str, bytes: &[u8], out: &mut String) {
    out.push_str(&format!("{label} ({} bytes)\n", bytes.len()));
    for line in bytes.chunks(32) {
        out.push_str("  ");
        for b in line {
            out.push_str(&format!("{b:02x}"));
        }
        out.push('\n');
    }
}

// `ClientConfig` is `#[non_exhaustive]`: outside its crate a struct expression is not allowed.
#[allow(clippy::field_reassign_with_default)]
#[test]
fn client_publish_transcript_is_byte_identical() {
    let mut cfg = ClientConfig::default();
    cfg.app = "live".into();
    cfg.stream_key = "key".into();
    cfg.tc_url = Some("rtmp://example.test:1935/live".into());
    let mut c = ClientSession::new(cfg);
    let mut s = ServerSession::with_defaults();
    let mut out = String::new();

    let mut to_server = c.start();
    hex("C0+C1", &to_server, &mut out);
    for round in 0..8 {
        let (reply, _) = s.handle_data(&to_server).expect("server");
        hex(&format!("server reply {round}"), &reply, &mut out);
        let (next, _) = c.handle_data(&reply).expect("client");
        hex(&format!("client output {round}"), &next, &mut out);
        if c.is_publishing() {
            break;
        }
        to_server = next;
    }
    assert!(c.is_publishing(), "client must reach Publishing");
    let a = c.send_audio(40, &[0xAF, 0x01, 0x11, 0x22]).unwrap();
    hex("audio", &a, &mut out);
    let v = c
        .send_video(40, &[0x17, 0x01, 0, 0, 0, 0x33, 0x44])
        .unwrap();
    hex("video", &v, &mut out);
    let m = c
        .send_metadata(&[("width".to_string(), Amf0Value::Number(1280.0))])
        .unwrap();
    hex("metadata", &m, &mut out);

    let path = format!(
        "{}/tests/golden/client_publish.golden",
        env!("CARGO_MANIFEST_DIR")
    );
    if std::env::var_os("GOLDEN_UPDATE").is_some() {
        std::fs::write(&path, &out).unwrap();
        return;
    }
    assert_eq!(
        out,
        std::fs::read_to_string(&path).expect("golden"),
        "client_publish.golden differs"
    );
}
