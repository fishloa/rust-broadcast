# dvb-t2mi — spec table reference

One-table-per-file spec reference, organised as `docs/{tables,descriptors,enums,text}/<spec>/`. Transcribed from the canonical PDFs in the workspace [`specs/`](../../specs/) (the authoritative source); each `enums/` entry co-locates a `.toml` drift-guard with its page. Excluded from the published crate.

## Tables (wire-format syntax)

| Spec | Files |
|---|---|
| TS 102 773 | [`tables/ts_102_773/`](tables/ts_102_773/) — L1-current/future signalling, T2-MI functions, FEF sub-parts |

## Enums (coded-value tables)

| Spec | Files |
|---|---|
| TS 102 773 | [`enums/ts_102_773/`](enums/ts_102_773/) — `packet_type`, `addressing_function_tag`, `bandwidth`, `frequency_source`, `s1_field`, `subpart_variety` |
| EN 302 755 | [`enums/en_302_755/`](enums/en_302_755/) — `guard_interval`, `l1_modulation`, `l1_code_rate`, `l1_fec_type`, `pilot_pattern`, `tx_input_stream_type`, `plp_type`, `plp_payload_type`, `plp_modulation`, `plp_fec_type`, `plp_mode`, `aux_stream_type`, `t2_version` |
