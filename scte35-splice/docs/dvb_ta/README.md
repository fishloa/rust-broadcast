# DVB Targeted Advertising — binary SCTE 35 profile (ETSI TS 103 752-1)

_Source: ETSI TS 103 752-1 V1.2.1 (2024-01), render-verified against `specs/etsi_ts_103_752-1_v01.02.01_dvb_ta.pdf`_

ETSI TS 103 752-1 ("Carriage and signalling of placement opportunity information
in DVB Transport Streams", the DVB Targeted Advertising / DVB-TA Part 1
deliverable) defines a **typed DVB profile over ANSI/SCTE 35**. It does not
re-define the SCTE 35 wire format; instead it:

1. **profiles** how the base SCTE 35 structures (`splice_info_section()`,
   `splice_insert()`, `time_signal()`, `segmentation_descriptor()`) are
   constrained and used for Digital Advertising Substitution (DAS); and
2. **adds a small amount of NEW binary syntax** — the DVB DAS descriptor, the
   binary DSM-CC stream-event payload, and a Compact SCTE 35 encoding for
   watermark carriage.

This directory transcribes **only the new/profiled binary (wire) syntax**, which
extends the existing `dvb-scte35` crate. XML / MPD profiling and architectural
prose are explicitly out of scope (see "Out of scope" below).

## NEW binary syntax defined here (transcribable in `dvb-scte35`)

| Doc | Spec | New structure |
|-----|------|---------------|
| [`das-descriptor.md`](das-descriptor.md) | §5.3.5.16, Tables 1–2 (p.17) | `DVB_DAS_descriptor()` — a private SCTE 35 splice descriptor (tag `0xF0`, identifier `"DVB_"`) |
| [`dsmcc-stream-event.md`](dsmcc-stream-event.md) | §6.3.1, Tables 3–4 (pp.18–20) | `DSM-CC_stream_event_payload_binary()` — binary payload wrapping (or referencing) a full SCTE 35 section, base-64 encoded into a DSM-CC stream event |
| [`compact-scte35.md`](compact-scte35.md) | §8.3.3, Tables 5–10 (pp.26–28) | `compact_SCTE_35()` / `compact_time_signal()` / `compact_splice_insert()` — a compact binary alternative for low-capacity watermark carriage |

## PROFILING of base SCTE 35 (already typed in `dvb-scte35`; constraints only)

| Doc | Spec | What it constrains |
|-----|------|--------------------|
| [`scte35-profiling.md`](scte35-profiling.md) | §5.3.4–5.3.5 (pp.14–17) | Section structure + `splice_insert()` / `time_signal()` / `segmentation_descriptor()` field constraints, PPO/DPO `segmentation_type_id` usage, `segmentation_upid_type`/UPID format, segment/sub-segment numbering rules |

No NEW wire structure is introduced by the profiling clauses — they pin the
values/usage of fields that base SCTE 35 already defines and that `dvb-scte35`
already parses. Each constraint is cited so the typed layer can enforce it.

## Out of scope (NOT transcribed)

- **XML / MPD profiling** — the spec mentions XML-based decisioning interfaces
  (the companion ETSI TR 103 752-2 covers "Interfacing to an advert decisioning
  service"); TS 103 752-1 itself carries no XML syntax tables.
- **Architecture / deployment prose** — §4 Overview, §8.1–8.2 watermark
  architecture (Figures 3–4), §6.4 timing of signalling, Annex A (HbbTV ADB2
  deployment), Annex B (UX considerations) — all informative.
- **§8.4 ATSC A/335** — references the external ATSC A/335 video-watermark
  carrier; no DVB-defined binary syntax (informative guidance + usage rules
  only).
- **SCTE 104 (§5.2, §7.4)** — the automation→encoder interface; covered by the
  separate `scte104` crate, not new here.
- The base **SCTE 35** structures themselves (`splice_info_section()`,
  `splice_insert()`, `time_signal()`, `segmentation_descriptor()`, CRC_32) —
  defined in ANSI/SCTE 35 and already implemented in `dvb-scte35`. This profile
  only references and constrains them.

## Normative cross-references used by this profile

- **[1] ANSI/SCTE 35** — base splice information (`splice_info_section()` Table 5;
  encryption algorithms Table 27).
- **[2] ANSI/SCTE 104** — automation interface (pre_roll_time etc.).
- **[3] IETF RFC 3986** — URI format for the UPID (`urn:<reverse-domain>:<id>`).
- **[4] IETF RFC 4648** — base-64 encoding of the DSM-CC stream-event payload.
- **[5] ETSI TS 101 162** — `private_data_specifier` / CA_system_id allocation.
- **[6] DVB URI** — carousel-object naming in the stream-event payload.
- **[7] ETSI TS 103 286-2** — TEMI timeline-descriptor receiver support.
- **[8] ISO/IEC 13818-1** — TEMI/PTS timing relationship (clause U.3.6),
  `private_stream_1` stream_id `1011 1101`, stream_type 6.
- **[9] ETSI TS 102 809** — DSM-CC stream-event descriptor carriage (245-byte
  per-event payload limit).
- **[10] ATSC A/335** — video watermark carrier (out of scope).
