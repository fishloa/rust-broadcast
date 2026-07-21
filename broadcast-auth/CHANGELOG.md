# Changelog

All notable changes to `broadcast-auth` are documented here. The format
follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the
project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0] - 2026-07-21

### Added
- **`Verifier::forwarded` — reverse-proxy forwarded-auth scheme** (issue #663
  extensibility wave part 1): trusts that a fronting reverse proxy has
  already authenticated the caller and forwards the authenticated username
  in a configured header (conventionally `X-Forwarded-User`) — authenticated
  iff that header is present and non-empty. Unlike Basic/Digest/Bearer this
  scheme has no `Credentials`/challenge-response round-trip at all;
  [`Verifier::challenge`] just names the scheme (`"Forwarded"`) for
  diagnostics. `Verifier::forwarded_for` reads back a second configured
  header (conventionally `X-Forwarded-For`) for tracing/observability only —
  no trust decision is made from it. **Safe ONLY behind a trusted reverse
  proxy that strips any client-supplied copies of both headers before
  forwarding** — see the `server` module docs' trust-assumption note.

### Changed (breaking)
- **`RequestContext` gained `headers`/`peer_addr`.** `headers: &[(&str,
  &str)]` (every request header, looked up case-insensitively via the new
  `RequestContext::header`) and `peer_addr: Option<SocketAddr>` (the
  transport peer), attached via the new `with_headers`/`with_peer_addr`
  builders. `RequestContext::new` defaults both to empty/`None`, so every
  existing 2-arg call site (client-side `respond`/`Authenticator` use) is
  unaffected. This is what lets a server-side `Verifier` scheme see beyond
  `Authorization` — the mechanism `Verifier::forwarded` needs.
- **`Verifier::verify` now takes `&RequestContext` instead of
  `(Option<&str>, &str, &str)`.** Basic/Digest/Bearer verification is
  unchanged in outcome — they now read the `Authorization` header out of
  `ctx.headers` instead of taking it as a direct argument. Every in-tree
  caller (this crate's own tests, `multimux`'s output-auth gate) is updated;
  external callers must build a `RequestContext` (`RequestContext::new(method,
  uri).with_headers(headers)`) instead of passing the header directly.
- **`Credentials`/`AuthSpec`/`OutputAuthSpec`-adjacent enums are
  `#[non_exhaustive]` where not already** (see `multimux`'s changelog for
  its own enums) — a future scheme addition is now non-breaking. Internal
  matches across this crate and its consumers were already/are now
  exhaustive with no wildcard needed (all matched from within the crate that
  defines them); this only affects external crates matching on these types.

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
- **`RequestContext`'s `Debug` no longer leaks the `Authorization`/
  `Proxy-Authorization` header value.** Pre-release audit finding: the
  derived `Debug` printed every header verbatim, including a Basic
  credential's reversible base64 `user:pass` — reachable through a bare
  `tracing::debug!(?ctx, ...)` call. Replaced with a manual `Debug` (the same
  redaction pattern as `Credentials`) that renders the value of any header
  case-insensitively named `authorization`/`proxy-authorization` as
  `"<redacted>"`; every other field/header renders normally.
- **`Verifier::verify`'s Digest field parse is now capped at 64 fields.**
  Pre-release audit finding: `verify_digest` split the `Authorization`
  header's fields into a `HashMap` with no bound, so a pathologically large
  header forced an unbounded per-request allocation. A real Digest response
  carries under 15 fields; a header with more than `MAX_DIGEST_FIELDS` (64)
  comma-separated fields is now rejected (`AuthResult::Unauthorized`) before
  the map is built. Not a substitute for a transport-level header-size cap,
  which callers should also enforce.

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
