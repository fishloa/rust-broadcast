---
status: DRAFT
---
# compliance-probe 0.4.0

_Released 2026-10-16._

**Breaking release, mostly from the epoch cascade plus one public field type change.** `PcrSample::pid` is now `mpeg_ts::pid::Pid` instead of a bare integer (#1159), and the crate moves to the new caret epochs of `broadcast-common`, `dvb-conformance`, `media-doctor`, `media-plane`, `mpeg-ts`, `scte35-splice` and `timed-metadata`. It also fixes the PCR drift estimate across a PCR wrap: the inter-PCR delta used `wrapping_sub(..) % (2^33*300)`, which is wrong whenever the PCR wraps or steps backward, so a steady PCR crossing the roughly 26.5 hour wrap is no longer discarded as a discontinuity and keeps about 0 ppm drift.

## Why this version

- **Proposed version: 0.3.0 -> 0.4.0** (major-class: 0.x minor-epoch bump).
- **Own API/behaviour change class:** `PcrSample::pid` changes its public field type (source-breaking for code that reads or constructs it); the PCR drift fix is a behaviour correction.
- **cargo-semver-checks vs live 0.3.0 (all features):** none flagged. It does not see a changed field type, so the CHANGELOG is the primary evidence.
- **Epoch cascade:** from `broadcast-common`, `dvb-conformance`, `media-doctor`, `media-plane`, `mpeg-ts`, `scte35-splice` and `timed-metadata`, all of which move epoch in this wave.
- **Caret epoch:** 0.3 -> 0.4; every workspace dependent must move with it.

## Dependency changes

```toml
-broadcast-common = { path = "../broadcast-common", version = "9.4",  default-features = false }
+broadcast-common = { path = "../broadcast-common", version = "10.0", default-features = false }
-dvb-conformance  = { path = "../dvb-conformance",  version = "11.1", default-features = false }
+dvb-conformance  = { path = "../dvb-conformance",  version = "12.0", default-features = false }
-mpeg-ts          = { path = "../mpeg-ts",          version = "0.5",  default-features = false }
+mpeg-ts          = { path = "../mpeg-ts",          version = "0.6",  default-features = false }
-scte35-splice    = { path = "../scte35-splice",    version = "3.0",  default-features = false }
+scte35-splice    = { path = "../scte35-splice",    version = "4.0",  default-features = false }
-timed-metadata   = { path = "../timed-metadata",   version = "0.6",  default-features = false }
+timed-metadata   = { path = "../timed-metadata",   version = "0.7",  default-features = false }
-media-doctor     = { path = "../media-doctor",     version = "0.10", default-features = false }
+media-doctor     = { path = "../media-doctor",     version = "0.11", default-features = false }
-media-plane      = { path = "../media-plane",      version = "0.6",  default-features = false }
+media-plane      = { path = "../media-plane",      version = "0.7",  default-features = false }
```

Dev-dependencies gained the path-only `test-fixtures` (not published).

## Breaking changes and migration

`PcrSample::pid` is now `mpeg_ts::pid::Pid` (a 13-bit PID newtype; `Pid::new(u16)` builds one, masking to the low 13 bits, `Pid::try_new` rejects out-of-range values, and `.value()` returns the integer). Convert at the boundary:

```rust
use mpeg_ts::pid::Pid;

// before: sample.pid was an integer
// let pid: u16 = sample.pid;
// after
let pid: u16 = sample.pid.value();
// comparing against a known PID
let is_video = sample.pid == Pid::new(0x0100);
```

The crate must also resolve the new sibling epochs listed above; in particular a `dvb-conformance` 12 `ConformanceEvent` carries its PID as `Option<Pid>`.

## What changed (from CHANGELOG `[Unreleased]`)

### Documentation
- README rewritten to the workspace crate-README standard: what the crate does and does not do, a standards table (edition, scope, status), feature flags, a doctested quick start, the implemented API surface, design notes and related crates. Where the crate has a library target the crate-root docs are now `include_str!("../README.md")`, so docs.rs and crates.io show the same text.

### Changed (BREAKING: epoch — requires broadcast-common 10)
- Builds against new caret epochs of workspace siblings (`broadcast-common` 9 -> 10, `dvb-conformance` 11 -> 12, `media-doctor` 0.10 -> 0.11, `media-plane` 0.6 -> 0.7, `mpeg-ts` 0.5 -> 0.6, `scte35-splice` 3 -> 4, `timed-metadata` 0.6 -> 0.7). A caret-epoch change is major-class for this crate under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2), whatever its own API diff. `Serialize::to_bytes` was removed (use `try_to_bytes`); consumers must resolve a single `broadcast-common` 10.

### Changed
- Internal: the 27 MHz PCR constants / modular delta / `base * 300 + extension` arithmetic now come from `broadcast_common::clock33` (`PCR_HZ`, `PCR_WRAP_27MHZ`, `pcr_forward_distance`, `pcr_from_parts`, `pcr_split`) instead of a local copy; no behaviour change (#1206).

### Fixed

- PCR drift estimate: the inter-PCR delta used `u64::wrapping_sub(..) % (2^33*300)`, which is wrong whenever the PCR wraps (or steps backward) because 2^64 is not a multiple of the PCR modulus; the delta is now the true modular forward distance, so a steady PCR crossing the ~26.5 h wrap keeps ~0 ppm drift instead of being discarded as a discontinuity.

### Changed

- `PcrSample::pid` is `mpeg_ts::pid::Pid` (#1159); the `TsHeader::pid` type change is absorbed internally.
