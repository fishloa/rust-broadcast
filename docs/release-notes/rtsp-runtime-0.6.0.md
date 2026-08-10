# rtsp-runtime 0.6.0

**Release date:** 2026-08-10

MSRV raised to 1.95.0, plus a README install-snippet correction. No
functional or API change; nothing breaks.

## What changed

- MSRV raised to **1.95.0** (issue #949), part of a workspace-wide bump that
  removes the MSRV split `webrtc-runtime`'s optional `media` feature used to
  require. No functional or API change on its own.
- Doc accuracy (#941 row 6): README install snippet corrected from `"0.3"` to
  `"0.5"` (the crate was 0.5.0 at the time of the fix).

## Migration

No API changes; no action required beyond building with rustc >= 1.95.0.
