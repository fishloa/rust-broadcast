# Changelog

All notable changes to smooth-runtime will be documented in this file.

## [Unreleased]
### Changed
- URL-reference resolution now uses the shared guarded resolver in `broadcast-common::url_ref` (control characters rejected before joining) (issue #2458).
### Added
- `server::SmoothOrigin` (feature `std`, issue #2453): the live Smooth Streaming origin engine, the server-side counterpart of `SmoothClient`. Given a `media_plane::Trunk`, the program's `SegmentProvider`/`TrackProvider` and its init segment, it renders the MS-SSTR client Manifest through `broadcast_smooth::SmoothManifest::live(..)`/`to_xml` and resolves `SmoothRequest::Fragment` to re-muxed fragment bytes, answering `EgressResponse::Ready`/`Await`/`BadRequest`/`NotFound` without I/O or a clock read. Codec parameters come from `transmux::resolve_codec`, the resolver the packager uses, so the manifest advertises what the fragments carry.
- `server::SmoothOrigin::stream_keys`/`fragments`, and `SmoothRequest::parse`: the streams a manifest advertises and the request grammar (`QualityLevels(BITRATE)/Fragments(KIND=START)`) that names them. A second stream of a kind is named `TYPE_N`, as the packager names it, so two audio tracks stay distinct.
- `Output::TracksChanged(Vec<TrackSpec>)`: after the tracks are announced, `select_video_quality` accepts any level of the same codec family (H264/AVC1, H265/HEVC) even when its SPS/PPS and resolution differ, and the engine re-announces the full track set once, before the first sample of the new level. A level of another codec family is still refused.
### Fixed
- A fragment whose start time the window does not hold is `EgressResponse::NotFound` instead of a fragment assembled from a neighbouring chunk: a start between two chunks (inside the window's span but on no advertised `c@t`) and a start after the last chunk's end are both refused. The window's bounds are computed with checked arithmetic, so a chunk ending too close to `u64::MAX` cannot wrap the comparison. A segment that slides out between describing the window and reading it back is also `NotFound`, not an unusable-request error (issue #2506).
- The quality list offered to the host and the stream it switches are the same stream even when an earlier video stream is skipped.
- `SmoothClient::select_video_quality(index)` switches the `QualityLevel` of the first video stream for the fragments requested afterwards; a different codec configuration is re-announced (see `Output::TracksChanged`).
- `TimeScale="0"` (on the manifest or a `StreamIndex`) no longer yields a track with a zero timescale: such a stream is skipped, and a manifest with no stream left is `SmoothError::NoUsableStream`.
- A stream whose chunk timeline cannot be enumerated (`MissingDuration`, `NonMonotonicTime`, a too-long run) now ends the session with the new `SmoothError::Timeline` instead of being planned as empty, which left a static session neither announcing tracks nor ending.
- A fragment whose `tfxd` time does not fit an `i64` (or whose rebased timestamps would overflow) is dropped as `Dropped::Fragment` instead of overflowing the rebase offset (a debug-build panic, a wrapped dts in release).
### Added
- The crate: one sans-IO Smooth Streaming client engine (`SmoothClient`) replacing the two diverged pull clients in `broadcast-client` and `multimux` (#2370). It is the union of both: manifest fetch and live refresh with retry, protection refusal, stream and quality selection, chunk plans, fragment URLs, first-fragment track-id discovery, synthetic init segments, `tfxd` timestamp rebasing, the encrypted-fragment probe, live join look-ahead inside the DVR window, and discontinuities.
- `FetchFailure::retry_after_us`: the server's `Retry-After` wait (RFC 9110 §10.2.3). A manifest or fragment retry waits at least that long, never less than the configured backoff (#2393).
