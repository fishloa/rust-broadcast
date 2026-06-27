# segmentation upid type

_ANSI/SCTE 35 2023r1 §10.3.3.1 Table 22 — segmentation_upid_type values_

> Values rendered from the co-located drift-guard [`segmentation_upid_type.toml`](./segmentation_upid_type.toml) — the spec defines these inline in a larger syntax table, so there is no standalone table to transcribe. The drift test keeps this list in lockstep with the Rust enum.

| value | variant | spec meaning |
|---|---|---|
| 0x00 | `NotUsed` | Not Used |
| 0x01 | `UserDefinedDeprecated` | User Defined (deprecated) |
| 0x02 | `Isci` | ISCI |
| 0x03 | `AdId` | Ad-ID |
| 0x04 | `Umid` | UMID |
| 0x05 | `IsanDeprecated` | ISAN (deprecated) |
| 0x06 | `Isan` | ISAN |
| 0x07 | `Tid` | TID |
| 0x08 | `Ti` | TI |
| 0x09 | `Adi` | ADI |
| 0x0A | `Eidr` | EIDR |
| 0x0B | `AtscContentIdentifier` | ATSC Content Identifier |
| 0x0C | `Mpu` | MPU |
| 0x0D | `Mid` | MID |
| 0x0E | `AdsInformation` | ADS Information |
| 0x0F | `Uri` | URI |
| 0x10 | `Uuid` | UUID |
| 0x11 | `Scr` | SCR |
