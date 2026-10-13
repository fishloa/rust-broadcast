# ISO/IEC 23009-1 (DASH) — MPD timing rules

MPD timeline / segment-timing model used by `timed-metadata` (DASH ↔ SCTE-35 / HLS
`EXT-X-DATERANGE`). Source: ISO/IEC 23009-1:2012 (1st ed.) — `§` + line cites below refer to a
local, gitignored `specs/fulltext/iso_iec_23009-1_dash_2012.md` pdf2md dump (per
`specs/MEDIA-SPECS-LOCAL.md`) that was never committed and is not reproducible from this
checkout, so those `L…` anchors are not independently verifiable; treat the spec § citations as
authoritative. Events and the `emsg` box are in `emsg-rules.md` (23009-1:2022 ed.5 §5.10).

## Presentation hierarchy & timeline — §5.3.2 (L675)

- **Media Presentation timeline** = concatenation of all **Period** timelines, common to all
  Representations in a Period (§3.1.21 L221).
- `PeriodStart` (L685): `Period@start` if present; else previous `Period@start` +
  previous `Period@duration`; else (dynamic, first/no-duration) an Early Available Period (L689).
  `Period@duration` sets the next PeriodStart (L721).
- Hierarchy: MPD → Period → AdaptationSet → Representation → segment info (SegmentBase/List/Template).

## Segment timescale & presentation offset — §5.3.9.2.2 (L1385)

- **`@timescale`** (L1395): units/second for all duration/time values in the Segment Information.
  `@d`/`@t`/`@duration`/`@presentationTimeOffset` are in these ticks; seconds = value / `@timescale`
  (L1403).
- **`@presentationTimeOffset`** (L1401): the Representation's offset **relative to Period start**;
  seconds = `@presentationTimeOffset / @timescale`.
- A multi-segment Representation **shall** carry **either `@duration` or `SegmentTimeline`**, never
  both (L1381). `@startNumber` (default 1) numbers the first Media Segment (L1446, L1676).

## SegmentTemplate substitution — §5.3.9.4.4 (L1597)

- Template identifiers (Table 16, L1613), case-sensitive, optional `printf` width `%0[width]d`:
  `$$` → `$`; `$RepresentationID$` → `Representation@id`; `$Number$` → segment number (default
  width 1); `$Bandwidth$` → `Representation@bandwidth`; `$Time$` → the `SegmentTimeline@t` (`S@t`).
- `$Number$` and `$Time$` are **mutually exclusive** in one template (L1628).
- `$Number$` addressing: start time of segment N = `(N - startNumber) × @duration`; duration =
  `@duration` except the last segment (L1688).

## SegmentTimeline — §5.3.9.6 (L1732)

- Ordered list of **`S`** entries; each `S` = run of contiguous equal-duration segments. `S@d`
  (mandatory) = duration in `@timescale` ticks; `S@r` (default 0) = **repeat count minus one**
  (r=3 ⇒ 4 segments); `S@t` (optional) = MPD start time of the run's first segment, relative to
  Period start (L1746, Table 17 L1774).
- **Default `S@t`** when absent (L1791): 0 for the first `S`; else `prev S@t + prev@d × (prev@r+1)`.
- **`S@t` greater than that derived value signals a timeline discontinuity** — a gap with no segment
  data (L1789).
- Textual `S` order = time/number order (L1752); `@d ≤ MPD@maxSegmentDuration` (L1750).

### Negative `S@r` and live availability (DASH-IF IOP v4.3 §3.10.2.2.4 / §3.10.2.2.6)

Source: DASH-IF Interoperability Points v4.3 (free, https://dashif.org/docs/DASH-IF-IOP-v4.3.pdf),
which restates 23009-1 §5.3.9.6.2 normatively. For the `s`-th `S` (`t[s]`, `d[s]`, `r[s]` the repeat
*count*), `ts` = `@timescale`, `PSwc[i]`/`PEwc[i]` the Period start/end in wall-clock seconds:

- `@r >= 0`: `r[s]` = `@r + 1`.
- `@r < 0`: if `t[s+1]` (the next `S`'s explicit `@t`) is present, `r[s] = ceil((t[s+1] - t[s]) / d[s])`;
  else `r[s] = ceil((PEwc[i] - PSwc[i] - t[s]/ts) * ts / d[s])` (the last segment may not exist yet).
- `S@t` is relative to the Period start and absolute (not 0-based): a live timeline's first `S@t`
  is typically huge; `$Time$` substitutes it unchanged.
- Segment availability (dynamic): `SAST[k] = PSwc[i] + (t[s] + d[s] - o[i,r]) / ts`, with
  `PSwc[i] = AST + Period@start` and `o` = `@presentationTimeOffset` — so `MPD@availabilityStartTime`
  (AST, §5.3.1.2) is the wall-clock anchor of the whole presentation, mandatory for `type="dynamic"`.

## SegmentTimeline ↔ `sidx` binding — §5.3.9.6.1 (L1754)

When `SegmentTemplate` + `$Time$` are used, the MPD timing **shall** mirror the in-segment `sidx`
(ISOBMFF §8.16.3):
- at least one `sidx` shall be present (L1756);
- `@timescale` == the first `sidx`'s `timescale` (L1760);
- `S@t` == the first `sidx`'s `earliest_presentation_time` for that segment (L1762);
- `S@d` == sum of `subsegment_duration` in the first `sidx` of that segment (L1764);
- the `$Time$` substitution uses that earliest presentation time (L1766).
