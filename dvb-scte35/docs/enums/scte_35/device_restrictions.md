# device restrictions

_ANSI/SCTE 35 2023r1 §10.3.3.1 Table 21 — device_restrictions values (2 bits)_

> Values rendered from the co-located drift-guard [`device_restrictions.toml`](./device_restrictions.toml) — the spec defines these inline in a larger syntax table, so there is no standalone table to transcribe. The drift test keeps this list in lockstep with the Rust enum.

| value | variant | spec meaning |
|---|---|---|
| 0x00 | `RestrictGroup0` | Restricted for device group 0 (out-of-band defined) |
| 0x01 | `RestrictGroup1` | Restricted for device group 1 |
| 0x02 | `RestrictGroup2` | Restricted for device group 2 |
| 0x03 | `None` | No device restrictions |
