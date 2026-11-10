---
status: DRAFT
---
# broadcast-client 0.5.0

_Released 2026-11-10._

Breaking-class release (0.4 -> 0.5): test-hook feature renamed `__test-seams` -> `test-seams`; sibling epochs move.

Version: 0.4.0 -> 0.5.0.

## Breaking changes and migration

- The test-only feature is renamed `__test-seams` -> `test-seams`.

```toml
# before
broadcast-client = { version = "0.4", features = ["__test-seams"] }
# after
broadcast-client = { version = "0.5", features = ["test-seams"] }
```

## Fixed

- Join-point audio end comparison uses `clock33::pcr_signed_distance`.
- Every sample's end reported to buffer controller in same call.
- RTSP/RTMP connection loss hinted as `SourceEnded`/`SourceError`.
- Live after finite connection keeps stall memory and pending-track placeholder.
- Protocol-error hint, `buffer_input_log_seed`, contiguous video media end reporting.

## Other changes

- Test-only feature spelling unified.

---

Published from tag `broadcast-client-v0.5.0`.
