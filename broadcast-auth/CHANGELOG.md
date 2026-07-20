# Changelog

All notable changes to `broadcast-auth` are documented here. The format
follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the
project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Security
- **`Credentials`'s `Debug` no longer leaks the raw password/token.** Pre-
  release audit finding: `#[derive(Debug)]` printed `Basic`/`Digest`'s
  `password` and `Bearer`'s `token` verbatim under `{:?}` — reachable through
  any embedding struct that also derives `Debug` (e.g.
  `ll-hls-runtime::client::tokio_client::TokioClientConfig`,
  `rtsp-runtime::client::ClientSession`). Replaced with a manual `Debug` (the
  same pattern already used for `Authenticator`/`Verifier` one file over)
  that shows the username (not secret) for `Basic`/`Digest` and redacts
  `password`/`token` as `"***"`.

### Added
- Initial release: a shared, scheme-agnostic `Credentials` model (`Basic` /
  `Digest` / `Bearer`, `#[non_exhaustive]`) and a challenge->response helper
  (`respond`, and the stateful `Authenticator` for session reuse) — Basic and
  Digest delegate to `http-auth` (RFC 7617/7616; RTSP's reuse per RFC 2326
  §14/§16), Bearer needs no challenge round-trip (RFC 6750). Extracted from
  `rtsp-runtime::auth` so RTSP and (future) HTTP clients share one auth
  implementation instead of duplicating it (#663 multimux-hub P3b).
- `tests/label_coverage.rs` #204 drift-guard (`Error`/`Credentials`
  SKIP-listed — see the test's doc comment for why `Credentials` has no
  `Display`).
- **`server::Verifier` — the server-side challenge+verify half** (issue #663
  "shared output auth", `docs/superpowers/specs/2026-07-18-multimux-hub-design.md`):
  built from a configured `Credentials` + realm, `Verifier::challenge()`
  renders the `WWW-Authenticate` value for a `401` (Basic/Digest realm+nonce,
  Bearer the bare `"Bearer"` token) and `Verifier::verify(authorization,
  method, uri) -> AuthResult` checks an incoming `Authorization` header.
  Promoted from `multimux::testutil`'s test-only mock auth server (which
  already did a real, independent RFC 7616 §3.4.1 Digest computation rather
  than a literal-string match) into this shared crate, so it is now the
  *production* verifier multimux's output-auth middleware uses, not a
  test-only fixture duplicated elsewhere. Basic/Bearer compare in constant
  time; Digest recomputes `HA1`/`HA2`/`response` (`qop=auth`/`algorithm=MD5`)
  and also checks the client's claimed `uri` against the actual request URI.
  A `Digest`-scheme `Verifier` generates one random server nonce at
  construction and holds it for its lifetime — see the module doc's
  replay caveat. New `base64`/`md-5`/`rand` dependencies (all already
  transitive via `http-auth` at the same versions — no new lock entries).
