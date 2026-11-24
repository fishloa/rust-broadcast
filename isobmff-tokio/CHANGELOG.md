# Changelog

All notable changes to `isobmff-tokio` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- `DriverStep::Metadata`: `AsyncFileReader::step` stores the movie's and each
  track's typed metadata and returns a borrow, like the track table, instead of
  swallowing the reader's `Step::Metadata`. The async driver has no
  forward-only form (it seeks), and its `read_sample` reads every sample of a
  fragmented fixture.

### Changed

- A fragmented source's `Sample` steps now arrive with the sample data still
  ahead of the read position (the reader emits a fragment once its `mdat` is
  located), so `read_sample` after a `Sample` step no longer needs a seek past
  bytes a forward walk would have consumed.

## [0.1.0] - unreleased

### Added

- `AsyncFileReader<R: AsyncRead + AsyncSeek + Unpin>`: answer the `isobmff`
  reader's structure demands over a tokio source, yield
  `DriverStep::{Tracks, Fragment, Sample, Done}`, and read one sample's bytes
  with `read_sample`. It reuses the blocking driver's buffer-sizing helpers and
  `DriverError`, so the two cannot drift.
