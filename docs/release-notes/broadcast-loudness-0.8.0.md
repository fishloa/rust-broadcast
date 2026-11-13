# broadcast-loudness 0.8.0

_Released 2026-11-14._

Major-class release (0.7 -> 0.8) for epoch purity: additions and fixes listed below; its sibling dependencies move to new caret epochs, which the workspace treats as a major-class change (docs/RELEASE-AUDIT.md section 2).

Version: 0.7.0 -> 0.8.0.

## Added

- `Error::InvalidChannelWeights` (`Error` is `#[non_exhaustive]`).

## Fixed

- `Max M`/`Max S` sliding-window sum re-based on ring once per window.
- Rejected push leaves meter exactly as it was.
- `LoudnessMeter::new` rejects `ChannelLayout::Custom` with non-finite/negative/all-zero weights.

## Dependencies
- Sibling dependencies move to new caret epochs: `broadcast-common` 11 -> 12.

---

Published from tag `broadcast-loudness-v0.8.0`.
