# splice command type

_ANSI/SCTE 35 2023r1 §9.6.1 Table 7 — splice_command_type values_

> Values rendered from the co-located drift-guard [`splice_command_type.toml`](./splice_command_type.toml) — the spec defines these inline in a larger syntax table, so there is no standalone table to transcribe. The drift test keeps this list in lockstep with the Rust enum.

| value | variant | spec meaning |
|---|---|---|
| 0x00 | `SpliceNull` | splice_null |
| 0x04 | `SpliceSchedule` | splice_schedule |
| 0x05 | `SpliceInsert` | splice_insert |
| 0x06 | `TimeSignal` | time_signal |
| 0x07 | `BandwidthReservation` | bandwidth_reservation |
| 0xFF | `PrivateCommand` | private_command |
