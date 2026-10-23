#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]
#![cfg_attr(docsrs, feature(doc_cfg))]

mod authenticator;
pub mod challenge;
mod credentials;
mod error;
mod request;
mod server;
pub mod signed_url;

pub use authenticator::{Authenticator, respond};
pub use challenge::{BearerError, Challenge, Scheme, parse_challenges};
pub use credentials::Credentials;
pub use error::{Error, Result};
pub use request::RequestContext;
pub use server::{AuthResult, DIGEST_NC_TRACK_CAP, DIGEST_NONCE_LIFETIME, NC_WINDOW, Verifier};
pub use signed_url::SignedUrlKeySet;

#[cfg(test)]
mod tests {
    use super::*;

    // The exact nonce rtsp-runtime's own digest test vector uses
    // (`rtsp-runtime/tests/io_loopback.rs::digest_auth_over_loopback` and
    // `rtsp-runtime/src/auth.rs` unit tests), reused here so both crates are
    // known to agree on the same wire bytes.
    const DIGEST_CHALLENGE: &str = "Digest realm=\"IP Camera\",\
        nonce=\"dcd98b7102dd2f0e8b11d0f600bfb0c093\",qop=\"auth\",algorithm=MD5";

    #[test]
    fn basic_header_value_is_base64_of_user_colon_pass() {
        let value = respond(
            "Basic realm=\"IP Camera\"",
            &RequestContext::new("DESCRIBE", "rtsp://c/live"),
            Credentials::new("admin", "12345"),
        )
        .unwrap();
        // base64("admin:12345")
        assert_eq!(value, "Basic YWRtaW46MTIzNDU=");
    }

    #[test]
    fn digest_response_matches_known_challenge_shape() {
        let mut auth =
            Authenticator::from_challenge(DIGEST_CHALLENGE, Credentials::new("admin", "12345"))
                .unwrap();
        let value = auth
            .authorization(&RequestContext::new(
                "DESCRIBE",
                "rtsp://camera.example.com/live",
            ))
            .unwrap();
        assert!(value.starts_with("Digest "), "got: {value}");
        for needle in ["response=", "realm=", "nonce=", "uri=", "cnonce=", "nc="] {
            assert!(value.contains(needle), "missing {needle} in {value}");
        }
        assert!(value.contains("uri=\"rtsp://camera.example.com/live\""));
        assert!(value.contains("nonce=\"dcd98b7102dd2f0e8b11d0f600bfb0c093\""));
    }

    #[test]
    fn digest_nc_advances_across_calls_on_the_same_authenticator() {
        let mut auth =
            Authenticator::from_challenge(DIGEST_CHALLENGE, Credentials::new("admin", "12345"))
                .unwrap();
        let ctx = RequestContext::new("DESCRIBE", "rtsp://camera.example.com/live");
        let first = auth.authorization(&ctx).unwrap();
        let second = auth.authorization(&ctx).unwrap();
        assert_ne!(first, second, "nc=00000001 vs nc=00000002 must differ");
        assert!(first.contains("nc=00000001"), "got: {first}");
        assert!(second.contains("nc=00000002"), "got: {second}");
    }

    #[test]
    fn bearer_header_value_is_bearer_token_no_challenge_needed() {
        let mut auth =
            Authenticator::from_challenge("", Credentials::bearer("mytoken123")).unwrap();
        let value = auth
            .authorization(&RequestContext::new("GET", "/stream"))
            .unwrap();
        assert_eq!(value, "Bearer mytoken123");
    }

    #[test]
    fn bearer_via_one_shot_respond_ignores_challenge_value() {
        let value = respond(
            "Basic realm=\"irrelevant, bearer never challenges\"",
            &RequestContext::new("GET", "/stream"),
            Credentials::bearer("t"),
        )
        .unwrap();
        assert_eq!(value, "Bearer t");
    }

    #[test]
    fn credentials_new_is_answered_as_whichever_scheme_the_challenge_advertises() {
        let creds = Credentials::new("admin", "12345");
        let basic = respond(
            "Basic realm=\"r\"",
            &RequestContext::new("GET", "/x"),
            creds.clone(),
        )
        .unwrap();
        assert!(basic.starts_with("Basic "));

        let digest = respond(DIGEST_CHALLENGE, &RequestContext::new("GET", "/x"), creds).unwrap();
        assert!(digest.starts_with("Digest "));
    }

    #[test]
    fn unparseable_challenge_is_a_structured_error() {
        let err = respond(
            "not a real challenge",
            &RequestContext::new("GET", "/x"),
            Credentials::new("u", "p"),
        )
        .unwrap_err();
        assert!(matches!(err, Error::ChallengeParse(_)));
    }
}
