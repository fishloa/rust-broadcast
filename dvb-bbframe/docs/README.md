# dvb-bbframe — spec table reference

One-table-per-file spec reference, organised as `docs/{tables,descriptors,enums,text}/<spec>/`. Transcribed from the canonical PDFs in the workspace [`specs/`](../../specs/) (the authoritative source); each `enums/` entry co-locates a `.toml` drift-guard with its page. Excluded from the published crate.

## Tables (wire-format syntax)

| Spec | Files |
|---|---|
| EN 302 307-1 | [`tables/en_302_307_1/`](tables/en_302_307_1/) — S2 system configurations, BCH polynomials, interleaver |
| EN 302 307-2 | [`tables/en_302_307_2/`](tables/en_302_307_2/) — S2X constellation/label definitions, MODCOD, VL-SNR |
| EN 302 755 | [`tables/en_302_755/`](tables/en_302_755/) — T2 BBHeader, S1/S2 fields, L1-pre/post signalling, OFDM parameters, pilot patterns |

## Enums (coded-value tables)

| Spec | Files |
|---|---|
| EN 302 307-1 | [`enums/en_302_307_1/`](enums/en_302_307_1/) — MODCOD, q-values, ISSY field coding |
| EN 302 307-2 | [`enums/en_302_307_2/`](enums/en_302_307_2/) — MODCOD, coding parameters, VL-SNR PLFRAMES |
| EN 302 755 | [`enums/en_302_755/`](enums/en_302_755/) — rotation angle, padding types, FEF type, L1-ext block type, pilot/bit-permutation tables, `bufs_unit` (+ `ts_gs`) |
