# dvb-scte35 — spec table reference

ANSI/SCTE 35 2023r1 — Digital Program Insertion Cueing Message (syntax reference).
Excluded from the published crate.

## Tables (wire-format syntax)

| Spec | Files |
|---|---|
| SCTE 35 | [`tables/scte_35/`](tables/scte_35/) — splice_info_section, splice_insert, splice_schedule, time_signal, private_command, encryption algorithm |

## Descriptors (wire-format syntax)

| Spec | Files |
|---|---|
| SCTE 35 | [`descriptors/scte_35/`](descriptors/scte_35/) — splice descriptor, segmentation descriptor, avail descriptor, DTMF descriptor, time descriptor, audio descriptor |

## Enums (coded-value tables)

| Spec | Files |
|---|---|
| SCTE 35 | [`enums/scte_35/`](enums/scte_35/) — `splice_command_type`, `segmentation_type_id`, `segmentation_upid_type`, `device_restrictions` |
