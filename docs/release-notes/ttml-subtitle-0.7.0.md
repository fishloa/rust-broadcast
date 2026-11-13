# ttml-subtitle 0.7.0

_Released 2026-11-14._

Major-class release (0.6 -> 0.7) for epoch purity: additions and fixes listed below; its sibling dependencies move to new caret epochs, which the workspace treats as a major-class change (docs/RELEASE-AUDIT.md section 2).

Version: 0.6.0 -> 0.7.0.

## Fixed
- The profile validator checks `begin`/`dur`/`end` on a `<div>` nested in a `<div>` (and its `p`/`image` descendants), which were skipped.

## Dependencies
- Sibling dependencies move to new caret epochs: `broadcast-common` 11 -> 12.

---

Published from tag `ttml-subtitle-v0.7.0`.
