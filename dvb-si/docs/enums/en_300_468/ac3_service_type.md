# ac3 service type

_ETSI EN 300 468 Annex D Table D.4 — AC-3 service_type codes (3-bit field)_

> Values rendered from the co-located drift-guard [`ac3_service_type.toml`](./ac3_service_type.toml) — the spec defines these inline in a larger syntax table, so there is no standalone table to transcribe. The drift test keeps this list in lockstep with the Rust enum.

| value | variant | spec meaning |
|---|---|---|
| 0x00 | `CompleteMain` | Complete Main (CM) |
| 0x01 | `MusicAndEffects` | Music and Effects (ME) |
| 0x02 | `VisuallyImpaired` | Visually Impaired (VI) |
| 0x03 | `HearingImpaired` | Hearing Impaired (HI) |
| 0x04 | `Dialogue` | Dialogue (D) |
| 0x05 | `Commentary` | Commentary (C) |
| 0x06 | `Emergency` | Emergency (E) |
| 0x07 | `VoiceOver` | Voice Over (VO) |
