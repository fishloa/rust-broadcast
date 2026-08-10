# rtmp-runtime 0.6.0

**Release date:** 2026-08-10

MSRV raised to 1.95.0. No functional or API change; nothing breaks.

## What changed

- MSRV raised to **1.95.0** (issue #949), part of a workspace-wide bump that
  removes the MSRV split `webrtc-runtime`'s optional `media` feature used to
  require. Adopts let-chains and `is_multiple_of` where the 1.95 lints require
  them; no functional or API change.

## Migration

No API changes; no action required beyond building with rustc >= 1.95.0.
