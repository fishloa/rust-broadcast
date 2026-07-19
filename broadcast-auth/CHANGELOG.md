# Changelog

All notable changes to `broadcast-auth` are documented here. The format
follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the
project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
