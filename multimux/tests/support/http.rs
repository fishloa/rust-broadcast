//! The HTTP client the multimux library builds for its pulls
//! (`source::client_host::http_client`), for test clients: ring-backed TLS with
//! the platform trust roots, so no process-default rustls provider is needed.

use rustls_platform_verifier::BuilderVerifierExt;

pub fn client_builder() -> reqwest::ClientBuilder {
    let provider = std::sync::Arc::new(rustls::crypto::ring::default_provider());
    let tls = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .expect("ring provider supports the safe default protocol versions")
        .with_platform_verifier()
        .expect("platform verifier")
        .with_no_client_auth();
    reqwest::Client::builder().use_preconfigured_tls(tls)
}

pub fn client() -> reqwest::Client {
    client_builder().build().expect("reqwest client")
}
