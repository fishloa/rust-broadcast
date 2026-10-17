# rmt-flute 0.7.0

_Released 2026-10-17._

**Epoch-forced major-class release; no API change of its own.** `rmt-flute` follows the `broadcast-common` 9 -> 10 caret epoch. Its own change since 0.6.0 is documentation and manifest only: the README and crate-root docs are restructured (what is and is not covered, standards table RFC 5651/5775/6726/5740/5052, feature flags, design notes).

## Why this version

- **Proposed version:** 0.6.0 -> **0.7.0** (major-class: 0.x minor-epoch bump).
- **Own API/behaviour change class:** none (documentation and manifest only).
- **cargo-semver-checks vs live 0.6.0 (all features):** none flagged. (Lint-based: it misses type changes of existing fields/arguments, so the CHANGELOG BREAKING list is the primary evidence.)
- **Epoch cascade:** epoch cascade from broadcast-common. Siblings it depends on that move epoch in this wave: broadcast-common.
- **Caret epoch:** 0.6 -> 0.7; every workspace dependent must move with it.

## Dependency changes

```toml
-broadcast-common = "9.4"
+broadcast-common = "10.0"
```

## Breaking changes and migration

No `rmt-flute` type or function changed. The only source-level change you may hit is in `broadcast-common` 10:

`broadcast-common` 10 removed `Serialize::to_bytes()` (the default method that panicked when `serialize_into` failed). Code that serialises any `rmt-flute` type through it must switch to the fallible form:

```rust
// before
let bytes = value.to_bytes();
// after
let bytes = value.try_to_bytes()?;   // Result<Vec<u8>, _>
```

Your dependency graph must resolve a single `broadcast-common` 10 (a second copy on 9.x is a different `Serialize` trait and the types will not satisfy it).

`dvb-flute` and `atsc3-route` build on this crate; they move to `rmt-flute` 0.7 in the same wave.

## What changed (from CHANGELOG `[Unreleased]`)

#### Documentation
- README and crate-root docs restructured: what is / is not covered, standards table (RFC 5651/5775/6726/5740/5052), feature flags, design notes.

#### Changed (BREAKING: epoch — requires broadcast-common 10)
- Builds against new caret epochs of workspace siblings (`broadcast-common` 9 -> 10). A caret-epoch change is major-class for this crate under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2), whatever its own API diff. `Serialize::to_bytes` was removed (use `try_to_bytes`); consumers must resolve a single `broadcast-common` 10.

## Merged changes since `rmt-flute-v0.6.0`

- docs: cluster C crate READMEs + crate-root docs (standards tables, feature flags, scope, verified editions) (#1284)
- Fix the confirmed signalling/captions review findings (supersedes #1279) (#1282)
- test: shared test-fixtures loader crate, migrate hand-written loaders, drop CRC copies (#1214) (#1215)
- Cleanups: remove panicking Serialize::to_bytes (BREAKING, #1143) + serde default-features off in 32 no_std manifests with CI thumbv7em serde matrix (#1183) (#1185)
- Systemic umbrellas T1/T2/T14: serializer width guards (28 crates), hostile-count DoS fix, secrets hygiene, origin-scoped credentials, secure ingest default (BREAKING items marked, NO release) (#1182)
- Audit remediation: unsafe quarantine, panic removal, workspace lints/deps (#1156)
