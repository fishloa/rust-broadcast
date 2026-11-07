---
status: DRAFT
---

# broadcast-auth 0.5.2

_Released 2026-11-07._

Patch release: new helpers for URL userinfo and constant-time comparison, one new error variant pair, and the `rand` dependency is dropped. Nothing is removed; the `Error` enum is `#[non_exhaustive]`, so the two new variants are not a breaking change.

Version: 0.5.1 -> 0.5.2.

## Migration notes

### Migration

No migration is needed. `Error` is `#[non_exhaustive]`; a match that already has a wildcard arm keeps compiling.

## Added

- `credentials_from_url` / `strip_userinfo` (and `Error::InvalidUserinfo` / `Error::UserinfoNotRemovable`): decode a URL's percent-encoded userinfo into `Credentials` and remove it, shared by `multimux` instead of three private copies.
- Public `constant_time_eq`, the single constant-time comparison used by the Basic/Digest/Bearer verifiers, credentials and signed URLs.

## Other changes

- The Digest server nonce-signing secret is filled with `getrandom::fill` instead of `rand::random`; the `rand` dependency is dropped.

Internal changes with no public effect (shared helpers replacing private copies, README as crate-root docs, `Clone`/`PartialEq`/`Eq` derives on error enums where every field allows it) are listed in the CHANGELOG.

## Dependencies

No workspace sibling requirement changes epoch.

---

Published from tag `broadcast-auth-v0.5.2`.
