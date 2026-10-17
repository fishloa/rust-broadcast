# mpeg-ps 0.6.0

_Released 2026-10-16._

**Breaking release.** `SystemHeader` gains `trailing_reserved: Vec<u8>`, preserving the bytes `header_length` declares after the stream-bound loop instead of dropping them and shrinking `header_length` on re-serialize. Serializers now reject over-range fields (`FieldOverflow`) and any `SystemHeader` that would serialize to bytes that parse back differently (new `Error::InvalidField`), where before they were masked silently. The crate also builds against `broadcast-common` 10 and `mpeg-pes` 0.6.

## Why this version

- **Proposed version:** 0.5.0 -> **0.6.0** (major-class: 0.x minor-epoch bump).
- **Own API/behaviour change class:** major (`SystemHeader.trailing_reserved` field; stricter serializer; new `Error::InvalidField`).
- **cargo-semver-checks vs the live release (all features):** MAJOR (1 major, 0 minor lint failures: the new public field). (Lint-based: it misses type changes of existing fields and arguments, so the CHANGELOG BREAKING list is the primary evidence.)
- **Epoch cascade:** also forced by `broadcast-common` 9 -> 10 and `mpeg-pes` 0.5 -> 0.6.
- **Caret epoch:** 0.5 -> 0.6; every workspace dependent must move with it.

## Dependency changes

```toml
-broadcast-common = "9.4"
+broadcast-common = "10.0"
-mpeg-pes = "0.5"
+mpeg-pes = "0.6"
```

## Breaking changes and migration

```rust
// before
SystemHeader { /* .. */ reserved_bits, stream_bounds }
// after: struct literals need the new field (empty = nothing after the loop)
SystemHeader { /* .. */ reserved_bits, stream_bounds, trailing_reserved: Vec::new() }
```

Parsed headers need no change: `trailing_reserved` is filled on parse and written back, so parse -> serialize no longer shrinks `header_length`.

Serializer strictness (these were silently masked or mis-framed before):

- `SystemHeader` `rate_bound` (22 bits), `audio_bound` (6), `video_bound` (5), `P-STD_buffer_size_bound` (13), `stream_id_extension` (7) and `reserved_bits`, and the pack-header SCR `base` (33) / `extension` (9), return `FieldOverflow` when over range.
- The stream loop (Table 2-40, `nextbits() == '1'`) must re-parse to the same value, so these are rejected with the new `Error::InvalidField`: a `stream_id` without its MSB, `0xB7` without (or a non-`0xB7` with) a `stream_id_extension`, and a `trailing_reserved` whose first byte has the MSB set (it would be re-read as a stream-bound entry). Handle `Err` when you build headers by hand.

Manifest: `broadcast-common` 9.4 -> 10.0 and `mpeg-pes` 0.5 -> 0.6. `Serialize::to_bytes` was removed from `broadcast-common` 10; use `try_to_bytes`.

## What changed (from CHANGELOG `[Unreleased]`)

#### Documentation
- README rewritten to the workspace crate-README standard: what the crate does and does not do, a standards table (edition, scope, status), feature flags, a doctested quick start, the implemented API surface, design notes and related crates. Where the crate has a library target the crate-root docs are now `include_str!("../README.md")`, so docs.rs and crates.io show the same text.

#### Changed (BREAKING: epoch — requires broadcast-common 10)
- Builds against new caret epochs of workspace siblings (`broadcast-common` 9 -> 10, `mpeg-pes` 0.5 -> 0.6). A caret-epoch change is major-class for this crate under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2), whatever its own API diff. `Serialize::to_bytes` was removed (use `try_to_bytes`); consumers must resolve a single `broadcast-common` 10.

#### Fixed
- Serialize: `SystemHeader` `rate_bound` (22 bit), `audio_bound` (6), `video_bound` (5), `P-STD_buffer_size_bound` (13), `stream_id_extension` (7) and the pack-header SCR `base` (33) / `extension` (9) were masked silently; each is now a checked `fit_bits` returning `FieldOverflow`.
- Serialize -> parse is now the identity for the stream loop (Table 2-40, `nextbits() == '1'`): a `stream_id` without its MSB, `0xB7` without / a non-`0xB7` with a `stream_id_extension`, and a `trailing_reserved` whose first byte has the MSB set (re-read as a stream-bound entry) are rejected with the new `Error::InvalidField` instead of serializing to bytes that parse back differently (#1119 review).
- Serialize: `SystemHeader` `reserved_bits` was masked silently (`& MASK`), so an over-range hand-set value was written as a different value; it is now a checked `broadcast_common::len::fit_mask` returning `FieldOverflow` (the #1129/#1197 class, found by the extended `serialize_mask_guard` of adversarial review #1268).

#### Changed
- Internal: the 27 MHz PCR constants / modular delta / `base * 300 + extension` arithmetic now come from `broadcast_common::clock33` (`PCR_HZ`, `PCR_WRAP_27MHZ`, `pcr_forward_distance`, `pcr_from_parts`, `pcr_split`) instead of a local copy; no behaviour change (#1206).
#### Changed (breaking)
- `SystemHeader` gains `trailing_reserved: Vec<u8>`: bytes `header_length` declares after the stream-bound loop are preserved and re-serialized (previously dropped, shrinking `header_length`) (audit r14-MPS-W8, #1119).

## Merged changes since `mpeg-ps-v0.5.0`

- docs: cluster A crate READMEs to the crate-README standard (what/not, standards table, features, doctested quick start, API, design notes) (#1289)
- Fix the confirmed signalling/captions review findings (supersedes #1279) (#1282)
- Fix confirmed transport/ancillary review defects (supersedes #1277) (#1280)
- Fix the confirmed defects of the dvb-si descriptor review (supersedes #1268) (#1272)
- test: shared test-fixtures loader crate, migrate hand-written loaders, drop CRC copies (#1214) (#1215)
- clock33: shared PCR constants + modular distance; migrate 5 crates; fix ts-fix post-wrap observations (#1206) [stacked on #1196] (#1207)
- Cleanups: remove panicking Serialize::to_bytes (BREAKING, #1143) + serde default-features off in 32 no_std manifests with CI thumbv7em serde matrix (#1183) (#1185)
- Systemic umbrellas T1/T2/T14: serializer width guards (28 crates), hostile-count DoS fix, secrets hygiene, origin-scoped credentials, secure ingest default (BREAKING items marked, NO release) (#1182)
- Audit per-crate rollups: verified against main, 54 commits of real fixes across ~40 crates (BREAKING items marked, NO release) (#1179)
- Audit remediation: unsafe quarantine, panic removal, workspace lints/deps (#1156)
