---
status: DRAFT
---
# broadcast-auth 0.5.0

_Released 2026-10-16._

**Breaking release.** Secrets are wiped on drop and compared in constant time (#1142). `Credentials` now implements `Drop`, so a value can no longer be destructured by move.

## Why this version

- **Proposed version:** 0.4.0 -> **0.5.0** (major-class: 0.x minor-epoch bump).
- **Own API/behaviour change class:** major (Credentials gains Drop (cannot be destructured by move)).
- **cargo-semver-checks vs live 0.4.0 (all features):** none flagged. (Lint-based: it misses type changes of existing fields/arguments, so the CHANGELOG BREAKING list is the primary evidence.)
- **Epoch cascade:** none needed (own breaking change). Siblings it depends on that move epoch in this wave: none.
- **Caret epoch:** 0.4 -> 0.5; every workspace dependent must move with it.

## Dependency changes

No workspace sibling requirement changes.

## Breaking changes and migration

```rust
// before
let Credentials::Basic { username, password } = creds;      // move out
// after: borrow or clone (Credentials implements Drop and zeroizes the password/token)
let Credentials::Basic { username, password } = &creds;
let password = password.clone();
```

`PartialEq` on `Credentials` is constant-time (`subtle`); `Verifier`, the Digest nonce-signing key and `SignedUrlKeySet` wipe their secrets when dropped. New `zeroize` dependency; new `tests/secret_debug_guard.rs`.

## What changed (from CHANGELOG `[Unreleased]`)

#### Security (secrets and identity handling, #1142)

- **BREAKING:** `Credentials` now implements `Drop` (the password/token is wiped via `zeroize`), so a value can no longer be destructured by move; clone the fields or borrow them. `PartialEq` on `Credentials` compares secrets in constant time (`subtle`).
- `Verifier`, the Digest nonce-signing key and `SignedUrlKeySet` wipe their secrets when dropped.
- New `tests/secret_debug_guard.rs`: no derived `Debug` over a secret-named field and no hand-rolled constant-time compare.

#### Changed
- `Verifier`: the Basic/Bearer/Digest constant-time compare now uses `subtle::ConstantTimeEq` (one implementation, shared with `signed_url`) instead of a hand-rolled `fold` (audit r09-O1).

## Merged changes since `broadcast-auth-v0.4.0`

- Systemic umbrellas T1/T2/T14: serializer width guards (28 crates), hostile-count DoS fix, secrets hygiene, origin-scoped credentials, secure ingest default (BREAKING items marked, NO release) (#1182)
- Audit per-crate rollups: verified against main, 54 commits of real fixes across ~40 crates (BREAKING items marked, NO release) (#1179)
- Audit remediation: unsafe quarantine, panic removal, workspace lints/deps (#1156)
