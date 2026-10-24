---
source: "DASH-IF Interoperability Points v4.3 (2018-11-15), private/specs/dashif_iop_v4.3_guidelines.pdf"
clauses: "4, 4.1, 4.2, 4.3.1, 4.3.2 (4.3.2.1-4.3.2.2.9)"
pdf_pages: 82-92
printed_pages: 61-71
private_verbatim: private/specs-verified/dash/dashif-iop-4.3/ch04-01-live-intro-and-preliminaries.md
verified_against_render: true
notes_original_wording: true
---

# IOP v4.3 ch. 4 - live services: overview and availability-timing model (4.1, 4.2, 4.3.1, 4.3.2)

Printed page = PDF page - 21. All prose is paraphrased; formulas are exact. Verbatim text: see `private_verbatim` above.
The IOP restates the 23009-1 availability model (`@availabilityStartTime`, Period start, segment availability); 23009-1 is the authority
(`transmux/docs/codec/dash-23009-1-2022-*.md`).

## 4.1 Introduction (pp. 61-62)

Chapter 4 refines how the 2012-edition live features are used and defines one new operating point for the 2014-edition features. Two modes
result, compared in IOP Table 7 (summarised):

| Feature | Simple live | Main live |
|---|---|---|
| `MPD@type` | static or dynamic | static or dynamic |
| MPD updates | yes | yes |
| What triggers an update | the `minimumUpdatePeriod` attribute | inband event messages carried in segments |
| Segment URL generation | from the MPD only | from MPD plus segment information |
| Timeline gaps | described by the MPD for the whole content | may be signalled per Representation |
| Segment starts with | closed GOP | closed GOP |
| "Support of Simple Live" row | Yes | No |
| "Support of Main Live" row | Yes | Yes |

> **Note:** the heading of the update-trigger row is printed "MPD updated triggered". The last two rows are read exactly as printed
> (Simple column Yes/Yes, Main column No/Yes); their intended meaning is not explained in the text (p. 61-62).

Use cases and architecture behind the operating points are in IOP Annex B (see `./annex-b-live-use-cases.md`).

## 4.2 Dynamic vs live presentations (pp. 62-65)

A `dynamic` MPD means media becomes available (and may stop being available) over time; two consequences: not all content exists yet, and clients
are bound to a schedule set by the author. Three service flavours:

1. **Dynamic distribution of existing content** - segment list fully known, one MPD, no updates (Annex B use cases 2, 3).
2. **MPD-controlled live** - MPD updated now and then; client works from MPD only (use cases 4, 5).
3. **MPD- and segment-controlled live** - MPD may change at short notice; client must parse segments (use cases 4, 5 plus advanced cases).

Joining needs an MPD (fetched at join time, or delivered earlier, e.g. via a service guide). A client with a clock synchronised to the server should:
find the active Period(s) and the one holding the live edge; pick Adaptation Sets, then a Representation in each; locate the live-edge segment of each
Representation; request segments. Later MPD refreshes are triggered either by the MPD itself or by information inside segments.

**Client models (Figure 4, p. 64; own redraw).** Three layered clients, each = download engine -> buffer -> decode/render, fed by an MPD server, an
MPD-update server and a segment server:

```
 Dynamic Segment Download : engine builds segment list from ONE MPD, joins at live edge or inside the time-shift buffer
 Simple Live Client       : + re-fetch the MPD when the current one expires (expired = no longer usable for URL generation)
 Main Live Client         : + generates segment URLs itself, parses segments (box parsing box -> engine), refreshes MPD on expiry info found in segments
```

Where each is specified: dynamic download 4.3, simple live 4.4, main live 4.5, live as on-demand 4.6, client/server clock sync 4.7, robustness 4.8, operating points 4.9.

> **Note:** the numbered list of the two "effects" above is numbered 3 and 4 in the PDF (continuation numbering, p. 62); the figure's exact
> connector wiring between the MPD servers and the three engines is only partly legible.

## 4.3.1 Dynamic download - assumptions (p. 65)

The client holds a single MPD instance that describes the whole presentation. Segments appear over time following the media timeline. The section is
mainly there to explain the availability timing model used by all live modes.

## 4.3.2 Preliminaries

### 4.3.2.1 MPD information needed (pp. 65-67)

A dynamic MPD gives each segment an *availability start time*: the earliest instant at which the server promises a 2xx answer. The client is
assumed to share the server's wall clock (by some external means or via 4.7). Information set used (IOP Table 8; name | status | gloss):

| Item | Status | Gloss |
|---|---|---|
| `MPD@type` | mandatory, "dynamic" | |
| `MPD@availabilityStartTime` | mandatory | anchor of the MPD on the wall clock ("AST") |
| `MPD@mediaPresentationDuration` | mandatory for these cases | total duration |
| `MPD@suggestedPresentationDelay` | optional, recommended | delay relative to availability start ("SPD") |
| `MPD@minBufferTime` | mandatory | used with `@bandwidth` ("MBT") |
| `MPD@timeShiftBufferDepth` | optional, recommended | ("TSB") |
| `Period@start` | mandatory on the first Period | relative to AST |
| `Representation@availabilityTimeOffset` | optional, defaulted | earlier availability; may also sit on BaseURL/defaults; `INF` = everything available from AST |
| `SegmentTemplate@media` | mandatory | |
| `SegmentTemplate@startNumber` | optional, defaulted | number of first segment in the Period |
| `SegmentTemplate@timescale` | optional, defaulted | |
| `@duration` or `SegmentTimeline` | exactly one per Representation | segment durations |

### 4.3.2.2 Deriving the segment list

**Terms.** *Available segment* = GET on its URL returns the segment with a 2xx. *Valid segment URL* = URL promised to point to a segment during
its availability period. *NOW* = server wall-clock time; every wall-clock MPD value is interpreted against NOW.

**MPD without updates (4.3.2.2.3).** `type` is dynamic; `mediaPresentationDuration` present *or* last Period has `@duration`; `minimumUpdatePeriod`
absent. `timeShiftBufferDepth` and `suggestedPresentationDelay` recommended.

**Periods (4.3.2.2.4).** For Period i: start `PSwc[i]` and end `PEwc[i]` on the wall clock.
- `PSwc[i]` = AST + `@start` if `@start` is present; otherwise the previous Period's start plus its `@duration` (if both `@start` and a previous
  `@duration` exist, `@start` wins). 23009-1 5.3.2.1 is the basis.
- `PEwc[i]` of the last Period = AST + MPDur, where MPDur is `mediaPresentationDuration` or (start of last Period + its `@duration`); for any other
  Period `PEwc[i] = PSwc[i+1]`.
- An MPD update may extend the end of the last Period (see 4.4).

> **Note (p. 68):** the previous-Period start in the `@start`-absent rule is written `PSwc[i]` (not `i-1`) in the PDF.

**Segment timeline entries (4.3.2.2.5, pp. 68-69).** Notation: `ts` = `@timescale`; `ato` = sum of all `@availabilityTimeOffset` values on the levels
involved in forming the segment URL (0 if none); `t[s]`, `d[s]` = `@t`, `@d` of the s-th `S`; `r[s]` = number of segments of that entry.

With `SegmentTimeline` (and no `@duration`), N_S entries s = 1..N_S:
- if `@r` >= 0: `r[s]` = `@r` + 1 (and if `@r` ends before the entry's span, the Representation has a gap there with no media);
- else if `t[s+1]` exists: `r[s] = ceil((t[s+1] - t[s]) / d[s])`;
- else: `r[s] = ceil((PEwc[i] - PSwc[i] - t[s]/ts) * ts / d[s])`  (the PDF has one extra closing parenthesis).

With `@duration` and no timeline: N_S = 1, `t[s] = 0`, `d[s]` = `@duration`, `r[s] = ceil((PEwc[i] - PSwc[i] - t[s]/ts) * ts / d[s])`; the last segment may not
exist, in which case the real count is one less (see 4.4.3.6 in the IOP).

> **Note:** the PDF defines `r[s]` as `@r`+1 in the first case but then runs `for j = 1..r[s]` *after* emitting the first segment of each entry
> (4.3.2.2.7), which only works if `r[s]` means repeats; the IOP does not reconcile the two readings (p. 69-70).

**Per-segment timing (4.3.2.2.6).** Each media segment k has an earliest presentation time `EPT` and a duration; EPT can be estimated from the MPD
(availability start minus announced duration) or read exactly from the segment itself (see IOP 3.2.11).

**Segment list parameters (4.3.2.2.7, pp. 69-71).** Per Period i and Representation r the client can compute: presentation time offset `o[i,r]`,
`ato[r]`, first/last segment numbers `k1`, `k2`, availability start/end times `SAST`, `SAET` (also for the init segment, index 0), adjusted start
`ASAST`, duration `SD`, and `URL`. Algorithm (exact):

```
k = 0
SAST[0]  = PSwc[i];   ASAST[0] = PSwc[i] - ato
for s = 1 .. N_S[i,r]:
    k += 1
    SAST[k]  = PSwc[i] + (t[s] + d[s] - o[i,r]) / ts
    ASAST[k] = SAST[k] - ato
    SD[k]    = d[s] / ts
    SAET[k]  = SAST[k] + TSB + d[s] / ts
    if media template uses $Number$:  Address = @startNumber ; URL[k] = URLTemplate($Number$, Address)
    else:                              Address = t[s]          ; URL[k] = URLTemplate($Time$,   Address)
    for j = 1 .. r[s]:
        k += 1
        SAST[k]  = SAST[k-1] + d[s] / ts
        ASAST[k] = SAST[k] - ato
        SAET[k]  = SAST[k] + TSB + d[s] / ts
        SD[k]    = d[s] / ts
        Address  = Address + 1 (Number)   or   Address + d[s] (Time)
        URL[k]   = URLTemplate($Number$ or $Time$, Address)
k2 = k ;  SAET[0] = SAET[k2]
```

Derived rules (as printed):
- Not every listed segment is reachable at NOW, only those inside the availability window.
- `k1` = smallest k >= 1 with `SAST[k] >= NOW` (literally the first not-yet-available segment; see note).
- Latest available Period i[NOW] = the Period with the largest `PEwc[i]` such that `PEwc[i] <= NOW`.
- Live-edge segment k[NOW] = largest k >= 0 with `SAST[k] <= NOW`. k = 0 (the init segment) may be the answer; then the last media segment of the
  previous Period is the newest reachable media segment.
- With `@availabilityTimeOffset`, segments are obtainable from `ASAST[k]` rather than `SAST[k]`.

> **Notes (p. 70-71):** (a) the first segment of every `S` uses `t + d - o` (segment *end*) while repeats add `d/ts` to the previous `SAST`;
> `Address = @startNumber` is re-assigned inside the `for s` loop in the printed pseudo-code. (b) `k1` and i[NOW] definitions read oddly against
> the prose that surrounds them; they are kept literally. (c) `ato = INF` drives `ASAST` to minus infinity = available from AST.

**URL templating (4.3.2.2.8, p. 71).** Segment URLs are formed with the 23009-1 5.3.9.4 template rules, then BaseURLs resolved as in 23009-1 5.6.
The IOP permits only the `%0[width]d` format tag (no other printf forms), so substitution needs no library.

**Synchronised playout (4.3.2.2.9, p. 71).** Alignment across Representations (also across Adaptation Sets) and seamless switching use the segment's
earliest presentation time within the Period.
