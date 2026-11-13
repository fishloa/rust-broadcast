# dvb-tools 14.0.0

_Released 2026-11-14._

Major-class release (13 -> 14) for epoch purity: additions and fixes listed below; its sibling dependencies move to new caret epochs, which the workspace treats as a major-class change (docs/RELEASE-AUDIT.md section 2).

Version: 13.0.0 -> 14.0.0.

## Changed
- `pids` PCR wrap detection now uses `broadcast_common::clock33::pcr_signed_distance` (bidirectional) instead of a hand-rolled forward-only wrap (#2477).

## Fixed
- `pids` keeps each PID's unwrapped PCR in a signed value, so a backward PCR step taken near zero (an out-of-order or duplicate PCR early in a capture) no longer wraps to a huge positive span and reports a wrong bitrate: the bitrate is reported as unavailable, as for any other non-advancing PCR pair.
- `pids` takes the per-sample PCR step from `broadcast_common::clock33::pcr_signed_distance` instead of its own half-modulus comparison, so one shared wrap rule decides every wrap.
- File readers (`dump`, `pids`, ...) count the aligned packets at the end of the capture, and a capture shorter than five packets, which `TsResync` could not confirm before end of input.

## Dependencies
- Sibling dependencies move to new caret epochs: `broadcast-common` 11 -> 12, `dvb-si` 13 -> 14, `dvb-t2mi` 13 -> 14, `mpeg-ts` 0.7 -> 0.8.

---

Published from tag `v14.0.0`.
