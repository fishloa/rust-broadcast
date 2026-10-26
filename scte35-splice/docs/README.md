# scte35-splice — spec table reference

<!-- BEGIN private-pointer (docs/specs/PRIVATE-MAP.md) -->
## Private spec transcriptions

The page-verified transcriptions of ANSI/SCTE 35; ETSI TS 103 752-1 (DVB-TA) for this crate (30 files) live only in the private companion repository `rust-broadcast-private` (the `private/` submodule) at `private/specs-verified/scte/scte35-splice/`. The standards are (c) their publishers, so the text is not published here. Old public path to private path: [`docs/specs/PRIVATE-MAP.md`](../../docs/specs/PRIVATE-MAP.md); code comments keep their original `docs/...` citations.

Still public in this directory: `enums/dvb_ta/equivalent_segmentation_type.toml`, `enums/dvb_ta/timeline_type.toml`, `enums/scte_35/device_restrictions.toml`, `enums/scte_35/segmentation_type_id.toml`, `enums/scte_35/segmentation_upid_type.toml`, `enums/scte_35/splice_command_type.toml`.
<!-- END private-pointer -->

ANSI/SCTE 35 2023r1 — Digital Program Insertion Cueing Message (syntax reference).
Excluded from the published crate.

## Tables (wire-format syntax)

| Spec | Files |
|---|---|
| SCTE 35 | `tables/scte_35/` (private: `specs-verified/scte/scte35-splice/tables/scte_35/break-duration-982-table-15-pdf-pp-57-58.md`) — splice_info_section, splice_insert, splice_schedule, time_signal, private_command, encryption algorithm |

## Descriptors (wire-format syntax)

| Spec | Files |
|---|---|
| SCTE 35 | `descriptors/scte_35/` (private: `specs-verified/scte/scte35-splice/descriptors/scte_35/dtmf-descriptor-1032-table-19-pdf-pp-66-68.md`) — splice descriptor, segmentation descriptor, avail descriptor, DTMF descriptor, time descriptor, audio descriptor |

## Enums (coded-value tables)

| Spec | Files |
|---|---|
| SCTE 35 | [`enums/scte_35/`](enums/scte_35/) — `splice_command_type`, `segmentation_type_id`, `segmentation_upid_type`, `device_restrictions` |
