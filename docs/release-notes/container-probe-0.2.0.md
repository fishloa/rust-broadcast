# container-probe 0.2.0

_Released 2026-10-16._

**Epoch-forced major-class release; no source migration, one behaviour correction.** `container-probe` has no public API change. It ships as 0.2.0 because it builds against `broadcast-common` 10 and its versioned dev-dependencies (`mpeg-ts`, `mpeg-ps`, `st377-1`) move epoch, and the workspace's epoch-purity rule forbids publishing a crate that still requires the old epoch next to the new one. Its own change: a prefix too short for a literal magic that already contradicts it is now reported as "stop" for that prober instead of "read more", which changes what a caller's read loop is told for such input (details below). README and crate-root docs were rewritten to the workspace standard.

## Why this version

- **Proposed version: 0.1.1 -> 0.2.0** (major-class: 0.x minor-epoch bump).
- **Own API/behaviour change class:** patch. The verdict correction alters the `need_at_least` value returned for some prefixes (see below) but no signature or type.
- **cargo-semver-checks vs live 0.1.1 (all features):** none flagged. It is lint-based and cannot see behavioural changes, so the CHANGELOG is the primary evidence.
- **Epoch cascade:** from `broadcast-common` (9 -> 10), plus the dev-dependencies `mpeg-ps`, `mpeg-ts` and `st377-1`.
- **Caret epoch:** 0.1 -> 0.2; every workspace dependent must move with it.

## Dependency changes

```toml
-broadcast-common = { path = "../broadcast-common", version = "9.4",  default-features = false }
+broadcast-common = { path = "../broadcast-common", version = "10.0", default-features = false }
# [dev-dependencies]
-mpeg-ts = { path = "../mpeg-ts", version = "0.5" }
+mpeg-ts = { path = "../mpeg-ts", version = "0.6" }
-mpeg-ps = { path = "../mpeg-ps", version = "0.5" }
+mpeg-ps = { path = "../mpeg-ps", version = "0.6" }
-st377-1 = { path = "../st377-1", version = "0.4" }
+st377-1 = { path = "../st377-1", version = "0.5" }
+test-alloc = { path = "../test-alloc" }   # path-only, not published
```

## Breaking changes and migration

No source-level migration. Bump the requirement to `container-probe = "0.2"` and make sure your dependency graph resolves a single `broadcast-common` 10.

One behavioural difference to know about when you drive the documented read-more loop: for very short prefixes that already contradict a literal magic (for example `XY` against `OggS`, `FLV`, the ASF GUID, the MXF key, the program-stream start code, the EBML magic or `RIFF`..`WAVE`), that prober now answers "stop" (`None`) rather than `Insufficient`. A genuinely truncated prefix of the same magic still answers `Insufficient`. The visible consequence: eight zero bytes now ask for the TS lattice's structural need (1504 bytes) instead of the geometric floor (13). Code that already feeds `need_at_least` back into its read size needs no change.

## What changed (from CHANGELOG `[Unreleased]`)

### Documentation
- README rewritten to the workspace crate-README standard: what the crate does and does not do, a standards table (edition, scope, status), feature flags, a doctested quick start, the implemented API surface, design notes and related crates. Where the crate has a library target the crate-root docs are now `include_str!("../README.md")`, so docs.rs and crates.io show the same text.

### Changed (BREAKING: epoch — requires broadcast-common 10)
- Builds against new caret epochs of workspace siblings (`broadcast-common` 9 -> 10, `mpeg-ps` 0.5 -> 0.6, `mpeg-ts` 0.5 -> 0.6, `st377-1` 0.4 -> 0.5). A caret-epoch change is major-class for this crate under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2), whatever its own API diff. `Serialize::to_bytes` was removed (use `try_to_bytes`); consumers must resolve a single `broadcast-common` 10.

### Changed
- A prefix too short for a literal magic that already contradicts it (`XY` for `OggS`, `FLV`, ASF GUID, MXF key, PS start code, EBML magic, `RIFF`..`WAVE`) is now `None` ("stop") for that prober instead of `Insufficient` ("read more"); a genuine truncated prefix of the same magic stays `Insufficient`. Consequence: eight zero bytes now ask for the TS lattice's structural need (1504) instead of the geometric floor (13) (audit r01-O6, #1076).
- The ADTS and MP3 chain walkers are one generic `chain::longest_chain` (r01-O5); the crate docs' allocation claim is now accurate and pinned by `tests/alloc_bound.rs` (r01-O7).
