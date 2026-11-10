---
status: DRAFT
---
# buffering-runtime 0.2.2

_Released 2026-11-10._

Fix release (0.2.1 -> 0.2.2): error Display via thiserror; starvation and stall-memory fixes.

Version: 0.2.1 -> 0.2.2.

## Fixed

- Track registered while `Playing` no longer turns off starvation detection.
- PCR-less mode feeds jitter estimator one track's own end series.
- Restored stall memory keeps exact age when first clock reading smaller.
- `BufferController::on_track_media` clamps earlier `now_us`.

## Other changes

- `Error` derives `Display` from `thiserror` (messages unchanged); gains `thiserror` dependency.

---

Published from tag `buffering-runtime-v0.2.2`.
