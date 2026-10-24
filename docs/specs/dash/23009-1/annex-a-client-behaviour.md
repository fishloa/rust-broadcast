---
source_pdf: private/specs/iso_iec_23009-1_dash_2022_ed5.pdf
edition: ISO/IEC 23009-1:2022(E), 5th edition
clauses: "Annex A (informative) - Example DASH Client behaviour, A.1 to A.13"
pdf_pages: "218-241"
printed_pages: "210-233"
kind: implementer notes in original wording (not a transcription)
verbatim_source: private/specs-verified/dash/23009-1/annex-a-client-behaviour.md
verified: true
verification_date: 2026-10-24
verification: see VERIFICATION-C1.md
---

# Annex A - Example DASH client behaviour (implementer notes)

Annex A is informative: it describes one reasonable client, not a conformance requirement (A.1). These notes restate the algorithms in our
own words as step lists and formulas. Each part cites the sub-clause and printed page; the tool-converted original is in
`private/specs-verified/dash/23009-1/annex-a-client-behaviour.md`. Symbols follow the annex: `NOW` is the client's wall clock (which should
be disciplined to a global time standard), `FetchTime` is when the server handled the MPD request.

## A.2 Overview of the playback loop (pp. 210-211)

The annex assumes `MPD@type = dynamic`; static is a subset of the same behaviour.

- `FetchTime` is the time the *server* processed the MPD request, not the time the client finished receiving it; the client should
  compensate for delivery and processing delay. A fetch also counts as successful when the client merely confirms the MPD is unchanged.
- The MPD is fetched from its original location, or from an `MPD.Location` value when one is present (see A.11).

Loop of the example client:

1. Parse the MPD and choose a set of Adaptation Sets that suit the device, using each set's attributes, the `@group` attribute and any
   `Subset` constraint.
2. Choose one Representation per chosen set (normally by `@bandwidth`, bounded by decode/render capability) and build the list of
   currently accessible Segments for it at time `NOW` (A.3).
3. Request whole Segments or byte ranges of Segments from that list.
4. Buffer enough media before starting; `@minBufferTime`, the Representation `@bandwidth` and the measured throughput guide how much.
   Start rendering at a SAP `TSAP` no earlier than `availabilityStartTime + PeriodStart + TSAP` and no later than that plus
   `@timeShiftBufferDepth`, assuming throughput stays at or above the sum of the selected `@bandwidth` values. For dynamic services the
   recommended render instant is `availabilityStartTime + PeriodStart + TSAP + suggestedPresentationDelay`, especially when several
   devices should play in sync.
5. After start, keep requesting Segments (or parts). Switching Representation is allowed at any request for a Segment containing a SAP;
   aligned Representations make switching seamless; good switch points are advertised in the MPD or the Segment Index.
6. As `NOW` advances, extend each Segment list (A.3). Fetch a fresh MPD when **both** hold: `MPD@minimumUpdatePeriod` is present, and
   playback position is within a threshold of the end of the media the MPD describes for any Representation being consumed or about to be
   (threshold typically at least `@minBufferTime` plus the Segment duration or equivalent `SegmentTimeline` duration).
7. On fetching, update `FetchTime` and regenerate every Segment list.

## A.3 Segment list generation (pp. 211-214)

### A.3.1 Output structure (Table A.1, p. 212)

The example client builds, per Representation, this structure:

| Field | Count | Meaning in the example |
|---|---|---|
| Segments | 1 | container for the URL list |
| InitializationSegment | 0..1 | absent means every Media Segment is self-initializing |
| InitializationSegment.URL | 1 | URL, optionally with a byte range from the MPD |
| MediaSegment | 1..N | one entry per accessible Media Segment |
| MediaSegment.startTime | 1 | MPD start time relative to Period start |
| MediaSegment.duration | 1 | MPD duration |
| MediaSegment.URL | 1 | URL, optionally with a byte range |
| IndexSegment | 1..N | accessible Index Segments, if present |
| IndexSegment.URL | 1 | URL, optionally with a byte range |

Segment info can come from three forms (5.3.9): a `SegmentList`, a `SegmentTemplate`, or a single-Segment `BaseURL`+`SegmentBase`. The annex
covers the first two; the third is called straightforward. A Segment is fetchable at `NOW` iff its availability start <= `NOW` <= its
availability end. Segments of a Representation in a Period are indexed `i = 1..N`.

### A.3.2 Period start and end (p. 212)

- `MediaPresentationDuration` = `MPD@mediaPresentationDuration` if present, else `PeriodStart + Period@duration` of the last Period.
- `PeriodStart` per 5.3.2.1 (see [05-3-2-period.md](05-3-2-period.md)).
- `PeriodEnd`: for a non-last Period it is the next Period's `PeriodStart`. For the last Period:
  - no `minimumUpdatePeriod`: `availabilityStartTime + MediaPresentationDuration`;
  - with `minimumUpdatePeriod`: `min(FetchTime + minimumUpdatePeriod, availabilityStartTime + MediaPresentationDuration)`.

### A.3.3 Start time and duration (pp. 213-214)

Case 1, `@duration` present (in `SegmentTemplate`/`SegmentList` base information):

```
d                       = @duration / @timescale
MediaSegment[i].startTime = (i - 1) * d
MediaSegment[i].duration  = d                       (not the last Segment of the Period)
                          = PeriodEnd - MediaSegment[i].startTime   (last Segment of the Period)
```

Case 2, `SegmentTimeline` with `NS` `S` elements (s = 1..NS), all times divided by `@timescale`:

```
t[s] = S[s]@t / timescale ; d[s] = S[s]@d / timescale ; o = presentationTimeOffset / timescale
if S@r >= 0:   r[s] = S[s]@r + 1
N = 0
for s = 1..NS:
    N += 1; MediaSegment[N].startTime = t[s] - o; MediaSegment[N].duration = d[s]
    for j = 1..r[s]:
        N += 1; MediaSegment[N].startTime = MediaSegment[N-1].startTime + d[s]; MediaSegment[N].duration = d[s]
else (negative @r, open ended): the last Segment's duration is d[0] unless it is the last Segment of the Period,
        in which case duration = PeriodEnd - startTime.
```

> **Spec note:** the printed text sets `r[s] = @r + 1` and then loops `j = 1..r[s]`, which would emit `r+2` entries per `S` in total (one
> before the loop plus `r+1` inside). The `S@r` semantics elsewhere (see [05-3-9-6-segment-timeline.md](05-3-9-6-segment-timeline.md)) give `r+1`
> segments. Recorded as printed; an implementation should follow the clause 5.3.9.6 semantics, not this informative pseudo-code.
> Likewise the open-ended branch refers to `d[0]`, which the text does not define for 1-based `s`.

Case 3, neither `@duration` nor `SegmentTimeline`: one Segment, `startTime = 0`, `duration = PeriodEnd - PeriodStart`.

URL construction:

- `SegmentList`: all N URLs are listed explicitly.
- `SegmentTemplate` with `$Number$`: URL(i) substitutes `(i - 1) + @startNumber` for `$Number$` in `@media`.
- `SegmentTemplate` with `$Time$`: URL(i) substitutes `MediaSegment[i].startTime` (in timescale units, per 5.3.9.5.3) for `$Time$`.

### A.3.4 Accessible Segment list (p. 214)

- Segments exist only between `MPD@availabilityStartTime` and `MPD@availabilityEndTime` (for `NOW` outside, none are available).
- `availabilityTimeOffset` (ATO) is the sum of every `@availabilityTimeOffset` along the levels used to resolve the Segment URL; 0 if absent.
- For `MPD@type = dynamic`:

```
Tavail[i]     = availabilityStartTime + PeriodStart + MediaSegment[i].startTime + MediaSegment[i].duration - ATO
TavailEnd[i]  = availabilityStartTime + PeriodStart + MediaSegment[i].startTime + timeShiftBufferDepth + 2 * MediaSegment[i].duration
```

- MPD refresh bookkeeping: `CheckTime = FetchTime + MPD@minimumUpdatePeriod` (on the MPD media time axis). When playback time reaches
  `CheckTime - MPD@minBufferTime`, fetch a new MPD.
- Newest accessible Segment of the last Period: `imax = max { i : Tavail[i] <= min(CheckTime, NOW) }`.

## A.4 Seeking (pp. 214-215)

Target is media presentation time `TM` relative to `PeriodStart`. Presentation times in a Period are relative to `PeriodStart` less the
Representation's `@presentationTimeOffset` (`TO`) (7.2.1).

1. Pick the Segment index `i*` that is the largest `i` with `MediaSegment[i].startTime <= TM`; request `MediaSegment[i*].URL`.
2. MPD times can be approximate (SAP placement, track alignment, drift). The chosen Segment may start slightly after `TM`; then either
   move the seek target to the first sample of the fetched Segment or fetch the previous Segment too. During continuous play, including a
   Representation switch, data between `TM` and the Segment start is always available.
3. Accurate seeking needs a SAP: use the Segment Index (if any) to find SAPs and their times.
4. ISO BMFF with movie-fragment Segments: `moof`/`mdat` can locate SAPs; with no SAP before `TM`, take the previous Segment or the first
   SAP. Segments that begin with a SAP simplify this.
5. MPEG-2 TS: with an index plus PTS differences inside the Segment. If `TM,S` is the presentation time of the last SAP before the
   target and `PTSs` its PTS, the printed target is `[(TM - TM,S) * timescale + PTSs % 2^33]` (PDF p. 223 render; the `% 2^33` visually
   attaches to the `PTSs` term, i.e. the 33-bit wrap of the SAP's PTS; a safe implementation wraps the whole sum modulo 2^33).
6. Not all of a Segment is needed: fetch the Segment Index with a partial GET from the start of the Segment, map time to byte ranges and
   issue range requests, which lowers start-up delay.

## A.5 Trick modes (pp. 215-216)

- Pause/stop: stop issuing requests; resume from the Subsegment after the last one requested.
- `@maxPlayoutRate` on a Representation/Sub-Representation marks it usable for fast-forward at up to regular speed times that value with
  the same decoder profile/level.
- `@codingDependency = false` marks it usable for both fast-forward and fast-rewind.
- Sub-Representations plus Index Segments plus Subsegment Index boxes allow compact range requests: for the Sub-Representation with the
  wanted `@maxPlayoutRate`, ranges for its `@level` and for every level in `@dependencyLevel` are read from the Subsegment Index.

## A.6 Switching Representations (p. 216)

Switching equals tuning in at the new Representation from where the old one has been shown. Seek to a SAP in each media stream of the new
Representation at a presentation time `tp` slightly after the current one; present old up to (excluding) the SAP time and new from it.

| Signalled conditions | Client may |
|---|---|
| `@segmentAlignment` true and `@startWithSAP` 1 or 2 | switch at any Segment boundary: concatenate consecutive-numbered Segments from different Representations if `@bitstreamSwitching` is true on the set; otherwise load the new Representation's Initialization Segment (or Bitstream Switching Segment) first. No overlapped download/decode needed. |
| `@segmentAlignment` true, `@startWithSAP` 3, equal `@mediaStreamStructureId` | switch at any Segment boundary by concatenation, without re-initialising the decoder; `@bitstreamSwitching` should be true |
| Same on Subsegment level | `@subsegmentAlignment` true and `@subsegmentStartsWithSAP` with the same values and conditions |

## A.7 Reacting to HTTP errors (pp. 216-217)

- Success: 200 (GET) or 206 (partial GET); other 2xx and 3xx may occur. Handle redirects (301, 307 are routine) per RFC 7231.
- Repeated errors on the MPD request: terminating the service may be appropriate.
- 4xx on an Initialization Segment: the Period may be gone or not yet available.
- 4xx on a Media Segment: it may be expired or not yet available. First check clock accuracy against the global time standard; if the clock is
  good, or the error persists after correction, re-fetch the MPD.
- 5xx: re-fetch the MPD; if several `BaseURL` entries exist, try an alternate server hosting the same content.

## A.8 Encoder clock drift (p. 217)

- Mismatch between the end of a Representation and the next Period start can come from encoder clock error. Re-align media time at each
  Period start, detect large deviations of Segment start from media time and compensate even before the next Period.
- Long-run encoder/decoder clock difference makes playback lag real time or stall periodically.
- ISO BMFF method using Producer Reference Time boxes:
  1. Derive `r1`, the encoder clock rate relative to UTC (`r1 < 1` slower, `= 1` equal, `> 1` faster).
  2. Derive `r2`, the playout clock rate relative to UTC from a UTC source.
  3. Timescale factor `c = r1 / r2`.
  4. Receiver-timeline presentation time of each sample/access unit = (composition time or PES presentation time) multiplied by `c`.
- MPEG-2 TS segments: PCR-based drift control may be used.

## A.9 Playback across Period boundaries (pp. 217-218)

- Boundaries may force engine re-initialisation (codec, language, protection change); keep that delay small. A client is expected to play
  continuously across Periods, with implementation consequences.
- Same `AdaptationSet@id` plus identical Asset Identifier in the next Period: the client is advised to treat the sets as associated and keep
  playing the one with that `@id` unless it has better information. Same for Sub-Representation with identical Sub-Asset Identifier.
- If the sets are period-continuous or period-connected (5.3.2.4; see [05-3-2-period.md](05-3-2-period.md)), play seamlessly (4.5.1). The client
  would prefer the Representation with the same `@id`, but it may be missing, in which case it switches seamlessly to another Representation of the set.
  - Continuity signalled and playing continuously: ignore `@presentationTimeOffset` and keep consuming Segments.
  - Connectivity signalled: tell the decoder about a timeline discontinuity following `@presentationTimeOffset`, without necessarily
    re-initialising the decoder.

## A.10 Using @bandwidth and @minBufferTime (p. 218)

State: current buffer level `buffer`, estimated download rate `rate`, `MBT = @minBufferTime`, `BW[i] = @bandwidth` of Representation `i`.

- A starts-at-SAP stream can be sustained without draining the buffer if `BW[i] <= rate * buffer / MBT`.
- The model is idealised (constant rate, progressive download, no competing requests); apply margins for jitter, latency, scheduling.
- With whole-Segment downloads, replace `MBT` with the maximum Segment duration when it is larger: pick `i` with
  `BW[i] <= rate * buffer / max_segment_duration`.

## A.11 Location and reference resolution (pp. 218-219)

Cross-refs: `MPD.Location` (5.3.1.2), update rules (5.4.1), reference resolution (5.6.4), multiple `BaseURL` (5.6.5).

MPD update request URL, in priority order:
1. any `MPD.Location` value (if one or more are present);
2. otherwise, if the MPD request was redirected with a 3xx, the redirect target replaces the manifest URL;
3. otherwise the original manifest URL, if known;
4. otherwise updates are impossible and the client may end the service.
The resolved manifest URL is also the implicit base URI. `BaseURL` elements never apply to MPD updates.

Segment request URL, in priority order: an absolute `BaseURL`/absolute URL if present; else if the manifest URL (and so the base) is
known, resolve the relative base URL against it; else the service may be terminated.

## A.12 Resynchronization and early access (pp. 219-221)

### A.12.1-A.12.2 Motivation and box-structure resync (pp. 219-220)

Cases that benefit: low-latency live, fast channel acquisition, recovery after loss or underrun or before a stall, fast down-switch,
precise seeking. Restarting inside an ISO BMFF Segment needs, in order: locate the box structure; find a resync point with all
information needed to parse and decode; find the earliest presented time; process event messages; gather decryption info; start
elementary-stream decode. See 5.3.13 for the Resync element.

Ways to lock onto the box structure:
1. Segment Index gives times and byte offsets (rare in dynamic services).
2. A timed metadata track gives times and offsets (fine for regular live, poor for low latency).
3. Download from the start of the Segment and parse until the target (costs bandwidth/time).
4. Transport supplies the index of each resync point.
5. Scan for a byte pattern when the Segment start is unavailable, then parse normally to reach `emsg`, `prft`, `moof`, `mdat`.

### A.12.3 Using the Resync feature (pp. 220-221)

- With a timed metadata track listing resync points, jump to the listed index/time according to the point type.
- With no index information, parse the Segment, but first download only the part likely to contain the point, guided by `Resync@dT`,
  `Resync@dMax`, `Resync@dMin`.
- When `Resync@marker` is set, find the next resync point like this:
  1. Search for the 4-byte string `styp` in the Segment (offset `B*`).
  2. Check the following box type against the expected list: `styp`, `prft`, `emsg`, `moof`, `mdat`, `free`, `skip` (guards against chance matches).
  3. If it is one of those, the resync point index is `B* - 4`.
  4. Otherwise discard this hit and resume the search.
- Then derive the earliest presentation time (from `tfdt` and movie fragment header data) and the point type, either from `moof` contents
  or from CMAF compatibility brands:

| Brand | SAP type implied |
|---|---|
| `cmff` | 1 or 2 |
| `cmfl` | 0 |
| `cmfr` | 1, 2 or 3 |

### A.12.4 Early availability (p. 221)

For a Representation with `@availabilityTimeOffset`:

| Condition | Client behaviour |
|---|---|
| No `Resync` with `@dT`/`@dImin`, and `@availabilityTimeComplete` FALSE | do not use byte-range requests; no data is to be expected before the Segment's availability start even if requested |
| `Resync` with `@dT` and `@dImin` present | data should arrive progressively after a request; do not byte-range request parts not yet available |
| ... and `BaseURL@rangeAccess` true for the Representation | byte-range requests are allowed for ranges already available at `NOW` |

## A.13 Event streams and timed metadata (pp. 222-233)

### A.13.1-A.13.2 Architecture (Figure A.1, p. 222)

Redraw (PDF p. 230):

```
                +--------------------------- Application ---------------------------+
                  ^ control        | subscribe                      ^ Event/Metadata API
   +--------------+---------+      |                                |
   | Control, Selection &   |<-----+-- DASH Events & Metadata -------+--------------+
   | Heuristic Logic        |                                       | Event/Metadata
   +------------------------+                                       | Synchronizer & Dispatcher
   Manifest Parser --MPD events------------------------------------>|        ^
   ^                                                                 |        |
 DASH Access API --> Inband Event & moof Parser --inband events----->+ Event & Timed Metadata Buffer
   ^                         |  timed metadata --> Timed Metadata Track Parser --^
 HTTP Stack                  +-- media segments --> Media Buffer --> Media Decoder
```

Roles:
- The player may see event streams (MPD or inband, each scoped by a scheme/value pair) and Adaptation Sets carrying timed-metadata
  Representations per Period. Those the application may consume are called AEMS (application event or metadata streams).
- The application subscribes to AEMS and states a dispatch mode per stream.
- MPD events are parsed and put in the Event & Timed Metadata Buffer by presentation time.
- Media Segments are fetched into the Media Buffer; they are fully parsed only shortly before presentation. Parsing covers `sidx`, `emsg`,
  `prft`, `moof`. Inband events need `emsg` plus normally `moof`. A timed-metadata track needs the whole Representation (Initialization
  plus Media Segments).
- The buffer feeds the Synchronizer/Dispatcher: player-specific events go to the player's control logic; application events/samples go to
  the application per dispatch mode.
- The event buffer is managed together with the media buffer: keep an event while a corresponding media sample remains buffered (A.13.9).

### A.13.3 Inband event timing (Figure A.2, p. 224)

Three times per inband event on the media timeline:
- **LAT** (latest arrival time) = earliest presentation time of the Segment holding the `emsg`. With low-latency chunked delivery and an
  event at the start of a movie fragment that is not a Segment start, LAT is the fragment's earliest presentation time.
- **ST** (start time): when the event becomes active, computed from the `emsg` fields.
- **DU** (duration): how long it stays active, from the `emsg`.
ST >= LAT always; the player must have fetched and parsed the Segment by LAT.

### A.13.4 emsg start time (p. 225)

```
version 0:  ST = PST - PTO/TS + ( LAT + PTD/TS )
version 1:  ST = PST - PTO/TS + ( PT / TS )
```

Symbols: `PST` Period start of the Period carrying the stream; `PTO` presentation time offset of the Representation carrying the events
(`SegmentBase@presentationTimeOffset`); `PTD` = `emsg.presentation_time_delta` (v0); `TS` = `emsg.timescale`; `PT` = `emsg.presentation_time` (v1);
`LAT` as above.

> **Spec note:** the rendered formula writes `PS` for the Period start and the version 0 numerator over `TS` is not legible on the page
> image; the where-list defines `PTD`. The expression above is our reading of the where-list. Note also `PTO/TS` mixes the Representation
> timescale with the `emsg` timescale as printed. Do not treat this formula as normative; the normative event timing is in 5.10.3 (not in this PR).

Harmonised names: `scheme_id = scheme_id_uri`, `value`, `presentation_time = ST`, `duration = event_duration/timescale`,
`message_data = message_data[]`. Note 2: sample timescales may differ so ST may not coincide with a sample time.

### A.13.5 MPD event timing (Figure A.3, pp. 225-227)

```
ST = PST + (PT - PTO) / TS
```
`PTO` = `EventStream@presentationTimeOffset`, `TS` = `EventStream@timescale`, `PT` = `Event@presentationTime`; `LAT` = Period start.
Harmonised names: `scheme_id = EventStream@schemeIdUri`, `value = EventStream@value`, `presentation_time = ST`,
`duration = Event@duration / EventStream@timescale`, `id = Event@id`, `message_data = decode(Event@messageData)` where `decode(x) = x` when
`Event@contentEncoding` is absent and `decode(x) = base64decode(x)` when it is `base64`. Duration may be unknown.

### A.13.6 Timed metadata track samples (Figure A.4, pp. 227-228)

- Track conventions (ISO/IEC 14496-12): `stsd` holds a `URIMetaSampleEntry` (URI in a `URIBox` names the scheme); `hdlr` handler type is
  `meta`; `minf` uses the null media header `nmhd`.
- Sample timing: ST = sample presentation time, DU = sample duration, LAT = earliest presentation time of the carrying Segment. A sample
  cannot extend beyond the fragment that carries it.
- Harmonised names: `scheme_id` = track URI; `timescale` = `mdhd` timescale; `presentation_time = sample presentation time / timescale`;
  `duration = sample duration / timescale`; `message_data` = sample bytes from `mdat`.

### A.13.7 Dispatch modes (Figure A.5, p. 228-229)

| Mode | Dispatch instant | Player state | Notes |
|---|---|---|---|
| on-receive | at or before LAT, as soon as available | none needed | application has `ST - LAT` of lead time; application must track ST, DU and activation itself; duration is a property defined by the scheme owner |
| on-start | at ST (or immediately if playback starts, or seek lands, after ST but within the duration) | Active Event Table per subscription | duration = window of ST in which the event is to be delivered exactly once |

### A.13.8 Dispatch processing (pp. 229-230)

Common: parse the `emsg` or sample, read `scheme_uri`/`value`; if no subscription matches, stop.

on-receive: dispatch ST, id, DU, timescale and message data immediately.

on-start (per subscribed scheme/value, with an Active Event Table holding ids already dispatched):
1. Compute ST of the instance.
2. If current media presentation time < ST: wait/schedule (the printed step "go to step v" refers to dispatch at ST).
3. Compute `ET = ST + DU`; if current time > ET, finish without dispatching.
4. For events: if the id is already in the Active Event Table, finish; else add it.
5. Dispatch the message data at ST, or immediately if the current time already exceeds ST.

### A.13.9 Buffer model (p. 230)

Event/metadata buffer follows the media buffer's policy: while a media sample is buffered, keep the events/samples tied to it.

### A.13.10 API (pp. 230-232)

- Before use, the player gives the application the list of scheme/value pairs present: MPD events, inband events and timed-metadata track schemes.
- Subscription state machine (Figure A.6): UNSUBSCRIBED -> SUBSCRIBED on the first subscription; stays SUBSCRIBED while listeners are added or
  removed; returns to UNSUBSCRIBED on the last unsubscribe or when the Media Presentation ends. The state covers the whole set of streams,
  not one stream.

`subscribeEvent` parameters:

| Parameter | Meaning |
|---|---|
| `scheme_uri` | scheme to subscribe to; must come from the player's list. The value `urn:mpeg:dash:event:catchall:2020` subscribes to all schemes in the MPD (and `value` is then irrelevant). |
| `value` | optional filter within the scheme; no default, absent means no filtering |
| `dispatch_mode` | `on_receive` or `on_start`; default `on_receive` if omitted |
| `callback_handler` | asynchronously invoked on a match |

The MPD/`emsg` carries no dispatch-mode signalling; the application states it, so scheme owners should define the preferred mode.
On a match the player calls the handler with the Table A.2 parameters and should also pass the current presentation time.

| API parameter | MPD event source | Inband `emsg` source | Timed metadata source | Type | on-receive | on-start |
|---|---|---|---|---|---|---|
| scheme_id | `EventStream@schemeIdUri` | `scheme_id_uri` | track URI | string | Y | Y |
| value | `EventStream@value` | `value` | - | string | Y | Y |
| presentation_time | `Event@presentationTime` | `presentation_time` | sample presentation time | uint64 ms | Y | N |
| duration | `Event@duration` | `event_duration` | sample duration | uint32 ms | Y | N |
| id | `Event@id` | `id` | - | uint32 | Y | N |
| message_data | `Event@messageData` | `message_data[]` | sample data in `mdat` | uint8 x size | Y | Y |

Unknown duration is reported as the maximum value `0xFFFFFFFF` (4,294,967,295). For `emsg` v0 the player derives `presentation_time` from
`presentation_time_delta`. `unsubscribeEvent(scheme_uri, value, callback_handler)` removes the given listener, or all listeners for that
scheme/value when the handler is omitted or null.

### A.13.11 Internal object (Table A.3, p. 233)

EMIO fields: `scheme_id_uri` (string), `value` (string), `presentation_time` (u32), `duration` (u32), `id` (u32), `message_data()` (bytes).
Conversion: MPD events - per Period, per `EventStream`, per `Event`, compute presentation time and duration, add. Inband - per Segment, parse
event boxes and `moof`, compute the Segment's earliest presentation time, map each `emsg` to an EMIO. Timed metadata - per Segment parse
`moof`, per sample parse and map.

### A.13.12 Recommended dispatch mode for DASH-specific events (p. 233)

Use on-receive for every DASH-specific event defined in 5.10.4.

## Transcription notes

- Page numbers are printed numbers; PDF page = printed + 8.
- The PTS formula in A.4 (PDF p. 223) and the A.13.4/A.13.5 formulas (PDF pp. 233-234) were checked against the rendered pages; see VERIFICATION-C1.md.
