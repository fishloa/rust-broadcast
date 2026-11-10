---
status: DRAFT
---
# timed-metadata 0.10.0

_Released 2026-11-10._

Breaking-class release (0.9 -> 0.10): no API change in `timed-metadata` itself; the bump is required because the sibling `broadcast-hls` it depends on moves to a new epoch (0.6 -> 0.7). Two fixes ship alongside.

Version: 0.9.0 -> 0.10.0.

## Fixed
- Two Start Box..End Box segments on one Teletext row are separated by a space instead of being glued together.
- `Timeline` now seeds its 33-bit PTS unroller from the anchor, so a cue just after a PTS wrap that follows an anchor just before it is placed 2 s after the anchor instead of ~26.5 h before it (wrong `START-DATE`).

## Dependencies
- `broadcast-hls` 0.6 -> 0.7 (new epoch).
- `scte35-splice` 5.0 -> 5.0.1.
- `cc-data` 0.8 -> 0.8.1.

---

Published from tag `timed-metadata-v0.10.0`.
