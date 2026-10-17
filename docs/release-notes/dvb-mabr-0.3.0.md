# dvb-mabr 0.3.0

_Released 2026-10-16._

**Epoch-cascade release; no source migration.** `dvb-mabr` 0.3.0 changes no public API. It ships because it depends on `broadcast-common`, which moves from 9 to 10 in this wave, and the workspace epoch-purity rule forbids publishing a crate that still requires the old `broadcast-common` epoch next to the new one. The only own change is internal: optional XML attributes are read through `StartTag::attr_owned` (no behaviour change) and the manifest picks up the workspace `quick-xml` and lint tables.

## Why this version

- **Proposed version:** 0.2.0 -> **0.3.0** (major-class: 0.x minor-epoch bump).
- **Own API/behaviour change class:** none (internal refactor of optional-attribute reads and manifest tidy-up; parsed values and every error path are identical).
- **cargo-semver-checks vs live 0.2.0 (all features):** none flagged. (Lint-based: it misses type changes of existing fields/arguments, so the CHANGELOG BREAKING list is the primary evidence.)
- **Epoch cascade:** forced by `broadcast-common` 9 -> 10, the only workspace sibling it depends on that moves epoch in this wave.
- **Caret epoch:** 0.2 -> 0.3; every workspace dependent must move with it.

## Dependency changes

```toml
-broadcast-common = "9.4"
+broadcast-common = "10.0"
```

## Breaking changes and migration

None for source code. Change `dvb-mabr = "0.2"` to `dvb-mabr = "0.3"`. If your own crate also depends on `broadcast-common`, move it to `10` so the dependency graph resolves a single `broadcast-common` epoch; note that `Serialize::to_bytes` was removed from `broadcast-common` 10 (use `try_to_bytes`), which matters only if you call it on `dvb-mabr` types.

## What changed (from CHANGELOG `[Unreleased]`)

#### Documentation
- README and crate-root docs rewritten to the workspace documentation standard: what the crate does and does not do, a standards-supported table (edition, scope, status), feature flags, a doctested quick start, and related crates.

#### Changed (BREAKING: epoch — requires broadcast-common 10)
- Builds against new caret epochs of workspace siblings (`broadcast-common` 9 -> 10). A caret-epoch change is major-class for this crate under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2), whatever its own API diff. `Serialize::to_bytes` was removed (use `try_to_bytes`); consumers must resolve a single `broadcast-common` 10.

#### Changed
- Internal: optional attributes are read with `StartTag::attr_owned` instead of building a `require_attr` error and discarding it with `.ok()` (no behaviour change; the parsed values and every error path are identical). Manifest: `quick-xml` and the lint table now come from the workspace (`[lints] workspace = true`), `test-fixtures` is a dev-dependency.

## Merged changes since `dvb-mabr-v0.2.0`

- docs: correct false 'not vendored' / 'edition unverified' claims (#1311)
- docs: cluster C crate READMEs + crate-root docs (standards tables, feature flags, scope, verified editions) (#1284)
- Fix the confirmed signalling/captions review findings (supersedes #1279) (#1282)
- chore: pre-release hygiene (README coverage, snippet tool, docsrs, examples, CLAUDE.md) (#1278)
- docs: 2026-10 wave release preparation (plan, audit, draft notes, downstream migration); dvb-mabr CHANGELOG entry (#1275)
- test: shared test-fixtures loader crate, migrate hand-written loaders, drop CRC copies (#1214) (#1215)
- Cleanups: remove panicking Serialize::to_bytes (BREAKING, #1143) + serde default-features off in 32 no_std manifests with CI thumbv7em serde matrix (#1183) (#1185)
- Audit per-crate rollups: verified against main, 54 commits of real fixes across ~40 crates (BREAKING items marked, NO release) (#1179)
- Audit remediation: unsafe quarantine, panic removal, workspace lints/deps (#1156)
