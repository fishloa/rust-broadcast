# broadcast-auth 0.9.0

_Released 2026-11-22._

Minor for a 0.x crate, breaking: the server verifier's clock now reads `jiff::Timestamp`, which is the same instant type the signed-URL expiry gate uses. Hex, base64 and `form_urlencoded` now go through the workspace's shared codecs, and the error types derive `thiserror::Error` with unchanged messages.

## Breaking changes

- `Verifier::with_clock` takes `impl Fn() -> jiff::Timestamp` (was `Fn() -> std::time::SystemTime`). Migration: a caller that returned `UNIX_EPOCH + Duration` builds `jiff::Timestamp::from_second(secs)` instead (#3957).
- Builds against `broadcast-common` 14 (the `jiff` 1.0 epoch). A caret-epoch change is major-class under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2); a consumer must resolve a single `broadcast-common` 14 and a single `jiff` 1.x.

## Changed

- Hex and base64 go through `broadcast_common::{hex, base64}` and `form_urlencoded` through `url::form_urlencoded`; the direct `hex`, `base64` and `form_urlencoded` dependencies are gone. Output is byte-identical, pinned by an independent HMAC vector for `SignedUrlKeySet::sign` (#3957).
- Internal: error types derive `thiserror::Error` (messages unchanged) (#3974, #4020).

## Tests

The wire golden's `GOLDEN_BLESS` switch refuses to write under `CI`, and its PROVENANCE names the `http-auth` round trip as the independent oracle. `AuthState`'s 401/407 decision table, the cookie jar's `Secure`/prefix/path rules and caps, the Digest nonce-count window and field rejections, the signed-URL expiry gate against the real clock, and secret-free `Debug` impls each have a test that fails if the rule is dropped (audit 21/22 findings).

## Pre-release audit

Issues fixed in this crate by the 2026-11-22 pre-release audit: #4020 (`thiserror`), plus #4008/#3992's epoch truthfulness in the CHANGELOG.

---

Published from tag `broadcast-auth-v0.9.0`.
