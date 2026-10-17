---
status: DRAFT
---
# cc-data 0.7.0

_Released 2026-10-16._

**Breaking release (its own break, plus the `broadcast-common` 10 epoch).** `CcData` gains two public fields, `process_em_data_flag` (byte 0 bit 7) and `additional_data_flag` (bit 5), and `CcData::parse` now keeps those bits instead of rejecting real ATSC A/53 captures (shaka-player's `captions-test.ts` carries `process_em_data_flag = 0` on all 599 frames). Adding public fields breaks struct-literal construction, and `Error::InvalidFixedBits` no longer carries `what: "reserved/zero_bit"`. `Cea608Decoder`'s RCL no longer discards captions that real SCC files re-issue before EDM/EOC. README and crate-root docs were restructured (scope, standards table, feature table, compiled quick-start doctests).

## Why this version

- **Proposed version: 0.6.0 -> 0.7.0** (major-class: 0.x minor-epoch bump).
- **Own API/behaviour change class:** major. New public fields on `CcData`; a removed `InvalidFixedBits` payload value.
- **cargo-semver-checks vs live 0.6.0 (all features):** MAJOR (1 major, 0 minor lint failures), consistent with the CHANGELOG BREAKING entry.
- **Epoch cascade:** also from `broadcast-common` (9 -> 10).
- **Caret epoch:** 0.6 -> 0.7; every workspace dependent (`timed-metadata`, `caption-convert`, ...) must move with it.

## Dependency changes

```toml
-broadcast-common = { path = "../broadcast-common", version = "9.4",  default-features = false }
+broadcast-common = { path = "../broadcast-common", version = "10.0", default-features = false }
```

The optional `serde` dependency is now `default-features = false` with `features = ["derive", "alloc"]`, and `std` forwards `serde?/std`.

## Breaking changes and migration

`CcData` is `{ process_em_data_flag, process_cc_data_flag, additional_data_flag, reserved_byte1, triplets }`. Construct the two new fields with Table B.9's own defaults (`true` / `false`) to keep the previous wire output:

```rust
// before
let cc = CcData { process_cc_data_flag: true, reserved_byte1: 0xFF, triplets };
// after
let cc = CcData {
    process_em_data_flag: true,   // byte 0 bit 7, Table B.9 default
    process_cc_data_flag: true,
    additional_data_flag: false,  // byte 0 bit 5, Table B.9 default
    reserved_byte1: 0xFF,
    triplets,
};
```

Parsing: a header whose bit 7 is clear or bit 5 is set is now accepted and the bits are preserved verbatim, so parse then serialize stays byte-exact. `Error::InvalidFixedBits` is no longer returned for those two bits (it still reports other fixed-bit failures); drop any handling that matched on `what == "reserved/zero_bit"`.

## What changed (from CHANGELOG `[Unreleased]`)

### Documentation
- README and crate-root docs restructured: scope, standards table (TS 101 154 V2.6.1 vendored; CTA-608-E / 708-E stated as not vendored), feature table, compiled quick-start doctests.

### Changed (BREAKING: epoch — requires broadcast-common 10)
- Builds against new caret epochs of workspace siblings (`broadcast-common` 9 -> 10). A caret-epoch change is major-class for this crate under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2), whatever its own API diff. `Serialize::to_bytes` was removed (use `try_to_bytes`); consumers must resolve a single `broadcast-common` 10.

### Changed (BREAKING)
- `CcData` gains `process_em_data_flag` (byte 0 bit 7) and `additional_data_flag` (bit 5), constructed with Table B.9's own defaults (`true` / `false`). `CcData::parse` no longer rejects a header whose bit 7 is clear or bit 5 is set: real ATSC A/53 captures (shaka-player's `captions-test.ts`, all 599 frames) carry `process_em_data_flag = 0`. The bits are preserved verbatim so the round-trip stays byte-exact. `Error::InvalidFixedBits` no longer carries `what: "reserved/zero_bit"` (#1220).

### Fixed
- `Cea608Decoder`: RCL (Resume Caption Loading) no longer erases the non-displayed memory. It only selects pop-on style; ENM erases. Real SCC files (libcaption `tos.scc`, pycaption `example.scc`) re-issue RCL after loading text and before the EDM + EOC pair, and the decoder discarded every such caption (#1220).

### Changed
- Internal: odd-parity encode/check comes from `broadcast_common::parity` instead of a local copy (`timed_metadata::webvtt::teletext::{encode,decode}_odd_parity` stay as thin wrappers, public API unchanged); no behaviour change (#1212).
