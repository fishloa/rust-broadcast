# Changelog

All notable changes to `transmux-py` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.7.0] - 2026-11-16

### Dependencies
- `broadcast-common` 12 -> 13 and `transmux` 0.31 -> 0.32 (the 2026-11-15 wave, live on crates.io). The lockfile was refreshed for those two crates and their transitive tree (`broadcast-hls` 0.8, `broadcast-dash` 0.2, `broadcast-smooth` 0.2, `dvb-si` 15, `cc-data` 0.10, `jiff` in place of `chrono`, ...); the `pyo3` pin is unchanged.

### Changed
- Breaking dependency move to `transmux` 0.32: `Sample::timing` is the `SampleTiming` enum (read through `dts()`/`pts()`/`duration()`, each `Option`), timestamps are `Option`-carrying, and `movie_timescale` / a track's `timescale` are the non-zero `broadcast_common::timescale::Timescale`. The dict shape is unchanged: `movie_timescale`, `track["timescale"]`, `track["start_decode_time"]`, `track["track_id"]` and `track["source_pid"]` are still plain ints.
- `sample["duration"]` is `int | None`: it is `None` when the sample's source stated no span (a `SampleTiming::Timed` sample with no duration) instead of a fabricated number.
- `sample["composition_offset"]` is `int | None`: it is `None` for a sample with no absolute timestamps (a `SampleTiming::DurationOnly` sample) or no timing at all (a section-carried track), and an offset that does not fit `i32` now raises `ValueError` rather than clamping.
- On the checked fixture (`fixtures/ts/h264-aac.ts`) every sample states both, so the output is unchanged: 2 tracks, 75 video / 131 audio samples, `movie_timescale` 90000, PIDs 256/257.

## [0.6.0] - 2026-11-08

### Dependencies
- `pyo3` 0.24 -> 0.30 (abi3-py39 kept; `PyObject` is now `Py<PyAny>`).
- `transmux` 0.29 -> 0.30 and `broadcast-common` 10 -> 11 (the 2026-11 wave, now live on crates.io).
- `transmux` 0.30 makes `serde` opt-in. These bindings convert the `Media`/`Track`/`Sample` IR to `PyDict`s field by field and never used `serde`, so no feature change is needed.

### Changed
- Breaking dependency move to `transmux` 0.30. No binding code change was needed: `demux_ts` and its `Media`/`Track`/`Sample` dict shape are unchanged.
