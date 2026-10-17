# broadcast-auth 0.5.0

_Released 2026-10-17._

**Breaking release (its own break, not an epoch cascade).** Secrets are now wiped on drop and compared in constant time (#1142). `Credentials` implements `Drop`, so a value can no longer be destructured by move; that is a source-breaking change even though the type's fields and variants are unchanged. The README was restructured at the same time (what the crate does and does not do, standards table, feature flags, a doctested quick start).

## Why this version

- **Proposed version: 0.4.0 -> 0.5.0** (major-class: 0.x minor-epoch bump).
- **Own API/behaviour change class:** major. Adding `Drop` to `Credentials` forbids moving fields out of it.
- **cargo-semver-checks vs live 0.4.0 (all features):** none flagged. It does not see an added `Drop` impl, so the CHANGELOG BREAKING entry is the primary evidence.
- **Epoch cascade:** none. `broadcast-auth` depends on no workspace sibling.
- **Caret epoch:** 0.4 -> 0.5; every workspace dependent (`rtsp-runtime`, `hls-runtime`, `multimux` and others) must move with it.

## Dependency changes

No workspace sibling requirement changes. Third-party manifest changes since 0.4.0:

```toml
+zeroize = { version = "1", default-features = false, features = ["alloc"] }
-headers = "0.4"
+headers = { workspace = true }
```

`zeroize` is new and is what wipes the secrets; `headers` now takes its version from the workspace table (same `0.4` requirement).

## Breaking changes and migration

`Credentials` implements `Drop` (it zeroizes the password or token), so Rust rejects any pattern that moves a field out of an owned value (error E0509). Borrow the fields, or clone what you need:

```rust
use broadcast_auth::Credentials;

let creds = Credentials::Basic { username: "u".into(), password: "p".into() };

// before: move the fields out (no longer compiles)
// let Credentials::Basic { username, password } = creds else { unreachable!() };

// after: borrow, then clone only what must outlive `creds`
let Credentials::Basic { username, password } = &creds else { unreachable!() };
let (username, password) = (username.clone(), password.clone());
```

Behavioural notes that need no code change: `PartialEq` on `Credentials` now compares the secret in constant time (`subtle`); `Verifier`, the Digest nonce-signing key and `SignedUrlKeySet` wipe their secrets when dropped; `Verifier`'s Basic/Bearer/Digest comparison uses `subtle::ConstantTimeEq`. A new `tests/secret_debug_guard.rs` fails the build on a derived `Debug` over a secret-named field or a hand-rolled constant-time compare.

## What changed (from CHANGELOG `[Unreleased]`)

### Documentation
- README restructured: what the crate does and does not do, a standards-supported table (edition, scope, status), feature-flag table, a doctested quick start and related crates; crate-root docs now render the README (or are kept in step with it).

### Security (secrets and identity handling, #1142)

- **BREAKING:** `Credentials` now implements `Drop` (the password/token is wiped via `zeroize`), so a value can no longer be destructured by move; clone the fields or borrow them. `PartialEq` on `Credentials` compares secrets in constant time (`subtle`).
- `Verifier`, the Digest nonce-signing key and `SignedUrlKeySet` wipe their secrets when dropped.
- New `tests/secret_debug_guard.rs`: no derived `Debug` over a secret-named field and no hand-rolled constant-time compare.

### Changed
- `Verifier`: the Basic/Bearer/Digest constant-time compare now uses `subtle::ConstantTimeEq` (one implementation, shared with `signed_url`) instead of a hand-rolled `fold` (audit r09-O1).
