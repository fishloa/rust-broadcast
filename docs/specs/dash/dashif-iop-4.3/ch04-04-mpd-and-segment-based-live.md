---
source: "DASH-IF Interoperability Points v4.3 (2018-11-15), private/specs/dashif_iop_v4.3_guidelines.pdf"
clauses: "4.5 (4.5.1-4.5.3), 4.6 (4.6.1-4.6.4)"
pdf_pages: 114-124
printed_pages: 93-103
private_verbatim: private/specs-verified/dash/dashif-iop-4.3/ch04-04-mpd-and-segment-based-live.md
verified_against_render: true
notes_original_wording: true
---

# IOP v4.3 ch. 4.5-4.6 - MPD + segment based (Main) live; live content as on-demand

Printed page = PDF page - 21. Own-wording notes. Inband events (`urn:mpeg:dash:event:2012`, `emsg`, MPD validity expiry, patch, full MPD) are defined in 23009-1 5.10.4
(`private/specs-md/iso_iec_23009-1_dash_2022_ed5.md`, Table 40, ~l. 5326-5360); box layout: [`../../../../specs/rules/emsg-rules.md`](../../../../specs/rules/emsg-rules.md),
[`../../../../mp4-emsg/docs/emsg.md`](../../../../mp4-emsg/docs/README.md#private-spec-transcriptions). Previous [ch04-03](ch04-03-live-edge-and-mpd-updates.md); next [ch04-05](ch04-05-time-sync-robust-operation.md).

## 4.5 Main live (MPD + segment controlled)

### 4.5.1 MPD information (pp. 93-95)
Compared with the simple offering: `minimumUpdatePeriod` present and recommended 0 (instant updates); `publishTime` present (identifies MPD versions); every Representation of all audio Adaptation Sets
(or, with no audio, all video sets) **shall** carry an `InbandEventStream` with scheme `urn:mpeg:dash:event:2012` and `@value` 1 or 3; other Representations may; the element **shall** be signalled on
Adaptation Set level only. Segment derivation is as 4.4.2.2 (end of last Period = NOW + MUP; with MUP=0 exact `@r`).

Table 16 inventory (item | status): `MPD@type` dynamic (mandatory); `MPD@publishTime` mandatory (a later value means an update per 23009-1 5.4 of earlier ones); `MPD@availabilityStartTime` mandatory;
`MPD@minimumUpdatePeriod` mandatory, 0 recommended; `Period@start` mandatory; `AdaptationSet.InbandEventStream` mandatory; `SegmentTemplate@media` mandatory; `@startNumber` optional/defaulted; `@duration` or `SegmentTimeline`.

> **Notes (pp. 93-95):** (1) Table 16's gloss of `InbandEventStream` cites scheme `urn:mpeg:dash:event:2014` with values 1, 2 or 3, while the bullets above it, Table 17 and 4.5.2.2 (except its first bullet, which repeats `2014`) use `...:2012` with 1 or 3;
> 23009-1:2022 defines only `...:2012` (md l. 765, 5326). Match `...:2012`; `...:2014` looks like an IOP typo. (2) Cross-references to "Table 9" and "Table 6" are stale. (3) one list says all Representations of audio sets and another says Adaptation Set level only (paraphrased) in the same list are both printed.
> (4) The scheme attribute is spelt `@scheme_id_uri` here (the `emsg` field name) rather than MPD `@schemeIdUri`.

### 4.5.2 Service offering
**Event scheme values (IOP Table 17; p. 96).** Scheme `urn:mpeg:dash:event:2012`:
- value 1: MPD validity expiration events are present in the Representation; the expiry is carried at least in the last segment whose earliest presentation time precedes the event time.
- value 2: as 1, plus an MPD Patch in `message_data`.
- value 3: as 1, plus a full MPD in `message_data`.
DVB-DASH does not include value 3. Meaning of an expiry event: the MPD with a given publish time is valid only up to a given media presentation time.

> **Note:** the text calls the table "Table 10"; clause refs 5.10.4.2/.3/.4 in the table differ from the 2022-edition Table 40 cites for values 2 and 3 (4.5 - p. 96).

**Value-1 `emsg` semantics (pp. 96-97).** `message_data` = the `publishTime` of an MPD; the timeline after the event time (`presentation_time_delta`) is correctly described only by MPDs with a later publish time;
`event_duration` = remaining presentation duration from the event time (0 = presentation ends at event time; "0xFFFF" = unknown; both delta and duration 0 = presentation ended). A client at or after the event time
must therefore work with a newer MPD. Boxes in different segments may share an `id` but differ in `presentation_time_delta` when the segments' earliest presentation times differ.

> **Note (p. 97):** `0xFFFF` is printed, but the `emsg` event duration is 32 bit with unknown = `0xFFFFFFFF` (emsg-rules.md).

**Figure ("Figure 4" again; p. 97; own description).** Two MPD versions: publish times 09:06:31.6 and 09:06:41.4 (2012-11-01), both with an event stream of value 1; the second adds a Period starting at 118 s.
Segments (earliest presentation time/duration): 105 s/5 s, 110 s/5 s, 115 s/3 s (first Period); 0 s/5 s and 5 s/5 s (new Period). The 110 s and 115 s segments carry `emsg` boxes (scheme `...:2012`, value 1,
timescale 1, delta 8 and 3 respectively, duration 0xFFFF, id 12345, message = the first MPD's publish time) - both mark the event time 118 s.

**Offering rules (4.5.2.2; pp. 97-98).** `publishTime` **shall** be present. Table 18 = the simple offering plus an `InbandEventStream` (scheme `...:2012`, value 1 or 3). Validity expiry events **shall** be used. Requirements:
- all Representations of all audio Adaptation Sets **shall** carry the event stream (the printed first bullet says scheme `...:2014`; see note), value 1 or 3;
- for every newly published MPD whose changes go beyond (a) MUP change, (b) changed `S@r`, (c) new `S` element, (d) changes with no semantic effect (e.g. dropping data outside the time-shift buffer): publish with a new `publishTime`
  and add an `emsg` to each segment of each Representation that carries the event stream, value 1/3, whose `message_data` is the *previous* MPD's `publishTime`;
- recommended: all Representations of at least one media type/group carry the event stream.
Table 18 lists value "1 or 3" only (no 2).

### 4.5.3 Client (pp. 99-101)
Advanced client (Figure 7: simple chain plus a segment parser attached to the download engine) must parse segments to (a) extend the segment list (availability time and next URL from the segment index) and (b) update the MPD from `emsg` boxes
(scheme `...:2012`, value 1 or 3). Preconditions: `minimumUpdatePeriod = 0`, `publishTime` present, at least one Representation with the event stream, and `@duration` or timeline present.

**Validity expiry and updates (4.5.3.2).** The client **shall** download at least one Representation that carries the event stream and parse each segment at least to the first `moof`. Extract `ept` (earliest presentation time) and `dur`; if an `emsg` of the scheme with value 1/3 is found, also
`publish_time` (directly or from the patch), `ptd` (`presentation_time_delta`) and `ed` (event duration). After parsing, the segment goes to the media pipeline if used for rendering, or may be dropped (e.g. a muted-audio Representation used only for events).
- No expiry `emsg`: the current MPD is usable at least up to `ept + dur`.
- With expiry `emsg`: the MPD whose publish time equals `emsg.publish_time` is usable only up to `ept + ptd` (if `dur > ptd` the Period ends at `ept + ptd`); any MPD with a later publish time is usable at least to `ept + ptd`; before requesting a segment whose
  earliest presentation time exceeds `ept + ptd` the MPD **shall** be refetched and updated, or (value 3) the MPD carried in the message may be used.
> **Notes (p. 100):** DVB-DASH (TS 103 285 9.1.6) forbids downloading a Representation solely for its inband events, which sits against the rule above that a client shall download a Representation carrying the event stream (paraphrased); both are printed. The unit conversion of `ptd` from the box timescale is not stated.

**Extended segment information (4.5.3.3).** Parse to the first `moof`; `ept` = from the segment index if present, else `baseMediaDecodeTime` of the first `tfdt`; `dur` = segment index, else the summed sample durations of the first movie fragment. The client **should** use these to extend its segment
info (and a present timeline) and derive the next URL without refetching the MPD; refetch in doubt (new Adaptation Set selected, lost segments, other operational issues).

## 4.6 Live content republished as on-demand

### 4.6.1-4.6.2 Offering (pp. 101-102)
Typical goals: reuse live segments (same URLs, cache-friendly), fix live artefacts (variable durations, gaps), add ads, trim a longer stream. Recommended new MPD:
- `type = static`; `availabilityStartTime` removed, or kept (then `Period@start` including presentation time offset and start number, and the duration, must be adjusted to keep the wall-clock relation, without changing AST);
- simple live profile may be used; `timeShiftBufferDepth` and `minimumUpdatePeriod` **shall not** be present (clients ignore them for static MPDs per 23009-1);
- duration = `mediaPresentationDuration`, else last Period start + its `@duration`;
- Period structure may stay or change; continuous Periods are best merged; added ad Periods preferably on segment boundaries;
- a `SegmentTimeline` with exact timing may be used for seeking/gaps (may require parsing up to `tfdt`); keep the live templating mode;
- no validity-expiry events in the MPD (`emsg` boxes in segments need not be stripped).

### 4.6.3 Client (p. 102)
Treat as ordinary on-demand but expect leftovers: recognise timeline gaps (e.g. switch to a Representation without the gap), and **shall** ignore `emsg` boxes for which the MPD declares no matching inband event stream.

### 4.6.4 Live to on-demand transition under one URL (p. 103)
When the last segment's URL/time and duration are known: add `mediaPresentationDuration`, remove `minimumUpdatePeriod` (4.4.3.1). After the last segment is out, `type` may flip dynamic -> static with the 4.6.2 changes; clients then stop updating. Clients **should** cope with dynamic -> static
when `minimumUpdatePeriod` disappears.
