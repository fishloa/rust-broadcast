# Changelog

All notable changes to smooth-runtime will be documented in this file.

## [Unreleased]
### Fixed
- `select_video_quality` refuses (false, `NoSuchVariant` in `broadcast-client`) a level whose codec configuration differs from the announced track, instead of switching to frames the host was never told how to decode; and the quality list offered to the host and the stream it switches are the same stream even when an earlier video stream is skipped.
- `SmoothClient::select_video_quality(index)` switches the `QualityLevel` of the first video stream for the fragments requested afterwards; the announced track is not re-announced.
- `TimeScale="0"` (on the manifest or a `StreamIndex`) no longer yields a track with a zero timescale: such a stream is skipped, and a manifest with no stream left is `SmoothError::NoUsableStream`.
- A stream whose chunk timeline cannot be enumerated (`MissingDuration`, `NonMonotonicTime`, a too-long run) now ends the session with the new `SmoothError::Timeline` instead of being planned as empty, which left a static session neither announcing tracks nor ending.
- A fragment whose `tfxd` time does not fit an `i64` (or whose rebased timestamps would overflow) is dropped as `Dropped::Fragment` instead of overflowing the rebase offset (a debug-build panic, a wrapped dts in release).
### Added
- The crate: one sans-IO Smooth Streaming client engine (`SmoothClient`) replacing the two diverged pull clients in `broadcast-client` and `multimux` (#2370). It is the union of both: manifest fetch and live refresh with retry, protection refusal, stream and quality selection, chunk plans, fragment URLs, first-fragment track-id discovery, synthetic init segments, `tfxd` timestamp rebasing, the encrypted-fragment probe, live join look-ahead inside the DVR window, and discontinuities.
- `FetchFailure::retry_after_us`: the server's `Retry-After` wait (RFC 9110 §10.2.3). A manifest or fragment retry waits at least that long, never less than the configured backoff (#2393).
