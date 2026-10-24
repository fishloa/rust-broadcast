---
title: "Clause 9 Carriage of Generic Streams and Events (notes)"
source: ETSI TS 103 285 V1.4.1 (2023-09), private/specs/etsi_ts_103_285_v01.04.01_dvb_mpeg_dash_profile.pdf
clauses: 9.1.1-9.1.9
pdf_pages: 64-72
printed_pages: 64-72
private_verbatim: private/specs-verified/dash/etsi-ts-103-285/ch09-events.md
verified_against_render: true
notes_original_wording: true
verification_date: 2026-10-24
---

# ETSI TS 103 285 V1.4.1 - Clause 9 Carriage of Generic Streams and Events (notes)

Structured notes in the project's own wording (the standard's text is not reproduced; short quoted phrases only). Strength words are kept: **shall** = mandatory, **should** = recommended, **may** = optional. Verbatim, page-verified text: the private file named in the front matter. Back to [INDEX](INDEX.md).

Related: `emsg` wire format [mp4-emsg](../../../../mp4-emsg/docs/emsg.md), [emsg-rules](../../../../specs/rules/emsg-rules.md); inband-event rules in [dashif-iop-part10-v5.0.0](../dashif-iop-part10-v5.0.0/); MPD update/event semantics are 23009-1 ([dash-mpd-rules](../../../../specs/rules/dash-mpd-rules.md)).

## 9.1.1 Background (p.64-65)

Events travel either in the MPD (`EventStream`, visible while present) or inband in segments (`InbandEventStream`, only visible for segments the player has parsed and not discarded; late joiners miss them unless repeated). The clause covers programme metadata, application messages and player messages. `emsg` v0 gives the time relative to the segment, v1 an absolute media-timeline time.

## 9.1.2 Content programme metadata (p.65-67)

Scheme `urn:dvb:iptv:cpm:2014`, `@value` "1" (Table 23: TV-Anytime `BroadcastEvent` payload). Other values reserved; players shall ignore unknown values.
- `@xlink` "on request" on MPD events may be ignored; xlink on inband streams may be ignored.
- `presentationTime` (MPD `@presentationTime`, or inband `presentation_time_delta`) marks when the metadata applies; `duration` optional (minimum validity; undefined = until next event); `id` optional for MPD events, mandatory inband. MPD and inband ids are independent.
- Payload (`Event` text or `message_data`) is one or more `BroadcastEvent` elements which, placed in the `ProgramLocationTable` of the Table 24 default `TVAMain` skeleton (namespaces `urn:tva:metadata:2012`, `urn:tva:mpeg7:2008`), make a valid TV-Anytime document. `@messageData` is not used.
- Empty payload = no metadata. First element applies at the event time; later elements describe the future. Overlap: latest presentation time wins.
- XML inside an MPD `Event` must be escaped or in CDATA; no escaping inband.
- Live streams using inband events alone need periodic repetition; MPD events can serve joiners.
- Parental rating in `BroadcastEvent/InstanceDescription/ParentalGuidance` takes precedence over an MPD `Rating` of the same scheme while the event is active. Schemes: `urn:dvb:iptv:rating:2014` (id = minimum age per EN 300 468), `urn:dvb:iptv:guidance:2014` (id `G` = guidance text present, `W` = watershed). Other regional schemes may be needed. DASH `Rating` splits scheme/id, TV-Anytime joins them with `:` (URN) or `#` (URL).
- The informative example places a CPM `EventStream` (`presentationTime=300`, `duration=1500`, `id=0`) with CDATA payload in a Period.

## 9.1.3 Application messages (p.67-68)

No scheme is defined; many schemes may coexist; delivery to the application is not specified. `@presentationTimeOffset` on (Inband)EventStream: player shall subtract it from the event time to get Period-relative time. MPD `Event@contentEncoding` = `base64` (23009-1 5.9.7): player shall decode before passing on; binary content should use base64, text/XML either base64 or CDATA.

## 9.1.4 Events for the player (p.68)

- If an inband `urn:mpeg:dash:event:2012` stream with value 1 or 2 is on a Representation being decoded, the player shall refresh the MPD only on an expiration event or a seek; otherwise it follows `@minimumUpdatePeriod`. `@minimumUpdatePeriod` must still be present (typically small).
- MPD patch is not required; value 2 may be treated as value 1 (ignore payload, fetch the MPD).
- An event whose publish time is not newer than the player's MPD `@publishTime` may be ignored (e.g. after dynamic-to-static conversion).

## 9.1.5 Long inband events (p.68-69)

Repetition rules (content): same scheme/value/id; v0 repeats follow `presentation_time_delta` rules of 23009-1 5.10.3.3.4 and keep the same end time unless the end is being updated; v1 repeats keep the same presentation time.
Player rules:
- Event B repeats A if scheme, value and id match and B's presentation time precedes A's end time; compare against the immediately preceding repetition.
- Unknown duration = indefinite.
- Track at least 10 concurrent/overlapping inband events per stream; beyond that the oldest may be dropped (a later repeat then looks new).

## 9.1.6 Monitoring (p.69)

Players shall monitor InbandEventStreams of all Representations being decoded, shall not download a Representation just to read them, treat same scheme/value in several AdaptationSets as one stream (process one at a time), shall support v0 and should support v1. Providers must duplicate presentation-wide events enough that any Representation choice sees them; mixing an EventStream and an InbandEventStream with identical scheme/value is risky because some players do not merge them.

## 9.1.7 Hybrid Digital Radio events (p.69-70) - optional for content and players

Scheme `urn:dvb:dash:radiovis:2016`: value 1 text message (UTF-8, up to 128 chars), value 2 simple slideshow (UTF-8 URL, up to 512 chars), value 3 enhanced slideshow (UTF-8 JSON per ETSI TS 101 499 7.4.3, timing inside the JSON; absolute times convert by subtracting `MPD@availabilityStartTime`, ignorable if that is absent; event duration should be 0).
Common rules: players only need to pass data to an application; inband data is used in preference to out-of-band feeds; for values 1 and 2 only one event is active at a time (a new one truncates the previous); duration 0xFFFFFFFF means until updated (same id) or replaced by a different id; zero-length payload clears. Simple Profile content uses 1 or 2; Enhanced Profile content uses only 3.

## 9.1.8 DVB application signalling (p.71) - optional

MPD `EventStream` with `urn:dvb:dash:appsignalling:2016`, value "1" (others reserved, ignored). Time = `@presentationTime`; `@duration` optional (undefined = until next event). Payload is an ETSI TS 102 809 `ApplicationDiscovery` record with at least one application. Overlap: latest presentation time wins. Inband carriage is out of scope.

## 9.1.9 SCTE-35 ad slots (p.71-72)

`EventStream` scheme `urn:scte:scte35:2014:xml+bin`; `@value` should be absent; `@presentationTimeOffset` may be present (recommended equal to the SegmentBase offset after timescale conversion, default 0); `@timescale` shall give frame/segment accuracy (25 fps video needs 25 or a multiple). Per event: `@presentationTime` = splice time, on a segment boundary (frame-accurate for video); `@duration` mandatory (ad slot length for an "out", 0 for an "in", never 0xFFFFFFFF; no explicit duration -> 0); no `@messageData`, no `@contentEncoding`; the `Signal` namespace is declared on its `xmlns`; the `Binary` element holds the base64 `splice_info_section()`.

## Notes on the source

- p.68: typos "substract", "playerof", "playerignores"; p.65 "of the id field" for "or"; p.72 "restricted follows".
- p.71: the third bullet of 9.1.8 names an `ApplicationDiscovery` record, the paragraph beneath it says `ServiceDiscovery` elements.
- DVB addition relative to 23009-1: the CPM/radiovis/appsignalling schemes and the concrete player limits (10 events, repetition test).
