# dvb-conformance 11.0.0

_Released 2026-10-05._

### Fixed
- TR 101 290 v1.4.1 Table 5.0a indicators 1.3.a (`PAT_error_2`) / 1.5.a
  (`PMT_error_2`): the presence timers were refreshed by any payload-bearing
  TS packet on PID 0x0000 / a `program_map_PID`, so a stream whose PAT/PMT
  sections never complete, fail CRC-32, or carry the wrong `table_id` — the
  exact condition a Priority-1 "service not decodable" indicator exists for —
  kept the timer fresh forever and never fired. The indicators read "Sections
  with table_id 0x00 (0x02) do not occur at least every 0,5 s", so the timers
  are now refreshed only by a completed, CRC-valid section of the monitored
  `table_id` (#1096).
- TR 101 290 v1.4.1 Table 5.0a 1.5.a / 5.0b 1.6 / 5.0c 3.4: the
  `pmt_trackings` and `es_trackings` sets only ever grew, so a PAT version
  that removes a programme kept firing false `PMT_error_2` on its old
  `program_map_PID`, and a PMT version that drops or moves an ES PID kept
  firing false `PID_error` — while the removed-but-still-transmitting PID was
  never classed `Unreferenced_PID` (3.4) because it still looked referenced.
  The referenced sets are now rebuilt from every new PAT/PMT version (lineup
  changes such as regional opt-outs are routine), stale entries are dropped
  — including the 1.6 absence tracking, so 3.4 can actually fire — and newly
  discovered references are anchored at their discovery time (#1096).
- TR 101 290 v1.4.1 Table 5.0c indicator 3.2 (`SI_min_gap_error`) (and the
  3.1.a/3.5.a/3.6.a/3.7/3.8 rows sharing that timer): the 25 ms minimum-gap
  key was `(pid, table_id, section_number)`, but ETSI TR 101 211 §4.4 — the
  clause Table 5.0c cites for this dimension — defines the interval per
  sub-table: same `pid` and `table_id` with the same or a different
  `section_number`. The wrong key produced false `SI_min_gap_error` on
  EIT P/F actual (0x4E) in a multi-service multiplex (every service has its
  own sub-table — `table_id_extension = service_id` — with a section 0, and
  muxers emit them back to back), and false negatives for one sub-table's
  sections 0/1/2 sent 1 ms apart. The key is now
  `(pid, table_id, table_id_extension)` (#1096).
- Callers whose monotonic clock does not start at zero — `elapsed()` since
  the UNIX epoch (the obvious choice for an ingest service), a PCR-derived
  clock, or a capture `Timestamp` without an epoch subtraction — got an
  immediate false `PAT_error_2` (1.3.a) on the first in-sync packet, plus
  `TBsys`/`TBn` empty-interval (3.9) and data-delay (3.10) artefacts, because
  every presence timer and T-STD window was based on `Duration::ZERO`. All
  timers and windows are now anchored to the first observed `t`, and a
  reference (program_map_PID or ES PID) that a new PAT/PMT version only just
  started to list is measured from its discovery (#1096).
- Indicator 1.4 (`Continuity_count_error`): `check_cc` re-parsed
  `discontinuity_indicator` from raw bytes (magic masks over `raw[3..6]`)
  although the caller had already decoded the adaptation field via
  `TsPacket::parse`, so the two decoders could disagree — and the raw path
  read past the end of a packet whose `adaptation_field_length` pointed
  beyond the truncated buffer. The discontinuity decision now comes from the
  single parsed adaptation field (ISO/IEC 13818-1 §2.4.3.4) (#1096).
- TR 101 290 indicator 3.3 (`Buffer_error`): the T-STD `TBsys` model fed a
  whole reassembled PSI/SI section's byte length in one instant, at
  section-completion time, so any legally-paced section over 512 bytes
  (routine for SDT/NIT/EIT) raised a false `Buffer_error` regardless of how
  the bytes were actually spread across TS packets. `TBsys` is now fed per
  TS packet, as each packet's payload physically arrives (ISO/IEC 13818-1
  §2.4.2.3), so it drains correctly between packets instead of receiving
  the whole section at once (#1035).

---

Published from tag `dvb-conformance-v11.0.0`.
