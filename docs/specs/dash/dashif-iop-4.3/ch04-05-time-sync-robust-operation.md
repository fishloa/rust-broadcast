---
source: "DASH-IF Interoperability Points v4.3 (2018-11-15), private/specs/dashif_iop_v4.3_guidelines.pdf"
clauses: "4.7, 4.8, 4.9, 4.10"
pdf_pages: 124-131
printed_pages: 103-110
private_verbatim: private/specs-verified/dash/dashif-iop-4.3/ch04-05-time-sync-robust-operation.md
verified_against_render: true
notes_original_wording: true
---

# IOP v4.3 ch. 4.7-4.10 - clock sync, robustness, live operating points, live trick mode

Printed page = PDF page - 21. Own-wording notes. `UTCTiming` is defined in 23009-1 5.8.5.7 (`private/specs-md/iso_iec_23009-1_dash_2022_ed5.md` ~l. 735-748, 4726 ff.). DVB-DASH counterparts:
[`../etsi-ts-103-285/ch04-dash-constraints-http-sync.md`](../etsi-ts-103-285/ch04-dash-constraints-http-sync.md) (4.7), [`../etsi-ts-103-285/ch10-4-resiliency.md`](../etsi-ts-103-285/ch10-4-resiliency.md) (10.8).
Trick-mode signalling (3.2.9): [`dashif-iop-4.3-trickmode.md`](./dashif-iop-4.3-trickmode.md). Previous [ch04-04](ch04-04-mpd-and-segment-based-live.md); next [ch04-06](ch04-06-deployment-scenarios.md).

## 4.7 Clock synchronisation between client and server

**Why (p. 103).** Availability times are wall-clock values in the MPD, so servers and clients should both be tied to a globally accurate time base.

**Service provider (4.7.2, pp. 103-104)** - applies when the presentation is dynamic or `availabilityStartTime` is present:
- availability times **should** come from a device synchronised to an accurate source, preferably NTP;
- the MPD **should** contain at least one `UTCTiming` element whose `@schemeIdUri` is one of:
  `urn:mpeg:dash:utc:http-xsdate:2014`, `urn:mpeg:dash:utc:http-iso:2014`, `urn:mpeg:dash:utc:http-ntp:2014`, `urn:mpeg:dash:utc:ntp:2014`, `urn:mpeg:dash:utc:http-head:2014`, `urn:mpeg:dash:utc:direct:2014`;
- no `UTCTiming`: segments **shall** be available by their announced time judged against an accurate source;
- with `UTCTiming`: the referenced timing information **shall** be reachable, and segments **shall** be available by their announced time for any device using one of the announced methods.
Caveats for the last three schemes (listed `ntp`, `http-head`, `direct`): use them only if the author controls the whole distribution chain.
- `ntp`: both ends need an NTP client (hard in browsers).
- `http-head`: the `@value` server **should** be the one serving the segments so every response's `Date` header (RFC 7231 7.1.12) keeps the client in sync; edge caches with unsynchronised clocks corrupt `Date`, so do not use it unless that is verified.
- `direct`: the generator writes the wall-clock time into the MPD, so each response is personalised and must not be cached.
Segment unavailability from failures should be signalled with the 4.8 methods. Leap seconds (about one per 18 months): consider RFC 7164; MPD time does not track them, so a leap second may shift media against real time.

> **Notes (p. 104):** the IOP list omits `urn:mpeg:dash:utc:sntp:2014`, which 23009-1:2022 also defines. A stray word "CORS" ends the `http-head` item. `http-ntp` (HTTP-delivered NTP-format value) is distinct from `ntp`.

**Client (4.7.3, pp. 104-105)** - same applicability:
- no `UTCTiming`: take the time from the system clock, allowing for its inaccuracy near availability boundaries;
- one or more `UTCTiming`: **should** use at least one announced method to sync; **must not** request a segment before its availability start measured by any chosen method (lower-case "must" as printed);
- may account for timing-source accuracy and transmission delay when requesting;
- **shall** account for a time-zone difference between its own zone and the one in the MPD times;
- if segments are not there at their availability start, use the 4.8 recovery methods;
- avoid polling the `UTCTiming` server more often than needed.
DVB-DASH requires support for `http-xsdate` and `http-head` but lets providers add others and clients prefer others (TS 103 285 4.7).

## 4.8 Robust operation

### 4.8.2 Tools
**HTTP errors (4.8.2.1, pp. 105-106; restates 23009-1 A.7).**
- Normal answers: 200 (full GET), 206 (range GET); other 2xx/3xx may occur; clients **should** follow redirects (e.g. 301, 307), which can be routine.
- 4xx/5xx: react as RFC 7231 suggests. Repeated errors for the MPD may justify ending the session.
- 4xx for an init segment: the Period may no longer, or not yet, exist.
- 4xx for a media segment: the segment may be gone or not yet there; **should** first check clock accuracy (against the global standard or the MPD's time base); if the clock is believed good, or the error persists after correction, **should** check for an MPD update; with several BaseURLs **may** try another server's copy.
- 5xx: **should** check for an MPD update; with several BaseURLs **may** try another server.
No per-status table is given beyond these (404 is named only in 4.8.5); the finer categories are in DVB-DASH 10.8.5/10.8.6.

### 4.8.3 Segmenter sync loss - options in order of preference
1. Always offer a conforming stream; if the source/encoder dies, insert filler content, e.g. as a separate Period.
2. Use the Segment Timeline per 4.5.2.2; add a validity-expiry only for changes beyond the allowed list; an encoder failing for some Representations may simply add a segment to the timeline.
3. Early terminated Periods (23009-1 Cor.1 of the 2nd edition): Periods carrying both `@start` and `@duration`, where `@duration` is the media presentation duration actually represented; the MPD keeps updating via `minimumUpdatePeriod`, so the timeline continues, signalling an outage while the service goes on; the client decides what to do.
4. Codec-independent gap-filling segments (e.g. null segment): no standardised method.

### 4.8.4 Encoder clock drift
Do not lock the segmenter to the encoder clock. In MPD-based offerings add Periods in a period-continuous way. In segment-controlled offerings add the producer reference box so the pipeline sees drift; clients then parse segments to read it.

### 4.8.5 Segment unavailability
Signal with ordinary 404. If no `UTCTiming` was declared, also send the server `Date` header: a client whose clock matches that header can conclude that a 404 means true segment loss.

### 4.8.6 Switching redundant tools (preference order)
1. Redundant offering to the client (e.g. several BaseURLs) with the client picking whichever is available; 2. a new Period at the swap instant, period-continuous or period-connected if possible (non-continuous media time is allowed); 3. an entirely new MPD that discards earlier information but keeps some time continuity (not well supported by any standard).
Smoothness decreases from 1 to 3, but severe outages may force 3.

### 4.8.7-4.8.8 Requirements
The service **shall** follow "8.2 to 8.6" and the client **shall** implement methods for the offerings of "8.2 to 8.6".
> **Note (p. 107):** those section numbers look stale (robustness tools are 4.8.2-4.8.6; chapter 8 is the interoperability points).

## 4.9 Operating points

- **Simple Live (4.9.2):** allows formats from 23009-1 1st edition and IOP up to v2; client need not parse media segments. *Service conforms to:* 4.3.3, 4.4.3, 4.6.2, 4.7.2, 4.8.7. *Client conforms to:* 4.3.4, 4.4.3.5, 4.6.3, 4.7.3, 4.8.8.
- **Main Live (4.9.3):** allows 2nd-edition formats; client may need to parse segments. *Service:* 4.3.3 plus either 4.4.3 (no profile identifier needed) or 4.5.2 (profile identifier **shall** be added), plus 4.6.2, 4.7.2, 4.8.7. *Client:* 4.3.4, 4.4.3.5, 4.5.3, 4.6.3, 4.7.3, 4.8.8.
> **Note (pp. 108):** the client lists cite 4.4.3.5 (Last Segment Message) for "MPD update"; the update procedure is 4.4.4 - satisfy both. The profile identifiers themselves are in chapter 8.

## 4.10 Trick mode in live services (p. 109-110)
Purpose: a reduced-frame-rate or I-frame-only Adaptation Set to rewind/fast-forward inside the time-shift buffer (signalling: 3.2.9).
- Trick-mode segments **should** have the main set's segment duration or an integer multiple of it. Longer trick segments delay their availability by their own duration, so the live edge may lack trick mode. Author options (one or more): (a) one trick set with a segment per main segment (more requests while seeking);
  (b) one trick set whose segments aggregate several main segments (nothing at the live edge); (c) several trick sets with different segment durations, with a small time-shift window for the short-segment sets and only long-segment ones kept over the full buffer, switchable from long to short; (d) one indexed media segment per Period with period connectivity in main and trick sets (trick available only for non-live Periods).
- Client: minimise network requests, i.e. fetch the longer trick segments.
- Live-to-VoD conversion (4.6): offer trick sets as one indexed media segment per Period.
> **Note (p. 109):** the time-shift attribute is printed `@timeShiftBuffer` (split over a line break); Table 8 uses `timeShiftBufferDepth`.
