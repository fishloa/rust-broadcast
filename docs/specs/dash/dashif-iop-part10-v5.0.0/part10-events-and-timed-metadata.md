---
source_pdf: private/specs/dashif_iop_part10_v5.0.0_emsg.pdf
document: DASH-IF IOP-10 V5.0.0 (2023-01), "Interoperability Points; Part 10: Events and Timed Metadata"
clauses: whole document (front matter, 1-11, Annex A)
pdf_pages: 1-19 (PDF page = printed page)
private_verbatim: private/specs-verified/dash/dashif-iop-part10-v5.0.0/part10-events-and-timed-metadata.md
verification: private/specs-verified/dash/dashif-iop-part10-v5.0.0/VERIFICATION.md
verified_against_render: true
notes_original_wording: true
---

# DASH-IF IOP Part 10 v5.0.0 - events and timed metadata (client notes)

Original-wording notes. Every fact (field names, URNs, strengths of requirement) was checked against the
rendered pages; the verbatim, page-verified text lives in the private submodule path above. Cites are
`§clause (p. N)`. Strength words (shall / should / may) are the spec's own.

Related notes elsewhere in the repo (not duplicated here):
[`mp4-emsg/docs/emsg.md`](../../../../mp4-emsg/docs/README.md#private-spec-transcriptions) (box byte layout),
[`specs/rules/emsg-rules.md`](../../../../specs/rules/emsg-rules.md) (rules digest),
[`docs/dashif-ingest/DESIGN.md`](../../../dashif-ingest/DESIGN.md) (ingest-side companion),
[`transmux/docs/README.md`](../../../../transmux/README.md).

## Disagreements with other repo notes and internal oddities

1. **`InbandEventStream@presentationTimeOffset`.** `emsg-rules.md` says v1 `presentation_time` is adjusted by
   the `InbandEventStream` offset. Part 10 §6.1 (p. 9, NOTE and Table 6-2 row) says that since the 5th edition of
   23009-1 the `InbandEventStream` shall NOT carry that attribute; the offset of the carrying Representation's
   `SegmentTemplate`/`SegmentBase` applies. Resolve against ISO/IEC 23009-1:2022 §5.10.3.3.4 (outside this
   document) before coding.
2. **Ignore rule wording.** §6.1 (p. 8) says a box whose scheme/value *is announced* by an `InbandEventStream`
   "may be ignored" - almost certainly meant for the un-announced case, which §6.2.3.2 (p. 11) states
   correctly; §6.2.5 (p. 12) says only listed schemes are parsed. Treat: parse announced, may drop unannounced.
3. Table 6-2 types `presentation_time_delta`, `event_duration`, `id` as 32-bit unsigned and `presentation_time`
   as 64-bit unsigned; §9.2.6 (p. 17) calls the `emib` delta a 64-bit *signed* integer (different box).
4. Introduction list (p. 6) marks both part 8 "Audio" and part 10 "(this document)"; part 8 mark is a slip.
5. Scope/§4 (pp. 7-8) promise a client reference model and an application API; the published text has neither
   (§5 only points at 23009-1 Annex A.13).
6. Typos in the source: a page number fused into a word in §7.1.1.d (p. 13), `<!CDATA` for the XML CDATA
   opener (§7.1.2.e, §7.2.8), a misspelled `emeb` box name (§9.2.6), two misspellings in Table 10-1, and a
   missing letter in the IPR text (p. 5).
7. Example 9-1/9-2 (p. 17): `@timescale` appears on `SegmentTemplate`, while §9.2.5 item 7 wants the
   timescale to match `mdhd`; `associationId="1 2 4"` refers to ids not present in the example; the
   `@value` lists show stray spaces and the first example lists scheme URIs without separate value tokens.

## §5 Client conformance (p. 8)

A DASH-IF client conforms to 23009-1 Annex A.13 (event/metadata processing model) except sub-clause A.13.11,
which is optional.

## §6 Inband events (pp. 8-12)

### §6.1 What an inband event is

Carried in media segments in the `emsg` box (DASHEventMessageBox, 23009-1 §5.10.3.3). Presence should be
announced with an `InbandEventStream` element in the MPD.

`InbandEventStream` attribute guidance (Table 6-1, pp. 8-9):

| Attribute | Rule |
|---|---|
| `@xlink:href` | may be present when the element is remote |
| `@xlink:actuate` | `onLoad` or absent |
| `@schemeIdUri` | shall equal the `scheme_id_uri` of the boxes the client is expected to process |
| `@value` | should equal the box `value` (if the stream uses one); otherwise shall be absent |
| `@timescale` | should be absent; if present shall equal the box `timescale` |

`emsg` field usage (Table 6-2, pp. 9-10):

| Field | Type | Rule in this part |
|---|---|---|
| `version` | flag | 0 = segment-relative timing, 1 = representation-relative timing |
| `scheme_id_uri` | string | identifies the scheme; owner may be an outside body |
| `value` | string | per scheme guidelines; otherwise the empty string |
| `timescale` | u32 | should equal the Representation's `mdhd` timescale (CMAF requires equality) |
| `presentation_time` | u64 | event start relative to `Period@start` adjusted by the carrying Representation's `@presentationTimeOffset` (v1) |
| `presentation_time_delta` | u32 | event start offset, in `timescale`, from the segment's earliest presentation time (v0) |
| `event_duration` | u32 | duration in `timescale` units |
| `id` | u32 | distinguishes events sharing scheme+value, detects repeats; two instances are equivalent iff `scheme_id_uri`, `value` and `id` all match |
| `message_data` | bytes | scheme payload handed to the application; may be empty |

### §6.2 Authoring guidance a client should expect

- **Dispatch mode (§6.2.1, p. 10).** The scheme owner fixes it. *on-receive*: `event_duration` is the active
  interval counted from the event start (suits "state" events such as an on-screen text interval; dispatch as
  soon as the event is seen). *on-start*: `event_duration` is the window in which the client may still
  dispatch (suits "change of state" toggles and tracking beacons, where dispatch must hit the presentation time).
- **Duplication (§6.2.2, pp. 10-11).** Events should be repeated in every Representation of an Adaptation Set (so a
  switch at a segment or chunk boundary loses none); in every Adaptation Set of the same content type when only
  one is played, with time fields converted for each set's own offset/timescale; across all segments overlapping
  the active interval; and into the next Period (with the `InbandEventStream` in the MPD) when the event outlives
  the Period. A v1 box can be repeated byte-identical; a v0 box needs a recomputed delta, and for an event
  already started the duration must shrink to the remaining time (delta cannot be negative).
- **Lead time (§6.2.3, p. 11).** Scheme owner publishes a minimum advance offset and all boxes shall respect it;
  absent that, insert at least 4 s before the event start (2 s segments: repeat in the two prior segments and
  the start segment). The `InbandEventStream` should appear in the MPD before the boxes do. Boxes without a
  matching element may be ignored. Repeated v0 events may differ in `event_duration`; equivalent v1 instances
  may be identical.
- **Size (§6.2.4, p. 12).** Box size/frequency affects segment bandwidth.
- **MPD updates (§6.2.5, p. 12).** Only listed streams are parsed (this includes MPD-validity-expiration and
  callback events - they too must be announced). An MPD update may add an `InbandEventStream` to start
  processing.
- **Efficiency (§6.2.6).** Only application-relevant events, identical for all users, schemes added/removed by
  MPD update only for the span they matter.
- **Use cases (§6.3).** Tracking events; ID3 metadata.

## §7 MPD events (pp. 12-15)

### §7.1 `EventStream` / `Event` properties

`EventStream` sits inside a `Period`. `@timescale` should be set (default 1); `@schemeIdUri` shall identify the
external scheme; `@value` optional sub-scheme; dispatch mode is the scheme owner's choice, default on-receive,
and a client uses the mode it subscribed for; `@presentationTimeOffset` (default 0) aligns event times to
`Period@start`.

| `EventStream` attribute (Table 7-1, p. 13) | Rule |
|---|---|
| `@xlink:href` | may be used for a remote element |
| `@xlink:actuate` | `onLoad` or absent |
| `@schemeIdUri` | scheme of the stream |
| `@value` | may be absent, otherwise per the scheme |
| `@timescale` | shall be the timescale of `Event@presentationTime` and `Event@duration` |
| `@presentationTimeOffset` | shall align `Event@presentationTime` to `Period@start` |

`Event` rules (§7.1 item 2, p. 13): `@duration`, `@presentationTime` (start on the presentation timeline using the
stream's timescale and offset), `@id` (unique, usable for duplicate detection), `@contentEncoding` (`base64`
allowed for binary payloads), payload escaping (escape, or CDATA) when characters could break the XML, events may
be added/removed by MPD updates, payload belongs in the element body rather than `@messageData`, and the latest
arrival time of an MPD event depends on whether the Period is played from its start, randomly accessed, or the event
arrived in a later MPD update (LAT).

### §7.2 Authoring guidance

- List events by increasing presentation time (§7.2.2). Events without `@id` are always dispatched because
  equivalence cannot be tested; avoid repeating them (§7.2.3).
- Do not repeat events within a Period; do repeat a still-active event in the next Period, whose `EventStream`
  should then carry `@presentationTimeOffset` (§7.2.4).
- An event whose time equals the offset aligns with `Period@start`; a smaller time signals an event that began
  before the Period; using the same offset for events and the media they refer to is recommended (§7.2.5).
- Announce events with at least the scheme's advance offset, default 4 s or as early as possible; for dynamic MPDs
  the offset counts from `MPD@publishTime` (an event for wall-clock W should be present in a manifest published
  at W minus offset or earlier); static MPDs are exempt. Events may be removed once the segments overlapping
  their active interval are unavailable; open-ended events at any update (§7.2.6).
- Keep payloads small (players reportedly limit MPDs to roughly 256 kB - 1 MB); large metadata goes by reference or
  in a timed-metadata track (§7.2.7). Payloads that could break XML use CDATA or base64 `@contentEncoding`;
  embedded XML must declare its namespace (§7.2.8).
- On MPD update, do not change `EventStream@timescale/@presentationTimeOffset/@schemeIdUri/@value` or
  `Event@presentationTime/@contentEncoding/@messageData/@duration` of an existing instance (§7.2.9).
- Size control: smaller `@timeShiftBufferDepth`, remote Period/EventStream for old periods, MPD patching; MPD
  events are poor for high-frequency events when `minimumUpdatePeriod` is large - use inband events or a metadata
  track (§7.2.10).
- Use cases: programme information; ad slots and splice points via SCTE 35 (§7.3).

## §8 Same stream in MPD and inband (p. 15)

The same (`schemeIdUri`, `value`) may appear as both `EventStream` and `InbandEventStream`; a client may apply the
equivalence rule across both.

## §9 Timed-metadata Representations (pp. 16-17)

- Tracks are ISOBMFF metadata tracks (ISO/IEC 14496-12 §12.3), delivered like media segments (§9.1).
- Default dispatch is on-start; fetching ahead and dispatching on receipt is permitted (§9.2.1).
- All samples are sync samples; edit lists and composition offsets are allowed; timing is relative to `Period@start`
  adjusted by the Representation's `@presentationTimeOffset` (§9.2.2). A sample shall carry all metadata active
  or relevant over its duration (§9.2.3). Custom schemes use `URIMetaSampleEntry`; other `MetaDataSampleEntry`
  derivatives are allowed (§9.2.4).
- MPD signalling (§9.2.5, p. 16): one Representation in one Adaptation Set; optional `@lang`; a Supplemental- or
  EssentialProperty with `@schemeIdUri="urn:dashif:events:metadataconfiguration:2022"` whose `@value` lists the
  whitespace-separated scheme-id/value pairs; `@mimeType="application/mp4"`; `@contentType="meta"`; `@codecs`
  = sample-entry four-cc (`urim` for URIMetaSampleEntry, `evte` in the second example); `@timescale` matching
  `mdhd`; track should be gap-free; `Role` optional and never `main`; `@bandwidth` above the maximum segment
  bandwidth; `@associationId` may list the Representation ids the track applies to.
- Duplicate detection needs scheme support; MPEG-B part 18 (ISO/IEC 23001-18) event-message tracks carry ids:
  `emib` (event message instance box; fields scheme_id_uri, id, value, event_duration, presentation_time_delta,
  timing relative to the carrying sample, signed 64-bit delta so past and future events work) and `emeb` for
  sample spans with no active event (§9.2.6, p. 17).
- Metadata segments should be available before the audio/video of the same presentation time (§9.2.7). Use
  case: frequent per-frame metadata (§9.3).

## §10 Events a client must support (p. 18)

| Scheme | 23009-1 clause | Dispatch |
|---|---|---|
| `urn:mpeg:dash:event:2012` | 5.10.4 | on-receive |
| `urn:mpeg:dash:event:callback:2015` | 5.10.4.5 | on-start |
| `urn:mpeg:dash:event:ttfn:2016` | 5.10.4.6 | on-start |

## §11 and Annex A (pp. 18-19)

Other schemes are cataloged at the DASH-IF event-scheme registry (dashif.org/identifiers/event_schemes/).
Annex A is a change history: 0.9 (2022-05, community review), 0.91 (2022-12), 5.0.0 (published 2023-01-26).

## Verification

pdf2md hybrid over pp. 1-19 exited 0; pages 3-18 were compared with their rendering; clause numbers were taken
from the rendered contents because the text layer drops them. Detail: the private VERIFICATION.md above.
