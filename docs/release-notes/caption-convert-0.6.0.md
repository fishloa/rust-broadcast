---
status: DRAFT
---
# caption-convert 0.6.0

_Released 2026-11-10._

Breaking-class release (0.5 -> 0.6): no API change in `caption-convert` itself; the bump is required because the sibling `timed-metadata` it depends on moves to a new epoch (0.9 -> 0.10). Two fixes ship alongside.

Version: 0.5.0 -> 0.6.0.

## Fixed
- `cc_data()` with `process_cc_data_flag` clear is discarded instead of producing cues.
- A WebVTT cue identifier that merely starts with `NOTE` (for example `NOTES1`) is no longer dropped as a NOTE block; only `NOTE` alone or followed by a space or tab is.

## Dependencies
- `timed-metadata` 0.9 -> 0.10 (new epoch).
- `cc-data` 0.8 -> 0.8.1.

---

Published from tag `caption-convert-v0.6.0`.
