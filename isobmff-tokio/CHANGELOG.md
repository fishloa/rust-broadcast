# Changelog

All notable changes to `isobmff-tokio` will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0] - unreleased

### Added

- `AsyncFileReader<R: AsyncRead + AsyncSeek + Unpin>`: answer the `isobmff`
  reader's structure demands over a tokio source, yield
  `DriverStep::{Tracks, Fragment, Sample, Done}`, and read one sample's bytes
  with `read_sample`. It reuses the blocking driver's buffer-sizing helpers and
  `DriverError`, so the two cannot drift.
