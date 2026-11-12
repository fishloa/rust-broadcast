# Changelog

All notable changes to smooth-runtime will be documented in this file.

## [Unreleased]
### Added
- The crate: one sans-IO Smooth Streaming client engine (`SmoothClient`) replacing the two diverged pull clients in `broadcast-client` and `multimux` (#2370). It is the union of both: manifest fetch and live refresh with retry, protection refusal, stream and quality selection, chunk plans, fragment URLs, first-fragment track-id discovery, synthetic init segments, `tfxd` timestamp rebasing, the encrypted-fragment probe, live join look-ahead inside the DVR window, and discontinuities.
- `FetchFailure::retry_after_us`: the server's `Retry-After` wait (RFC 9110 §10.2.3). A manifest or fragment retry waits at least that long, never less than the configured backoff (#2393).
