
# buffering-runtime 0.2.0

_Released 2026-11-07._

Breaking-class release (0.1 -> 0.2): the `WRAP_27MHZ` re-export is removed. The rest of the diff is documentation of the start-gate measurements and tests.

Version: 0.1.0 -> 0.2.0.

## Breaking changes and migration

What the CHANGELOG records as breaking:

- The `WRAP_27MHZ` re-export; use `broadcast_common::clock33::PCR_WRAP_27MHZ`.

### Renamed and removed items

```text
buffering_runtime::WRAP_27MHZ
    -> broadcast_common::clock33::PCR_WRAP_27MHZ
```

## Documentation

- Start-fill cost of the granularity gate, measured (DESIGN 0.7d): on the Apple TV input logs (rai-3, rai-3 per-chunk, rai-3-stall, france-5; closed loop, tracks registered up front) the gate costs nothing at `initial_us` 500 ms and 1.8 s; the first frame is bound by the audio lead filling in one-PES lumps (lead 703 ms at the rai-3 start for a 500 ms fill) and by audio's first delivery arriving ~0.43-0.62 s after video's. A one-step gate would start 0.1-0.2 s earlier at 300 ms but stalls rai-3 and rai-3 per-chunk, so the two-step gate and its bound are unchanged. No behaviour change.

## Dependencies

Workspace sibling requirements move to the new epochs of this wave:

- `broadcast-common` 10 -> 11

These are caret-epoch moves, so a project that names several of these crates must move them together.

---

Published from tag `buffering-runtime-v0.2.0`.
