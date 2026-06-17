# ac4 channel mode

_ETSI EN 300 468 Table D.12 — AC-4 channel_mode codes (2-bit field)_

> Values rendered from the co-located drift-guard [`ac4_channel_mode.toml`](./ac4_channel_mode.toml) — the spec defines these inline in a larger syntax table, so there is no standalone table to transcribe. The drift test keeps this list in lockstep with the Rust enum.

| value | variant | spec meaning |
|---|---|---|
| 0x00 | `Mono` | Mono content |
| 0x01 | `Stereo` | Stereo content |
| 0x02 | `Multichannel` | Multichannel content |
