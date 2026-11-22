# hls-runtime 0.16.0

_Released 2026-11-22._

Minor for a 0.x crate, breaking on the epoch: the crate builds against `broadcast-hls` 0.9, `broadcast-common` 14, `media-plane` 0.14, `transmux` 0.33 and the rest of the wave, and its public surface exposes `broadcast_hls::{InstreamId, MediaSequence, PartIndex}`, so a consumer must move those siblings with it. One behaviour fix: a failed encrypted init no longer strands the segments queued behind its key.

## Breaking changes

- Builds against new caret epochs of workspace siblings (`broadcast-common` 13 -> 14, `broadcast-hls` 0.8 -> 0.9, `container-probe` 0.5 -> 0.6, `media-plane` 0.13 -> 0.14, `mpeg-ts` 0.9 -> 0.10, `timed-metadata` 0.11 -> 0.12, `transmux` 0.32 -> 0.33). A caret-epoch change is major-class under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2); consumers must resolve a single copy of each at the jiff 1.0 epoch, and the `InstreamId`/`MediaSequence`/`PartIndex` values this crate exposes are the 0.9 ones.

## Fixed

- `HlsClient::on_key` keeps decrypting the segments queued behind a key when the encrypted init waiting on that key fails to decrypt or is not an init: the init's error is still returned, but the segments wait for the refetched init instead of being stranded, so a VOD session plays and ends after `retry_init` rather than stalling (#3951).

## Tests

The blocking-reload and part-availability decisions, the origin engine's segment/part resolution over a `media-plane` `Trunk`, and the client's init-prefetch and range handling are covered by the crate's suites; the `no_handroll_guard` checks the HTTP/dates/URL lexical shapes.

## Pre-release audit

Issues fixed in this crate by the 2026-11-22 pre-release audit: #3992 (epoch line in the CHANGELOG).

---

Published from tag `hls-runtime-v0.16.0`.
