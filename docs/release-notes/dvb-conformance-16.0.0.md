# dvb-conformance 16.0.0

_Released 2026-11-22._

Lockstep major (15.1 -> 16.0). One functional fix; the rest is the audit-23 test-hardening pass over the TR 101 290 indicators. The major is the family's epoch move (`broadcast-common` 14, `dvb-si` 16, `mpeg-ts` 0.10), not an API change of this crate's own.

## Breaking changes

- Builds against `broadcast-common` 14, `dvb-si` 16 and `mpeg-ts` 0.10. A caret-epoch change is major-class under the epoch-purity rule (`docs/RELEASE-AUDIT.md` §2) whatever this crate's own API diff; a consumer must resolve a single copy of each.

## Fixed

- Per-PID `TBn` `Empty_buffer_error` (TR 101 290 indicator 3.9) can fire. Each PID's transport buffer now arms its empty-interval window at creation, rather than only at the first in-sync packet, when no per-PID buffer exists yet (#3483).

## Tests

The indicators are now probed at their exact limits and one tick past them: the per-PID TBn leak-rate estimator and `Data_delay_error` (3.10), the TBn eligibility set (ES PIDs only, never PAT/CAT/NIT/SDT/BAT/EIT/RST/TDT/TOT or the null PID), the empty-interval deadline, the drain clamp under a multi-year clock gap, TBsys limits and TBsys-fed CAT/NIT payloads, PAT/PMT presence timeouts with wrong-table_id stuffing, single-report CRC and absence rules, the 5 s PID window on a 7 s stream, every strict timing limit (PAT/PMT 500 ms, PID 5 s, SI repetition 10 s, Unreferenced_PID 500 ms, PCR repetition 100 ms, PCR delta, PTS 700 ms, EIT P/F pairing 2 s), EIT P/F per-section absence, legal non-actual table_ids, the reserved PID range, PCR silence timing, the PTS header guards, and every public label and event text (#3484-#3500).

## Pre-release audit

This crate carries no crate-specific finding; it takes the workspace-wide release-documentation issues #4004 (release note) and #4008 (epoch entry).

---

Published from tag `dvb-conformance-v16.0.0`.
