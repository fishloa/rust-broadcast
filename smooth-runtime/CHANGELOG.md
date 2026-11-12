# Changelog

All notable changes to smooth-runtime will be documented in this file.

## [Unreleased]
### Fixed
- `TimeScale="0"` (on the manifest or a `StreamIndex`) no longer yields a track with a zero timescale: such a stream is skipped, and a manifest with no stream left is `SmoothError::NoUsableStream`.
- A stream whose chunk timeline cannot be enumerated (`MissingDuration`, `NonMonotonicTime`, a too-long run) now ends the session with the new `SmoothError::Timeline` instead of being planned as empty, which left a static session neither announcing tracks nor ending.
- A fragment whose `tfxd` time does not fit an `i64` (or whose rebased timestamps would overflow) is dropped as `Dropped::Fragment` instead of overflowing the rebase offset (a debug-build panic, a wrapped dts in release).
### Added
- The crate: one sans-IO Smooth Streaming client engine (`SmoothClient`) replacing the two diverged pull clients in `broadcast-client` and `multimux` (#2370). It is the union of both: manifest fetch and live refresh with retry, protection refusal, stream and quality selection, chunk plans, fragment URLs, first-fragment track-id discovery, synthetic init segments, `tfxd` timestamp rebasing, the encrypted-fragment probe, live join look-ahead inside the DVR window, and discontinuities.
