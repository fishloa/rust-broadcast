# dvb-conformance 12.0.0

_Released 2026-10-16._

**Breaking release (its own break, lockstep major).** `ConformanceEvent::pid` is now `Option<mpeg_ts::pid::Pid>` instead of `Option<u16>` (serde output is unchanged: `Pid` serializes as the number). The release also corrects TR 101 290 indicator 3.3 (`Buffer_error`), which raised false errors on legal streams: the T-STD `TBsys` model is now fed per TS packet and only by PSI (PAT, CAT, PMT, NIT), and the report is latched per overflow episode. Other changes: the 27 MHz PCR arithmetic moved to `broadcast_common::clock33`, and the per-PID continuity state no longer allocates per packet (about 9% less time per packet on the 2.4 MB T2-MI fixture, release build). `dvb-conformance` is one of the five lockstep crates, so it takes the shared 12.0.0 version.

## Why this version

- **Proposed version: 11.1.0 -> 12.0.0** (major-class: semver major; lockstep with `dvb-si`, `dvb-t2mi`, `dvb-bbframe`, `dvb-tools`).
- **Own API/behaviour change class:** major. `ConformanceEvent::pid` changes its field type; indicator 3.3 results change (fewer false positives).
- **cargo-semver-checks vs live 11.1.0 (all features):** none flagged. It does not see a changed field type, so the CHANGELOG BREAKING entry is the primary evidence.
- **Epoch cascade:** also from `broadcast-common` (9 -> 10), `dvb-si` (11 -> 12) and `mpeg-ts` (0.5 -> 0.6).
- **Caret epoch:** 11 -> 12; every workspace dependent (`compliance-probe`, `dvb-tools`, `media-doctor` and others) must move with it.

## Dependency changes

```toml
-broadcast-common = { path = "../broadcast-common", version = "9.4",  default-features = false }
+broadcast-common = { path = "../broadcast-common", version = "10.0", default-features = false }
-dvb-si           = { path = "../dvb-si",           version = "11.1", default-features = false, features = ["ts"] }
+dvb-si           = { path = "../dvb-si",           version = "12.0", default-features = false, features = ["ts"] }
-mpeg-ts          = { path = "../mpeg-ts",          version = "0.5",  default-features = false }
+mpeg-ts          = { path = "../mpeg-ts",          version = "0.6",  default-features = false }
# [dev-dependencies]
-broadcast-common = { path = "../broadcast-common", version = "9.3" }
+broadcast-common = { path = "../broadcast-common", version = "10.0" }
+test-fixtures    = { path = "../test-fixtures" }   # path-only, not published
```

The optional `serde` dependency is now `default-features = false` with `features = ["derive", "alloc"]`, and `std` forwards `serde?/std`.

## Breaking changes and migration

`ConformanceEvent::pid` is `Option<Pid>` where `Pid` is `mpeg_ts::pid::Pid`. `Pid` implements `UpperHex`/`LowerHex`/`Display`, so formatting code keeps working; use `.value()` where you need the `u16`, and `Pid::new(u16)` to build one for a comparison:

```rust
use mpeg_ts::pid::Pid;

// before: event.pid: Option<u16>
// if event.pid == Some(0x0012) { .. }
// after: event.pid: Option<Pid>
if event.pid == Some(Pid::new(0x0012)) { /* EIT PID */ }
if let Some(pid) = event.pid {
    println!("{pid:04X}");              // still formats
    let raw: u16 = pid.value();         // the integer, when you need it
}
```

JSON/serde consumers see no change: `Pid` is transparent, so the field is still a number.

Behaviour change that needs no code change but alters results: indicator 3.3 (`Buffer_error`) no longer fires on a legally paced PSI/SI section over 512 bytes, nor on back-to-back DVB SI packets (SDT/BAT, EIT, RST, TDT/TOT); it is now raised once per overflow episode instead of once per packet while the buffer stays full. If you suppressed or baselined indicator 3.3 events on real streams, re-check those baselines.

## What changed (from CHANGELOG `[Unreleased]`)

### Documentation
- README rewritten to the workspace crate-README standard: what the crate does and does not do, a standards table (edition, scope, status), feature flags, a doctested quick start, the implemented API surface, design notes and related crates. Where the crate has a library target the crate-root docs are now `include_str!("../README.md")`, so docs.rs and crates.io show the same text.

### Changed (BREAKING: epoch — requires broadcast-common 10)
- Builds against new caret epochs of workspace siblings (`broadcast-common` 9 -> 10, `dvb-si` 11 -> 12, `mpeg-ts` 0.5 -> 0.6). A caret-epoch change is major-class for this crate under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2), whatever its own API diff. `Serialize::to_bytes` was removed (use `try_to_bytes`); consumers must resolve a single `broadcast-common` 10.

### Changed
- Internal: the 27 MHz PCR constants / modular delta / `base * 300 + extension` arithmetic now come from `broadcast_common::clock33` (`PCR_HZ`, `PCR_WRAP_27MHZ`, `pcr_forward_distance`, `pcr_from_parts`, `pcr_split`) instead of a local copy; no behaviour change (#1206).

### Changed
- The per-PID continuity-counter state reuses its `last_packet` buffer instead
  of allocating a fresh `Vec` on every payload-bearing packet (~9% less time
  per packet on the 2.4 MB T2-MI fixture, release build; r11-O-CONF-1, #1096).

### Changed (BREAKING)

- **BREAKING (#1159):** `ConformanceEvent::pid` is `Option<mpeg_ts::pid::Pid>` instead of `Option<u16>` (serde output unchanged: `Pid` is transparent).

### Fixed

- TR 101 290 indicator 3.3 (`Buffer_error`): the T-STD `TBsys` model fed a
  whole reassembled PSI/SI section's byte length in one instant, at
  section-completion time, so any legally-paced section over 512 bytes
  (routine for SDT/NIT/EIT) raised a false `Buffer_error` regardless of how
  the bytes were actually spread across TS packets. `TBsys` is now fed per
  TS packet, as each packet's payload physically arrives (ISO/IEC 13818-1
  §2.4.2.3), so it drains correctly between packets instead of receiving
  the whole section at once (#1035).
- Indicator 3.3 (`Buffer_error`), second half of #1035: TBsys is now fed only
  by PSI (PAT, CAT, PMT, NIT — H.222.0 §2.4.2.1 "system information for the
  program being decoded"), not by DVB SI (SDT/BAT, EIT, RST, TDT/TOT), whose
  back-to-back packets overflowed it seven times in 20 s of a real conformant
  multiplex (france-tnt-uhf32, PCR-timed; PID 0x0012). The report is also
  latched per overflow episode (re-armed when TBsys drains empty) instead of
  one event per packet while the buffer stays full.
