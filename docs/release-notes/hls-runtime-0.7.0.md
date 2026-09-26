# hls-runtime 0.7.0

**Release date:** 2026-08-16

SCTE-35 timed-metadata support in HLS playlists. No breaking API changes,
but the new `timed-metadata` dependency and the behavioural change (playlists
now carry `#EXT-X-DATERANGE` lines when events are present) warrant a minor
bump.

## Security

**GHSA-grg8-55qr-gxgf** — `client::tokio_client::TokioClient` sent `TokioClientConfig::auth`
credentials (Basic, Bearer, and Digest challenge answers) to every host a playlist's URLs
pointed at, not only the playlist's own origin. A malicious or compromised playlist could
redirect a segment, part or rendition-report request to an attacker-controlled host and receive
the `Authorization` header meant only for the origin server. `TokioClient` also retried any
`4xx` response with the full backoff schedule, which needlessly repeated credentialed requests
against a host that had already rejected them.

**Upgrade if you use `TokioClient` with `TokioClientConfig::auth` set.**

### Fixed
- `TokioClient` now sends `TokioClientConfig::auth` credentials only to the playlist URL's
  origin (scheme + host + port); requests a playlist directs to any other host go without an
  `Authorization` header.
- `TokioClient` no longer retries a resource fetch answered with a `4xx` other than
  `408`/`429` (e.g. a `404` for a stale preload hint); it goes to `HlsClient::on_error` at once
  instead of after the full backoff.

## What's new

- **SCTE-35 → `EXT-X-DATERANGE` rendering** (issue #965). `HlsOrigin`'s
  `render_playlist()` now queries the trunk's event ring per segment
  (`Trunk::events_in_segment`) and, for each resolved SCTE-35 event,
  emits a `#EXT-X-DATERANGE` line (via
  `timed_metadata::Timeline::to_daterange`). Events are rendered only
  once the trunk has a `time_anchor` (for the wall-clock `START-DATE`);
  non-SCTE-35 and unresolvable events are silently skipped.

## Dependencies

- New: `timed-metadata ^0.5` (unconditional; `no_std`+`alloc`, no new
  feature gates).
- Requires `media-plane ^0.4.1` (for `Trunk::time_anchor()`).

## Migration

No API changes. Playlists served by `HlsOrigin` will now include
`#EXT-X-DATERANGE` lines when SCTE-35 events are published to the trunk —
this is additive and spec-compliant (RFC 8216 §4.4.5.1). Clients that do
not understand `EXT-X-DATERANGE` are required by the spec to ignore unknown
tags.
