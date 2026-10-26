# dvb-t2mi — spec table reference

<!-- BEGIN private-pointer (docs/specs/PRIVATE-MAP.md) -->
## Private spec transcriptions

The page-verified transcriptions of ETSI EN 302 755 (DVB-T2); ETSI TS 102 773 (T2-MI); ETSI TS 101 191 for this crate (37 files) live only in the private companion repository `rust-broadcast-private` (the `private/` submodule) at `private/specs-verified/dvb/dvb-t2mi/`. The standards are (c) their publishers, so the text is not published here. Old public path to private path: [`docs/specs/PRIVATE-MAP.md`](../../docs/specs/PRIVATE-MAP.md); code comments keep their original `docs/...` citations.

Still public in this directory: `enums` (19).
<!-- END private-pointer -->

One-table-per-file spec reference, organised as `docs/{tables,descriptors,enums,text}/<spec>/`. Transcribed from the canonical PDFs in the workspace [`specs/`](../../specs/) (the authoritative source); each `enums/` entry co-locates a `.toml` drift-guard with its page. Excluded from the published crate.

## Tables (wire-format syntax)

| Spec | Files |
|---|---|
| TS 102 773 | `tables/ts_102_773/` (private: `specs-verified/dvb/dvb-t2mi/tables/ts_102_773/9-tr-papr-function.md`) — L1-current/future signalling, T2-MI functions, FEF sub-parts |
| TS 101 191 | `tables/ts_101_191/` (private: `specs-verified/dvb/dvb-t2mi/tables/ts_101_191/6-1-existing-individual-addressing-functions.md`) — §6.1 individual addressing functions 0x00-0x06 (Tables 7-13) |

## Enums (coded-value tables)

| Spec | Files |
|---|---|
| TS 102 773 | [`enums/ts_102_773/`](enums/ts_102_773/) — `packet_type`, `addressing_function_tag`, `bandwidth`, `frequency_source`, `s1_field`, `subpart_variety` |
| EN 302 755 | [`enums/en_302_755/`](enums/en_302_755/) — `guard_interval`, `l1_modulation`, `l1_code_rate`, `l1_fec_type`, `pilot_pattern`, `tx_input_stream_type`, `plp_type`, `plp_payload_type`, `plp_modulation`, `plp_fec_type`, `plp_mode`, `aux_stream_type`, `t2_version` |
