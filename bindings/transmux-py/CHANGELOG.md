# Changelog

All notable changes to `transmux-py` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.6.0] - 2026-11-08

### Dependencies
- `pyo3` 0.24 -> 0.30 (abi3-py39 kept; `PyObject` is now `Py<PyAny>`).
- `transmux` 0.29 -> 0.30 and `broadcast-common` 10 -> 11 (the 2026-11 wave, now live on crates.io).
- `transmux` 0.30 makes `serde` opt-in. These bindings convert the `Media`/`Track`/`Sample` IR to `PyDict`s field by field and never used `serde`, so no feature change is needed.

### Changed
- Breaking dependency move to `transmux` 0.30. No binding code change was needed: `demux_ts` and its `Media`/`Track`/`Sample` dict shape are unchanged.
