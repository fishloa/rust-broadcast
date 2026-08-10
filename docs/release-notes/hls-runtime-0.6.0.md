# hls-runtime 0.6.0

**Release date:** 2026-08-10

Adds a new read-only origin API for multimux's DVR catch-up serving, fixes
two integer-overflow bugs in the client's byte-range handling, corrects a
stale README dependency snippet, and carries the workspace-wide MSRV bump.
No breaking changes.

## What's new

- `server::HlsOrigin::closed_segments()` — a snapshot of the origin's
  currently-advertised closed segments (sequence number, absolute
  `start_ns`, duration, discontinuity bit) as the new public
  `server::ClosedSegment` (with a `ClosedSegment::new` constructor, since
  the type is `#[non_exhaustive]`). Reuses the origin's existing live-window
  cursor rather than requiring a caller to open a second one on the same
  `Trunk` just to learn the same window `render_playlist` already renders.
  Added for multimux's DVR catch-up serving (issue #900), which needs to
  merge this origin's live window with a different segment source (an
  on-disk archive) over the same sequence-number space.

## What changed

- MSRV raised to **1.95.0** (issue #949) — the workspace-wide move that
  removes the separate MSRV lane `webrtc-runtime`'s optional `media` feature
  previously required. Only let-chain / `is_multiple_of` style adoption
  where the 1.95 lints require it.

## Fixed

- README install snippet corrected from `"0.1"` to `"0.5"` (issue #941 rows
  7-8), and the README now states explicitly that `server::HlsOrigin` does
  not emit `EXT-X-SKIP`/`CAN-SKIP-UNTIL`/`EXT-X-RENDITION-REPORT`, so the
  bundled client's Playlist Delta Update support
  (`ClientSession::merge_delta`) cannot be exercised against the bundled
  server. Documentation only — the bundled server has never implemented
  server-side delta updates.
- `client::HlsClient` no longer panics (debug) or silently computes a wrong
  byte range (release) from an untrusted remote playlist's
  `EXT-X-BYTERANGE`/preload-hint length (RFC 8216bis §4.4.4.9/§4.4.5.3):
  `offset + length`, and the per-URL omitted-offset running cursor it
  accumulates into, is now checked, returning the new
  `client::Error::ByteRangeOverflow` instead of wrapping. `merge_delta`'s
  `EXT-X-SKIP`/`SKIPPED-SEGMENTS` handling (§4.4.5.2) got the same guard,
  falling back to its existing "return the delta unmerged" path instead of
  panicking. `client::tokio_client::TokioClient`'s `Range:`-header builder
  got the matching guard (`TokioError::ByteRangeOverflow`) as
  defense-in-depth.

## Migration

No API changes; no action required beyond building with rustc >= 1.95.0. A
caller that was relying on `HlsClient`/`TokioClient` wrapping on an
overflowing byte range will now see `Error::ByteRangeOverflow` /
`TokioError::ByteRangeOverflow` instead — this is a bug fix, not a signature
change.
