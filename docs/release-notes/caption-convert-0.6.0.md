# caption-convert 0.6.0

_Released 2026-11-14._

Major-class release (0.5 -> 0.6) for epoch purity: additions and fixes listed below; its sibling dependencies move to new caret epochs, which the workspace treats as a major-class change (docs/RELEASE-AUDIT.md section 2).

Version: 0.5.0 -> 0.6.0.

## Fixed
- `cc_data()` with `process_cc_data_flag` clear is discarded instead of producing cues.
- A WebVTT cue identifier that merely starts with `NOTE` (for example `NOTES1`) is no longer dropped as a NOTE block; only `NOTE` alone or followed by a space or tab is.

## Dependencies
- Sibling dependencies move to new caret epochs: `broadcast-common` 11 -> 12, `cc-data` 0.8 -> 0.9, `dvb-vbi` 0.6 -> 0.7, `timed-metadata` 0.9 -> 0.10.

---

Published from tag `caption-convert-v0.6.0`.
