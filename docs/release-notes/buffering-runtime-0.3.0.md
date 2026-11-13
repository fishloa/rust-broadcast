# buffering-runtime 0.3.0

_Released 2026-11-14._

Major-class release (0.2 -> 0.3) for epoch purity: additions and fixes listed below; its sibling dependencies move to new caret epochs, which the workspace treats as a major-class change (docs/RELEASE-AUDIT.md section 2).

Version: 0.2.1 -> 0.3.0.

## Fixed

- Track registered while `Playing` no longer turns off starvation detection.
- PCR-less mode feeds jitter estimator one track's own end series.
- Restored stall memory keeps exact age when first clock reading smaller.
- `BufferController::on_track_media` clamps earlier `now_us`.

## Other changes

- `Error` derives `Display` from `thiserror` (messages unchanged); gains `thiserror` dependency.

## Dependencies
- Sibling dependencies move to new caret epochs: `broadcast-common` 11 -> 12.

---

Published from tag `buffering-runtime-v0.3.0`.
