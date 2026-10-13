//! Secret handling (audit umbrella #1142): `Debug` never prints a bearer token,
//! TURN credential or ICE password, and a bearer token is sent only to the
//! origin it was configured for — a `Location` naming another host never
//! receives it.

use http::StatusCode;
use webrtc_runtime::ice::IceServer;
use webrtc_runtime::whep::player::{self, WhepPlayer};
use webrtc_runtime::whip::client::{self, WhipClient};

const ENDPOINT: &str = "https://whip.example.com/pub";
const SDP_OFFER: &[u8] = b"v=0\r\no=- 0 0 IN IP4 0.0.0.0\r\n";
const SDP_ANSWER: &[u8] = b"v=0\r\no=- 1 1 IN IP4 127.0.0.1\r\n";
const TOKEN: &str = "tok-7c1f0d2e-secret";

fn bearer_of(req: &client::HttpRequest) -> Option<String> {
    req.headers
        .get("Authorization")
        .map(|v| v.to_str().unwrap().to_string())
}

fn established_client(location: &str) -> WhipClient {
    let mut c = WhipClient::new(ENDPOINT.into(), Some(TOKEN.into()));
    let _ = c.offer(SDP_OFFER.to_vec()).unwrap();
    c.on_response(
        client::HttpResponse::new(StatusCode::CREATED)
            .with_content_type("application/sdp")
            .unwrap()
            .with_location(location)
            .unwrap()
            .with_etag("e")
            .unwrap()
            .with_body(SDP_ANSWER.to_vec()),
    )
    .unwrap();
    c
}

fn established_player(location: &str) -> WhepPlayer {
    let mut p = WhepPlayer::new(ENDPOINT.into(), Some(TOKEN.into()));
    let _ = p.offer(SDP_OFFER.to_vec()).unwrap();
    p.on_response(
        player::HttpResponse::new(StatusCode::CREATED)
            .with_content_type("application/sdp")
            .unwrap()
            .with_location(location)
            .unwrap()
            .with_etag("e")
            .unwrap()
            .with_body(SDP_ANSWER.to_vec()),
    )
    .unwrap();
    p
}

#[test]
fn whip_client_sends_the_bearer_only_to_its_own_origin() {
    // Same-origin absolute and relative session URLs carry the token.
    for location in ["https://whip.example.com/session/1", "/session/1"] {
        let mut c = established_client(location);
        assert_eq!(
            bearer_of(&c.terminate().unwrap()),
            Some(format!("Bearer {TOKEN}")),
            "{location}"
        );
    }
    // Another host, another port, another scheme, or a network-path
    // reference to another host never does.
    for location in [
        "https://evil.example.net/session/1",
        "https://whip.example.com:8443/session/1",
        "http://whip.example.com/session/1",
        "//evil.example.net/session/1",
    ] {
        let mut c = established_client(location);
        assert_eq!(bearer_of(&c.terminate().unwrap()), None, "{location}");
        let mut c = established_client(location);
        assert_eq!(
            bearer_of(&c.flush_candidates(b"a=ice-ufrag:t\r\n".to_vec()).unwrap()),
            None,
            "{location}"
        );
    }
}

#[test]
fn whep_player_sends_the_bearer_only_to_its_own_origin() {
    let mut p = established_player("/session/1");
    assert_eq!(
        p.terminate()
            .unwrap()
            .headers
            .get("Authorization")
            .map(|v| v.to_str().unwrap().to_string()),
        Some(format!("Bearer {TOKEN}"))
    );
    let mut p = established_player("https://evil.example.net/session/1");
    assert!(
        p.terminate()
            .unwrap()
            .headers
            .get("Authorization")
            .is_none()
    );
}

#[test]
fn debug_never_prints_the_bearer_token_or_turn_credential() {
    let c = WhipClient::new(ENDPOINT.into(), Some(TOKEN.into()));
    let p = WhepPlayer::new(ENDPOINT.into(), Some(TOKEN.into()));
    for text in [format!("{c:?}"), format!("{p:?}")] {
        assert!(!text.contains(TOKEN), "{text}");
        assert!(text.contains("redacted"), "{text}");
        assert!(text.contains(ENDPOINT), "{text}");
    }
    let ice = IceServer {
        url: "turn:turn.example.com".into(),
        username: Some("alice".into()),
        credential: Some("turn-credential-xyz".into()),
    };
    let text = format!("{ice:?}");
    assert!(!text.contains("turn-credential-xyz"), "{text}");
    assert!(text.contains("alice"), "{text}");
}

#[cfg(feature = "media")]
#[test]
fn media_transport_config_debug_hides_the_ice_passwords() {
    use webrtc_runtime::media::{MediaTransportConfig, SetupRole};
    let cfg = MediaTransportConfig {
        local_addr: "127.0.0.1:9".parse().unwrap(),
        local_ice_ufrag: "lufrag".into(),
        local_ice_pwd: "local-ice-password-123456".into(),
        remote_ice_ufrag: "rufrag".into(),
        remote_ice_pwd: "remote-ice-password-123456".into(),
        is_controlling: false,
        local_setup: SetupRole::Passive,
        remote_fingerprint: "sha-256 00".into(),
        stun_server: None,
        max_remote_candidates: 8,
    };
    let text = format!("{cfg:?}");
    assert!(!text.contains("local-ice-password"), "{text}");
    assert!(!text.contains("remote-ice-password"), "{text}");
    assert!(text.contains("lufrag"), "{text}");
}
