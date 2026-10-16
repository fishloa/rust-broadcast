---
status: DRAFT
---
# dvb-tools 12.0.0

_Released 2026-10-16._

**Breaking release.** Lockstep major (CLI behaviour). `t2mi --inner` without `--plp` now exits non-zero when the capture carries more than one PLP (each PLP is an independent transport stream; the old behaviour interleaved them into a garbled mux), `t2mi --pid` rejects PIDs above 0x1FFF, every subcommand streams the capture in 1 MiB reads (peak memory on a 152 MB capture: 19 MB), and a closed stdout pipe (`| head`) is a clean stop.

## Why this version

- **Proposed version:** 11.1.0 -> **12.0.0** (major-class: semver major).
- **Own API/behaviour change class:** major (t2mi --inner without --plp now exits non-zero on multi-PLP captures (CLI behaviour)).
- **cargo-semver-checks vs live 11.1.0 (all features):** tool error. (Lint-based: it misses type changes of existing fields/arguments, so the CHANGELOG BREAKING list is the primary evidence.)
- **Epoch cascade:** none needed (own breaking change). Siblings it depends on that move epoch in this wave: broadcast-common, dvb-si, dvb-t2mi, mpeg-ts.
- **Caret epoch:** 11 -> 12; every workspace dependent must move with it.

## Dependency changes

```toml
-broadcast-common = "9.4"
+broadcast-common = "10.0"
-dvb-si = "11.1"
+dvb-si = "12.0"
-dvb-t2mi = "11.1"
+dvb-t2mi = "12.0"
-mpeg-ts = "0.5"
+mpeg-ts = "0.6"
```

## Breaking changes and migration

```text
# before: silently interleaved every PLP into one garbled stream on stdout
dvb-tools t2mi capture.ts --inner > out.ts
# after: exits non-zero listing the PLPs; select one explicitly
dvb-tools t2mi capture.ts --inner --plp 0 > out.ts
```

Scripts that parsed `t2mi --pid 0x2000` output must drop the PID; it is now a usage error.

## What changed (from CHANGELOG `[Unreleased]`)

#### Changed
- Internal: the 27 MHz PCR constants / modular delta / `base * 300 + extension` arithmetic now come from `broadcast_common::clock33` (`PCR_HZ`, `PCR_WRAP_27MHZ`, `pcr_forward_distance`, `pcr_from_parts`, `pcr_split`) instead of a local copy; no behaviour change (#1206).

#### Fixed

- `t2mi` (event dump and `--inner`) treats a closed stdout pipe (`| head`) as a clean stop: it returns immediately with success instead of continuing through the capture and failing on the final flush, and the event dump no longer panics inside `println!` (r11-W-DT-3, #1100).
- Every subcommand now streams the capture in 1 MiB reads through the stateful TS resynchroniser instead of reading the whole file and then building a second full-size `Vec` of packets: peak memory on a 152 MB capture is 19 MB (r11-W-DT-4). A mid-file read error is reported and exits non-zero.

#### Changed

- **BREAKING (#1034):** `t2mi --inner` without `--plp` now scans the capture first and, when it carries more than one PLP, exits non-zero listing the PLPs and asking for `--plp N` — each PLP is an independent transport stream, and the old behaviour interleaved them into one garbled mux on stdout. A single-PLP capture needs no `--plp`.
- `t2mi --pid` rejects a PID above `0x1FFF` as a usage error (was accepted and matched nothing). All other text/JSON output is byte-identical.

## Merged changes since `v11.1.0`

- test: shared test-fixtures loader crate, migrate hand-written loaders, drop CRC copies (#1214) (#1215)
- clock33: shared PCR constants + modular distance; migrate 5 crates; fix ts-fix post-wrap observations (#1206) [stacked on #1196] (#1207)
- Audit critical-bug verification pass: 72 issues re-verified against main; fixes for #1012 #1022 #1034 #1035 #1056 + regression tests (NO release) (#1180)
- Audit per-crate rollups: verified against main, 54 commits of real fixes across ~40 crates (BREAKING items marked, NO release) (#1179)
- Typed ids wave 1: Pid newtype (mpeg-ts and all non-dvb-si APIs) + reserved-bit preservation for all dvb-si descriptors and tables (BREAKING, NO release) (#1177)
