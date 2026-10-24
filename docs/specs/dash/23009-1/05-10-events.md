---
topic: ISO/IEC 23009-1:2022 (Ed.5) clause 5.10 - Events: MPD events, inband events (emsg), DASH-specific events (own-words implementer notes)
clauses: 5.10.1-5.10.4.6
printed_pages: 135-146
pdf_pages: 143-154
verbatim_source: private/specs-verified/dash/23009-1/05-9_to_5-10.md
status: derived from the page-verified private transcription; not a copy of the spec text
---

# 5.10 Events

## Model

Events are timed (start presentation time, optional duration) pieces of aperiodic information for the client or an application. Events of one type are grouped into an **Event Stream** identified by the (scheme, value) pair so a client/app can subscribe or ignore. Two carriages: events in the MPD (5.10.2) and events inband in Segments as `emsg` boxes (5.10.3). DASH's own events (5.10.4) use the same machinery. The Event Stream timing follows the timing of a media Representation in a Period.

## 5.10.2 MPD events

Rules:

- An `EventStream` sits in a Period; events end at the end of the Period even if their start or duration extends past it. A Period shall contain at most one EventStream for each (`@schemeIdUri`, `@value`) pair.
- Structure is descriptor-like (scheme URI + optional value), with `@timescale` to place events on the Period timeline. The meaning of the data is the application's.
- Events inside an EventStream are ordered by non-decreasing presentation time.

EventStream (Table 38):

| Name | Use | Type | Default | Gloss |
|---|---|---|---|---|
| @xlink:href | O | anyURI | - | remote EventStream entity ([`05-5-mpd-assembly.md`](05-5-mpd-assembly.md)) |
| @xlink:actuate | OD | onLoad/onRequest | onRequest | not allowed without `@xlink:href` |
| @xlink:type / @xlink:show | fixed | - | simple / embed | in the syntax only |
| @schemeIdUri | M | xs:anyURI | - | message scheme; URN or URL (URL should embed a month-year; may resolve to a scheme description) |
| @value | O | xs:string | - | value within the scheme, owner-defined |
| @timescale | O | xs:unsignedInt | 1 if absent on any level | ticks per second for durations/times of the Events |
| @presentationTimeOffset | OD | xs:unsignedLong | 0 | event-stream time that aligns with Period start: event time on the Period timeline = event presentationTime - this value; **not applied to inband event streams**; seconds = value / `@timescale` |
| Event | 0..N | EventType | - | one event |

Event (Table 39; mixed content allowed in the schema):

| Name | Use | Type | Default | Gloss |
|---|---|---|---|---|
| Event (content) | - | string or foreign-namespace XML | - | the message; either a string (optionally encoded per `@contentEncoding`) or XML from another namespace, not a mix; new schemes should use string content with Base64 where needed |
| @presentationTime | OD | xs:unsignedLong | 0 | start, in `@timescale` ticks, relative to Period start after `@presentationTimeOffset` |
| @duration | O | xs:unsignedLong | unknown | duration in `@timescale` ticks; interpretation up to the scheme owner |
| @id | O | xs:unsignedInt | - | instance id; events with equivalent content/attributes shall have the same id; scope = same (`@schemeIdUri`, `@value`) |
| @contentEncoding | O | enumeration { base64 } | none | content (and `@messageData`) is Base64 (RFC 4648); the client decodes before handing to the application |
| @messageData | O | xs:string | - | deprecated string carriage, kept for compatibility; preferred: message in the element content; expected to be removed in later editions |

```text
EventStreamType: sequence Event*, any(##other)*; attributes xlink:href, xlink:actuate(default onRequest), xlink:type(fixed simple),
  xlink:show(fixed embed), schemeIdUri (anyURI, required), value, timescale (unsignedInt), presentationTimeOffset (unsignedLong, default 0)
EventType (mixed): any(##other)*; presentationTime (unsignedLong, default 0), duration (unsignedLong), id (unsignedInt),
  contentEncoding (ContentEncodingType), messageData (string, deprecated); @* ##other
ContentEncodingType = xs:string enumeration { base64 }
```

## 5.10.3 Inband event signalling

MPD signalling (5.10.3.2): inband streams are declared with `InbandEventStream` at Adaptation Set or Representation level, typed like EventStream with restrictions: `@timescale` may be absent, `@presentationTimeOffset` shall be absent, no `Event` children. Several InbandEventStream elements may be in one Representation. If the same (scheme, value) is carried in several Representations the streams shall be semantically equivalent. Inband messages without a matching MPD declaration are ignored by clients (a client only processes inband messages that have a corresponding InbandEventStream); the box format is in 5.10.3.3. The `emsg` box is also the carrier for DASH-specific operations (5.10.4); messages whose scheme is not in the MPD should not be present and are ignored by a client.

Placement of `emsg` in an ISO BMFF Media Segment: before the first `moof` of the segment; or between an `mdat` and a following `moof`, in which case an equivalent `emsg` with the same `id` shall also be present before the first `moof` of any Segment (so clients may process only start-of-segment boxes).

### Event Message box (`emsg`), 5.10.3.3

Box type `emsg`, container: Segment, not mandatory, zero or more per segment. FullBox, version 0 or 1:

| Field | v0 | v1 | Gloss |
|---|---|---|---|
| scheme_id_uri | string (null-terminated UTF-8), first | string, after id | message scheme (URN or URL, same month-year recommendation as for descriptors); defines syntax of message_data |
| value | string, second | string, last | value within scheme |
| timescale | uint32 | uint32 (first) | ticks per second for event_duration and presentation_time(_delta); should equal the timescale of a track in the carrying Segment and be identical for all events in one Event Stream |
| presentation_time_delta | uint32 | - | event time minus earliest presentation time of this Segment; earliest presentation time = `earliest_presentation_time` of the first `sidx` if one exists, else the earliest of any access unit in the Segment |
| presentation_time | - | uint64 | event time on the movie timeline, in `timescale` units, adjusted by InbandEventStream@presentationTimeOffset (in the stream's own timescale); shall not be less than the earliest presentation time of the carrying Segment |
| event_duration | uint32 | uint32 | duration in media presentation time; 0xFFFFFFFF = unknown; meaning of the value up to the scheme owner |
| id | uint32 | uint32 | instance id; same id within the same (scheme_id_uri, value) scope means equivalent message, processing one is enough |
| message_data[] | rest of box | rest of box | message body; may be empty; syntax defined by the scheme owner |

Field order v0: scheme_id_uri, value, timescale, presentation_time_delta, event_duration, id, message_data. v1: timescale, presentation_time, event_duration, id, scheme_id_uri, value, message_data.

### Carriage of emsg in MPEG-2 TS (5.10.3.3.5)

- TS packets carrying `emsg` use the reserved fixed PID 0x0004; the packet that starts a box has `payload_unit_start_indicator` = 1 and its payload starts with the box; the `Box.type` field shall be complete in that first packet and the payload at least 8 bytes.
- Box data continues in following packets of the same PID; the last packet is padded with adaptation-field stuffing.
- A segment contains only complete boxes; with `@bitstreamSwitching` set and subsegments used, a subsegment contains only complete `emsg` boxes.
- `transport_scrambling_control` of PID 0x0004 packets shall be '00'.

### Inband event alignment (5.10.3.3.6)

If an AdaptationSet has an InbandEventStream and `AdaptationSet@segmentAlignment` is true, event message boxes in non-overlapping Segments shall be aligned: if the Segment of one Representation (earliest presentation time T) holds `emsg` boxes, identical `emsg` boxes shall be present in the non-overlapping Segments of all the other Representations of the set. Hence all Representations then carry the events; if earliest presentation times differ while the Segments are still non-overlapping, alignment still applies and the boxes sit at the start of both Segments.

## 5.10.4 DASH-specific events

Scheme `urn:mpeg:dash:event:2012` (Table 40, inband `@value`): the `emsg` message_data holds a **DASHEvent** structure:

```text
DASHEvent { string publish_time;               // MPD@publishTime
            if (emsg.value == 2) string mpd_patch;   // an MPD patch, clause 5.15
            if (emsg.value == 3) string mpd;         // a full MPD }
```

| InbandEventStream@value | Meaning |
|---|---|
| 1 | MPD validity expiration events (5.10.4.2) are signalled in this Representation, at least in the last Segment whose earliest presentation time is smaller than the event time |
| 2 | as 1, and the message also carries an MPD patch document (5.15) |
| 3 | as 1, and the message also carries a complete MPD (5.10.4.4) |

Other values reserved for ISO/IEC.

**MPD validity expiration (5.10.4.2)**: tells the client that the MPD with the given publish time can be used only up to a given media presentation time. Shall be signalled for every update that extends the timeline, except updates that only (a) change `@minimumUpdatePeriod`, (b) change `SegmentTimeline.S@r`, (c) add a `SegmentTimeline.S` element. With scheme `urn:mpeg:dash:event:2012` and value 1 the box shall say: publish_time = `MPD@publishTime` of the MPD being expired; the media presentation time beyond the event time (given by presentation_time_delta in v0 / presentation_time in v1) is correctly described only by MPDs with a later publish time; event duration = remaining duration of the Media Presentation from the event time (0 = presentation ends at the event time; the text gives 0xFFFF as unknown). A client at or after the event time must use an MPD newer than publish_time. Boxes in different segments may have the same id with different presentation times when earliest presentation times differ. Figure 5 (printed p. 144) shows two MPDs (publishTime ...31.6 and ...41.4), segments with ept 105 s/110 s/115 s, and two `emsg` boxes (presentation_time_delta 8 and 3, id 12345, timescale 1) announcing a new Period at 118 s.

**MPD Patch event (5.10.4.3)** (value 2): payload is a valid MPD patch; applying it shall give a result parse-tree-identical (before XLink resolution) to the MPD with that publish_time retrieved at event time; see 5.15.4.

**MPD Update event (5.10.4.4)** (value 3): a complete MPD follows publish_time.

**Callback event (5.10.4.5)**, `urn:mpeg:dash:event:callback:2015`: client issues an HTTP GET to a URL and ignores the response (status should be 2xx, body small or absent; GET not HEAD for IAB VAST alignment; secure handling is the adopting system's job). Inband (Table 41): `value` 1, message_data = null-terminated HTTP/HTTPS URL; response not provided or discardable. In the MPD (Table 42): `EventStream@schemeIdUri` = the URN, `EventStream@value` = 1, `Event@messageData` = the HTTP URL; response discarded without parsing.

**Presentation termination event (5.10.4.6)**, `urn:mpeg:dash:event:ttfn:2016`, inband or MPD event: the presentation will end earlier than the MPD says (primarily for clients that do not expect an MPD update). Values (Table 43): 0 = end of presentation including chained-to and fallback MPDs; 1 = end of this presentation, the chained-to presentation (if any) stays valid (if the chaining descriptor's `earliestTimeToResolve` is later than the time derived from `presentation_time_delta`, that is an error that triggers the fallback presentation if possible); 2 = end due to an unspecified error, the fallback presentation is valid and the client is expected to switch to it.

## Quirks

- The DASHEvent / Table 40 cross-references: value 3 text cites 5.10.4.4 as the clause defining "validity expiration events", whereas validity expiration is 5.10.4.2.
- 5.10.4.2 says "0xFFFF" for unknown duration; the `emsg` semantics say 0xFFFFFFFF.
- 5.10.2.1 says "subcrlause" (typo) when pointing at the XML syntax.
- The `emsg` placement note and the "equivalent emsg with the same id before the first moof" rule are the printed basis for de-duplicating mid-segment boxes.
- Table 38's `@presentationTimeOffset` sentence ends with a double full stop.

Verbatim text: see `private/specs-verified/dash/23009-1/05-9_to_5-10.md` section 5.10 (printed pp. 135-146).
