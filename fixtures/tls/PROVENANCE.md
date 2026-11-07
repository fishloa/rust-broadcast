# Provenance

## `localhost-cert.der` / `localhost-key.der`

Shared by `multimux`'s own
`rtsps://` (RTSP-over-TLS) loopback test (`multimux/src/source/rtsp.rs`, issue
#804). A self-signed `CN=localhost` cert/key pair `rtsp-runtime`'s own
`rtsp-runtime/tests/it/io_loopback.rs::tls_full_session_over_loopback` already uses to run a
real `tokio-rustls` TLS handshake against `127.0.0.1` loopback with server
name `"localhost"`; not secret (self-signed, loopback-only test fixture).
