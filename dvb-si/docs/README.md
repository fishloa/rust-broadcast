# dvb-si — spec table reference

One-table-per-file spec reference, organised as `docs/{tables,descriptors,enums,text}/<spec>/`. Transcribed from the canonical PDFs in the workspace [`specs/`](../../specs/) (the authoritative source); each `enums/` entry co-locates a `.toml` drift-guard with its page. Excluded from the published crate.

## Tables (wire-format syntax)

| Spec | Files |
|---|---|
| EN 300 468 | [`tables/en_300_468/`](tables/en_300_468/) — `10-running-status-section.md`, `11-stuffing-section.md`, `11a-satellite-access-section.md`, `11c-satellite-position-v2-info.md`, `11d-cell-fragment-info.md`, `11e-time-association-info.md`, `11g-beamhopping-time-plan-info.md`, `11h-satellite-position-v3-info.md`, `11i-interpolation-method-for-ephemeris-data.md`, `124-roll-off-factor.md`, `144b-s2xv2-satellite-delivery-system-info.md`, `144f-postamble-pli.md`, `162-production-disparity-hint-info.md`, `163-discontinuity-information-section.md`, `164-selection-information-section.md`, `18-void.md`, `3-network-information-section.md`, `35-modulation-scheme-for-cable.md`, `36-inner-fec-scheme.md`, `39-roll-off-factor.md`, `4-bouquet-association-section.md`, `40-modulation-system-for-satellite.md`, `5-service-description-section.md`, `514-repetition-rates-and-random-access-hand-transcribed.md`, `61-mobile-hand-over-info.md`, `64-event-linkage-info.md`, `65-extended-event-linkage-info.md`, `68-target-service-matching-rules.md`, `7-event-information-section.md`, `8-time-and-date-section.md`, `9-time-offset-section.md`, `figure-a1-character-code-table-00-default-latin-alphabet.md`, `table-144c1-ncr-reference.md` |
| EN 300 706 | [`tables/en_300_706/`](tables/en_300_706/) — Teletext packet coding |
| EN 300 743 | [`tables/en_300_743/`](tables/en_300_743/) — DVB subtitling segments |
| EN 301 192 | [`tables/en_301_192/`](tables/en_301_192/) — MPE, data carousel, INT syntax |
| ISO/IEC 13818-1 | [`tables/iso_13818_1/`](tables/iso_13818_1/) — MPEG-2 TS packet, PSI sections |
| TS 101 154 | [`tables/ts_101_154/`](tables/ts_101_154/) — AV coding constraints |
| TS 102 006 | [`tables/ts_102_006/`](tables/ts_102_006/) — UNT, SSU sections |
| TS 102 323 | [`tables/ts_102_323/`](tables/ts_102_323/) — TV-Anytime metadata sections |
| TS 102 366 | [`tables/ts_102_366/`](tables/ts_102_366/) — AC-3 / E-AC-3 frame-header |
| TS 102 772 | [`tables/ts_102_772/`](tables/ts_102_772/) — MPE-IFEC sections |
| TS 102 809 | [`tables/ts_102_809/`](tables/ts_102_809/) — AIT, protection message sections |

## Descriptors (wire-format syntax)

| Spec | Files |
|---|---|
| EN 300 468 | [`descriptors/en_300_468/`](descriptors/en_300_468/) — 72 per-tag descriptor syntax files |
| EN 301 192 | [`descriptors/en_301_192/`](descriptors/en_301_192/) — MPE, INT, data carousel descriptors |
| EN 303 560 | [`descriptors/en_303_560/`](descriptors/en_303_560/) — TTML subtitling descriptors |
| ISO/IEC 13818-6 | [`descriptors/iso_13818_6/`](descriptors/iso_13818_6/) — DSM-CC carousel descriptors |
| TS 102 006 | [`descriptors/ts_102_006/`](descriptors/ts_102_006/) — SSU descriptors |
| TS 102 323 | [`descriptors/ts_102_323/`](descriptors/ts_102_323/) — TV-Anytime metadata descriptors |
| TS 102 727 | [`descriptors/ts_102_727/`](descriptors/ts_102_727/) — MHP deferred descriptors |
| TS 102 772 | [`descriptors/ts_102_772/`](descriptors/ts_102_772/) — MPE-IFEC descriptor |
| TS 102 809 | [`descriptors/ts_102_809/`](descriptors/ts_102_809/) — AIT application descriptors |

## Enums (coded-value tables)

| Spec | Files |
|---|---|
| EN 300 468 | [`enums/en_300_468/`](enums/en_300_468/) — `table_id`, `descriptor_tag`, `extension_tag`, `service_type`, `subtilting_type`, `teletext_type`, `linkage_type`, `announcement_type`, `running_status`, `scrambling_mode`, `polarization`, `fec_outer`, `ts_gs_mode`, `s2x_mode`, `t2_siso_miso`, `sh_diversity_mode`, `c2_tuning_frequency_type`, `ac3_service_type`, `ac4_channel_mode`, `control_remote_access` (+ 62 numbered `*-coding.md` tables) |
| EN 301 192 | [`enums/en_301_192/`](enums/en_301_192/) — `int_action_type` (+ 12 numbered coding/value tables) |
| EN 303 560 | [`enums/en_303_560/`](enums/en_303_560/) — segment/font-type coding tables |
| ISO/IEC 13818-1 | [`enums/iso_13818_1/`](enums/iso_13818_1/) — `stream_type`, `alignment_type`, `audio_type` |
| ISO/IEC 13818-6 | [`enums/iso_13818_6/`](enums/iso_13818_6/) — `biop_tag`, `biop_object_kind`, tap-use values |
| TS 101 154 | [`enums/ts_101_154/`](enums/ts_101_154/) — display-horizontal-size value tables |
| TS 101 162 | [`enums/ts_101_162/`](enums/ts_101_162/) — registration/allocation templates + domain-name registries |
| TS 102 006 | [`enums/ts_102_006/`](enums/ts_102_006/) — `descriptor_type`, `unt_action_type` (+ 6 numbered coding tables) |
| TS 102 323 | [`enums/ts_102_323/`](enums/ts_102_323/) — `crid_type`, `crid_authority_policy`, `link_type`, `tva_running_status` (+ 13 coding/value tables) |
| TS 102 809 | [`enums/ts_102_809/`](enums/ts_102_809/) — `control_code`, `reference_type` (+ 12 coding/value tables) |

## Text (prose references)

| Standard | Files |
|---|---|
| ISO/IEC 13818-6 | [`text/iso_13818_6/`](text/iso_13818_6/) — DSM-CC carousel layouts (hand-transcribed) |
| TR 101 211 | [`text/tr_101_211/`](text/tr_101_211/) — SI implementation guidelines |
| TR 101 290 | [`text/tr_101_290/`](text/tr_101_290/) — TS monitoring indicator tables |
