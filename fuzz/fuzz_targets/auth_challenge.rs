#![no_main]

use broadcast_auth::{Authenticator, Credentials};
use libfuzzer_sys::fuzz_target;

// Fuzz `broadcast-auth`'s client-side `WWW-Authenticate` challenge parser
// (RFC 7235 / RFC 7616 Digest / RFC 7617 Basic / RFC 6750 Bearer) on
// arbitrary UTF-8 text. Must not panic on any challenge, however malformed.
fuzz_target!(|data: &[u8]| {
    if let Ok(s) = core::str::from_utf8(data) {
        let _ = Authenticator::from_challenge(s, Credentials::new("fuzz-user", "fuzz-pass"));
    }
});
