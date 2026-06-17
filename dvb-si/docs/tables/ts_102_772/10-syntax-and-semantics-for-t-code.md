## Table 10 — Syntax and semantics for T_code
_§6.2, PDF pp. 22-22_

| Bit rate | Description |
|---|---|
| 00 | Reed Solomon code ([1], clause 9.5.1) |
| 01 | Raptor Codes ([2], clause C.4) |
| 01 to 11 | Reserved for future use |

> **Spec note:** the PDF's Table 10 literally lists the reserved range as `01 to 11`, which textually overlaps the `01` (Raptor) row above it. This is reproduced verbatim from the spec — it is an apparent editorial quirk in EN/TS 102 772 V1.1.1, not a transcription choice. Implementations should treat `01` as Raptor and `10`/`11` as reserved.

