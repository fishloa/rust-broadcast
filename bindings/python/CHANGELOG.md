# Changelog

All notable changes to `dvb-si-py` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.5.0] - 2026-11-16

### Dependencies
- `dvb-si` 14 -> 15, `dvb-t2mi` 14 -> 15 and `mpeg-ts` 0.8 -> 0.9 (the 2026-11-15 wave, live on crates.io). The lockfile was refreshed for those three crates only (plus their transitive `broadcast-common` 12 -> 13, the new `jiff`/`jiff-static`/`jiff-core` tree, and the removal of `chrono`); the `pyo3` / `pythonize` pins are unchanged.
- `dvb-si` 15 replaced its `chrono` feature with `jiff`, so the binding now requests `features = ["ts", "serde", "jiff"]`. Nothing in the binding ever named `chrono` or read a decoded instant (`EitEvent::start_time_chrono`, now `start_time_utc`, TOT/TDT `utc_time`), so no Rust code change was needed.

### Changed
- No Python-visible change. The dict schema is the one `serde` emits, which serializes those instants through `jiff::Timestamp` exactly as `chrono` did (RFC 3339 with a `Z` suffix, e.g. `"2023-01-01T12:34:56Z"`), and they were never reachable through `parse_section` / `Demux` / `T2miDemux` output anyway (an EIT event dict still carries the raw `start_time_raw` MJD/BCD bytes). Output verified unchanged on `fixtures/ts/tnt-5w-12732v-isi6-10s.ts` (PAT/PMT/SDT/NIT and 229 EIT sections decoding 984 events); the 5 Python tests pass.

## [0.4.0] - 2026-11-08

### Dependencies
- `pyo3` 0.24 -> 0.29 and `pythonize` 0.24 -> 0.29 (abi3-py39 kept; `PyObject` is now `Py<PyAny>`). `pyo3` stays on 0.29 because `pythonize` 0.29 pins it and two `pyo3` versions cannot link; the move to 0.30 waits for a `pythonize` release that supports it.
- `dvb-si` 12 -> 13, `dvb-t2mi` 12 -> 13 and `mpeg-ts` 0.6 -> 0.7 (the 2026-11 wave, now live on crates.io).
- The bindings already request `serde` explicitly (`features = ["ts", "serde", "chrono"]`), so the `serde`-opt-in change in `dvb-si` 13 needs no code change.

### Changed
- Breaking dependency move: the bindings now build against the `dvb-si` 13 / `dvb-t2mi` 13 / `mpeg-ts` 0.7 lockstep. No binding code change was needed: the Python surface (`parse_section`, `Demux`, `T2miDemux`) is unchanged and the 5 Python tests pass.
