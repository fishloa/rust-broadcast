# ts gs

_EN 302 755 Table 1 — TS/GS field values (MATYPE-1 bits [7:6], 2-bit field)_

> Values rendered from the co-located drift-guard [`ts_gs.toml`](./ts_gs.toml) — the spec defines these inline in a larger syntax table, so there is no standalone table to transcribe. The drift test keeps this list in lockstep with the Rust enum.

| value | variant | spec meaning |
|---|---|---|
| 0x00 | `Gfps` | Generic Packetized Stream |
| 0x01 | `Gcs` | Generic Continuous Stream |
| 0x02 | `Gse` | Generic Encapsulated Stream |
| 0x03 | `Ts` | Transport Stream (MPEG-2 TS, 188-byte packets) |
