# ISO/IEC 23009-1 (DASH) — MPD timing rules we depend on

Curated **semantic** rules for the DASH Media Presentation Description (MPD), focused on the
**timeline / segment-timing** model the `timed-metadata` crate needs to map DASH events to/from
SCTE-35 + HLS `EXT-X-DATERANGE`. Source:
`specs/fulltext/iso_iec_23009-1_dash_2012.md` (gitignored pdf2md of the copyrighted PDF; regenerate
with pdf2md). Each rule cites the spec § and line. Decisions cite here.

> ⚠️ **Edition note (not a blocker).** This is the **2012 first edition** — no `emsg`,
> `EventStream`/`InbandEventStream`, or MPD events (added in 23009-1:2014+). This doc therefore
> covers only the MPD **timing** model (stable since 2012). **`emsg` is fully covered separately**
> in [`emsg-rules.md`](emsg-rules.md), grounded on the free **DASH-IF IOP Part 10** source already
> on disk and implemented in the `mp4-emsg` crate — nothing further needs vendoring for it.

## Presentation hierarchy & timeline — §5.3.2 (fulltext L675)

- **Media Presentation timeline** = concatenation of all **Period** timelines, common to all
  Representations in a Period (§3.1.21 L221).
- `PeriodStart` derivation (L685): `Period@start` if present; else previous `Period@start` +
  previous `Period@duration`; else (dynamic, first/no-duration) an Early Available Period (L689).
  `Period@duration` sets the next PeriodStart (L721). For seeking, `@start`/prev `@duration` should
  be present (L695).
- Hierarchy: MPD → Period → AdaptationSet → Representation → segment info (SegmentBase/List/Template).

## Segment timescale & presentation offset — §5.3.9.2.2 (fulltext L1385)

- **`@timescale`** (L1395): units/second for all duration/time values in the Segment Information.
  All `@d`/`@t`/`@duration`/`@presentationTimeOffset` are in **these ticks** — divide by `@timescale`
  for seconds (L1403). This is the conversion factor `timed-metadata` uses against the 90 kHz /
  27 MHz PTS clock.
- **`@presentationTimeOffset`** (L1401): the Representation's presentation-time offset **relative to
  the start of the Period**; seconds = `@presentationTimeOffset / @timescale`. Subtract it to align
  segment times to the Period origin.
- A multi-segment Representation **shall** carry **either `@duration` or `SegmentTimeline`**, never
  both (L1381). `@startNumber` (default 1) numbers the first Media Segment (L1446, L1676).

## SegmentTemplate substitution — §5.3.9.4.4 (fulltext L1597)

- Template identifiers (Table 16, L1613), case-sensitive, optional `printf` width `%0[width]d`:
  - `$$` → literal `$`; `$RepresentationID$` → `Representation@id` (no format tag);
  - **`$Number$`** → the segment number (default width 1);
  - `$Bandwidth$` → `Representation@bandwidth`;
  - **`$Time$`** → the `SegmentTimeline@t` (`S@t`) of the segment being accessed.
- **`$Number$` and `$Time$` are mutually exclusive** in one template (L1628).
- `$Number$` addressing: MPD start time of segment N = `(N - startNumber) × @duration`; duration =
  `@duration` except the last segment (L1688).

## SegmentTimeline — §5.3.9.6 (fulltext L1732) — arbitrary durations + discontinuity

- `SegmentTimeline` = ordered list of **`S`** entries; each `S` = run of contiguous equal-duration
  segments. `S@d` (mandatory) = duration in `@timescale` ticks; `S@r` (default 0) = **repeat count
  minus one** (r=3 ⇒ 4 segments); `S@t` (optional) = MPD start time of the run's first segment,
  relative to Period start (L1746, Table 17 L1774).
- **Default `S@t`** when absent (L1791): 0 for the first `S`; otherwise `prev S@t + prev@d × (prev@r+1)`.
- **`S@t` greater than that derived value signals a timeline discontinuity** (a gap with no segment
  data) (L1789). A timed-metadata mapping must treat these gaps as real (not interpolate across).
- Textual `S` order = time/number order (L1752); `@d ≤ MPD@maxSegmentDuration` (L1750).

## SegmentTimeline ↔ `sidx` binding — §5.3.9.6.1 (fulltext L1754) — the cross-layer anchor

When `SegmentTemplate` + `$Time$` are used, the MPD timing **shall** mirror the in-segment `sidx`
(ISOBMFF §8.16.3 — see `isobmff-rules.md`):
- at least one `sidx` shall be present (L1756);
- `@timescale` **==** the first `sidx`'s `timescale` field (L1760);
- `S@t` **==** the first `sidx`'s `earliest_presentation_time` for that Media Segment (L1762);
- `S@d` **==** sum of all `subsegment_duration` in the first `sidx` of that segment (L1764);
- the `$Time$` URL substitution uses that earliest presentation time (L1766).

This is the precise rule that lets `timed-metadata` derive a segment's media time from either the
MPD or the `sidx` and have them agree — the conversion contract for SCTE-35/emsg ↔ DASH timeline.

## Code-conformance notes (tracked — NOT yet applied; `timed-metadata` / future DASH crate)

1. Timeline math uses `@timescale` (§5.3.9.2.2 L1395) + `@presentationTimeOffset` (L1401) to convert
   segment ticks ↔ Period-relative seconds; never assume 90 kHz.
2. `SegmentTimeline` default `S@t` derivation and **discontinuity** detection (§5.3.9.6 L1789) — a
   converter must surface gaps, not smooth them.
3. MPD `@timescale`/`S@t`/`S@d` must agree with the segment's `sidx` (§5.3.9.6.1 L1760) — the
   `timed-metadata` ↔ `mp4-emsg` round-trip relies on this single source of truth.
4. **Vendor a 23009-1:2014+ edition** before curating `emsg`/`EventStream` rules (absent here).
