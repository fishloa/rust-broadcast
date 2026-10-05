# broadcast-auth 0.4.0

_Released 2026-10-05._

### Changed

- Dependency bumps, non-breaking (no public API change): `hmac` 0.13, `sha2` 0.11, `md-5` 0.11 (RustCrypto 0.13 generation) and `base64` 0.23. Digest MD5 responses and HMAC-SHA256 signed URLs are byte-identical; new tests pin the RFC 2617 §3.5 and RFC 7617 §2 examples.
- `lru` for the Digest nonce-count table, `hex` for nonces (no behaviour change).
- The configured realm is stored as it is rendered (control characters other than HTAB dropped), so a realm containing CR/LF now authenticates a Digest client; Bearer tokens are compared trimmed on both sides (a configured token with surrounding spaces verifies, as before).

### Fixed
- `Verifier::verify` now matches the `Basic`/`Digest`/`Bearer` auth-scheme
  token case-insensitively (RFC 7235 §2.1: `auth-scheme` is a `token`), and
  the Digest field-list parser now treats a `"…"` quoted field value as
  opaque rather than splitting a literal comma inside it (#1087).
- `check_digest` now rejects a request whose `qop` is not `auth` or whose
  `algorithm` (when present) is not `MD5` — previously neither field's value
  was checked before being hashed into the response formula, so a request
  that mislabeled either but still matched this server's fixed `qop=auth`/
  MD5 hash construction was accepted (#1087).
- `WWW-Authenticate` realm is rendered as an escaped quoted-string (a `"`, `\`, CR or LF in the realm can no longer break or split the header); Bearer header injection (above); a Digest `username` with `"`/`\` now verifies. HTAB is kept in the realm (legal `qdtext`, RFC 9110 §5.6.4).

### Changed (breaking)
- Signed-URL query is `application/x-www-form-urlencoded`: `kid` (defect 6) and `ip` are percent-encoded (`ip=2001:db8::1` -> `ip=2001%3Adb8%3A%3A1`; verification percent-decodes, so a `+` in a URL minted by an older version now reads as a space). A bare `ip` key (no `=`) is rejected instead of ignored.
- Digest `Authorization` is parsed with `http-auth`'s `ChallengeParser`: quoted-pairs are honoured, parameter names are case-insensitive, a repeated parameter or a second challenge is rejected, and a raw non-ASCII `username` is rejected — a non-ASCII user authenticates through the now-supported RFC 7616 §3.4.4 `username*` (RFC 8187 `UTF-8''<pct-encoded>`; other charsets, malformed percent-encoding, invalid UTF-8, and `username` together with `username*` are rejected); `userhash=true` is rejected.
- Basic/Bearer use `headers::Authorization`: a Basic user-id containing `:` can no longer match (RFC 7617 §2); non-UTF-8 payloads are `Unauthorized`.
- `Error::InvalidBearerToken` (new, `#[non_exhaustive]`): a Bearer token that cannot be a header value is refused instead of emitted.
- `digest-uri` match requires the client's absolute-form `uri` in normalised spelling (`url::Url` round-trip) and an exact path+query match: a client that hashed `http://H:80/x` or `rtsp://[2001:DB8::1]/x` (upper-case host/IPv6 literal, default port) was accepted before and now gets a 401; send the normalised spelling. This is the substitution guard (`http://h/a/../b`, `%2e%2e`, `HTTP://` stay rejected).

### Added
- Digest `username*` support (above); new dependencies `percent-encoding`, `url`, `form_urlencoded`, `headers`, `lru`, `hex`.

---

Published from tag `broadcast-auth-v0.4.0`.
