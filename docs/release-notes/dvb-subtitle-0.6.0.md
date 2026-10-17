# dvb-subtitle 0.6.0

_Released 2026-10-16._

**Breaking release.** Serializers now reject a value wider than its wire bit-field with `Error::FieldOverflow` (or the new `Error::InvalidValue` for CLUT entries) instead of masking it into a different valid-looking segment; the CamelCase constant re-exports become SCREAMING_SNAKE; and the crate builds against `broadcast-common` 10. Parsing of valid streams is unchanged. The `mpeg-pes` 0.6 bump affects only the dev-dependency used by the real-capture integration test.

## Why this version

- **Proposed version:** 0.5.0 -> **0.6.0** (major-class: 0.x minor-epoch bump).
- **Own API/behaviour change class:** major (CamelCase consts renamed SCREAMING_SNAKE; `Error::InvalidValue`; serializers reject over-range fields; `Reserved(v)` payloads returned verbatim by `to_bits`).
- **cargo-semver-checks vs the live release (all features):** MAJOR (1 major, 0 minor lint failures: the constant renames). (Lint-based: it misses type changes of existing fields and arguments, so the CHANGELOG BREAKING list is the primary evidence.)
- **Epoch cascade:** also forced by `broadcast-common` 9 -> 10 (normal dependency) and `mpeg-pes` 0.5 -> 0.6 (dev-dependency only).
- **Caret epoch:** 0.5 -> 0.6; every workspace dependent must move with it.

## Dependency changes

```toml
-broadcast-common = "9.4"
+broadcast-common = "10.0"
-mpeg-pes = "0.5" # dev-dependency
+mpeg-pes = "0.6" # dev-dependency
```

## Breaking changes and migration

Constants (rename at the import site; the values are unchanged):

```rust
// before                       // after
DataIdentifier                  DATA_IDENTIFIER
EndOfPesMarker                  END_OF_PES_MARKER
SubtitleStreamId                SUBTITLE_STREAM_ID
SyncByte                        SYNC_BYTE
```

Serializer strictness (behavioural). A value that does not fit its wire field used to be shifted or masked into a neighbouring field and serialized as a different, valid-looking segment (the #1129 class). Now:

- `ClutDefinitionSegment` serialize rejects a reduced-range Y/Cr/Cb/T value wider than its 6/4/4/2-bit slot and a `reserved` flags value outside bits `[4:1]` with the new `Error::InvalidValue`.
- Version nibbles, `reserved*` runs, `page_state`, `region_depth`, `object_coding_method` codes, pixel codes, alternative-CLUT fields and disparity fields return `Error::FieldOverflow` when over range; `Reserved(v)` payloads are returned verbatim by `to_bits`. `interval_duration` over 24 bits is rejected with `Error::FieldOverflow`. If you build segments from computed values, handle `Err` from `Serialize::serialize_into`/`try_to_bytes`.

The only manifest change is `broadcast-common` 9.4 -> 10.0 (and the dev-dependency above); `Serialize::to_bytes` was removed from `broadcast-common` 10, use `try_to_bytes`.

## What changed (from CHANGELOG `[Unreleased]`)

#### Documentation
- README and crate-root docs rewritten to the workspace documentation standard: what the crate does and does not do, a standards-supported table (edition, scope, status), feature flags, a doctested quick start, and related crates.

#### Changed (BREAKING: epoch — requires broadcast-common 10)
- Builds against new caret epochs of workspace siblings (`broadcast-common` 9 -> 10, `mpeg-pes` 0.5 -> 0.6). A caret-epoch change is major-class for this crate under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2), whatever its own API diff. `Serialize::to_bytes` was removed (use `try_to_bytes`); consumers must resolve a single `broadcast-common` 10.

#### Changed (BREAKING, #1202)
- **BREAKING (behavioural):** serializers reject a value wider than its wire bit-field with `Error::FieldOverflow` instead of shifting/masking it into a different valid-looking segment (the #1129 class): the `*_version_number` nibbles, `reserved*` runs, `page_state` / `region_level_of_compatibility` / `region_depth` / `object_coding_method` codes, the 4/2-bit region pixel codes, the alternative-CLUT parameter fields and 8/10-bit entry values, and the disparity-signalling fractional shift / reserved flags. The `Reserved(v)` payload of those code enums is now returned verbatim by `to_bits` (so the serializer's range check can see a wide value) instead of being masked. Guarded by `tests/serialize_mask_guard.rs` (lexical) and `tests/serialize_field_width.rs` (real-capture segments, one field one step over range).
- New `tests/hostility.rs`: real M6 data fields truncated at every length and mutated at every byte never panic.

#### Fixed

- **`disparity_shift_update_sequence` `interval_duration` over 24 bits is rejected (#1129)** (was `as u8` of its three bytes, silently dropping the top byte); returns `Error::FieldOverflow`.

#### Changed (BREAKING)
- The CamelCase constant re-exports are now SCREAMING_SNAKE: `DataIdentifier`
  -> `DATA_IDENTIFIER`, `EndOfPesMarker` -> `END_OF_PES_MARKER`,
  `SubtitleStreamId` -> `SUBTITLE_STREAM_ID`, `SyncByte` -> `SYNC_BYTE`
  (#1108 DS-O1).
- `ClutDefinitionSegment` serialize now rejects a reduced-range CLUT entry whose
  Y/Cr/Cb/T value is wider than its 6/4/4/2-bit slot, and a `reserved` flags
  value outside bits `[4:1]`, with the new `Error::InvalidValue`; previously the
  value was shifted into its neighbour's bits (#1108 DS-O2).

## Merged changes since `dvb-subtitle-v0.5.0`

- docs: cluster C crate READMEs + crate-root docs (standards tables, feature flags, scope, verified editions) (#1284)
- Fix the confirmed signalling/captions review findings (supersedes #1279) (#1282)
- O1: ordering hygiene — registry tables in wire-value order + ordering guard (#1238)
- Audit sweep of the nine small crates: fit_mask serializers, hostility + guard tests (#1202) [stacked on #1199] (#1203)
- Cleanups: remove panicking Serialize::to_bytes (BREAKING, #1143) + serde default-features off in 32 no_std manifests with CI thumbv7em serde matrix (#1183) (#1185)
- Systemic umbrellas T1/T2/T14: serializer width guards (28 crates), hostile-count DoS fix, secrets hygiene, origin-scoped credentials, secure ingest default (BREAKING items marked, NO release) (#1182)
- Audit per-crate rollups: verified against main, 54 commits of real fixes across ~40 crates (BREAKING items marked, NO release) (#1179)
- Audit remediation: unsafe quarantine, panic removal, workspace lints/deps (#1156)
