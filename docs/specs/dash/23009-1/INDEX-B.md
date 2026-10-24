# ISO/IEC 23009-1:2022 (DASH, 5th edition) - public clause map, part B (clauses 5.3.10 to 7)

Public, own-words implementer notes for **ISO/IEC 23009-1:2022(E)** clauses 5.3.10 to 7 (printed pp. 93-178, PDF pp. 101-186; printed page = PDF page - 8), written to
sit beside the part-A notes (clauses 3 to 5.3.9, PR #1516, index `INDEX.md` there). The spec text itself is **not** reproduced here (the repository is public); the
page-verified verbatim transcription lives in the private submodule at `private/specs-verified/dash/23009-1/` (branch `dash-specs-B` of `rust-broadcast-private`),
together with `VERIFICATION-B.md` (commands, exit codes, every page compared, every correction, gaps), which exists **only** in private.

How to read a note: each file gives what the clause defines, the behaviour an implementer needs (shall/should/may as the clause states them, formulas as formulas), an
inventory table (name | use | type | default | gloss), enumerations and URN values, a compact structure outline in place of the XML syntax, a "Quirks" list of
oddities of the print (cross-references that point to the wrong clause, misspelt attribute names, ...), and a citation of the private verbatim text.

## Clause map

| Clause | Subject | Public notes | Private verbatim file | Printed pp. |
|---|---|---|---|---|
| 5.3.10 | Label, GroupLabel | [`05-3-10-label.md`](05-3-10-label.md) | `05-3-10_to_5-3-13.md` | 93-94 |
| 5.3.11 | Preselection (descriptor + element, conformance orders) | [`05-3-11-preselection.md`](05-3-11-preselection.md) | `05-3-10_to_5-3-13.md` | 95-99 |
| 5.3.12 | Initialization Set / Group / Presentation | [`05-3-12-initialization-set.md`](05-3-12-initialization-set.md) | `05-3-10_to_5-3-13.md` | 99-103 |
| 5.3.13 | Resynchronization (Resync) | [`05-3-13-resynchronization.md`](05-3-13-resynchronization.md) | `05-3-10_to_5-3-13.md` | 103-106 |
| 5.4 | MPD updates, MPD Reset | [`05-4-mpd-updates.md`](05-4-mpd-updates.md) | `05-4_to_5-7.md` | 106-108 |
| 5.5 | MPD assembly (XLink, resolve-to-zero) | [`05-5-mpd-assembly.md`](05-5-mpd-assembly.md) | `05-4_to_5-7.md` | 108-110 |
| 5.6 | Base URL processing | [`05-6-base-url.md`](05-6-base-url.md) | `05-4_to_5-7.md` | 110-113 |
| 5.7 | Program information | [`05-7-program-information.md`](05-7-program-information.md) | `05-4_to_5-7.md` | 113-114 |
| 5.8.1-5.8.4 | Descriptors: generic, content protection, role, ..., UTCTiming, output protection | [`05-8-descriptors.md`](05-8-descriptors.md) | `05-8.md` | 114-122 |
| 5.8.5 | Specific schemes: all scheme URIs, UTC timing methods (Table 35), role values (Table 34), HDCP (Table 36) | [`05-8-5-schemes.md`](05-8-5-schemes.md) | `05-8.md` | 122-133 |
| 5.9 | DASH metrics descriptor | [`05-9-metrics.md`](05-9-metrics.md) | `05-9_to_5-10.md` | 133-135 |
| 5.10 | Events: MPD events, inband `emsg`, DASH-specific events (validity expiry, patch, update, callback, termination) | [`05-10-events.md`](05-10-events.md) | `05-9_to_5-10.md` | 135-146 |
| 5.11 | MPD chaining (regular, fallback) | [`05-11-mpd-chaining.md`](05-11-mpd-chaining.md) | `05-11_to_5-15.md` | 146-148 |
| 5.12 | Producer Reference Time | [`05-12-producer-reference-time.md`](05-12-producer-reference-time.md) | `05-11_to_5-15.md` | 148-150 |
| 5.13 | Leap seconds | [`05-13-leap-seconds.md`](05-13-leap-seconds.md) | `05-11_to_5-15.md` | 150-152 |
| 5.14 | Content Popularity Rate | [`05-14-content-popularity-rate.md`](05-14-content-popularity-rate.md) | `05-11_to_5-15.md` | 152-154 |
| 5.15 | MPD patch framework | [`05-15-mpd-patch.md`](05-15-mpd-patch.md) | `05-11_to_5-15.md` | 155-159 |
| 6 | Segment formats (types, ISO BMFF, MPEG-2 TS, index segments, `pcrb`) | [`06-segment-formats.md`](06-segment-formats.md) | `06-segment-formats.md` | 159-172 |
| 7 | Combined semantics: timeline formulas, authoring rules | [`07-combined-semantics.md`](07-combined-semantics.md) | `07-combined-semantics.md` | 172-178 |

## Tables and figures covered

Tables 24-48, 50, 51 (Table 49 is an XML listing): 24-25 Label/GroupLabel; 26 Preselection; 27 InitializationSet; 28 Resync; 29 XLink attributes; 30 BaseURL; 31 ProgramInformation;
32 DescriptorType; 33 ContentProtection; 34 Role values; 35 UTC timing methods; 36 HDCP values; 37 Metrics; 38 EventStream; 39 Event; 40 `urn:mpeg:dash:event:2012` values;
41-42 callback event; 43 presentation-termination values; 44 MPD chaining value parameters; 45 ProducerReferenceTime; 46 LeapSecondInformation; 47 ContentPopularityRate;
48-49 PatchLocation; 50 Patch; 51 ISO BMFF Resync point box order. Figures: 4 (initialization hierarchy), 5 (MPD validity expiration), 6 (Representation Index Segment).

## Cross-cutting facts

- All scheme URIs of 5.8.5 and the other clauses in one table: [`05-8-5-schemes.md`](05-8-5-schemes.md).
- Timing formulas of clause 7 (T_M = T_P - T_O; PTS-based T_P for MPEG-2 TS): [`07-combined-semantics.md`](07-combined-semantics.md). Segment availability derivations are part-A (5.3.9.5.3).

## Gaps (not covered by these sources / these notes)

- Annexes A-K (annex B schema, C MIME types, D metric keys, E byte-range templating, F other formats, G examples, H SRD) and clause 8 (profiles).
- No element tables for `InitializationGroup` / `InitializationPresentation` in clause 5.3.12 (prose and Figure 4 only).
- No `dvb:` attributes in clause 5.6 (not part of ISO/IEC 23009-1).
- Figures are described, not reproduced.
