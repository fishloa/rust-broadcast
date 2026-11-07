# Changelog

All notable changes to `dvb-si-py` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Dependencies
- `pyo3` 0.24 -> 0.29 and `pythonize` 0.24 -> 0.29 (abi3-py39 kept; `PyObject` is now `Py<PyAny>`). `pyo3` stays on 0.29 because `pythonize` 0.29 pins it and two `pyo3` versions cannot link; the move to 0.30 waits for a `pythonize` release that supports it.
- Pending, after the 2026-11 wave is live on crates.io: `dvb-si` 12 -> 13, `dvb-t2mi` 12 -> 13 and `mpeg-ts` 0.6 -> 0.7. The `Cargo.toml` pins are deliberately unchanged until then because this crate is outside the workspace and consumes the published crates by version.
- The bindings already request `serde` explicitly (`features = ["ts", "serde", "chrono"]`), so the `serde`-opt-in change in `dvb-si` 13 needs no code change.

### Changed
- Pending: the `parse_section` JSON round trip and the `T2miPump` constructor must be rechecked against the `dvb-si` 13 / `dvb-t2mi` 13 error enums, which wrap the shared `broadcast_common::error` kinds (`Error::BufferTooShort(e)` instead of a struct variant) and replace `ReservedBitsViolation` with typed errors. The next release is `0.4.0` through the PyPI tag lane.
