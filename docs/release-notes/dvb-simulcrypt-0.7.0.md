---
status: DRAFT
---
# dvb-simulcrypt 0.7.0

_Released 2026-10-16._

**Epoch-cascade release; no source migration.** `dvb-simulcrypt` 0.7.0 changes no code and no public API. It ships because it depends on `broadcast-common`, which moves from 9 to 10 in this wave, and the workspace epoch-purity rule forbids publishing a crate that still requires the old `broadcast-common` epoch next to the new one. Its own changes since 0.6.0 are documentation: the protocol_version registry in `docs/` now lists ACG-EIS (`0x04`) and SIMCOMP-MUXCONFIG (`0x04`), and the README and crate-root docs follow the workspace crate-documentation standard.

## Why this version

- **Proposed version:** 0.6.0 -> **0.7.0** (major-class: 0.x minor-epoch bump).
- **Own API/behaviour change class:** none (documentation only).
- **cargo-semver-checks vs the live release (all features):** none flagged. (Lint-based: it misses type changes of existing fields and arguments, so the CHANGELOG BREAKING list is the primary evidence.)
- **Epoch cascade:** forced by `broadcast-common` 9 -> 10, the only workspace sibling it depends on that moves epoch in this wave.
- **Caret epoch:** 0.6 -> 0.7; every workspace dependent must move with it.

## Dependency changes

```toml
-broadcast-common = "9.4"
+broadcast-common = "10.0"
```

## Breaking changes and migration

None for source code. Change `dvb-simulcrypt = "0.6"` to `dvb-simulcrypt = "0.7"`. If your own crate also depends on `broadcast-common`, move it to `10` so the graph resolves a single `broadcast-common` epoch; `Serialize::to_bytes` was removed from `broadcast-common` 10 (use `try_to_bytes`).

## What changed (from CHANGELOG `[Unreleased]`)

#### Documentation
- `docs/`: the protocol_version registry now lists ACG-EIS (`0x04`, TS 103 197 §11.5) and SIMCOMP-MUXCONFIG (`0x04`, §12.3), both verified on the rendered pages (PDF pp.166, 177); the "not transcribed" notes are removed. The crate models only ECMG-SCS, EMMG/PDG-MUX and C(P)SIG-(P)SIG; no code change.
- README and crate-root docs rewritten to the workspace documentation standard: what the crate does and does not do, a standards-supported table (edition, scope, status), feature flags, a doctested quick start, and related crates.

#### Changed (BREAKING: epoch — requires broadcast-common 10)
- Builds against new caret epochs of workspace siblings (`broadcast-common` 9 -> 10). A caret-epoch change is major-class for this crate under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2), whatever its own API diff. `Serialize::to_bytes` was removed (use `try_to_bytes`); consumers must resolve a single `broadcast-common` 10.

## Merged changes since `dvb-simulcrypt-v0.6.0`

- feat(dvb-si): type structures the vendored specs define (AIT providers, 0x15, ext 0x10/0x13, quality AU body) (#1314)
- docs: cluster C crate READMEs + crate-root docs (standards tables, feature flags, scope, verified editions) (#1284)
- Fix the confirmed signalling/captions review findings (supersedes #1279) (#1282)
- test: shared test-fixtures loader crate, migrate hand-written loaders, drop CRC copies (#1214) (#1215)
- Cleanups: remove panicking Serialize::to_bytes (BREAKING, #1143) + serde default-features off in 32 no_std manifests with CI thumbv7em serde matrix (#1183) (#1185)
- Systemic umbrellas T1/T2/T14: serializer width guards (28 crates), hostile-count DoS fix, secrets hygiene, origin-scoped credentials, secure ingest default (BREAKING items marked, NO release) (#1182)
- Audit remediation: unsafe quarantine, panic removal, workspace lints/deps (#1156)
