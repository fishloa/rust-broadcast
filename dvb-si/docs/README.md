# dvb-si — documentation and spec reference

Everything here is excluded from the published crate (`exclude = ["docs/**"]`); it is the
reviewable ground truth behind the code.

## Start here

| Document | What it is |
|---|---|
| [`COVERAGE.md`](COVERAGE.md) | **Generated** coverage reference: every table_id, descriptor tag and tag extension with its standard, verified clause, Rust type and status; the private/regional descriptors and their scopes; carousel, text and time handling; every allocated code that is **not** implemented; and a citation cross-check. Regenerate with `python3 tools/gen-dvbsi-coverage.py` — `cargo test -p dvb-si --test docs_coverage_drift` fails when it is stale. |
| [`coverage/allocations.tsv`](coverage/allocations.tsv) | The hand-transcribed allocation lists (table_id, descriptor tags, extension tags, AIT/INT/UNT/DII/RNT namespaces) with verified clause numbers — the "what the standards define" input to `COVERAGE.md`. |
| [`reserved-bits-edition-audit.md`](reserved-bits-edition-audit.md) | Per-field evidence for every reserved-bit default, edition by edition (EN 300 468 V1.3.1 … V1.20.1). |
| [`en300468-v1.20.1-delta.md`](en300468-v1.20.1-delta.md) | What changed between EN 300 468 V1.19.1 and V1.20.1, and what that meant for the crate. |

## Spec transcriptions

One table per file, organised as `{tables,descriptors,enums,text}/<spec>/`. Transcribed from the
vendored spec PDFs (in the private `private/` submodule — `private/specs/*.pdf`, with
whole-document markdown in `private/specs-md/`), and checked against them; each `enums/` entry
co-locates a `.toml` drift guard with its page. Module `//!` docs cite these files.

<!-- BEGIN GENERATED: dvb-si-docs-index (tools/gen-dvbsi-coverage.py) -->

| Kind | Standard | Files |
|---|---|---|
| Table / section syntax | [ETSI EN 300 468 — DVB Service Information](tables/en_300_468/) | 33 |
| Table / section syntax | [ETSI EN 300 706 — Enhanced Teletext](tables/en_300_706/) | 19 |
| Table / section syntax | [ETSI EN 300 743 — DVB subtitling](tables/en_300_743/) | 35 |
| Table / section syntax | [ETSI EN 301 192 — DVB data broadcasting (MPE, INT, data carousel)](tables/en_301_192/) | 16 |
| Table / section syntax | [Rec. ITU-T H.222.0 \| ISO/IEC 13818-1 — MPEG-2 Systems](tables/iso_13818_1/) | 9 |
| Table / section syntax | [ETSI TS 101 154 — audio/video coding in DVB](tables/ts_101_154/) | 38 |
| Table / section syntax | [ETSI TS 102 006 — System Software Update](tables/ts_102_006/) | 6 |
| Table / section syntax | [ETSI TS 102 323 — TV-Anytime carriage](tables/ts_102_323/) | 88 |
| Table / section syntax | [ETSI TS 102 366 — AC-3 / E-AC-3](tables/ts_102_366/) | 13 |
| Table / section syntax | [ETSI TS 102 772 — MPE-IFEC](tables/ts_102_772/) | 11 |
| Table / section syntax | [ETSI TS 102 809 — application signalling (AIT)](tables/ts_102_809/) | 15 |
| Descriptor syntax | [ETSI EN 300 468 — DVB Service Information](descriptors/en_300_468/) | 78 |
| Descriptor syntax | [ETSI EN 301 192 — DVB data broadcasting (MPE, INT, data carousel)](descriptors/en_301_192/) | 38 |
| Descriptor syntax | [ETSI EN 303 560 — DVB TTML subtitling](descriptors/en_303_560/) | 23 |
| Descriptor syntax | [GY/T 270-2013 — China DTV closed captions](descriptors/gy_t_270/) | 1 |
| Descriptor syntax | [GY/T 420-2025 — China UHD encapsulation (AVS2 video)](descriptors/gy_t_420/) | 1 |
| Descriptor syntax | [GY/T 422-2025 — China multi-audio (audio preselection profile)](descriptors/gy_t_422/) | 1 |
| Descriptor syntax | [Rec. ITU-T H.222.0 \| ISO/IEC 13818-1 — MPEG-2 Systems](descriptors/iso_13818_1/) | 40 |
| Descriptor syntax | [ISO/IEC 13818-6 — DSM-CC (hand transcription, provenance-documented)](descriptors/iso_13818_6/) | 4 |
| Descriptor syntax | [ISO/IEC 14496-1 — MPEG-4 Systems (FlexMux buffer descriptors)](descriptors/iso_14496_1/) | 1 |
| Descriptor syntax | [NorDig Unified Requirements v3.2.2 — logical channel descriptors](descriptors/nordig/) | 2 |
| Descriptor syntax | [T/AI 109.6-2022 — AVS3 intelligent media format](descriptors/t_ai_109_6/) | 1 |
| Descriptor syntax | [T/AI 109.7-2024 — AVS3 audio encapsulation](descriptors/t_ai_109_7/) | 2 |
| Descriptor syntax | [T/UWA 012.2-2023 — UWA UHD TS encapsulation](descriptors/t_uwa_012_2/) | 1 |
| Descriptor syntax | [ETSI TS 102 006 — System Software Update](descriptors/ts_102_006/) | 25 |
| Descriptor syntax | [ETSI TS 102 323 — TV-Anytime carriage](descriptors/ts_102_323/) | 11 |
| Descriptor syntax | [ETSI TS 102 727 — DVB-MHP (descriptor subset)](descriptors/ts_102_727/) | 9 |
| Descriptor syntax | [ETSI TS 102 772 — MPE-IFEC](descriptors/ts_102_772/) | 1 |
| Descriptor syntax | [ETSI TS 102 809 — application signalling (AIT)](descriptors/ts_102_809/) | 22 |
| Descriptor syntax | [ETSI TS 102 825 — CPCM](descriptors/ts_102_825/) | 2 |
| Coded-value tables (with `.toml` drift guards) | [ETSI EN 300 468 — DVB Service Information](enums/en_300_468/) | 84 |
| Coded-value tables (with `.toml` drift guards) | [ETSI EN 301 192 — DVB data broadcasting (MPE, INT, data carousel)](enums/en_301_192/) | 14 |
| Coded-value tables (with `.toml` drift guards) | [ETSI EN 303 560 — DVB TTML subtitling](enums/en_303_560/) | 3 |
| Coded-value tables (with `.toml` drift guards) | [Rec. ITU-T H.222.0 \| ISO/IEC 13818-1 — MPEG-2 Systems](enums/iso_13818_1/) | 12 |
| Coded-value tables (with `.toml` drift guards) | [ISO/IEC 13818-6 — DSM-CC (hand transcription, provenance-documented)](enums/iso_13818_6/) | 3 |
| Coded-value tables (with `.toml` drift guards) | [ETSI TS 101 154 — audio/video coding in DVB](enums/ts_101_154/) | 2 |
| Coded-value tables (with `.toml` drift guards) | [ETSI TS 101 162 — DVB identifier registries](enums/ts_101_162/) | 74 |
| Coded-value tables (with `.toml` drift guards) | [ETSI TS 102 006 — System Software Update](enums/ts_102_006/) | 9 |
| Coded-value tables (with `.toml` drift guards) | [ETSI TS 102 323 — TV-Anytime carriage](enums/ts_102_323/) | 19 |
| Coded-value tables (with `.toml` drift guards) | [ETSI TS 102 809 — application signalling (AIT)](enums/ts_102_809/) | 15 |
| Prose references | [ISO/IEC 13818-6 — DSM-CC (hand transcription, provenance-documented)](text/iso_13818_6/) | 20 |
| Prose references | [ETSI TR 101 211 — DVB SI implementation guidelines](text/tr_101_211/) | 9 |
| Prose references | [ETSI TR 101 290 — DVB measurement guidelines](text/tr_101_290/) | 4 |

<!-- END GENERATED: dvb-si-docs-index -->
