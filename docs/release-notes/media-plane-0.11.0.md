---
status: DRAFT
---
# media-plane 0.11.0

_Released 2026-11-10._

Breaking-class release (0.10 -> 0.11): sibling epochs move; no breaking API change of its own.

Version: 0.10.0 -> 0.11.0.

## Added

- `Trunk::open_segment`: lowest segment above last closed one with resident parts.

## Fixed

- `SampleCursor`: back-to-back writer re-issues with no data report exactly one `Discontinuity`.

## Dependencies

- `broadcast-common` 10 -> 11

---

Published from tag `media-plane-v0.11.0`.
