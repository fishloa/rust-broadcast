#![no_main]

use broadcast_auth::{RequestContext, SignedUrlKeySet, Verifier};
use libfuzzer_sys::fuzz_target;

// Fuzz `broadcast-auth`'s server-side signed-URL token parser (issue #747):
// the `exp`/`kid`/`sig`/`ip` query-string parsing, base64url decode, and the
// constant-time HMAC compare, all driven by attacker-controlled query bytes
// appended to a fixed path. Must not panic on any input, however malformed —
// a peer_addr is attached (also arbitrary-derived) so the IP-scoping path is
// exercised too.
fuzz_target!(|data: &[u8]| {
    let Ok(query) = core::str::from_utf8(data) else {
        return;
    };
    let keys = SignedUrlKeySet::new([("fuzz-kid".to_string(), vec![0x42u8; 32])]).unwrap();
    let verifier = Verifier::signed_url(keys);
    let uri = format!("/stream/media.m3u8?{query}");
    let peer: std::net::SocketAddr = "203.0.113.7:9999".parse().unwrap();
    let ctx = RequestContext::new("GET", &uri).with_peer_addr(peer);
    let _ = verifier.verify(&ctx);
});
