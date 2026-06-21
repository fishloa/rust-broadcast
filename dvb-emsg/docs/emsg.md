# DASH Event Message Box (`emsg`) — field set & semantics

_Source: DASH-IF IOP Part 10 (Events and Timed Metadata) V5.0.0 (2023-01),
§6.1 + Table 6-2 "Recommended usage of fields in the DASHEventMessageBoxes"
(PDF pp. 8–10), render-verified._

This document transcribes the **field set and semantics** of the DASH Event
Message Box (`emsg`) as carried inband in DASH/CMAF media segments. The `emsg`
box delivers sparse, timed application events (e.g. SCTE 35 splice signalling,
ID3 metadata, ad/tracking triggers) alongside the media.

> ## ⚠ Softer footing than a fully-free spec — read this first
>
> DASH-IF Part 10 gives the **field semantics and types** (Table 6-2) but
> **does not** reproduce the normative ISOBMFF box syntax. The normative
>
> ```
> aligned(8) class EventMessageBox extends FullBox('emsg', version, flags = 0)
> ```
>
> declaration — with its exact field ordering, the `version`-gated branch
> (`presentation_time_delta`/`presentation_time`), and the null-terminated
> string layout — lives in **ISO/IEC 23009-1 §5.10.3.3** (referenced throughout
> Part 10 as "MPEG-DASH [1] clause 5.10.3.3"). That ISO spec is **paid and not
> vendored** in this repo. Part 10 §6.1 states verbatim:
>
> > "Inband events are signalled in media segments using the
> > `DASHEventMessageBox` as defined in MPEG-DASH [1] clause 5.10.3.3."
>
> and Table 6-2 introduces the field list with:
>
> > "Each Inband event is defined by the following parameters in the MPEG-DASH
> > [1] `DASHEventMessageBox` 5.10.3.3, shown in Table 6-2."
>
> **Consequence:** the field *names, types, and meanings* below are
> render-verified from a free spec (DASH-IF Part 10). The byte-exact wire
> layout / field ordering of the `'emsg'` `FullBox` (and the v0/v1 ordering
> difference) is **reconstructed from the ISO box syntax**, which is **not in
> this repo** — treat that ordering as the well-known public `emsg` layout
> (widely reproduced, e.g. in MPEG-DASH and CMAF), but flag it as the one part
> not directly verifiable against a vendored source. ⚠

---

## emsg box versions

Part 10 §6.1 / Table 6-2 (`emsg.version`, "flag" type):

> "Defined by the version flag, version 0 is used for segment relative timing,
> version 1 for representation relative timing."

- **version 0** — timing is relative to the **segment's earliest presentation
  time (EPT)**; carries `presentation_time_delta` (u32).
- **version 1** — timing is relative to the **media representation / `Period@start`**
  (after `@presentationTimeOffset` adjustment); carries `presentation_time`
  (u64).

The two versions carry the *same* logical field set **except** for the
presentation-time field, whose type and reference point differ by version.

---

## Field set — DASH-IF Part 10 Table 6-2 (pp. 9–10), render-verified

| Field | Field type | Comment (transcribed from Table 6-2) |
|-------|------------|--------------------------------------|
| `emsg.version` | flag | Defined by the version flag, version 0 is used for segment relative timing, version 1 for representation relative timing. |
| `emsg.scheme_id_uri` | string | Signals a uri to define the scheme as defined by the developer of the scheme (which may be another external organization). |
| `emsg.value` | string | Field may be set according to the guidelines provided by the used scheme, otherwise it shall be set to the empty string. |
| `emsg.timescale` | unsigned int 32 | This field should be the same as in mdhd box of the initialization segment of the Representation. **NOTE:** In a CMAF track, the mdhd timescale and this field are required to be equal according to CMAF [3]. |
| `emsg.presentation_time` | unsigned int 64 | **(version 1 only)** Shall be set to the relative time offset of the event start time. `presentation_time` is relative to `Period@start` adjusted with the `@presentationTimeOffset` associated to the media representation carrying the `DASHEventMessageBox`. **NOTE:** the InbandEventStream element, since the 5th edition of MPEG-DASH no longer carries the `@presentationTimeOffset`. |
| `emsg.presentation_time_delta` | unsigned int 32 | **(version 0 only)** Shall be set to the relative time offset of event start time in the scale of `timescale`. `presentation_time_delta` is relative to the segment's earliest presentation time. |
| `emsg.event_duration` | unsigned int 32 | Shall be set to the event duration in units of the `timescale`. |
| `emsg.id` | unsigned int 32 | Unique identifier to distinguish events with the same `scheme_id_uri` and `value` and to detect repetitions. Two Inband event instances are identified to be equivalent if they have identical values in the following fields: `scheme_id_uri`, `value`, `id`. |
| `emsg.message_data` | unsigned int 8[] | Shall be set to the message data payload, i.e. the data to be passed to the application containing the scheme specific message. It may be empty if no message data is needed to be passed to the application. |

### Field notes

- **`scheme_id_uri`** and **`value`** are **null-terminated UTF-8 strings** on
  the wire (per the ISO box syntax — see caveat). `value` is set to the **empty
  string** (a lone `0x00` terminator) when the scheme defines no value.
- **`message_data[]`** is the **remaining bytes** of the box after the fixed
  fields and the two null-terminated strings — its length is implicit from the
  box size, not carried as a count.
- **`presentation_time` (v1, u64)** vs **`presentation_time_delta` (v0, u32)**
  are mutually exclusive: exactly one is present, selected by `version`.

---

## Wire layout (reconstructed from ISO/IEC 23009-1 §5.10.3.3 — ⚠ not vendored)

The following ordering is the well-known public `emsg` `FullBox` layout. It is
**not** transcribed from a vendored source (see top caveat); it is included for
implementation orientation only and must be confirmed against ISO/IEC 23009-1
before being treated as normative.

**version 0** (segment-relative):

```
aligned(8) class DASHEventMessageBox extends FullBox('emsg', version = 0, flags = 0) {
    string   scheme_id_uri;            // null-terminated UTF-8
    string   value;                    // null-terminated UTF-8
    unsigned int(32) timescale;
    unsigned int(32) presentation_time_delta;
    unsigned int(32) event_duration;
    unsigned int(32) id;
    unsigned int(8)  message_data[];   // remaining bytes
}
```

**version 1** (representation/Period-relative):

```
aligned(8) class DASHEventMessageBox extends FullBox('emsg', version = 1, flags = 0) {
    unsigned int(32) timescale;
    unsigned int(64) presentation_time;
    unsigned int(32) event_duration;
    unsigned int(32) id;
    string   scheme_id_uri;            // null-terminated UTF-8
    string   value;                    // null-terminated UTF-8
    unsigned int(8)  message_data[];   // remaining bytes
}
```

> ⚠ Note the **field ordering differs between v0 and v1** in the public ISO
> layout: v0 places the two strings *first* (before the integer fields); v1
> places the integer fields first and the strings last. DASH-IF Part 10
> Table 6-2 lists the fields by *meaning*, not by wire order, so this ordering
> is the part most reliant on the (paid, non-vendored) ISO §5.10.3.3 source —
> verify before implementing a parser.

---

## SCTE 35 carriage in `emsg`

DASH-IF Part 10 §7.3 "Common use-cases" (p. 15) lists, for MPD events:

> "Ad slots and splice points signalling using SCTE-35 [i.1]"

and the timed-metadata examples (§9.2.5 / Examples 9-1, 9-2, pp. 16–17) use the
SCTE 35 scheme URI directly in the MPD signalling, e.g.:

```
value="urn:scte:scte35:2013:bin  urn:mpeg:dash:event:2012 "
```

(Example 9-1) and:

```
schemeIdUri="urn:dashif:events:metadataconfiguration:2022"
value="urn:scte:scte35:2013:bin "
```

(Example 9-2, Event Message Track based on MPEG-B Part 18 [4]).

**SCTE 35 in emsg:** the `emsg.scheme_id_uri` is set to a SCTE 35 scheme URI of
the form **`urn:scte:scte35:...`** (e.g. `urn:scte:scte35:2013:bin` for the
binary `splice_info_section`), and the SCTE 35 `splice_info_section` bytes are
carried verbatim in **`emsg.message_data[]`**.

> ⚠ The exact SCTE 35 scheme-URI strings and the message_data binding rules are
> defined by **SCTE 214-1** ("MPEG DASH for IP-Based Cable Services Part 1: MPD
> Constraints and Extensions", listed as informative reference [i.2] in Part 10
> §2.2) and the SCTE 35 spec itself ([i.1] ANSI/SCTE 35 2020) — neither is
> transcribed here. Part 10 only shows the scheme URI in the timed-metadata MPD
> examples above; it does not itself enumerate the SCTE 35 emsg scheme URIs.

---

## Related: Event Message Track boxes (MPEG-B Part 18) — context only

Part 10 §9.2.6 (p. 17) notes that **MPEG-B Part 18 [4]** (ISO/IEC 23001-18)
defines an Event Message **Track** carrying:

- `EventMessageInstanceBox` (`emib`) — carries event messages in samples;
- `EventMessageEmptyBox` (`emeb`) — marks sample durations with no active event.

> "The emib box has fields that correspond to DASHEventMessageBox, namely
> `scheme_id_uri`, `value`, `id`, `value`, `event_duration` and
> `presentation_time_delta`, and can therefore be used directly for carriage of
> DASH events in timed metadata tracks. … As the `presentation_time_delta` is a
> 64 bit signed integer both advance and past events can be signalled."

This is **out of scope** for the `emsg` box itself (it is a separate ISO/IEC
23001-18 box family) and is noted here only to disambiguate it from `emsg`. ⚠
The `emib`/`emeb` syntax is in ISO/IEC 23001-18 (paid, not vendored).

---

## ⚠ Flags summary

1. **Box syntax is paid-spec footing.** Field *semantics/types* are from free
   DASH-IF Part 10 Table 6-2; the byte-exact `FullBox` layout (incl. the v0/v1
   field-ordering difference and the null-terminated-string encoding) is from
   ISO/IEC 23009-1 §5.10.3.3, **not vendored**. This deliverable is softer
   footing than a fully-free transcription — flagged per the brief.

2. **v0/v1 field ordering differs** in the public ISO layout (strings-first in
   v0, integers-first / strings-last in v1). Table 6-2 lists fields by meaning,
   not wire order. Verify ordering against §5.10.3.3 before implementing.

3. **`message_data[]` length is implicit** (box-size derived), not a counted
   field.

4. **SCTE 35 scheme URIs** (`urn:scte:scte35:...`) and the message_data binding
   are defined by SCTE 214-1 / SCTE 35, not by Part 10. Part 10 only shows the
   URI in MPD timed-metadata examples (9-1, 9-2).

5. **`emib`/`emeb` (MPEG-B Part 18)** are a *separate* box family for Event
   Message *Tracks*, not the `emsg` box; noted for disambiguation only, syntax
   not vendored.
