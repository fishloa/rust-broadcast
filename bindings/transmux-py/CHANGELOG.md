# Changelog

All notable changes to `transmux-py` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Dependencies
- `pyo3` 0.24 -> 0.30 (abi3-py39 kept; `PyObject` is now `Py<PyAny>`).
- Pending, after the 2026-11 wave is live on crates.io: `transmux` 0.29 -> 0.30. The `Cargo.toml` pin is deliberately unchanged until then because this crate is outside the workspace and consumes the published crate by version.
- `transmux` 0.30 makes `serde` opt-in. These bindings convert the `Media`/`Track`/`Sample` IR to `PyDict`s field by field and never used `serde`, so no feature change is needed.

### Changed
- Pending: recheck `demux_ts` against the `transmux` 0.30 error enum (shared `broadcast_common::error` kinds are tuple variants) and the 0.30 IR. The next release is `0.6.0` through the PyPI tag lane.
