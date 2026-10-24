---
source: "DASH-IF Interoperability Points v4.3 (2018-11-15), private/specs/dashif_iop_v4.3_guidelines.pdf"
clauses: "4.11 (4.11.1-4.11.5)"
pdf_pages: 131-143
printed_pages: 110-122
private_verbatim: private/specs-verified/dash/dashif-iop-4.3/ch04-06-deployment-scenarios.md
verified_against_render: true
notes_original_wording: true
---

# IOP v4.3 ch. 4.11 - deployment scenario: reliable, consistent-delay live service

Printed page = PDF page - 21. Own-wording notes. Written mainly for packagers/MPD generators; transcribed because a client must understand how losses, program changes and duration jitter show up in the MPD. Previous [ch04-05](ch04-05-time-sync-robust-operation.md).

## 4.11.2 Scenario (pp. 110-112)
**Chain (Figure 8; own redraw):** source encoder (events such as ad opportunities, e.g. SCTE-35) -> ABR encoders (with conditioning) -> multi-bitrate streams carrying segment boundaries and event triggers -> MPD generator/packager/segmenter -> origin server -> one or more CDNs <-> DASH client;
an optional MPD-modification step (customisation per receiver, e.g. dropping unsuitable Adaptation Sets, regional/targeted splicing) feeds client and CDN. Faults can occur at every stage (marked with lightning symbols). The packager does segmentation from inband info, ISO BMFF encapsulation, dynamic MPD generation, event handling.
**Goals/considerations:** (1) consistent, preferably small, broadcast-like latency; (2) program changes at short notice (change lead time = media time between the announcement and the change) which may alter audio languages/formats, video resolution/format (HD/SD, HDR/SDR), codecs/profile/level, number of Representations, or the distribution network (PMT changes with media timeline discontinuities if TS is the source);
(3) ABR-encoder-driven slight segment duration variation (e.g. aligning an IDR to a scene change); (4) unplanned losses (an encoder, one component/Adaptation Set, or all encoding failing; the MPD stays up; single segments lost give the client a 404); (5) minimise MPD update cost (client uplink requests, full-MPD downloads, parsing, server writes - the first two matter most).

## 4.11.3 Technologies chosen
**Consistent latency (p. 113).** Levers: segment duration (1-2 s possible, but short segments cost compression efficiency, request volume and TCP throughput; see 4.3.3.2.2 and Annex B.4); chunked delivery announced via `availabilityTimeOffset`; tight client/server sync per 4.7 (`http-xsdate` normally suffices; accurate NTP recommended but not required if the time API exists);
clients **should** consistently join a little behind the live edge (example: 4 segments; fewer in managed networks) and the author may supply SPD (4.3.4.4.2).

**Unanticipated new Periods (pp. 113-114).** The provider must keep `minimumUpdatePeriod` low so clients do not rely on future segments; segments with availability start earlier than (request time + MUP) will appear on schedule. With `MUP = 0` there is no promise about future segments and the client must revalidate before each segment request. Two client options (combinable): (1) revalidate with every segment request per 4.4.4, ideally with conditional GET;
(2) rely on validity-expiry events if the MPD announces them. In option 1 the generator checks MPD validity per generated segment and rewrites the MPD only if it became invalid.

**Segment duration variations (p. 114).**
- Option 1: signal constant `@duration` and tolerate +/-50% of the duration: per IOP 3.2.7.1 the segment's MPD start time is about `EPT - PTO` and **shall** lie within `[EPT - PTO - 0.5*DUR, EPT - PTO + 0.5*DUR]`. Encoders should respect this virtual segmentation, though they may occasionally break it. If accumulated drift can no longer be compensated, add a new Period (all Representations) with updated availability parameters, signalled period-continuous (4.3.3.3.2); the client needs no segment parsing.
- Option 2: Segment Timeline per 4.5.2.2: end the last `S`'s repeat, add a new `S` with the new duration; the client detects changes by parsing segments for earliest presentation time and duration, independently of MPD updates.
> **Note:** the first sentence of this clause is garbled in the PDF (p. 114).

**Losses (p. 114-116; Figure 9 own redraw).** Loss shapes: none/concealed; inside a segment; at a segment end; at the expected segment boundary (next segment starts late); loss of the boundary itself; across several segments. Losses may corrupt an AU or be a non-coded source frame (prior AU lengthened, still conforming). Service losses typically last until the next random access point, usually the next segment start.
Packager options (not exclusive): (1) Main-live with clients that read the timeline - signal gaps in the Segment Timeline; (2) Simple-live clients - signal gaps by an early terminated Period; (3) gap-filling segments - not recommended by the IOP because the client cannot know and so cannot switch to an unaffected Representation. Also, the same content may be offered on other BaseURLs and temporary unavailability signalled in the MPD.

**Minimising MPD updates (pp. 116-117).** With low MUP: avoid client requests by inband expiry events (needs packager support); avoid full MPD downloads by conditional GET (works when the MPD uses `@duration` or a timeline with `@r=-1`); parse load disappears with either; server writes also depend on the `@duration`/`@r=-1` style. Clients **should** support several of these tools.

## 4.11.4 Packager configurations from a "segment stream" (pp. 117-121)
**Model.** One segment stream per Representation; same segmentation across a set; each segment has duration `d[i]` and earliest presentation time `ept[i]`; nominal duration d0; losses and program changes (with change lead time) may appear. Availability should follow shortly after full receipt. Figure 10 (own description): (1) boundaries `SAST[i]..` with duration jitter `d0 + eps` / `d0 - eps`; (2) a lost tail of one segment and a whole lost segment; (3) a change announced at the start of segment (j,i+1) takes effect after (j,i+2), so change lead time = `d[j,i+1] + d[j,i+2]`, and a new Period j+1 starts with new numbering.
> **Note (p. 118):** a box in Figure 10 is labelled `d[j+1.1]` (sic).

**Simple live configuration (4.11.4.2).** `minimumUpdatePeriod <=` the change lead time; `@timescale` = media timescale; `@duration` = d0; `$Number$`. Per new segment, rewrite the MPD only when needed: if the next segment's MPD start time leaves `[EPT - PTO - 0.5*DUR, EPT - PTO + 0.5*DUR]`, start a new Period with `Period@start` such that the MPD time is right, `presentationTimeOffset` = EPT of its first segment, `startNumber` = its first segment, and period continuity signalled per Adaptation Set.
On an encoder failure: end the segment at its last sample (possibly corrupt); new MPD with `minimumUpdatePeriod = 0`; if all/many Representations fail set `Period@duration` to the media time still available; if only some fail, shorten the last segment's presentation duration to the last available time (printed as "`@presentationDuration`"); this tells clients no media exists for the announced segment duration and they revalidate each segment duration; keep the MPD unchanged until the encoder resumes or the presentation ends.
With a long MUP, clients may request non-existent segments and thereby learn to revalidate; the generator **should** (if able) add the `lmsg` brand to the last segment and put that brand in the HTTP content type, so clients refetch the MPD and see the early terminated Period.
After resumption: a new Period whose `presentationTimeOffset` **shall** equal the earliest presentation time of the first new segment (connectivity signalled if appropriate), and MUP back to the minimum change lead time.
On an announced program change: MUP = 0; at the change write a new MPD with all parameters and reset MUP to <= the change lead time.
> **Notes (pp. 119-120):** `@presentationDuration` is not a 23009-1 attribute (intent: duration of the last segment/Period); "MDP" is a typo for MPD; the `lmsg` advice is addressed to a "DASH client" but concerns the generator.

**Main live configuration (4.11.4.3).** MUP = 0; `@timescale` = media timescale; Segment Timeline with `$Number$` or `$Time$`; `MPD@publishTime` set and updated on every new MPD; durations signalled accurately per 4.5.2.2 (new `S` when duration changes; `@t`, `@d` exact);
`@r` of the last `S` may be `-1` (MPD rewritten only on duration change) or the exact published count (MPD rewritten per segment).
On an encoder failure: end the segment at its last sample; add an `emsg` (expiry <= current segment duration) to the last segment of the affected Representations, or all in the set, or all in the MPD - caution: if not on all, switching to an affected Representation may hit a non-existent segment with the client unaware it needs an MPD update - and it **shall** go on every Representation that announces the event stream;
update the MPD so the last generated segment is in the timeline with no new `S`; after recovery write a new `S` with `S@t` = earliest presentation time of the new segment (client resumes the Representation at its next update); if the outage is long, end the Period, drop the Representation temporarily and signal continuity.
On a program change: `emsg` in the last segment of all Representations announcing the expiry event stream, then a new MPD with all parameters.

## 4.11.5 Client requirements (pp. 121-122)
- Simple live: follow 4.3.4 and 4.4.4; handle losses signalled by early terminated Periods.
- Main live: follow 4.3.4 and 4.5.3; handle losses signalled by timeline gaps: look for an unaffected alternative Representation or conceal errors, and poll MPD updates to see when the Representation returns.
