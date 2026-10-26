# dvb-bbframe — spec table reference

<!-- BEGIN private-pointer (docs/specs/PRIVATE-MAP.md) -->
## Private spec transcriptions

The page-verified transcriptions of ETSI EN 302 307-1 (DVB-S2); ETSI EN 302 307-2 (DVB-S2X); ETSI EN 302 755 (DVB-T2) for this crate (121 files) live only in the private companion repository `rust-broadcast-private` (the `private/` submodule) at `private/specs-verified/dvb/dvb-bbframe/`. The standards are (c) their publishers, so the text is not published here. Old public path to private path: [`docs/specs/PRIVATE-MAP.md`](../../docs/specs/PRIVATE-MAP.md); code comments keep their original `docs/...` citations.

Still public in this directory: `enums/en_302_755/bufs_unit.toml`, `enums/en_302_755/ts_gs.toml`, `tables/en_302_307_1/10-optimum-constellation-radius-ratios-γ1-and-γ2-linear-channel-for-32-apsk.md`, `tables/en_302_307_1/13-esno-performance-at-quasi-error-free-per-10⁷-awgn-channel.md`, `tables/en_302_307_1/9-optimum-constellation-radius-ratio-γ-linear-channel-for-16apsk.md`, `tables/en_302_307_2/10b-optimum-constellation-radius-ratios-for-242apsk-γ-r-r-γ-r-r.md`.
<!-- END private-pointer -->

One-table-per-file spec reference, organised as `docs/{tables,descriptors,enums,text}/<spec>/`. Transcribed from the canonical PDFs in the workspace [`specs/`](../../specs/) (the authoritative source); each `enums/` entry co-locates a `.toml` drift-guard with its page. Excluded from the published crate.

## Tables (wire-format syntax)

| Spec | Files |
|---|---|
| EN 302 307-1 | `tables/en_302_307_1/` (private: `specs-verified/dvb/dvb-bbframe/tables/en_302_307_1/2-system-interfaces.md`) — S2 system configurations, BCH polynomials, interleaver |
| EN 302 307-2 | `tables/en_302_307_2/` (private: `specs-verified/dvb/dvb-bbframe/tables/en_302_307_2/14a-optimum-constellation-radius-ratios-for-128apsk.md`) — S2X constellation/label definitions, MODCOD, VL-SNR |
| EN 302 755 | `tables/en_302_755/` (private: `specs-verified/dvb/dvb-bbframe/tables/en_302_755/60-amplitudes-of-the-scattered-pilots.md`) — T2 BBHeader, S1/S2 fields, L1-pre/post signalling, OFDM parameters, pilot patterns |

## Enums (coded-value tables)

| Spec | Files |
|---|---|
| EN 302 307-1 | `enums/en_302_307_1/` (private: `specs-verified/dvb/dvb-bbframe/enums/en_302_307_1/5a-coding-parameters-for-normal-fecframe-nldpc-64-800.md`) — MODCOD, q-values, ISSY field coding |
| EN 302 307-2 | `enums/en_302_307_2/` (private: `specs-verified/dvb/dvb-bbframe/enums/en_302_307_2/19d-coding-parameters-for-vl-snr-plframes-for-short-fecframe-n-16-200.md`) — MODCOD, coding parameters, VL-SNR PLFRAMES |
| EN 302 755 | [`enums/en_302_755/`](enums/en_302_755/) — rotation angle, padding types, FEF type, L1-ext block type, pilot/bit-permutation tables, `bufs_unit` (+ `ts_gs`) |
