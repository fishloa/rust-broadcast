#![no_main]

use broadcast_auth::{Credentials, RequestContext, Verifier};
use libfuzzer_sys::fuzz_target;

// Fuzz `broadcast-auth`'s server-side Digest `Authorization` header parser:
// RFC 7235 auth-param parsing via `http-auth`'s `ChallengeParser`, quoted-pair
// unescaping, the RFC 7616 §3.4.4 `username*` (RFC 8187 ext-value) decoder, and
// the digest-uri normalisation check, all driven by an attacker-controlled
// header value. Must not panic on any input, however malformed.
fuzz_target!(|data: &[u8]| {
    let Ok(header) = core::str::from_utf8(data) else {
        return;
    };
    let verifier = Verifier::new(
        Credentials::Digest {
            username: "admin".into(),
            password: "pw".into(),
        },
        "r",
    );
    let headers = [("authorization", header)];
    let _ = verifier.verify(&RequestContext::new("GET", "/x").with_headers(&headers));
});
