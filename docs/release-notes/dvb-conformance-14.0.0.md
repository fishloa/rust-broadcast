# dvb-conformance 14.0.0

_Released 2026-11-14._

Major-class release (13 -> 14) for epoch purity: additions and fixes listed below; its sibling dependencies move to new caret epochs, which the workspace treats as a major-class change (docs/RELEASE-AUDIT.md section 2).

Version: 13.0.0 -> 14.0.0.

## Changed
- Legal-duplicate TS packet detection now imports `mpeg_ts::ts_dup` instead of `broadcast_common::ts_dup` (#2466).

## Added
- `ConformanceMonitor::indicator_counts` with the `IndicatorCount` record, so a live monitor publishes exactly the accounting the monitor itself keeps.
- `ConformanceMonitor` now accumulates per-indicator event counts, exposed via `indicator_counts()`.

## Fixed
- A CRC-failed PAT, PMT, NIT, SDT, EIT or other SI section no longer changes monitor state: it no longer rebuilds the referenced PID sets (false PMT_error_2 / lost tracking) or refreshes the 3.2 repetition timers, min-gap trackers, `_other` trackers and EIT P/F pairing, so a permanently corrupt NIT now raises 3.2 and a bit-flipped service_id no longer creates a phantom EIT_PF_error.
- The `_other` repetition errors (3.1.b, 3.5.b, 3.6.b) are raised again on every later outage; the per-key once-per-lifetime latch is gone.
- 2.3b compares the PCR step with the limit at 27 MHz resolution, so a 100.5 ms step is flagged (it was floored to 100 ms).
- A legal duplicate packet (H.222.0 2.4.3.3) is no longer fed to section reassembly, which appended it to the section in progress and raised a false CRC_error.
- EIT_PF_actual section '0' or '1' vanishing while its partner keeps arriving is detected after the P/F interval (3.6.a, reported as `EitError`).
- 2.3a is raised when a PCR PID keeps sending packets but stops carrying a PCR, instead of only when the next PCR finally arrives.
- 2.3a is reported once per gap: after it fires for a silence the on-PCR-arm 2.3a stays latched until a fresh PCR on that PID resets it, so a later distinct outage (PCR recovered in between) raises a separate 2.3a.
- 2.5 skips PES packets whose `stream_id` has no optional header (program_stream_map, padding, private_stream_2, ECM, EMM, DSMCC, H.222.1 type E, program_stream_directory).
- The EIT P/F pairing map is bounded (1024 sub-tables).

## Dependencies
- Sibling dependencies move to new caret epochs: `broadcast-common` 11 -> 12, `dvb-si` 13 -> 14, `mpeg-ts` 0.7 -> 0.8.

---

Published from tag `v14.0.0`.
