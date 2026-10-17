# mpeg-ts 0.6.1

_Released 2026-10-17._

**Patch release, additive API.** `mpeg-ts` 0.6.0 skipped the 188 bytes of a packet with a broken sync byte and only counted the loss internally, so callers could not tell that a transport stream had been damaged. 0.6.1 reports every span the resynchroniser discards after its first lock. Nothing in 0.6.0 changes; existing code keeps compiling and behaving the same. `transmux` 0.27.1 builds on this (it needs `mpeg-ts` 0.6.1).

## Why this version

- **Proposed version:** 0.6.0 -> **0.6.1** (patch: a reporting gap fixed through an additive API; no existing signature or behaviour changed).
- **Caret epoch:** unchanged (0.6); dependents on `mpeg-ts = "0.6"` pick it up without edits.

## Using it

```rust
let mut rs = mpeg_ts::resync::TsResync::new();
// ... feed bytes, take packets as before ...
for loss in rs.take_losses() {
    // loss.offset, loss.length, loss.packet_index, loss.truncated_tail
}
rs.finish(); // reports a tail that never re-locked / a truncated final packet
```

Junk before the first lock (for example a capture that starts mid-packet) is deliberately not reported as a loss.

## What changed

### Added
- `resync::ResyncLoss`, `TsResync::take_losses()` and `TsResync::finish()`: every span the resynchroniser discards *after* its first lock (broken sync byte, garbage between packets, a tail that never re-locks, a truncated trailing packet) is now reported with its stream byte offset, length and packet index instead of only bumping `ResyncStats` (the cause of a silent sample loss in `transmux`'s `TsDemux`). Additive; junk before the first lock is still only counted.

---

Published from tag `mpeg-ts-v0.6.1`.
