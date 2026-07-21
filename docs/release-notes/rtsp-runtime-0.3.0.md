# rtsp-runtime 0.3.0 — 2026-07-21

Additive minor: authentication is now shared with `multimux`'s HTTP input
adapters via the new `broadcast-auth` crate, plus previously-unreleased
async-socket/TLS hardening (issue #663, the multimux-hub epic).

## Highlights

- **`auth` now delegates to `broadcast-auth`.** `rtsp_runtime::Credentials`/
  `Authenticator` are re-exported from the shared
  [`broadcast-auth`](../../broadcast-auth) crate instead of wrapping
  `http-auth` directly — a refactor with no behaviour change for existing
  callers (same `Credentials::new(user, pass)` API, same transparent-401-
  retry behaviour), extracted so RTSP and HTTP clients (`multimux`'s
  TS-over-HTTP/HLS-pull input adapters) share one auth implementation
  instead of duplicating it. Adds `Credentials::bearer(token)` (RFC 6750),
  previously unsupported.
- **`rtsps://` over TLS + the async socket adapter.** `io::AsyncRtspClient`/
  `io::AsyncRtspServer` (feature `tokio`) drive the sans-IO engine over a
  real `tokio::net::TcpStream`; `connect_tls`/`accept_tls` (feature `tls`)
  wrap it in `tokio-rustls` (default port 322). The TLS client config now
  explicitly selects the `aws-lc-rs` crypto provider
  (`rustls::crypto::aws_lc_rs::default_provider()`) rather than relying on
  `ClientConfig::builder()`'s process-global default — another crate in the
  same build (e.g. a `reqwest` pulling `aws-lc-rs`) could otherwise leave no
  unambiguous default and panic. New `connect_tls_with` lets a caller supply
  a pre-configured `ClientSession` (e.g. one already carrying
  `Credentials`) to the TLS connect path.
- **Security fix (pre-release audit):** `ClientSession`'s derived `Debug`
  embeds `Option<Credentials>` directly — it now correctly inherits
  `Credentials`'s redacting `Debug` rather than any risk of a raw secret
  leaking through, with a regression test guarding it.
- `tests/label_coverage.rs` #204 drift-guard added; named
  `DEFAULT_SESSION_SEED` and field-mutation round-trip bites for
  `Transport`/`InterleavedFrame`.

## Compatibility

- Breaking (internal only): `auth::Credentials`/`Authenticator` are now
  re-exports of `broadcast_auth` types rather than locally-defined ones —
  the public API surface (`Credentials::new`, `Authenticator::from_challenge`,
  `authorization`) is unchanged for any existing caller; a caller building a
  `RequestContext` directly (new, optional) constructs it via
  `broadcast_auth::RequestContext`.
- MSRV unchanged (1.86). New dependency: `broadcast-auth` (path, `0.1`),
  replacing the direct `http-auth` dependency (still pulled in transitively
  by `broadcast-auth`).
