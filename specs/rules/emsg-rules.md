# DASH `emsg` (Event Message Box) — behavioural rules we depend on

Curated **semantic** rules for the inband DASH/CMAF **Event Message Box** (`emsg`). The normative
**box byte-syntax** is MPEG-DASH (ISO/IEC 23009-1:2014+) **§5.10.3.3** — *not* in our vendored 2012
DASH PDF. The **field semantics + version split + authoring/timing rules** come from a free source
already on disk: **DASH-IF IOP Part 10 (Events & Timed Metadata) v5.0.0** —
`specs/fulltext/dashif_iop_part10_v5_emsg.md` (gitignored pdf2md of
`specs/dashif_iop_part10_v5.0.0_emsg.pdf`). Each rule cites § + line. Consumers: **`mp4-emsg`
(already implements this)** and `timed-metadata`. Decisions cite here.

> ✅ **Not a gap.** `mp4-emsg` already implements the box, grounded on this same DASH-IF Part 10
> source and transcribed in `mp4-emsg/docs/emsg.md`. This doc is the workspace-level curated summary;
> the exact SDL byte layout lives in `mp4-emsg/docs/emsg.md` (acknowledged softer footing — the
> normative SDL is 23009-1:2014+ §5.10.3.3, which we have not vendored).

## Box & versions — §6.1 (fulltext L205, L233)

- Inband events are signalled in media segments by the **`DASHEventMessageBox` (`emsg`)**, a
  `FullBox('emsg', version, flags=0)`, defined in MPEG-DASH §5.10.3.3 (L205).
- **Two versions, and the body field ORDER differs** (the trap `mp4-emsg/src/lib.rs` documents):
  - **version 0** — *segment-relative* timing. Body: the two **null-terminated UTF-8 strings first**
    (`scheme_id_uri`, `value`), then `timescale`, `presentation_time_delta`(u32), `event_duration`,
    `id`, then `message_data[]`.
  - **version 1** — *representation/`Period@start`-relative* timing. Body: **integer fields first**
    (`timescale`, `presentation_time`(u64), `event_duration`, `id`), then the two strings, then
    `message_data[]`.
  - Selecting the timing variant **is** selecting the box version (L233). Only 0/1 defined.

## Fields — Table 6-2 (fulltext L229)

- **`scheme_id_uri`** (string) — scheme URI defined by the scheme owner (L235).
- **`value`** (string) — per the scheme's guidelines; else the **empty string** (L237).
- **`timescale`** (u32) — should equal the `mdhd` timescale of the Representation's init segment
  (L239); in **CMAF this equality is required** (L241). Same factor used by `timed-metadata`.
- **`presentation_time`** (u64, **v1**) — event start time **relative to `Period@start` adjusted by
  the media representation's `@presentationTimeOffset`** (L243). Since DASH 5th ed.
  `InbandEventStream` no longer carries `@presentationTimeOffset` — use the SegmentTemplate/
  SegmentBase one (dash-mpd-rules `@presentationTimeOffset`, L245).
- **`presentation_time_delta`** (u32, **v0**) — event start **relative to the segment's earliest
  presentation time (EPT)** in `timescale` units (L247). Non-negative — can't signal a past start.
- **`event_duration`** (u32) — event duration in `timescale` units (L249).
- **`id`** (u32) — unique per (`scheme_id_uri`,`value`); dedup/repetition key (L251).
- **`message_data`** (u8[]) — opaque scheme payload, may be empty (L259). For SCTE-35
  (`urn:scte:scte35…`) it carries a `splice_info_section` (see `mp4-emsg::EmsgBox::is_scte35`).
- **Event equivalence** = identical (`scheme_id_uri`, `value`, `id`) (L253).

## Repetition / timing rules (relevant to `timed-metadata` conversions)

- **v1 repeats verbatim** across segments — `presentation_time` is media-timeline-absolute, so the
  same box may appear in 2+ segments unchanged (L294).
- **v0 must be rewritten per segment** — `presentation_time_delta` is EPT-relative, so each repeat
  needs a different delta; a started-in-the-past event updates `event_duration` to the remaining
  time (L300). A v0↔v1 converter must recompute the timing field against EPT vs Period origin.
- Repeat across all segments in the active duration (L302); duplicate across Representations of an
  AdaptationSet so switching doesn't drop an event (L284); when crossing Adaptation Sets with
  different `@presentationTimeOffset`/timescale, **adjust** time fields to align (L288).
- **Dispatch modes** (L267): *on-receive* (`event_duration` = active interval, "state" events) vs
  *on-start* (`event_duration` = valid dispatch window, "change-of-state" toggles).
- **Early insertion** (L312): insert ≥ the scheme's in-advance offset, default **≥4 s** before the
  event start (e.g. 2-s segments → the event box repeats in the 2 prior segments + the start segment).

## MPD signalling — `InbandEventStream` (fulltext L209, L219)

- An `InbandEventStream` element announces a scheme/value the client should process; boxes with an
  unannounced (`scheme_id_uri`,`value`) **may be ignored** (L209). `@schemeIdUri` must match the box
  `scheme_id_uri` (L219); `@value` should match `value` (L221); `@timescale`, if present, must match
  the box `timescale` (L223).

## Code-conformance notes (tracked)

1. `mp4-emsg`: already honours the **v0/v1 field-order difference**, null-terminated strings, and
   `size`/`version` recomputed on serialize (no raw passthrough) — verified against this source.
2. `timed-metadata`: v0 (EPT-relative `presentation_time_delta`) ↔ v1 (Period-origin
   `presentation_time`) conversion must recompute the timing field + honour the
   `@presentationTimeOffset` rule (L243/L245) and `timescale` (L239) — agreeing with the segment's
   `sidx` / MPD `SegmentTimeline` (see `isobmff-rules`/`dash-mpd-rules`).
3. To fully spec-ground the **byte layout** (vs the current DASH-IF + `mp4-emsg/docs/emsg.md`
   footing), vendor ISO/IEC 23009-1:2014+ §5.10.3.3.
