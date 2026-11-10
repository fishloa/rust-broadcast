---
status: DRAFT
---
# ttml-subtitle 0.6.1

_Released 2026-11-10._

Patch release: profile validator fix.

Version: 0.6.0 -> 0.6.1.

## Fixed
- The profile validator checks `begin`/`dur`/`end` on a `<div>` nested in a `<div>` (and its `p`/`image` descendants), which were skipped.

---

Published from tag `ttml-subtitle-v0.6.1`.
