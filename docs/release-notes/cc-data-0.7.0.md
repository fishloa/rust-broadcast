---
status: DRAFT
---
# cc-data 0.7.0

_Released 2026-10-16._

**Breaking release.** `CcData` keeps `process_em_data_flag` (byte 0 bit 7) and `additional_data_flag` (bit 5) instead of rejecting real ATSC A/53 captures (shaka-player `captions-test.ts`, all 599 frames carry `process_em_data_flag = 0`), and `Cea608Decoder`'s RCL no longer discards captions.

## Why this version

- **Proposed version:** 0.6.0 -> **0.7.0** (major-class: 0.x minor-epoch bump).
- **Own API/behaviour change class:** major (CcData gains fields; Error::InvalidFixedBits payload changed (semver-checks major)).
- **cargo-semver-checks vs live 0.6.0 (all features):** MAJOR (1 major, 0 minor lint failures). (Lint-based: it misses type changes of existing fields/arguments, so the CHANGELOG BREAKING list is the primary evidence.)
- **Epoch cascade:** Error::InvalidFixedBits payload changed (semver-checks major). Siblings it depends on that move epoch in this wave: broadcast-common.
- **Caret epoch:** 0.6 -> 0.7; every workspace dependent must move with it.

## Dependency changes

```toml
-broadcast-common = "9.4"
+broadcast-common = "10.0"
```

## Breaking changes and migration

```rust
// before: CcData { cc_count, em_data, entries }  (parse failed when bit 7 was clear / bit 5 set)
// after: two typed flags, Table B.9 defaults true / false when hand-built
CcData { process_em_data_flag: true, additional_data_flag: false, .. }
```

`Error::InvalidFixedBits` no longer carries `what: "reserved/zero_bit"`.

## What changed (from CHANGELOG `[Unreleased]`)

#### Changed (BREAKING)
- `CcData` gains `process_em_data_flag` (byte 0 bit 7) and `additional_data_flag` (bit 5), constructed with Table B.9's own defaults (`true` / `false`). `CcData::parse` no longer rejects a header whose bit 7 is clear or bit 5 is set: real ATSC A/53 captures (shaka-player's `captions-test.ts`, all 599 frames) carry `process_em_data_flag = 0`. The bits are preserved verbatim so the round-trip stays byte-exact. `Error::InvalidFixedBits` no longer carries `what: "reserved/zero_bit"` (#1220).

#### Fixed
- `Cea608Decoder`: RCL (Resume Caption Loading) no longer erases the non-displayed memory. It only selects pop-on style; ENM erases. Real SCC files (libcaption `tos.scc`, pycaption `example.scc`) re-issue RCL after loading text and before the EDM + EOC pair, and the decoder discarded every such caption (#1220).

#### Changed
- Internal: odd-parity encode/check comes from `broadcast_common::parity` instead of a local copy (`timed_metadata::webvtt::teletext::{encode,decode}_odd_parity` stay as thin wrappers, public API unchanged); no behaviour change (#1212).

#### Changed
- Internal: adapted to `broadcast-common`'s removal of the panicking `Serialize::to_bytes` (tests, examples and docs now use `try_to_bytes`); requires the new `broadcast-common` major (umbrella #1143).

## Merged changes since `cc-data-v0.6.0`

- captions: mux.js/shaka media (unclear licence) moved to the private submodule; tests read private/fixtures/captions (#1261) (#1262)
- tests: one fixture resolver (public -> private/fixtures -> .test-streams); move local-only captures to the private repo (#1259) (#1260)
- Real CEA-608/708 captures: fixtures, tests, and the bugs they exposed (#1220) (#1221)
- broadcast_common::parity: shared odd-parity helpers; migrate cc-data, timed-metadata, caption-convert (#1212) (#1213)
- Audit sweep of the nine small crates: fit_mask serializers, hostility + guard tests (#1202) [stacked on #1199] (#1203)
- Cleanups: remove panicking Serialize::to_bytes (BREAKING, #1143) + serde default-features off in 32 no_std manifests with CI thumbv7em serde matrix (#1183) (#1185)
- Systemic umbrellas T1/T2/T14: serializer width guards (28 crates), hostile-count DoS fix, secrets hygiene, origin-scoped credentials, secure ingest default (BREAKING items marked, NO release) (#1182)
- Audit remediation: unsafe quarantine, panic removal, workspace lints/deps (#1156)
