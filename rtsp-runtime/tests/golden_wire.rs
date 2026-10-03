//! Byte-for-byte goldens for the RTSP wire output (W1-R-low-a, spec §6).
//!
//! `session_transcript.golden` is a full client<->server exchange, both ends the
//! sans-IO cores, with a fixed User-Agent and a deterministic session id.
//! `transport_parsed.golden` is the `Debug` of each parsed `Transport` (semantic
//! pin, must never change). `transport_header.golden` is the serialized header
//! text (byte pin; Task 4 may change parameter ORDER only, and says so in the
//! CHANGELOG). Regenerate only with `GOLDEN_UPDATE=1`.

use rtsp_runtime::{ClientSession, ServerSession, Transport, TransportSpec};

const URI: &str = "rtsp://example.test/stream";

fn golden(name: &str, actual: &str) {
    let path = format!("{}/tests/golden/{name}", env!("CARGO_MANIFEST_DIR"));
    if std::env::var_os("GOLDEN_UPDATE").is_some() {
        std::fs::write(&path, actual).expect("write golden");
        return;
    }
    let want = std::fs::read_to_string(&path)
        .expect("read golden (run once with GOLDEN_UPDATE=1 on main)");
    assert_eq!(actual, want, "golden {name} differs");
}

fn exchange(
    label: &str,
    req: Vec<u8>,
    c: &mut ClientSession,
    s: &mut ServerSession,
    out: &mut String,
) {
    out.push_str(&format!(
        "--- {label} request ---\n{}",
        String::from_utf8_lossy(&req)
    ));
    let (resp, _) = s.handle_request(&req).expect("server handles request");
    out.push_str(&format!(
        "--- {label} response ---\n{}",
        String::from_utf8_lossy(&resp)
    ));
    c.handle_data(&resp).expect("client handles response");
}

#[test]
fn session_transcript_is_byte_identical() {
    let mut c = ClientSession::new().with_user_agent("golden-ua");
    let mut s = ServerSession::new(|| 0)
        .with_session_seed(0xABCD)
        .with_session_timeout(60);
    let mut out = String::new();
    let r = c.options(URI).unwrap();
    exchange("OPTIONS", r, &mut c, &mut s, &mut out);
    let r = c.describe(URI).unwrap();
    // DESCRIBE has no SDP route in the bare ServerSession: record the request only.
    out.push_str(&format!(
        "--- DESCRIBE request ---\n{}",
        String::from_utf8_lossy(&r)
    ));
    let t = Transport::single(TransportSpec::rtp_avp_tcp_interleaved(0, 1));
    let r = c.setup(URI, &t).unwrap();
    exchange("SETUP", r, &mut c, &mut s, &mut out);
    let r = c.play(URI).unwrap();
    exchange("PLAY", r, &mut c, &mut s, &mut out);
    let r = c.get_parameter(URI, b"").unwrap();
    out.push_str(&format!(
        "--- GET_PARAMETER request ---\n{}",
        String::from_utf8_lossy(&r)
    ));
    let r = c.pause(URI).unwrap();
    exchange("PAUSE", r, &mut c, &mut s, &mut out);
    let r = c.teardown(URI).unwrap();
    exchange("TEARDOWN", r, &mut c, &mut s, &mut out);
    golden("session_transcript.golden", &out);
}

const TRANSPORT_CASES: &[&str] = &[
    "RTP/AVP/TCP;interleaved=0-1",
    "RTP/AVP;unicast;client_port=8000-8001",
    "RTP/AVP;unicast;client_port=8000-8001;server_port=9000-9001;ssrc=DEADBEEF",
    "RTP/AVP/UDP;multicast;destination=224.2.0.1;port=3456-3457;ttl=16",
    "RTP/AVP;unicast;mode=\"RECORD\";append;interleaved=2-3",
    "RTP/AVP;multicast;layers=2;ttl=5",
    "RTP/AVP/TCP;unicast;interleaved=4,RTP/AVP;unicast;client_port=5000-5001",
    "RTP/AVP;unicast;source=10.0.0.1;destination=10.0.0.2",
    "RTP/AVP;unicast;interleaved=6",
];

#[test]
fn transport_parse_and_header_goldens() {
    let (mut parsed, mut header) = (String::new(), String::new());
    for case in TRANSPORT_CASES {
        let t = Transport::parse(case).unwrap_or_else(|e| panic!("{case}: {e}"));
        parsed.push_str(&format!("{case}\n  {t:?}\n"));
        header.push_str(&format!("{case}\n  {}\n", t.to_header_value().unwrap()));
    }
    golden("transport_parsed.golden", &parsed);
    golden("transport_header.golden", &header);
}
