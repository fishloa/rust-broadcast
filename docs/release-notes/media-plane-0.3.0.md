# media-plane 0.3.0

**Release date:** 2026-08-05

`set_tracks` now wakes all listening cursors so egress consumers learn about mid-stream track additions without waiting for the next sample. Epoch bump to `transmux` 0.23 (^0.21 → ^0.23).

## What's changed

- `set_tracks` wakes listeners immediately.
- Requires `transmux` 0.23.

## Migration

Requires `transmux` 0.23.
