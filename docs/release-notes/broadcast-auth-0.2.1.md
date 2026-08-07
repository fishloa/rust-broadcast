# broadcast-auth 0.2.1

**Release date:** 2026-08-02

Adds HMAC-SHA256 signed-URL output auth and `#[non_exhaustive]` drift-guard on public enums.

## What's new

- `SignedUrl` verifier — HMAC-SHA256 query-string token verification with configurable expiry. Used by `multimux` 0.6's signed-URL output auth.
- `#[non_exhaustive]` on `AuthScheme`, `AuthError`.
- `label_coverage` drift-guard test.

## Migration

No breaking changes (`#[non_exhaustive]` is additive for match-with-wildcard consumers).
