---
status: DRAFT
---
# compliance-probe 0.8.0

_Released 2026-11-10._

Additive release (0.7 -> 0.8): SCTE-35 cancel/encrypted handling; sibling epochs move.

Version: 0.7.0 -> 0.8.0.

## Added

- `Scte35Sanity::Cancelled` and `Scte35Sanity::Encrypted`.
- `metric_names::SCTE35_ENCRYPTED_TOTAL`.
- `"cancel"` value of `kind` label on `compliance_probe_scte35_cues_total`.

## Fixed

- `check_section`: `splice_event_cancel_indicator = 1` is `Cancelled`; component-mode judged by component times.
- Well-formed encrypted `splice_info_section` is `Encrypted`, not malformed.

## Other changes

- `Probe::drain_byte_tap` documents shared arrival time.

## Dependencies

- `dvb-si` 12 -> 13
- `broadcast-common` 10 -> 11

---

Published from tag `compliance-probe-v0.8.0`.
