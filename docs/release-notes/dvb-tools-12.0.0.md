# dvb-tools 12.0.0

_Released 2026-10-17._

**Breaking release (CLI behaviour).** Lockstep major. `t2mi --inner` without `--plp` now exits non-zero when the capture carries more than one PLP (each PLP is an independent transport stream; the old behaviour interleaved them into a garbled mux), `t2mi --pid` rejects a PID above `0x1FFF`, and the `dump --json` schema grows (typed reserved-bit keys, `"trailing": []`, a typed body for descriptor tag `0x3F`), which a consumer that rejects unknown keys must accommodate. Every subcommand now streams the capture in 1 MiB reads (peak memory on a 152 MB capture: 19 MB), and a closed stdout pipe (`| head`) is a clean stop. The binary builds against `dvb-si` 12, `dvb-t2mi` 12, `mpeg-ts` 0.6 and `broadcast-common` 10.

## Why this version

- **Proposed version:** 11.1.0 -> **12.0.0** (major-class: semver major; lockstep with `dvb-si`, `dvb-t2mi`, `dvb-bbframe` and `dvb-conformance`).
- **Own API/behaviour change class:** major (`t2mi --inner` without `--plp` exits non-zero on multi-PLP captures; `t2mi --pid` validates its range; `dump --json` output grows).
- **cargo-semver-checks vs the live release (all features):** tool error (cargo-semver-checks could not analyse this binary crate). (Lint-based: it misses type changes of existing fields and arguments, so the CHANGELOG BREAKING list is the primary evidence.)
- **Epoch cascade:** none needed for its own sake; the lockstep major also moves it with `dvb-si` 11 -> 12, `dvb-t2mi` 11 -> 12, `mpeg-ts` 0.5 -> 0.6 and `broadcast-common` 9 -> 10.
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

A single-PLP capture needs no `--plp`. On a non-seekable input (a pipe via `/dev/stdin`, a FIFO, process substitution) the PLPs are recovered in a single pass with the output buffered in memory and written only if exactly one PLP was seen (regular files keep the constant-memory two-pass scan); several PLPs still exit non-zero listing them. This also fixes a case where such an input wrote zero packets and exited 0.

Scripts that passed `t2mi --pid 0x2000` (or any value above `0x1FFF`) must drop it: it is now a usage error rather than matching nothing.

`dump --json` output grows (for example `fixtures/ts/m6-single.ts` goes from 9715 to 9895 bytes): descriptors and tables now carry their reserved bits as typed `reserved_future_use*` / `reserved*` keys, list-bearing structures carry `"trailing": []`, and descriptor tag `0x3F` (`extension_descriptor`) decodes to a typed `Mpeg2Extension` body instead of `unknown`. Existing keys keep their names and values, so a consumer that ignores unknown keys is unaffected; one that rejects them must be updated. `services`, `epg`, `pids` and `t2mi` output is byte-identical except where `t2mi` now parses individual-addressing packets it previously failed on (the `dvb-t2mi` `function_length` framing fix).

Other behaviour: a closed stdout pipe (`| head`) ends `t2mi` cleanly with exit 0 instead of failing on the final flush or panicking inside `println!`; a mid-file read error is reported and exits non-zero.

## What changed (from CHANGELOG `[Unreleased]`)

#### Documentation
- README rewritten to the workspace crate-README standard: what the crate does and does not do, a standards table (edition, scope, status), feature flags, a doctested quick start, the implemented API surface, design notes and related crates. Where the crate has a library target the crate-root docs are now `include_str!("../README.md")`, so docs.rs and crates.io show the same text.

#### Changed (BREAKING: epoch — requires broadcast-common 10)
- Builds against new caret epochs of workspace siblings (`broadcast-common` 9 -> 10, `dvb-si` 11 -> 12, `dvb-t2mi` 11 -> 12, `mpeg-ts` 0.5 -> 0.6). A caret-epoch change is major-class for this crate under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2), whatever its own API diff. `Serialize::to_bytes` was removed (use `try_to_bytes`); consumers must resolve a single `broadcast-common` 10.

#### Changed
- Internal: the 27 MHz PCR constants / modular delta / `base * 300 + extension` arithmetic now come from `broadcast_common::clock33` (`PCR_HZ`, `PCR_WRAP_27MHZ`, `pcr_forward_distance`, `pcr_from_parts`, `pcr_split`) instead of a local copy; no behaviour change (#1206).

#### Fixed

- `t2mi` (event dump and `--inner`) treats a closed stdout pipe (`| head`) as a clean stop: it returns immediately with success instead of continuing through the capture and failing on the final flush, and the event dump no longer panics inside `println!` (r11-W-DT-3, #1100).
- Every subcommand now streams the capture in 1 MiB reads through the stateful TS resynchroniser instead of reading the whole file and then building a second full-size `Vec` of packets: peak memory on a 152 MB capture is 19 MB (r11-W-DT-4). A mid-file read error is reported and exits non-zero.

- `t2mi --inner` without `--plp` on a non-seekable input (a pipe via `/dev/stdin`, a FIFO, process substitution) no longer writes zero packets and exits 0: the PLP-discovery pass added by #1034 consumed the stream before the real pass. A non-regular input is now recovered in a single pass (output buffered in memory, written only if exactly one PLP was seen; several PLPs still exit non-zero listing them). Regular files keep the constant-memory two-pass scan.

#### Changed

- **BREAKING (#1034):** `t2mi --inner` without `--plp` now scans the capture first and, when it carries more than one PLP, exits non-zero listing the PLPs and asking for `--plp N` — each PLP is an independent transport stream, and the old behaviour interleaved them into one garbled mux on stdout. A single-PLP capture needs no `--plp`.
- `t2mi --pid` rejects a PID above `0x1FFF` as a usage error (was accepted and matched nothing). `dump --json` output grows (found by the differential sweep, docs/DIFFERENTIAL-SWEEP-2026-10.md; e.g. `fixtures/ts/m6-single.ts` 9715 -> 9895 bytes): descriptors and tables now carry their reserved bits as typed `reserved_future_use*` / `reserved*` keys, list-bearing structures carry `"trailing": []`, and descriptor tag 0x3F (`extension_descriptor`) decodes to a typed `Mpeg2Extension` body instead of `unknown`. Existing keys keep their names and values; a consumer that rejects unknown keys must be updated. `services`, `epg`, `pids` and `t2mi` output is byte-identical except where `t2mi` now parses individual-addressing packets it previously failed on (dvb-t2mi function_length framing fix).

## Merged changes since `v11.1.0`

- docs(dvb-tools): CHANGELOG states the dump --json schema growth found by the differential sweep (#1324)
- docs: correct false 'not vendored' / 'edition unverified' claims (#1311)
- docs: cluster A crate READMEs to the crate-README standard (what/not, standards table, features, doctested quick start, API, design notes) (#1289)
- Fix the confirmed signalling/captions review findings (supersedes #1279) (#1282)
- fix: dvb-si tables/core review defects + CI-exposed corpus canaries (supersedes #1270) (#1274)
- test: shared test-fixtures loader crate, migrate hand-written loaders, drop CRC copies (#1214) (#1215)
- clock33: shared PCR constants + modular distance; migrate 5 crates; fix ts-fix post-wrap observations (#1206) [stacked on #1196] (#1207)
- Audit critical-bug verification pass: 72 issues re-verified against main; fixes for #1012 #1022 #1034 #1035 #1056 + regression tests (NO release) (#1180)
- Audit per-crate rollups: verified against main, 54 commits of real fixes across ~40 crates (BREAKING items marked, NO release) (#1179)
- Typed ids wave 1: Pid newtype (mpeg-ts and all non-dvb-si APIs) + reserved-bit preservation for all dvb-si descriptors and tables (BREAKING, NO release) (#1177)
