# dvb-conformance 11.0.0

_Released 2026-10-05._

**Major by lockstep; no public API change of its own.** The crate version moves with the DVB lockstep because its dependencies cross caret epochs (`dvb-si` 10 to 11, `mpeg-ts` 0.4 to 0.5). The substantive content is a set of TR 101 290 indicator corrections (#1096, #1035) that change what the monitor reports on the same input: fewer false positives, and Priority-1 faults that previously never fired now do. If you alert on these indicators, expect the alert set to change. Read with [dvb-si 11.0.0](dvb-si-11.0.0.md) and [mpeg-ts 0.5.0](mpeg-ts-0.5.0.md).

## Dependency changes

```toml
-broadcast-common = { version = "9.3", default-features = false }
+broadcast-common = { version = "9.4", default-features = false }
-dvb-si           = { version = "10",  default-features = false, features = ["ts"] }
+dvb-si           = { version = "11.0", default-features = false, features = ["ts"] }
-mpeg-ts          = { version = "0.4", default-features = false }
+mpeg-ts          = { version = "0.5", default-features = false }
```

## Behaviour changes: indicators that now fire (or stop firing wrongly)

- 1.3.a `PAT_error_2` and 1.5.a `PMT_error_2` (TR 101 290 v1.4.1 Table 5.0a) now fire when they should. The presence timers were refreshed by any payload-bearing packet on PID 0x0000 or a `program_map_PID`, so a stream whose PAT/PMT sections never complete, fail CRC-32 or carry the wrong `table_id` kept the timer fresh forever, which is exactly the "service not decodable" condition the indicator exists for. Timers are now refreshed only by a completed, CRC-valid section of the monitored `table_id` (#1096).
- 1.5.a, 1.6 `PID_error` and 3.4 `Unreferenced_PID`: the referenced PMT and ES sets only ever grew, so a PAT version that removed a programme kept firing false `PMT_error_2` on its old PID, a PMT that dropped or moved an ES PID kept firing false `PID_error`, and a removed-but-still-transmitting PID was never classed `Unreferenced_PID` because it still looked referenced. The sets are now rebuilt from every new PAT/PMT version, stale entries (including 1.6 absence tracking) are dropped, and new references are anchored at their discovery time (#1096).
- 3.2 `SI_min_gap_error` (and the 3.1.a/3.5.a/3.6.a/3.7/3.8 rows sharing that timer): the 25 ms gap key is now `(pid, table_id, table_id_extension)`, matching the per-sub-table interval in ETSI TR 101 211 §4.4. The old key `(pid, table_id, section_number)` produced false errors on EIT P/F actual (0x4E) in multi-service multiplexes (each service is its own sub-table with a section 0, sent back to back) and missed sections 0/1/2 of one sub-table sent 1 ms apart (#1096).
- 3.3 `Buffer_error`: the `TBsys` model was fed a whole reassembled section's length at section completion, so any legally paced PSI/SI section over 512 bytes (routine for SDT/NIT/EIT) raised a false error. It is now fed per TS packet as each payload arrives (ISO/IEC 13818-1 §2.4.2.3) (#1035).
- 1.4 `Continuity_count_error`: `check_cc` no longer re-parses `discontinuity_indicator` from raw bytes. The decision comes from the single parsed adaptation field (ISO/IEC 13818-1 §2.4.3.4), so two decoders can no longer disagree, and the raw path no longer reads past a truncated packet whose `adaptation_field_length` pointed beyond the buffer (#1096).

## Clock anchoring

Callers whose monotonic clock does not start at zero (`elapsed()` since the UNIX epoch, a PCR-derived clock, or a capture timestamp without an epoch subtraction) previously got an immediate false `PAT_error_2` on the first in-sync packet, plus `TBsys`/`TBn` empty-interval (3.9) and data-delay (3.10) artefacts, because every presence timer and T-STD window was based on `Duration::ZERO`. All timers and windows are now anchored to the first observed `t`. (#1096)

---

Published from tag `dvb-conformance-v11.0.0`.
