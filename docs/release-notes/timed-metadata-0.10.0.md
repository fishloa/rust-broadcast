# timed-metadata 0.10.0

_Released 2026-11-14._

Major-class release (0.9 -> 0.10) for epoch purity: additions and fixes listed below; its sibling dependencies move to new caret epochs, which the workspace treats as a major-class change (docs/RELEASE-AUDIT.md section 2).

Version: 0.9.0 -> 0.10.0.

## Fixed
- Two Start Box..End Box segments on one Teletext row are separated by a space instead of being glued together.
- `Timeline` now seeds its 33-bit PTS unroller from the anchor, so a cue just after a PTS wrap that follows an anchor just before it is placed 2 s after the anchor instead of ~26.5 h before it (wrong `START-DATE`).

## Also in this release

### Changed
- `PTS_HZ` is a re-export of `broadcast_common::clock33::PTS_HZ` instead of a second definition; its path and value are unchanged (#2343).

## Dependencies
- Sibling dependencies move to new caret epochs: `broadcast-common` 11 -> 12, `broadcast-hls` 0.6 -> 0.7, `cc-data` 0.8 -> 0.9, `dvb-vbi` 0.6 -> 0.7, `mp4-emsg` 0.6 -> 0.7, `scte35-splice` 5 -> 6.

---

Published from tag `timed-metadata-v0.10.0`.
