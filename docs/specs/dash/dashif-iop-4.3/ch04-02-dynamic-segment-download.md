---
source: "DASH-IF Interoperability Points v4.3 (2018-11-15), private/specs/dashif_iop_v4.3_guidelines.pdf"
clauses: "4.3.3, 4.3.4, 4.3.5"
pdf_pages: 92-106
printed_pages: 71-85
private_verbatim: private/specs-verified/dash/dashif-iop-4.3/ch04-02-dynamic-segment-download.md
verified_against_render: true
notes_original_wording: true
---

# IOP v4.3 ch. 4.3.3-4.3.5 - dynamic download: what the service offers, what the client does

Printed page = PDF page - 21. Own-wording notes; formulas exact. Previous: [ch04-01](ch04-01-live-intro-and-preliminaries.md); next: [ch04-03](ch04-03-live-edge-and-mpd-updates.md).

## 4.3.3 Service offering (pp. 71-80)

### 4.3.3.1 Guarantees the server must give (pp. 71-72)
The MPD must satisfy IOP ch. 3 and contain at least the Table 8 items ([ch04-01](ch04-01-live-intro-and-preliminaries.md)). For an MPD reachable at NOW:
1. every announced segment of every Representation/Period is retrievable no later than its `SAST[k,i,r]`;
2. and stays retrievable at least until `SAET[k,i,r]`;
3. `SAST[k,i,r] <= PSwc[i] + SD[k,r,i] + EPT[k,r,i]` (a segment is available before its own period-relative end);
4. delivered over a constant-rate channel at the Representation's `@bandwidth`, presentation time PT is reachable at the client within `PT + MBT`.

### 4.3.3.2 Basic offering
- **Parameters (IOP Table 9):** dynamic type, AST=START, duration=PDURATION, SPD, MBT, TSB, BaseURL `http://example.com/`, `Period@start`=PSTART,
  template `$RepresentationID$/$Number$`, `startNumber`=1, `@duration`=SDURATION. MPD published before START; no redundancy.
- **Derived list:** `k1 = 1`, `k2 = ceil(PDURATION/SDURATION)`; for k = 1..k2: `SAST[k] = START + PSTART + k*SDURATION`,
  `SAET[k] = SAST[k] + TSB + SDURATION`, `SD[k] = SDURATION`, URL = base + `RepresentationID`/k; init segment: `SAST[0] = START + PSTART`, `SAET[0] = SAET[k2]`.
- **Parameter guidance (4.3.3.2.2, pp. 73-75):**
  - TSB: short when consuming at the live edge but not below max(4 x SDURATION, 6 s); may be huge (even > PDURATION) if access is unrestricted.
  - SPD: provide when synchronised playout across devices or a defined live edge is wanted; weigh end-to-end latency, client buffering need, SDURATION, TSB;
    reasonable = 2-4 x SDURATION and not under 4 s.
  - SDURATION: affects latency, switch/random-access granularity (each segment starts at a SAP), compression efficiency, start-up, request rate,
    network variability; sensible range 1-10 s.
  - MBT: describes content burstiness (the "leaky bucket" size that makes `@bandwidth` true), not network behaviour, and gives no buffering order to the client.
    Meaning: from any segment start, at constant rate = `@bandwidth`, each PT arrives by `PT + MBT`. Default advice: max GOP size (often = max segment
    duration); may be lower than the max segment duration, should not be higher.
  - Naive rate selection from buffer level, estimated throughput `rate`, MBT and each `BW[i]`: pick i with `BW[i] <= rate * buffer / MBT`. At segment granularity
    use max segment duration instead of MBT when MBT is smaller: `BW[i] <= rate * buffer / max_segment_duration`. The model's idealisations (constant rate,
    progressive playout, no competing requests) rarely hold; low-latency settings may differ.
- **Worked example (IOP Table 10; p. 75):** duration 43 s, SPD 15 s, MBT 5 s, TSB 25 s, 5 s segments, start 0 give `k1=1, k2=9`, `SAST[k] = START + 5k`,
  `SAET[k] = SAST[k] + 30 s`, init available `START` .. `START + 75 s`.

> **Note (p. 75):** the text introduces the example as "Table 12" but the table is captioned Table 10.

**Figure 5 (p. 76; own redraw):** ten boxes, 0 (init) and 1..9, per time condition (B = valid-not-yet-available, G = available, R = no longer available):

| NOW | 0 | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9 |
|---|---|---|---|---|---|---|---|---|---|---|
| < START | B | B | B | B | B | B | B | B | B | B |
| START .. START+SD | G | B | B | B | B | B | B | B | B | B |
| START+3SD .. +4SD | G | G | G | G | B | B | B | B | B | B |
| START+7SD .. +8SD | G | R | G | G | G | G | G | G | B | B |
| START+10SD .. +11SD | G | R | R | R | R | G | G | G | G | G |
| > START+15SD | R | R | R | R | R | R | R | R | R | R |

### 4.3.3.3 Periods (pp. 76-79)
- Each Period's duration should be as close as possible to the media it really holds; recommended = longest Representation in it.
- Multi-Period offer (IOP Table 11; inventory): three Periods, Period 1 and 3 use template `1/$RepresentationID$/$Number$` with SDURATION1, Period 2 uses `2/...` with
  SDURATION2 and a Representation-level `availabilityTimeOffset` ATO2; Period 3 restarts the main content with `startNumber = STARTNUMBER2` and a `presentationTimeOffset` PTO.
- Per Period p: `PSwc[p]` from `Period@start` (PSTART, PSTART2, PSTART3), `PEwc` = next Period start or `START + PDURATION`;
  `k2 = ceil((period length)/segment duration)`; `SAST[k] = PSwc[p] + k*SDURATION_p`; `ASAST[k] = SAST[k] - ATO2` (Period 2); `SAET[k] = SAST[k] + TSB + SDURATION_p`;
  init: `SAST[0] = PSwc[p]`, `SAET[0] = SAET[k2]`. Period 3 URL number = `k + STARTNUMBER2 - 1` (k counts from the Period start).
- To keep `Period@start` exact (so a client does not search the wrong Period), express it in the video timescale, at most 90 kHz, via `xs:duration`.
- Continuous Periods (IOP 3.2.12) should be used when Periods exist only for robustness/MPD changes; if continuity of the timeline fails but all other conditions hold, use period connectivity.

> **Notes (p. 77):** the first Period start is `PSTART` in the table but `PSTART1` in the derivation; reproduced as is.

### 4.3.3.4 Segment Timeline (pp. 78-79)
- Use `SegmentTimeline` instead of `@duration` for variable durations or gaps (23009-1 5.3.9.6). Table 12 is Table 9 with the timeline element in place of `@duration`.
- Irregular (shorter) segments must be announced with a new `S`. With `$Time$`, MPD times must equal media-internal presentation times; with `$Number$` they may
  deviate from the documented EPT by at most 0.5 s.
- A single MPD cannot serve live content not known in advance; the timeline fits best when updates are possible (4.4). TSB/SPD follow 4.3.3.2.2.

> **Note:** the printed sentence "If the segment timeline is and the `$Number$` template is used" lacks a word.

### 4.3.3.5 Joining (pp. 79-80)
A dynamic MPD implies joining at the live edge. For restart-from-beginning style services (content published progressively but meant to be played from the start), signal the start
in the MPD *URL fragment* (23009-1 C.4, plus the `posix` clock of 2014 Amd.3): `#t=<time>`, `#period=<Period@id>&t=<time>` (period id URL-encoded, must match a Period@id),
`#t=posix:<unix seconds>`; `#t=posix:now` = live edge. The `t` value uses the W3C Media Fragments field syntax. An anchor should point at a time whose segments are currently available.

## 4.3.4 Client operation (pp. 80-85)

### 4.3.4.1 Single-Period flow (Figure 6: download engine -> buffer -> decode/render)
Assume Table 8 info, one Period, no fragment parameters. Reference behaviour:
1. Parse the MPD; choose Adaptation Sets that suit the device.
2. Choose one Representation per set (mainly `@bandwidth`, also decode/render capability).
3. Build the accessible-segment list for each choice using the current time JOIN and find the live-edge segment (4.3.4.2).
4. Fetch init segments, then whole segments or byte ranges. Request the next segment at max(download of current finished, `SAST` of next); with
   `availabilityTimeOffset`, at the adjusted time. Rate adaptation by buffer fullness etc. The first media segment is typically the live-edge one, but other choices may reduce start-up.
5. Fill the buffer, start decode/render; sync across selections using exact media presentation times (4.3.2.2.9). Playout is continuous. If `suggestedPresentationDelay` exists it
   *may* be used as presentation delay PD; otherwise a live-edge client should choose PD typically between `minBufferTime` and `timeShiftBufferDepth`. Recommended render time of the first
   sample of segment k: `PSwc[i] + (EPT(k) - o[r,i]) + PD`.
6. Request segments only from the generated list within their availability window.
7. Keep fetching; Representation switching is allowed at a segment starting with a SAP and, when it is done there, it **shall** be seamless.
8. As NOW advances, extend the list (4.3.4.2).
9. At the end of announced media: the presentation ends, a new Period starts (4.3.4.3) or the MPD is refetched (4.4).

### 4.3.4.2 Segment list for the simple offering
`k1=1`; `k2 = ceil(PDURATION/SDURATION)`; `SAST[k] = START + PSTART + k*SDURATION` (k = 0..k2); `ASAST[k] = SAST[k] - ATO`; `SAET[k] = SAST[k] + TSB + SDURATION` (k=1..k2);
`SAET[0] = SAET[k2]`; `SD[k]=SDURATION`;
`k[NOW] = MIN(floor((NOW - START - PSTART)/SDURATION), k2)`; `k*[NOW] = MAX(k1, floor((NOW - START - PSTART - TSB)/SDURATION))` (the PDF is missing one closing parenthesis).
k[NOW] = 0 means only the init segment is out; live edge = k[NOW]; with ATO, segments may be fetched from `ASAST[k]`.

### 4.3.4.3 Multi-Period
Latest Period / segment at NOW, i* = last Period (conditions as printed, evaluated in this order):
- `NOW <= PSwc[1]`: nothing yet;
- `NOW > PSwc[i*]`: newest segment is `k2[i*]`;
- `NOW > PSwc[i*] + TSB`: nothing any more;
- `PSwc[1] < NOW <= PEwc[i*]`: find i' with `PSwc[i'] < NOW <= PEwc[i']`, `k[NOW] = MIN(floor((NOW - PEwc[i'] - PSwc[i'])/SDURATION[i']), k2)`; if k[NOW]=0 only the init segment exists (or the previous Period's last segment).

> **Notes (p. 83):** the third branch is unreachable after the second; `NOW - PEwc[i'] - PSwc[i']` mixes end and start of the same Period (a Period-relative value would be `NOW - PSwc[i']`).
> Do not implement these literally; the per-Period SAST/SAET model of 4.3.2.2.7 is the intent.

Crossing a Period boundary: re-select Adaptation Sets/Representations (steps 1-2) and repeat steps 3-6. Audio/video switching across Periods need not be seamless in general;
per 23009-1 7.2.1 playout may need adjusting at the end of a Period to meet the next `PeriodStart` (small overlaps/gaps; and if the first access unit's time differs from `presentationTimeOffset`).
Clients *should* play continuously but may need to re-initialise the pipeline, which *should* be kept short; with dynamic MPDs, adjust so that (actual playout time - Period start) stays constant (no drift).
- Same Asset Identifier in the next Period: *should* identify an associated Period and, absent other info, continue with the associated Adaptation Set.
- Period-continuous (signalled): *shall* play seamlessly across the boundary, ignoring presentation time offset; may keep the Representation with the same `@id`, but if it is missing *shall* switch seamlessly to another one in that set.
- Period-connected (signalled): *should* avoid re-initialising decoders; *should* tell the decoder about the timeline discontinuity according to `presentationTimeOffset`, may keep feeding segments without re-init; the offset *should* be used for seamless play.
Definitions of continuity/connectivity: IOP 3.2.12; 23009-1 5.3.2.4, 5.8.4.5.

### 4.3.4.4 Joining and initial buffering
- Start at the MPD anchor time if present; otherwise at the live edge for dynamic MPDs.
- Two strategies: (a) every client uses the same delay PD relative to availability start (possibly the SPD; clients may ignore SPD); (b) each client picks its own PD for stable quality.
  Render EPT(k) at `PSwc[i] + (EPT(k) - o[r,i]) + PD`.
- Start-up trade-offs: fetch the *next* segment (largest initial buffer, long start-up), the *latest available* segment (large buffer, long start-up), or the *earliest* downloadable segment (smaller buffer, reasonable
  start-up; refill by fetching faster than real time on a low-bitrate Representation). Advanced: start rendering at the sample matching `PSwc[i] + (PT - o[r,i]) + PD = NOW`; start rendering with a partly downloaded segment; use ATO to fetch earlier.

### 4.3.4.5 Minimum client duties
A client consuming dynamic MPDs **shall** support single- and multi-Period content, and **shall** play seamlessly across Period boundaries when the content is offered seamlessly.

## 4.3.5 DVB-DASH alignment (p. 85)
Pointers: DVB-DASH 10.8.6 (player behaviour on HTTP response codes), 10.9.2 (live-edge guidance), 10.8.2.1 (weights/priorities across multiple BaseURLs). See `../etsi-ts-103-285/`.
