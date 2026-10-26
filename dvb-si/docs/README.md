# dvb-si — documentation and spec reference

Everything here is excluded from the published crate (`exclude = ["docs/**"]`); it is the
reviewable record behind the code (the verbatim spec transcriptions are in the private companion repo).

## Start here

| Document | What it is |
|---|---|
| [`COVERAGE.md`](COVERAGE.md) | **Generated** coverage reference: every table_id, descriptor tag and tag extension with its standard, verified clause, Rust type and status; the private/regional descriptors and their scopes; carousel, text and time handling; every allocated code that is **not** implemented; and a citation cross-check. Regenerate with `python3 tools/gen-dvbsi-coverage.py` — `cargo test -p dvb-si --test docs_coverage_drift` fails when it is stale. |
| [`coverage/allocations.tsv`](coverage/allocations.tsv) | The hand-transcribed allocation lists (table_id, descriptor tags, extension tags, AIT/INT/UNT/DII/RNT namespaces) with verified clause numbers — the "what the standards define" input to `COVERAGE.md`. |
| [`reserved-bits-edition-audit.md`](reserved-bits-edition-audit.md) | Per-field evidence for every reserved-bit default, edition by edition (EN 300 468 V1.3.1 … V1.20.1). |
| [`en300468-v1.20.1-delta.md`](en300468-v1.20.1-delta.md) | What changed between EN 300 468 V1.19.1 and V1.20.1, and what that meant for the crate. |

## Spec transcriptions

The page-verified spec transcriptions (one table per file, organised as `{tables,descriptors,enums,text}/<spec>/`) are
**not published here**: the standards are (c) their publishers, so the verbatim text lives only in the private
companion repository (`private/` submodule, under `private/specs-verified/dvb/`), next to the vendored PDFs
(`private/specs/*.pdf`) and whole-document markdown (`private/specs-md/`). They are checked against those PDFs.
What stays public: the `enums/<spec>/*.toml` drift guards (machine data read by `tests/spec_drift.rs`),
`coverage/allocations.tsv`, the generated `COVERAGE.md`, and the own-words audits listed above.
Module `//!` docs keep their historical `docs/<kind>/<spec>/<file>.md` citations; the old-path to private-path
table is [`docs/specs/PRIVATE-MAP.md`](../../docs/specs/PRIVATE-MAP.md).

<!-- BEGIN GENERATED: dvb-si-docs-index (tools/gen-dvbsi-coverage.py) -->

| Kind | Standard | Private path |
|---|---|---|
| Table / section syntax | ETSI EN 300 468 — DVB Service Information | `private/specs-verified/dvb/dvb-si/tables/en_300_468/` |
| Table / section syntax | ETSI EN 300 706 — Enhanced Teletext | `private/specs-verified/dvb/dvb-si/tables/en_300_706/` |
| Table / section syntax | ETSI EN 300 743 — DVB subtitling | `private/specs-verified/dvb/dvb-subtitle/tables/en_300_743/` |
| Table / section syntax | ETSI EN 301 192 — DVB data broadcasting (MPE, INT, data carousel) | `private/specs-verified/dvb/dvb-si/tables/en_301_192/` |
| Table / section syntax | Rec. ITU-T H.222.0 \| ISO/IEC 13818-1 — MPEG-2 Systems | `private/specs-verified/dvb/dvb-si/tables/iso_13818_1/` |
| Table / section syntax | ETSI TS 101 154 — audio/video coding in DVB | `private/specs-verified/dvb/dvb-si/tables/ts_101_154/` |
| Table / section syntax | ETSI TS 102 006 — System Software Update | `private/specs-verified/dvb/dvb-si/tables/ts_102_006/` |
| Table / section syntax | ETSI TS 102 323 — TV-Anytime carriage | `private/specs-verified/dvb/dvb-si/tables/ts_102_323/` |
| Table / section syntax | ETSI TS 102 366 — AC-3 / E-AC-3 | `private/specs-verified/dvb/dvb-si/tables/ts_102_366/` |
| Table / section syntax | ETSI TS 102 772 — MPE-IFEC | `private/specs-verified/dvb/dvb-si/tables/ts_102_772/` |
| Table / section syntax | ETSI TS 102 809 — application signalling (AIT) | `private/specs-verified/dvb/dvb-si/tables/ts_102_809/` |
| Descriptor syntax | ETSI EN 300 468 — DVB Service Information | `private/specs-verified/dvb/dvb-si/descriptors/en_300_468/` |
| Descriptor syntax | ETSI EN 301 192 — DVB data broadcasting (MPE, INT, data carousel) | `private/specs-verified/dvb/dvb-si/descriptors/en_301_192/` |
| Descriptor syntax | ETSI EN 303 560 — DVB TTML subtitling | `private/specs-verified/dvb/dvb-si/descriptors/en_303_560/` |
| Descriptor syntax | GY/T 270-2013 — China DTV closed captions | `private/specs-verified/dvb/dvb-si/descriptors/gy_t_270/` |
| Descriptor syntax | GY/T 420-2025 — China UHD encapsulation (AVS2 video) | `private/specs-verified/dvb/dvb-si/descriptors/gy_t_420/` |
| Descriptor syntax | GY/T 422-2025 — China multi-audio (audio preselection profile) | `private/specs-verified/dvb/dvb-si/descriptors/gy_t_422/` |
| Descriptor syntax | Rec. ITU-T H.222.0 \| ISO/IEC 13818-1 — MPEG-2 Systems | `private/specs-verified/dvb/dvb-si/descriptors/iso_13818_1/` |
| Descriptor syntax | ISO/IEC 13818-6 — DSM-CC (hand transcription, provenance-documented) | `private/specs-verified/dvb/dvb-si/descriptors/iso_13818_6/` |
| Descriptor syntax | ISO/IEC 14496-1 — MPEG-4 Systems (FlexMux buffer descriptors) | `private/specs-verified/dvb/dvb-si/descriptors/iso_14496_1/` |
| Descriptor syntax | NorDig Unified Requirements v3.2.2 — logical channel descriptors | `private/specs-verified/dvb/dvb-si/descriptors/nordig/` |
| Descriptor syntax | T/AI 109.6-2022 — AVS3 intelligent media format | `private/specs-verified/dvb/dvb-si/descriptors/t_ai_109_6/` |
| Descriptor syntax | T/AI 109.7-2024 — AVS3 audio encapsulation | `private/specs-verified/dvb/dvb-si/descriptors/t_ai_109_7/` |
| Descriptor syntax | T/UWA 012.2-2023 — UWA UHD TS encapsulation | `private/specs-verified/dvb/dvb-si/descriptors/t_uwa_012_2/` |
| Descriptor syntax | ETSI TS 102 006 — System Software Update | `private/specs-verified/dvb/dvb-si/descriptors/ts_102_006/` |
| Descriptor syntax | ETSI TS 102 323 — TV-Anytime carriage | `private/specs-verified/dvb/dvb-si/descriptors/ts_102_323/` |
| Descriptor syntax | ETSI TS 102 727 — DVB-MHP (descriptor subset) | `private/specs-verified/dvb/dvb-si/descriptors/ts_102_727/` |
| Descriptor syntax | ETSI TS 102 772 — MPE-IFEC | `private/specs-verified/dvb/dvb-si/descriptors/ts_102_772/` |
| Descriptor syntax | ETSI TS 102 809 — application signalling (AIT) | `private/specs-verified/dvb/dvb-si/descriptors/ts_102_809/` |
| Descriptor syntax | ETSI TS 102 825 — CPCM | `private/specs-verified/dvb/dvb-si/descriptors/ts_102_825/` |
| Coded-value tables (with `.toml` drift guards) | ETSI EN 300 468 — DVB Service Information | `private/specs-verified/dvb/dvb-si/enums/en_300_468/` |
| Coded-value tables (with `.toml` drift guards) | ETSI EN 301 192 — DVB data broadcasting (MPE, INT, data carousel) | `private/specs-verified/dvb/dvb-si/enums/en_301_192/` |
| Coded-value tables (with `.toml` drift guards) | ETSI EN 303 560 — DVB TTML subtitling | `private/specs-verified/dvb/dvb-si/enums/en_303_560/` |
| Coded-value tables (with `.toml` drift guards) | Rec. ITU-T H.222.0 \| ISO/IEC 13818-1 — MPEG-2 Systems | `private/specs-verified/dvb/dvb-si/enums/iso_13818_1/` |
| Coded-value tables (with `.toml` drift guards) | ISO/IEC 13818-6 — DSM-CC (hand transcription, provenance-documented) | `private/specs-verified/dvb/dvb-si/enums/iso_13818_6/` |
| Coded-value tables (with `.toml` drift guards) | ETSI TS 101 154 — audio/video coding in DVB | `private/specs-verified/dvb/dvb-si/enums/ts_101_154/` |
| Coded-value tables (with `.toml` drift guards) | ETSI TS 101 162 — DVB identifier registries | `private/specs-verified/dvb/dvb-si/enums/ts_101_162/` |
| Coded-value tables (with `.toml` drift guards) | ETSI TS 102 006 — System Software Update | `private/specs-verified/dvb/dvb-si/enums/ts_102_006/` |
| Coded-value tables (with `.toml` drift guards) | ETSI TS 102 323 — TV-Anytime carriage | `private/specs-verified/dvb/dvb-si/enums/ts_102_323/` |
| Coded-value tables (with `.toml` drift guards) | ETSI TS 102 809 — application signalling (AIT) | `private/specs-verified/dvb/dvb-si/enums/ts_102_809/` |
| Prose references | ISO/IEC 13818-6 — DSM-CC (hand transcription, provenance-documented) | `private/specs-verified/dvb/dvb-si/text/iso_13818_6/` |
| Prose references | ETSI TR 101 211 — DVB SI implementation guidelines | `private/specs-verified/dvb/dvb-si/text/tr_101_211/` |
| Prose references | ETSI TR 101 290 — DVB measurement guidelines | `private/specs-verified/dvb/dvb-si/text/tr_101_290/` |

<!-- END GENERATED: dvb-si-docs-index -->
