---
status: DRAFT
---
# ts-fix 0.7.0

_Released 2026-10-16._

**Breaking release.** `PcrRestamp::Interpolate` kept extrapolating from the pre-wrap rate forever after a legal 33-bit PCR wrap (`wrapping_sub % (2^33*300)` is garbage when `obs < last`); fixed via `broadcast_common::clock33::pcr_forward_distance`. PID parameters are `Pid`, and `--keep-pids` rejects PIDs above 0x1FFF.

## Why this version

- **Proposed version:** 0.6.0 -> **0.7.0** (major-class: 0.x minor-epoch bump).
- **Own API/behaviour change class:** major (PidFilter/AccessUnit take Pid).
- **cargo-semver-checks vs live 0.6.0 (all features):** MAJOR (1 major, 0 minor lint failures). (Lint-based: it misses type changes of existing fields/arguments, so the CHANGELOG BREAKING list is the primary evidence.)
- **Epoch cascade:** none needed (own breaking change). Siblings it depends on that move epoch in this wave: broadcast-common, dvb-conformance, dvb-si, mpeg-pes, mpeg-ts, scte35-splice.
- **Caret epoch:** 0.6 -> 0.7; every workspace dependent must move with it.

## Dependency changes

```toml
-broadcast-common = "9.4"
+broadcast-common = "10.0"
-dvb-conformance = "11.1"
+dvb-conformance = "12.0"
-dvb-si = "11.1"
+dvb-si = "12.0"
-mpeg-pes = "0.5"
+mpeg-pes = "0.6"
-mpeg-ts = "0.5"
+mpeg-ts = "0.6"
-scte35-splice = "3.0" # dev-dependency
+scte35-splice = "4.0" # dev-dependency
```

## Breaking changes and migration

```rust
// before
PidFilter::keep([0x100u16, 0x101]);   reconstruct_access_units(ts, &[0x100]);
// after
PidFilter::keep([Pid::new(0x100), Pid::new(0x101)]);   reconstruct_access_units(ts, &[Pid::new(0x100)]);
```

`AccessUnit` and `PcrDiscontinuity::pid` are `Pid`; `AccessUnit` is `#[non_exhaustive]` with `AccessUnit::new`.

## What changed (from CHANGELOG `[Unreleased]`)

#### Fixed
- `PcrRestamp::Interpolate` stopped adopting observed PCRs after a legal 33-bit PCR wrap: the forward distance was computed as `obs.wrapping_sub(last) % (2^33 * 300)`, which is garbage whenever `obs < last` (2^64 is not a multiple of the non-power-of-two PCR modulus), so every post-wrap observation was discarded and the output was extrapolated from the pre-wrap rate forever. The constant-rate `pcr_wrap` fixture passed only by coincidence; `tests/pcr_wrap_observed_after_wrap.rs` (post-wrap rate change) fails on the old code and passes now. Both distance computations use `broadcast_common::clock33::pcr_forward_distance` (#1206).

#### Changed (BREAKING)

- **BREAKING (#1159):** `PidFilter::keep` takes `IntoIterator<Item = Pid>`; `reconstruct_access_units(ts, &[Pid])`, `AccessUnit::pid` and `PcrDiscontinuity::pid` are `Pid`; `AccessUnit` is now `#[non_exhaustive]` with a new `AccessUnit::new` constructor. CLI: `--keep-pids` now rejects a PID above `0x1FFF` (was silently accepted and never matched).

## Merged changes since `ts-fix-v0.6.0`

- test: gate binary-spawning tests on the cli feature (required-features) (#1258)
- test: shared test-fixtures loader crate, migrate hand-written loaders, drop CRC copies (#1214) (#1215)
- clock33: shared PCR constants + modular distance; migrate 5 crates; fix ts-fix post-wrap observations (#1206) [stacked on #1196] (#1207)
- Cleanups: remove panicking Serialize::to_bytes (BREAKING, #1143) + serde default-features off in 32 no_std manifests with CI thumbv7em serde matrix (#1183) (#1185)
- Audit critical-bug verification pass: 72 issues re-verified against main; fixes for #1012 #1022 #1034 #1035 #1056 + regression tests (NO release) (#1180)
- Typed ids wave 2: TrackId/ProgramNumber + HLS naming newtypes, Pid inside dvb-si (BREAKING, NO release) (#1178)
- Typed ids wave 1: Pid newtype (mpeg-ts and all non-dvb-si APIs) + reserved-bit preservation for all dvb-si descriptors and tables (BREAKING, NO release) (#1177)
- Audit systemic wave 1: reserved-bit spec cleanup, push mux, T9 clock33, rtcp profile extensions (release set: lockstep 11.1.0 + cascade) (#1173)
- Audit remediation: unsafe quarantine, panic removal, workspace lints/deps (#1156)
