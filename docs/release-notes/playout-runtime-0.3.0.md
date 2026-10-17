---
status: DRAFT
---
# playout-runtime 0.3.0

_Released 2026-10-16._

**Epoch-forced major-class release; no API change of its own.** `playout-runtime` follows the `broadcast-common` 9 -> 10, `scte35-splice` 3 -> 4, `ssai-runtime` 0.2 -> 0.3 and `mp4-emsg` 0.4 -> 0.5 caret epochs. Its own change since 0.2.1 is documentation only: the README and crate-root docs are rewritten to the workspace standard (what it does and does not do, standards table: SCTE 35 2023r1 `splice_insert` only, feature table, compiling quick start).

## Why this version

- **Proposed version:** 0.2.1 -> **0.3.0** (major-class: 0.x minor-epoch bump).
- **Own API/behaviour change class:** none (documentation only).
- **cargo-semver-checks vs live 0.2.1 (all features):** none flagged. (Lint-based: it misses type changes of existing fields/arguments, so the CHANGELOG BREAKING list is the primary evidence.)
- **Epoch cascade:** epoch cascade from broadcast-common. Siblings it depends on that move epoch in this wave: broadcast-common, mp4-emsg, scte35-splice, ssai-runtime.
- **Caret epoch:** 0.2 -> 0.3; every workspace dependent must move with it.

## Dependency changes

```toml
-broadcast-common = "9.4"
+broadcast-common = "10.0"
-scte35-splice = "3.0"
+scte35-splice = "4.0"
-ssai-runtime = "0.2"
+ssai-runtime = "0.3"
-mp4-emsg = "0.4" # dev-dependency
+mp4-emsg = "0.5" # dev-dependency
```

## Breaking changes and migration

No `playout-runtime` type or function changed. What you may hit comes from the siblings:

- `broadcast-common` 10 removed `Serialize::to_bytes()` (the default method that panicked when `serialize_into` failed). Code that serialises any `playout-runtime` type through it must switch to the fallible form:

```rust
// before
let bytes = value.to_bytes();
// after
let bytes = value.try_to_bytes()?;   // Result<Vec<u8>, _>
```

Your dependency graph must resolve a single `broadcast-common` 10 (a second copy on 9.x is a different `Serialize` trait and the types will not satisfy it).
- The SCTE 35 `splice_insert()` messages it emits are built with `scte35-splice` 4.0, which now rejects over-range `sap_type`/`encryption_algorithm` with `FieldOverflow` instead of masking them (see the scte35-splice 4.0.0 note); the values `playout-runtime` writes are in range, so its output is unchanged.

## What changed (from CHANGELOG `[Unreleased]`)

#### Documentation
- README and crate-root docs rewritten to the cluster standard: what it does / does not, standards table (SCTE 35 2023r1 `splice_insert` only), feature table, compiling quick start.

#### Changed (BREAKING: epoch — requires broadcast-common 10)
- Builds against new caret epochs of workspace siblings (`broadcast-common` 9 -> 10, `mp4-emsg` 0.4 -> 0.5, `scte35-splice` 3 -> 4, `ssai-runtime` 0.2 -> 0.3). A caret-epoch change is major-class for this crate under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2), whatever its own API diff. `Serialize::to_bytes` was removed (use `try_to_bytes`); consumers must resolve a single `broadcast-common` 10.

## Merged changes since `playout-runtime-v0.2.1`

- docs: cluster C crate READMEs + crate-root docs (standards tables, feature flags, scope, verified editions) (#1284)
- Fix the confirmed signalling/captions review findings (supersedes #1279) (#1282)
- clock33: shared PCR constants + modular distance; migrate 5 crates; fix ts-fix post-wrap observations (#1206) [stacked on #1196] (#1207)
- Audit sweep of the nine small crates: fit_mask serializers, hostility + guard tests (#1202) [stacked on #1199] (#1203)
- Cleanups: remove panicking Serialize::to_bytes (BREAKING, #1143) + serde default-features off in 32 no_std manifests with CI thumbv7em serde matrix (#1183) (#1185)
