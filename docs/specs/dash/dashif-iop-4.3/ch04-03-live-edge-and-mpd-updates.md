---
source: "DASH-IF Interoperability Points v4.3 (2018-11-15), private/specs/dashif_iop_v4.3_guidelines.pdf"
clauses: "4.3.6, 4.4 (4.4.1-4.4.4)"
pdf_pages: 106-114
printed_pages: 85-93
private_verbatim: private/specs-verified/dash/dashif-iop-4.3/ch04-03-live-edge-and-mpd-updates.md
verified_against_render: true
notes_original_wording: true
---

# IOP v4.3 ch. 4.3.6 and 4.4 - live edge sanity rules and MPD-controlled updates (Simple Live)

Printed page = PDF page - 21. Own-wording notes; formulas exact. MPD update rules restate 23009-1 clause 5.4. Previous [ch04-02](ch04-02-dynamic-segment-download.md); next [ch04-04](ch04-04-mpd-and-segment-based-live.md).

## 4.3.6 Live-edge checks (pp. 85-86)
- If the MPD has `@publishTime` = PUBT, then at publication every segment computed per 4.3.4.2/4.3.4.3 with NOW = PUBT **shall** already be available.
- If additionally a Representation's timeline ends in an `S` with non-negative `@r`, the last described segment **shall** have `SAST < PUBT` and `SAST + duration > PUBT`.
- Client side: do not request exactly at the computed availability instant when unsure of sync; on trouble such as 404 **should** back off slightly.
- Author side: may add a small safety delay between actual publication and the availability time in the MPD (costs a little latency).

## 4.4 Simple live: MPD updates

### 4.4.1 Why MPDs change (pp. 86-87)
Reasons: unknown end, ad-driven Period insertion, adding/removing Representations or Adaptation Sets, backend operational problems (4.8), changing segment durations. The MPD then describes
only a limited horizon and the client must re-check after expiry (23009-1 5.4). Presence of `MPD@minimumUpdatePeriod` means the MPD will be updated. Two service types: MPD-controlled
(client polls the MPD per `minimumUpdatePeriod`; this section) and MPD-plus-segment controlled (client parses segments for expiry events; 4.5, based on 23009-1 5.10). A service may serve both types of client.

> **Note (p. 87):** the text refers to "Annex A" for the overview of generating an offering for both client types; Annex A of this document is "Examples for profile signalling" (live use cases are Annex B).

### 4.4.2 Preliminaries (pp. 87-88)
*MPD instances* = the versions of the MPD that exist over time. Information set (IOP Table 13; name | status):

| Item | Status |
|---|---|
| `MPD@type` | mandatory, dynamic |
| `MPD@availabilityStartTime` (AST) | mandatory |
| `MPD@minimumUpdatePeriod` | mandatory, except when `mediaPresentationDuration` is present (then the MPD is an ordinary 4.3 instance) |
| `Period@start` (PS) | mandatory |
| `SegmentTemplate@media` | mandatory |
| `SegmentTemplate@startNumber` (SSN) | optional, defaulted |
| `@duration` or `SegmentTimeline` | exactly one (MD[k] = duration/timescale; timeline may have gaps; Table 13 omits "per Representation") |

Segment derivation is as 4.3.2.2 with one change: for the last Period, if `minimumUpdatePeriod` (MUP) is present, **`PEwc[i] = NOW + MUP`** (the end keeps moving as NOW advances).
With `MUP = 0` the MPD lists exactly the segments already on the server, so `@r` counts can be exact.

### 4.4.3 Service offering
**General (pp. 88-90).** The 4.3.3.1 guarantees hold at every NOW, with PEwc as above. Basic offering (IOP Table 14) = Table 9 minus duration/SPD/MBT/TSB, plus `@publishTime` = PUBTIME1 and `@minimumUpdatePeriod` = MUP;
PUBTIME1 typically before `START + PSTART`. Segment info follows from 4.3.2.2 and 4.4.2.2 with `PEwc[1] = NOW + MUP`.
- Ending the presentation with an update: add `mediaPresentationDuration` = PDURATION, remove `minimumUpdatePeriod`, change `publishTime` to PUBTIME2; it must be published no later than the end of the presentation minus MUP: `PUBTIME2 <= START + PSTART + PDURATION - MUP`; conversion to on-demand: 4.6.
- MUP can change mid-stream but takes effect only after a media-time delay equal to the previous MUP. A new Period added by an update must be published no later than (new Period start - previous MUP).
- Track-fragment decode times should neither wrap nor exceed 2^53 (ECMAScript limit): pick a timescale that avoids it (32-bit timescales preferred for deployed browsers) or, if large timescales / long sessions need 64-bit decode times, still keep them under 2^53.

**Choosing MUP (p. 90).** Short MUP = changes announced on short notice but more MPD traffic. `MUP = 0` = period end is NOW: every announced segment is available at any time, instant changes are possible
because the client must revalidate before each segment request.

**Permitted updates (p. 90; per 23009-1 5.4).**
- `MPD@id` (if present), every `Period@id` and every `AdaptationSet@id` stay unchanged across updates (a removed Period excepted).
- A Representation with the same `@id` in the same Period must stay functionally equivalent and yield functionally identical segments at the same indices.
- Updates only extend the timeline; earlier information is never invalidated (failover: 4.8). `availabilityStartTime` and `Period@start` **shall not** change.
- When Representations/Adaptation Sets are added/removed or segment locations change, update the MPD and keep Adaptation Sets period-continuous (4.3.3.3.2).
- Real-time clients are expected to use `Period@id` to find the Period they are playing across updates.

**Segment Timeline with `MUP > 0` (pp. 90-91).** Per 4.3.3.4, and for every Representation using a timeline: the last `S` of the last regular Period has negative `@r`; only `$Number$` addressing. New `S` elements may be appended, but so that clients with the old MPD
can still derive segment info up to the Period end (they may get slightly wrong availability times until they update). IOP Table 15 illustrates (`S@r = -1`).

> **Note (p. 91):** Table 15 shows the `$Time$` template and a `SegmentTemplate@d` attribute (23009-1 has `@duration`; `@d` exists on `S`), and talks of increasing "RVALUE" while showing `-1`; this contradicts the "`$Number$` only" requirement. Follow the requirement; treat the table as an illustration with errors.

**Last segment message (pp. 91-92).** A packager may flag the last segment of a Representation by the `lmsg` compatibility brand in that segment (brand registered in 23009-1:2022 clause 7.3.1; `@segmentProfiles` per 23009-1 Representation semantics).
- If `lmsg` is used, `@segmentProfiles` **should** list `'lmsg'`; when it does, the last segment of the Representation in a Period **shall** carry the brand.
- Non-live MPD (no `minimumUpdatePeriod`) with `lmsg`: the client **should** inspect at least the last two segments of a Period and not request beyond the one with the brand; it may inspect every segment.
- Live MPD (`minimumUpdatePeriod` present, dynamic) with `'lmsg'` in `@segmentProfiles`: the brand **shall** be used to signal the last segment.
- Traffic reduction at the live edge: use conditional GET (RFC 7232) for the MPD; with `lmsg` the client may refetch the MPD only when a segment arrives with the brand, otherwise continue by template.
- With `MUP > 0`, every segment whose availability start is earlier than (request time + MUP) will eventually appear at its computed time; but servers cannot expect clients to re-poll for URL-scheme changes (e.g. at a Period start).

**Signalling the last segment number (p. 92).** If the final segment named by the MPD will not be delivered, tell clients by one of: `lmsg` (above); a timeline with `@r >= 0`; or a supplemental descriptor with scheme
`http://dashif.org/guidelines/last-segment-number` whose `@value` is the last segment number.

> **Note (p. 92):** the third option is printed without the bullet dash of the first two; "Supplemental Descriptor" means `SupplementalProperty`.

### 4.4.4 Client operation (pp. 92-93)
Extends 4.3.4.1/4.3.4.3 for an MPD with `minimumUpdatePeriod` (Table 14 style, no fragment parameters).
1. **Fetch** the MPD from its initial location, or from `MPD.Location` if present. `FetchTime` = the time the *server* processes the request (account for delivery/processing delay; do not use the arrival time). A fetch succeeds if a new MPD arrives or the client verifies the old one is unchanged. Over HTTP, use conditional GET (RFC 7232).
2. **Segment list:** build as before, with the end of the last Period = `FetchTime + MUP`.
3. **Refresh:** when requesting segments whose availability start is close to the validity time `FetchTime + MUP`, fetch the MPD again (same location rule; conditional GET), build a new list from the new `FetchTime` and MUP, find the currently used Adaptation Sets/Representations in it and carry on.

> **Notes (p. 93):** 4.4.2.2 sets the end of the last Period to NOW + MUP while 4.4.4 anchors validity at `FetchTime + MUP`; the latter is the client procedure. A stray "them," appears in the printed 4.4.4 refresh sentence.
