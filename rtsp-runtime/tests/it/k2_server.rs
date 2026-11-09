//! Backlog sweep K2: server content hooks and reversed ranges.

use rtsp_runtime::{ServerSession, Transport};

fn wire(s: &str) -> Vec<u8> {
    s.replace('\n', "\r\n").into_bytes()
}

fn answer(s: &mut ServerSession, req: &str) -> String {
    let (resp, _) = s.handle_request(&wire(req)).unwrap();
    String::from_utf8(resp).unwrap()
}

#[test]
fn options_advertises_the_implemented_methods_in_public() {
    let mut s = ServerSession::new(|| 1);
    let text = answer(&mut s, "OPTIONS rtsp://h/s RTSP/1.0\nCSeq: 1\n\n");
    let public = text
        .lines()
        .find(|l| l.to_ascii_lowercase().starts_with("public:"))
        .unwrap_or_else(|| panic!("no Public header: {text}"));
    for m in ["OPTIONS", "DESCRIBE", "SETUP", "PLAY", "PAUSE", "TEARDOWN"] {
        assert!(public.contains(m), "{m} missing from {public}");
    }
}

#[test]
fn describe_returns_the_configured_sdp_with_its_content_type() {
    let sdp = "v=0\r\ns=x\r\n";
    let mut s = ServerSession::new(|| 1).with_describe_sdp(sdp);
    let text = answer(&mut s, "DESCRIBE rtsp://h/s RTSP/1.0\nCSeq: 1\nAccept: application/sdp\n\n");
    assert!(text.starts_with("RTSP/1.0 200"), "{text}");
    assert!(text.to_ascii_lowercase().contains("content-type: application/sdp"), "{text}");
    assert!(text.ends_with(sdp), "{text}");
}

#[test]
fn udp_setup_reply_carries_the_servers_port_pair() {
    let mut s = ServerSession::new(|| 1).with_server_ports(6970, 6971);
    let text = answer(
        &mut s,
        "SETUP rtsp://h/s/t RTSP/1.0\nCSeq: 1\nTransport: RTP/AVP;unicast;client_port=5000-5001\n\n",
    );
    let line = text.lines().find(|l| l.starts_with("Transport:")).unwrap();
    let t = Transport::parse(line["Transport:".len()..].trim()).unwrap();
    assert_eq!(t.first().unwrap().server_port, Some((6970, 6971)), "{line}");
    assert_eq!(t.first().unwrap().client_port, Some((5000, 5001)));
}

#[test]
fn tcp_setup_reply_has_no_server_port() {
    let mut s = ServerSession::new(|| 1).with_server_ports(6970, 6971);
    let text = answer(
        &mut s,
        "SETUP rtsp://h/s/t RTSP/1.0\nCSeq: 1\nTransport: RTP/AVP/TCP;interleaved=0-1\n\n",
    );
    assert!(!text.contains("server_port"), "{text}");
}

#[test]
fn reversed_port_and_channel_ranges_are_rejected() {
    assert!(Transport::parse("RTP/AVP;unicast;client_port=5001-5000").is_err());
    assert!(Transport::parse("RTP/AVP/TCP;interleaved=3-1").is_err());
    assert!(Transport::parse("RTP/AVP;multicast;port=9-8").is_err());
    assert!(Transport::parse("RTP/AVP;unicast;client_port=5000-5001").is_ok());
}
