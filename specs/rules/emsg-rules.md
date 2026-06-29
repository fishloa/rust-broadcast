# DASH `emsg` (Event Message Box) — behavioural rules we depend on

Curated **semantic** rules for the inband DASH/CMAF **Event Message Box** (`emsg`). **Normative
source (vendored, official ISO publicly-available standard):** ISO/IEC 23009-1:**2022 (ed.5)**
**§5.10** — `specs/fulltext/iso_iec_23009-1_dash_2022.md` (gitignored pdf2md of
`specs/iso_iec_23009-1_dash_2022_ed5.pdf`). Companion authoring/usage source: **DASH-IF IOP Part 10
v5.0.0** — `specs/fulltext/dashif_iop_part10_v5_emsg.md`. Citations are `2022 L…` for the normative
spec and `IOP L…` for DASH-IF. Consumers: **`mp4-emsg` (implements this)** and `timed-metadata`.

> ✅ **Fully grounded on the current edition.** The normative SDL is on disk (2022 ed.5 §5.10.3.3);
> `mp4-emsg/docs/emsg.md` matches it. The 2012 DASH PDF predates events — ignore it for `emsg`.

## Box & versions — §5.10.3.3 (2022 L5231) — the normative SDL

- `emsg` Container = **Segment**, Mandatory No, Quantity Zero or more (2022 L5233). Boxes whose
  scheme/value isn't declared in the MPD should not be present and are ignored by clients (2022 L5229).
- **`aligned(8) class DASHEventMessageBox extends FullBox('emsg', version, flags=0)`** (2022 L5241).
  Body field ORDER differs by version (the trap `mp4-emsg/src/lib.rs` documents):
  - **version 0** — *segment-relative*. **strings first**: `scheme_id_uri`(C-string), `value`
    (C-string), then `timescale`(u32), `presentation_time_delta`(u32), `event_duration`(u32),
    `id`(u32), then `message_data[]`.
  - **version 1** — *Movie-timeline*. **integers first**: `timescale`(u32), `presentation_time`(u64),
    `event_duration`(u32), `id`(u32), then `scheme_id_uri`, `value`, then `message_data[]`.
  - Only 0/1 defined. `message_data[]` fills the remainder of the box.

## Fields — §5.10.3.3.4 (2022 L5265)

- **`scheme_id_uri`** (null-terminated UTF-8 C-string) — identifies the message scheme; URN or URL
  syntax; URL form recommended to carry an `mmyyyy` date (2022 L5267). Defines `message_data`'s meaning.
- **`value`** (C-string) — scheme-defined value space (2022 L5271); IOP: else empty (IOP L237).
- **`timescale`** (u32) — ticks/sec for the `event_duration` **and** the `presentation_time_delta`/
  `presentation_time` fields (**both versions** in ed.5); should equal the timescale of a track in
  the carrying Segment, and be identical for all events in one Event Stream (2022 L5275). IOP: equals
  the init segment's `mdhd` timescale; **CMAF requires equality** (IOP L239/L241).
- **`presentation_time`** (u64, **v1**) — event time **on the Movie timeline**, in `timescale`,
  **adjusted by `InbandEventStream@presentationTimeOffset`** (in `@timescale`); **shall not be less
  than the carrying Segment's EPT** (2022 L5279).
- **`presentation_time_delta`** (u32, **v0**) — delta between event time and the **segment's earliest
  presentation time (EPT)** (2022 L5277). EPT = first `sidx`'s `earliest_presentation_time` if a
  segment index is present, else the earliest AU presentation time. Non-negative — can't signal a
  past start.
- **`event_duration`** (u32) — duration in media presentation time, in `timescale` units;
  **`0xFFFFFFFF` = unknown duration** (scheme-defined interpretation) (2022 L5287). *(Confirmed
  against ed.5; the older 2019 pdf2md had dropped digits to `0xFFFF`.)*
- **`id`** (u32) — instance id, scoped to the (`scheme_id_uri`,`value`) pair; same id in that scope ⇒
  **equivalent**, process any one (2022 L5289).
- **`message_data`** (u8[]) — fills the remainder of the box; may be empty (2022 L5293). For SCTE-35
  (`urn:scte:scte35…`) it carries a `splice_info_section` (see `mp4-emsg::EmsgBox::is_scte35`).

## Carriage in **MPEG-2 TS** — §5.10.3.3.5 (2022 L5295) — workspace-relevant

DASH segments may be **MPEG-2 TS** (not just ISOBMFF), and the `emsg` box can ride in TS:
- **Reserved fixed PID `0x0004`** for `emsg`-carrying packets (2022 L5299). (Matches the
  `0x0004` "adaptive-streaming" PID in `h222_0-rules.md` — the cross-layer tie-in.)
- The packet carrying the **start** of an `emsg` sets **`payload_unit_start_indicator = 1`**, payload
  begins with the box, the full `Box.type` present in that first packet, payload ≥ 8 bytes (2022 L5301).
- Continuation packets follow on the same PID; the **last packet is padded with adaptation-field
  stuffing** — exactly the AF stuffing model in `h222_0-rules.md` §2.4.3.5.
- A segment shall contain only complete boxes.
- → `mpeg-ts`/`dvb-si` could surface a PID-`0x0004` `emsg` reassembled across TS packets and hand the
  bytes to `mp4-emsg` — a concrete future integration point.

## MPD `EventStream` / `Event` — §5.10.2 (2022 L5059) — for `timed-metadata`

- `EventStream` (Table, 2022 L5059): `@schemeIdUri` (M), `@value` (O), `@timescale` (O, default 1),
  `@xlink:href`/`@actuate` for external streams; `Event` 0..N, **ordered by non-decreasing
  presentation time**.
- `Event`: `@presentationTime` (OD, default 0, relative to **Period start**, ÷`@timescale` → s),
  `@duration` (O, ÷`@timescale`; absent = unknown), `@id` (O, scoped to the `@schemeIdUri`+`@value`
  pair), `@messageData` (O, compact-string alternative to an XML body).
- The MPD-event vs inband-`emsg` correspondence is the `timed-metadata` mapping target (same scheme/
  value/id identity; timeline = composition time after edit list, per `dash-mpd-rules`/`isobmff-rules`).

## Repetition / timing rules (relevant to `timed-metadata` conversions)

- **v1 repeats verbatim** across segments — `presentation_time` is media-timeline-absolute, so the
  same box may appear in 2+ segments unchanged (IOP L294).
- **v0 must be rewritten per segment** — `presentation_time_delta` is EPT-relative, so each repeat
  needs a different delta; a started-in-the-past event updates `event_duration` to the remaining
  time (IOP L300). A v0↔v1 converter must recompute the timing field against EPT vs Period origin.
- Repeat across all segments in the active duration (IOP L302); duplicate across Representations of an
  AdaptationSet so switching doesn't drop an event (IOP L284); when crossing Adaptation Sets with
  different `@presentationTimeOffset`/timescale, **adjust** time fields to align (IOP L288).
- **Dispatch modes** (IOP L267): *on-receive* (`event_duration` = active interval, "state" events) vs
  *on-start* (`event_duration` = valid dispatch window, "change-of-state" toggles).
- **Early insertion** (IOP L312): insert ≥ the scheme's in-advance offset, default **≥4 s** before the
  event start (e.g. 2-s segments → the event box repeats in the 2 prior segments + the start segment).

## MPD signalling — `InbandEventStream` (IOP L209, L219)

- An `InbandEventStream` element announces a scheme/value the client should process; boxes with an
  unannounced (`scheme_id_uri`,`value`) **may be ignored** (IOP L209). `@schemeIdUri` must match the box
  `scheme_id_uri` (IOP L219); `@value` should match `value` (IOP L221); `@timescale`, if present, must
  match the box `timescale` (IOP L223).

## Code-conformance notes (tracked)

1. `mp4-emsg`: already honours the **v0/v1 field-order difference**, null-terminated strings, and
   `size`/`version` recomputed on serialize (no raw passthrough) — verified against this source.
2. `timed-metadata`: v0 (EPT-relative `presentation_time_delta`) ↔ v1 (Movie-timeline
   `presentation_time`) conversion must recompute the timing field + honour the
   `InbandEventStream@presentationTimeOffset` rule (2022 L5279) and `timescale` (2022 L5275) —
   agreeing with the segment's `sidx` / MPD `SegmentTimeline` (see `isobmff-rules`/`dash-mpd-rules`).
3. ✅ Byte layout now spec-grounded on ISO/IEC 23009-1:2022 ed.5 §5.10.3.3 (vendored) — supersedes
   the earlier DASH-IF-only footing; `mp4-emsg` matches it.
4. `mpeg-ts`/`dvb-si` (future): reassemble a PID-`0x0004` `emsg` from TS packets (PUSI start,
   AF-stuffing tail) and hand to `mp4-emsg` (§5.10.3.3.5, 2022 L5295).
