---
status: DRAFT
---
# dvb-vbi 0.5.0

_Released 2026-10-16._

**Breaking release.** Reserved and stuffing data are preserved typed fields (`LineHeader::reserved`, `WssDataField::reserved_tail`, `DataUnit::stuffing`), so parse -> serialize is byte-exact for non-conformant input; a data unit padded to the mandated 0x2C length now parses.

## Why this version

- **Proposed version:** 0.4.2 -> **0.5.0** (major-class: 0.x minor-epoch bump).
- **Own API/behaviour change class:** major (LineHeader/WssDataField/DataUnit gain fields).
- **cargo-semver-checks vs live 0.4.2 (all features):** MAJOR (1 major, 0 minor lint failures). (Lint-based: it misses type changes of existing fields/arguments, so the CHANGELOG BREAKING list is the primary evidence.)
- **Epoch cascade:** none needed (own breaking change). Siblings it depends on that move epoch in this wave: broadcast-common.
- **Caret epoch:** 0.4 -> 0.5; every workspace dependent must move with it.

## Dependency changes

```toml
-broadcast-common = "9.4"
+broadcast-common = "10.0"
```

## Breaking changes and migration

```rust
// before                                         // after
LineHeader { field_parity, line_offset }          LineHeader::new(..)     // reserved defaults to 0b11
WssDataField { .. }                               WssDataField::new(..)   // reserved_tail defaults to 0b11
DataUnit { .. }                                   DataUnit::..            // stuffing defaults to 0
```

Stuffing bytes must be `0xFF` (else `Error::InvalidField`). New opt-in strict `DataUnit::validate` / `DataField::validate` / `DataUnit::padded_to` (EN 301 775 section 4.1/4.4.2).

## What changed (from CHANGELOG `[Unreleased]`)

#### Documentation
- README and crate-root docs rewritten to the workspace documentation standard: what the crate does and does not do, a standards-supported table (edition, scope, status), feature flags, a doctested quick start, and related crates.

#### Changed (BREAKING: epoch — requires broadcast-common 10)
- Builds against new caret epochs of workspace siblings (`broadcast-common` 9 -> 10). A caret-epoch change is major-class for this crate under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2), whatever its own API diff. `Serialize::to_bytes` was removed (use `try_to_bytes`); consumers must resolve a single `broadcast-common` 10.

#### Fixed (adversarial review of signalling/captions)
- **BREAKING (behavioural):** `DataUnit::serialize_into` and `DataUnit::padded_to` reject a non-zero `stuffing` on an `Opaque` or `Stuffing` payload (which own their whole `data_unit_length` body): `Opaque([DE, AD])` with stuffing 3 used to re-parse as `Opaque([DE, AD, FF, FF, FF])` with stuffing 0.

#### Changed (BREAKING)
- `LineHeader` gains `reserved: u8` (the 2-bit `reserved_future_use` prefix),
  `WssDataField` gains `reserved_tail: u8` (its trailing 2 bits) and
  `DataUnit` gains `stuffing: u8` (trailing `stuffing_byte` count): reserved
  and stuffing data are now preserved typed fields, so parse -> serialize is
  byte-exact for non-conformant input too (#1106). `LineHeader::new` /
  `WssDataField::new` default the reserved bits to `11`; `DataUnit`'s
  convenience constructors default `stuffing` to 0.
- A data unit whose `data_unit_length` exceeds its typed field (the EN 301 775
  Table 1 trailing `stuffing_byte`s, e.g. a VPS/WSS/CC unit in a
  `data_identifier` 0x10-0x1F field padded to the mandated 0x2C) now parses,
  instead of being rejected as a length mismatch (#1106 W3). The stuffing bytes
  must be `0xFF` (also for a bare `0xFF` stuffing unit), else
  `Error::InvalidField`.

#### Added
- `DataUnit::padded_to`, and `DataUnit::validate` / `DataField::validate`:
  the strict EN 301 775 §4.1 / §4.4.2 and Table 4/6/8/10/12 generation
  constraints (fixed 0x2C unit length for `data_identifier` 0x10-0x1F,
  per-unit line_offset / parity / framing_code / sample ranges, `11` reserved
  bits). Parsing stays tolerant; validate is opt-in (#1106 W3).

## Merged changes since `dvb-vbi-v0.4.2`

- docs: cluster C crate READMEs + crate-root docs (standards tables, feature flags, scope, verified editions) (#1284)
- Fix the confirmed signalling/captions review findings (supersedes #1279) (#1282)
- test: shared test-fixtures loader crate, migrate hand-written loaders, drop CRC copies (#1214) (#1215)
- Audit sweep of the nine small crates: fit_mask serializers, hostility + guard tests (#1202) [stacked on #1199] (#1203)
- Cleanups: remove panicking Serialize::to_bytes (BREAKING, #1143) + serde default-features off in 32 no_std manifests with CI thumbv7em serde matrix (#1183) (#1185)
- Systemic umbrellas T1/T2/T14: serializer width guards (28 crates), hostile-count DoS fix, secrets hygiene, origin-scoped credentials, secure ingest default (BREAKING items marked, NO release) (#1182)
- Audit per-crate rollups: verified against main, 54 commits of real fixes across ~40 crates (BREAKING items marked, NO release) (#1179)
