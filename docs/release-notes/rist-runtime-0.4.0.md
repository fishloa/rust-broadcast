---
status: DRAFT
---
# rist-runtime 0.4.0

_Released 2026-10-16._

**Epoch-forced major-class release.** `GenericNack`/`RangeNack` honour the RFC 3550 section 6.4.1 P bit (a P-bit NACK inside a compound used to fail with `BufferTooShort`), `RttEcho::parse` rejects a set P bit, and `RttEstimator::update` saturates.

## Why this version

- **Proposed version:** 0.3.0 -> **0.4.0** (major-class: 0.x minor-epoch bump).
- **Own API/behaviour change class:** minor (additive non_exhaustive Error::PaddingBitSet + NACK fixes).
- **cargo-semver-checks vs live 0.3.0 (all features):** none flagged. (Lint-based: it misses type changes of existing fields/arguments, so the CHANGELOG BREAKING list is the primary evidence.)
- **Epoch cascade:** epoch cascade from broadcast-common. Siblings it depends on that move epoch in this wave: broadcast-common, rtcp-packet.
- **Caret epoch:** 0.3 -> 0.4; every workspace dependent must move with it.

## Dependency changes

```toml
-broadcast-common = "9.4"
+broadcast-common = "10.0"
-rtcp-packet = "0.5"
+rtcp-packet = "0.6"
```

## Breaking changes and migration

`Error::PaddingBitSet` is added (`#[non_exhaustive]`, so a wildcard arm already covers it). The major is the `broadcast-common` epoch cascade; `RttEcho::parse` now rejects input it used to silently normalise.

## What changed (from CHANGELOG `[Unreleased]`)

#### Documentation
- README restructured: what the crate does and does not do, a standards-supported table (edition, scope, status), feature-flag table, a doctested quick start and related crates; crate-root docs now render the README (or are kept in step with it).

#### Changed (BREAKING: epoch — requires broadcast-common 10)
- Builds against new caret epochs of workspace siblings (`broadcast-common` 9 -> 10, `rtcp-packet` 0.5 -> 0.6). A caret-epoch change is major-class for this crate under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2), whatever its own API diff. `Serialize::to_bytes` was removed (use `try_to_bytes`); consumers must resolve a single `broadcast-common` 10.

#### Changed (breaking)
- `GenericNack` and `RangeNack` gain a `padding: Vec<u8>` field (RFC 3550 §6.4.1 padding octets without the count byte): a padded standalone NACK now round-trips byte-identically (P bit, count and fill preserved) instead of re-serializing with `P = 0` and the padding gone. A padding count that leaves a partial FCI / range entry is rejected with `InvalidPaddingCount` instead of silently dropping the orphan bytes. Struct literals must add `padding: Vec::new()`.
- `Error` gains `PaddingBitSet` (`#[non_exhaustive]`, so additive for matchers): `RttEcho::parse` rejects a set P bit (TR-06-1 §5.2.6 fixes `P = 0`; its padding is the message's own field) instead of silently clearing it on re-serialize (#1086/RIST-W5).

#### Fixed
- `GenericNack::parse` / `RangeNack::parse` honour the RFC 3550 §6.4.1 P bit: padding is validated and stripped (a zero or over-long count is `InvalidPaddingCount`), so padding words no longer parse as FCI / range entries, and a P-bit NACK inside a compound no longer fails with `BufferTooShort` (#1086/RIST-W5).
- `arq::rtt::RttEstimator::update` saturates instead of overflowing on an extreme sample (#1086/RIST-O3).

## Merged changes since `rist-runtime-v0.3.0`

- docs: clear per-crate documentation for cluster B (16 crates) (#1291)
- Fix the confirmed signalling/captions review findings (supersedes #1279) (#1282)
- Fix confirmed transport/ancillary review defects (supersedes #1277) (#1280)
- test: shared test-fixtures loader crate, migrate hand-written loaders, drop CRC copies (#1214) (#1215)
- Cleanups: remove panicking Serialize::to_bytes (BREAKING, #1143) + serde default-features off in 32 no_std manifests with CI thumbv7em serde matrix (#1183) (#1185)
- Systemic umbrellas T1/T2/T14: serializer width guards (28 crates), hostile-count DoS fix, secrets hygiene, origin-scoped credentials, secure ingest default (BREAKING items marked, NO release) (#1182)
- Audit per-crate rollups: verified against main, 54 commits of real fixes across ~40 crates (BREAKING items marked, NO release) (#1179)
