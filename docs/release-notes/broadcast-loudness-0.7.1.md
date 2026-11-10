---
status: DRAFT
---
# broadcast-loudness 0.7.1

_Released 2026-11-10._

Fix release (0.7.0 -> 0.7.1): windowed sum re-basing, rejected push atomicity, channel weight validation.

Version: 0.7.0 -> 0.7.1.

## Added

- `Error::InvalidChannelWeights` (`Error` is `#[non_exhaustive]`).

## Fixed

- `Max M`/`Max S` sliding-window sum re-based on ring once per window.
- Rejected push leaves meter exactly as it was.
- `LoudnessMeter::new` rejects `ChannelLayout::Custom` with non-finite/negative/all-zero weights.

---

Published from tag `broadcast-loudness-v0.7.1`.
