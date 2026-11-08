# Changelog

All notable changes to `dvb-si-py` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.4.0] - 2026-11-08

### Dependencies
- `pyo3` 0.24 -> 0.29 and `pythonize` 0.24 -> 0.29 (abi3-py39 kept; `PyObject` is now `Py<PyAny>`). `pyo3` stays on 0.29 because `pythonize` 0.29 pins it and two `pyo3` versions cannot link; the move to 0.30 waits for a `pythonize` release that supports it.
- `dvb-si` 12 -> 13, `dvb-t2mi` 12 -> 13 and `mpeg-ts` 0.6 -> 0.7 (the 2026-11 wave, now live on crates.io).
- The bindings already request `serde` explicitly (`features = ["ts", "serde", "chrono"]`), so the `serde`-opt-in change in `dvb-si` 13 needs no code change.

### Changed
- Breaking dependency move: the bindings now build against the `dvb-si` 13 / `dvb-t2mi` 13 / `mpeg-ts` 0.7 lockstep. No binding code change was needed: the Python surface (`parse_section`, `Demux`, `T2miDemux`) is unchanged and the 5 Python tests pass.
