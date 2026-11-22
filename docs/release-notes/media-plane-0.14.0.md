# media-plane 0.14.0

_Released 2026-11-22._

Minor for a 0.x crate. The new spoke is `push_session`, the sans-IO decision core for a push output, which `multimux` now drives instead of keeping its own copy of every decision. The rest is the epoch move and the audit fixes (a failed negotiation now hands its egress back).

## Breaking changes

- Builds against new caret epochs of `broadcast-common` 14, `broadcast-hls` 0.9, `caption-convert` 0.8, `container-probe` 0.6, `dvb-si` 16, `scte35-splice` 8, `timed-metadata` 0.12 and `transmux` 0.33. A caret-epoch change is major-class under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2) whatever this crate's own API diff; a consumer must resolve a single copy of each at the jiff 1.0 epoch, and the `Trunk`/`SegmentEntry` values it hands out carry those versions' sample types.

## Added

- `push_session::PushSession`, the sans-IO decision core for a connection-oriented push of samples: track renegotiation by generation, lag and degrade accounting, drop-versus-reconnect classification through `PushErrorKind`/`PushErrorClass`, `ReconnectCause`, `FlushResult` and `PushCounters`. It owns the connected `PushEgress` and returns one `PushDecision` per host input; it has no clock, no async and no I/O. `PreboundBinds::{tcp, udp}` and `Recorder::new`/`Recorded` take `parking_lot` locks.

## Fixed

- A negotiation that fails or is refused now closes the egress before the backoff instead of dropping it unclosed, and a redundant second connection is closed too, so an RTMP/SRT/RTSP teardown is graceful and no socket waits for `Drop` (#3987).

## Changed

- Internal: error types derive `thiserror::Error` (messages unchanged) (#3974, #4020).
- README documents `push_session::PushSession` (`PushDecision`, `ReconnectCause`, `FlushResult`, `PushCounters`) and corrects the `PushEgress` row: `multimux` drives it through `push_session` over its push transports (#4011).

## Tests

The audit/`push_session` suites pin the connect-time and mid-stream decisions, the loss accounting counters, the flush outcome and the give-up path, plus the `Await` egress's blocking-reload boundary and the cursor lag handling.

## Pre-release audit

Issues fixed in this crate by the 2026-11-22 pre-release audit: #3987 (egress returned on a failed negotiation), #3997 (transcription citations), #4002 (`PushDecision` `Debug`), #4011 (README), #4018 (duplicate headings), #4020 (`thiserror`), #4021 (test bullets under `### Tests`).

---

Published from tag `media-plane-v0.14.0`.
