# mpeg-ts 0.6.2

_Released 2026-10-18._

**Patch release, documentation only.** `TsResync::feed` and `TsResync::take_losses` now state that the loss list is unbounded and that only `take_losses()` drains it. No code, signature or behaviour changed from 0.6.1. Reported alongside the `transmux` draining review (issue #1352).

## Why this version

- **Proposed version:** 0.6.1 -> **0.6.2** (patch: rustdoc and README text only; the public API and behaviour are byte-for-byte those of 0.6.1).
- **Caret epoch:** unchanged (0.6); dependents on `mpeg-ts = "0.6"` need no edits.
- **No dependency changes.**

## Who is affected

Anyone feeding `TsResync` from a long-running or lossy source. Every span the resynchroniser discards after its first lock is queued until you call `take_losses()`; on a badly damaged input that queue grows without bound if it is never drained. Call `take_losses()` after each `feed` (or at least periodically) and once more after `finish()`.

## What changed

### Documentation
- `TsResync::feed` / `take_losses` now state that the loss list is unbounded and only `take_losses()` drains it (#1352). README updated to match.

---

Published from tag `mpeg-ts-v0.6.2`.
