# compliance-probe 0.8.0

_Released 2026-11-14._

Major-class release (0.7 -> 0.8) for epoch purity: additions and fixes listed below; its sibling dependencies move to new caret epochs, which the workspace treats as a major-class change (docs/RELEASE-AUDIT.md section 2).

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

## Also in this release

### Changed
- SCTE-35 cue sanity is the shared `scte35-splice` engine (`splice_event`/`judge_splice_time`) and TR 101 290 accounting is `dvb-conformance`'s own, so `compliance_probe_*` metrics report the same figures `media-doctor` does for the same stream; `Probe::indicator_counts` exposes the counts. Metric names and labels are unchanged.

## Dependencies
- Sibling dependencies move to new caret epochs: `broadcast-common` 11 -> 12, `dvb-conformance` 13 -> 14, `media-doctor` 0.14 -> 0.15, `media-plane` 0.10 -> 0.11, `mpeg-ts` 0.7 -> 0.8, `scte35-splice` 5 -> 6, `timed-metadata` 0.9 -> 0.10.

---

Published from tag `compliance-probe-v0.8.0`.
