# broadcast-auth 0.5.1

_Released 2026-11-01._

### Changed
- Dependency `lru` 0.18 -> 0.19.
- Dependency `rand` 0.8 -> 0.10 (internal use only: the per-verifier Digest nonce-signing secret; CSPRNG `rand::random()` / `ThreadRng` unchanged). No public API exposure.

### Added
- Typed `WWW-Authenticate` challenge parsing (`challenge::parse_challenges`): `Scheme`, `BearerError`, `Challenge` with RFC 6750 fields and RFC 9728 `resource_metadata` as a validated `url::Url`.
- `challenge::ParsedChallenges` and `challenge::MAX_CHALLENGES` (16); `Error::TooManyChallenges`.

### Added (continued: behaviour of the new `challenge` module)
- `challenge::parse_challenges` returns `ParsedChallenges { challenges, errors }`: a malformed challenge, a duplicate parameter or a bad `resource_metadata` no longer discards the valid challenges around it. More than 16 challenges in one call records `Error::TooManyChallenges` and ignores the rest. The `token68` form (`Bearer abc123==`) stays unsupported by `http-auth` and is reported in `errors`.
- A `resource_metadata` must be an absolute `https` URL (RFC 9728 §3); `http:`, `file:` and `javascript:` are `Error::InvalidResourceMetadata`.

---

Published from tag `broadcast-auth-v0.5.1`.
