# dvb-vbi 0.5.0

_Released 2026-10-16._

**Breaking release.** Reserved and stuffing bits are now preserved typed fields (`LineHeader::reserved`, `WssDataField::reserved_tail`, `DataUnit::stuffing`), so parse -> serialize is byte-exact for non-conformant input; a data unit padded to the mandated `0x2C` length now parses instead of being rejected; and `DataUnit::serialize_into`/`padded_to` refuse a non-zero `stuffing` on an `Opaque` or `Stuffing` payload. The crate also builds against `broadcast-common` 10. Anything that builds these structs with literals, or matches them exhaustively, must change.

## Why this version

- **Proposed version:** 0.4.2 -> **0.5.0** (major-class: 0.x minor-epoch bump).
- **Own API/behaviour change class:** major (`LineHeader`, `WssDataField` and `DataUnit` gain public fields; stuffing handling is stricter).
- **cargo-semver-checks vs the live release (all features):** MAJOR (1 major, 0 minor lint failures: the new public fields). (Lint-based: it misses type changes of existing fields and arguments, so the CHANGELOG BREAKING list is the primary evidence.)
- **Epoch cascade:** also forced by `broadcast-common` 9 -> 10.
- **Caret epoch:** 0.4 -> 0.5; every workspace dependent must move with it.

## Dependency changes

```toml
-broadcast-common = "9.4"
+broadcast-common = "10.0"
```

## Breaking changes and migration

Struct literals no longer compile because the structs gained a field. Use the constructors, which default the new fields to the canonical values:

```rust
// before
let h = LineHeader { field_parity: true, line_offset: 9 };
// after: `reserved` defaults to 0b11 (RESERVED_PREFIX)
let h = LineHeader::new(true, 9);

// before
let wss = WssDataField { header: h, wss_data_block: 0x1234 };
// after: `reserved_tail` defaults to 0b11
let wss = WssDataField::new(h, 0x1234);

// before
let du = DataUnit { id: DataUnitId::Vps, payload: DataUnitPayload::Vps(vps) };
// after: stuffing defaults to 0
let du = DataUnit::vps(vps);        // also DataUnit::wss(..), DataUnit::stuffing(len)
```

Behaviour changes:

- Parsing: a data unit whose `data_unit_length` exceeds its typed field (EN 301 775 Table 1 trailing `stuffing_byte`s, as in a VPS/WSS/CC unit in a `data_identifier` `0x10`-`0x1F` field padded to `0x2C`) now parses, with the extra bytes recorded in `DataUnit::stuffing`. Those bytes must be `0xFF`, otherwise `Error::InvalidField`.
- Serializing: `DataUnit::serialize_into` and `DataUnit::padded_to` reject a non-zero `stuffing` on an `Opaque` or `Stuffing` payload, which own their whole body (previously `Opaque([DE, AD])` with stuffing 3 re-parsed as `Opaque([DE, AD, FF, FF, FF])` with stuffing 0).
- New, opt-in: `DataUnit::padded_to(length)` builds the fixed-length form, and `DataUnit::validate` / `DataField::validate(data_identifier)` check the strict EN 301 775 section 4.1 / 4.4.2 generation constraints. Parsing stays tolerant; validation is never run implicitly.

Manifest: `broadcast-common` 9.4 -> 10.0. `Serialize::to_bytes` was removed from `broadcast-common` 10; use `try_to_bytes`.

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
