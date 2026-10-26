# P0 re-read: private verbatim vs public notes (5.3.9.5.3, 5.3.9.6, 5.3.2.1)

Done for P0 of `docs/dash-runtime/DESIGN.md`. Each clause's private verbatim text (`private/specs-verified/dash/23009-1/05-3-9-5-segment-information-derivation.md`,
`05-3-9-6-segment-timeline.md`, `05-3-2-period.md`) was read in full against the public notes
([`05-3-9-5-segment-information-derivation.md`](05-3-9-5-segment-information-derivation.md),
[`05-3-9-6-segment-timeline.md`](05-3-9-6-segment-timeline.md), [`05-3-2-period.md`](05-3-2-period.md)) sentence by sentence, then the printed page
numbers were checked against the PDF itself (`pdftotext -layout` of PDF pages 41-42 and 91-100, reading the printed footer number of each page).
Page convention of the private files, from their `VERIFICATION.md`: `<!--PB printed N-->` marks the END of printed page N.
Printed page = PDF page - 8. Public notes were corrected where listed; nothing quoted here is verbatim standard text beyond a few words.

## Verdict by clause

| Clause | Formulas | Defaults | Ranges / types | Page references |
| --- | --- | --- | --- | --- |
| 5.3.9.5.3 Media Segment information | all match; two internal inconsistencies of the standard, D2 and D3 | `startNumber` 1, `SegmentList@startNumber` 1: match | `@endNumber` rule (differs by at most 1, higher numbers invalid): match | wrong by one, D1 |
| 5.3.9.6 Segment timeline | all match; one standard-internal ambiguity D3 and one unreadable constraint D4 | `@r` 0, `@k` 1, `@t` default derivation: match | `@t/@n/@d/@k` unsignedLong, `@r` xs:integer: match | sub-clause labels wrong by one, D1 |
| 5.3.2.1 Period overview | all five rules match | `PeriodStart` of a first static Period 0: match | none | correct (pp. 33-34) |

## Discrepancies

### D1. Printed page numbers (public notes wrong; fixed)

Evidence: the printed footer of each PDF page.

| Where | Public said | PDF shows | Fix |
| --- | --- | --- | --- |
| 5.3.9.5 whole clause | pp. 83-87, PDF 91-95 | printed 84-87, PDF 92-95 (PDF 91 / printed 83 is the end of 5.3.9.4.4) | notes header and `INDEX.md` row corrected |
| 5.3.9.5.1 | p. 83 | p. 84 | corrected |
| 5.3.9.5.2 | pp. 83-84 | p. 84 | corrected |
| 5.3.9.5.3 | pp. 84-86 | pp. 85-87: heading and the `$Number$` bullet p. 85; the `@duration` start-time formula, `@endNumber`, availability start/end and ATO p. 86; the "MPD shall include URL information" rule p. 87 | corrected, quirk note now says p. 85 vs p. 86 |
| 5.3.9.5.4 | pp. 86-87 | p. 87 | corrected |
| 5.3.9.5.5 | p. 87 | p. 87 | unchanged |
| 5.3.9.6.1 | p. 88 | pp. 88-89 (the textual-order rule and the `$Time$`/sidx list are on 89) | corrected |
| 5.3.9.6.2 Table 22 | pp. 88-89 | pp. 89-90 (header and first rows 89, `@k`/`@r` rows 90) | corrected |
| 5.3.9.6.3 XML syntax | not labelled | p. 91 | none needed |
| 5.3.9.6.4 | p. 90 | p. 91 | corrected |
| 5.3.9.6.5 | pp. 90-91 | pp. 91-92 | corrected |
| quirk notes (`@k` pointer, "except ... are met", "seqment") | pp. 89, 87-88, 91 | pp. 90, 88, 92 | corrected |
| 5.3.2.1 | pp. 33-34 | pp. 33-34 | correct |

Not fixable here: the front matter of the private files `05-3-9-5-segment-information-derivation.md` (`pdf_pages: 91-95`, `printed_pages: 83-87`) carries the same off-by-one;
the private repo is a separate submodule. The other 5.3.9.x public notes (5.3.9.4 and the earlier clauses) were not re-measured against footers in this pass; a page-range sweep of
the whole index against the PDF footers is recommended before anyone relies on a page number there.

### D2. `$Number$` replacement formula, 5.3.9.5.3 (printed pp. 85 and 86)

The standard prints two readings, both reproduced in the public notes (they were already flagged):

- the `$Number$` bullet (p. 85): replace by `(k - 1) + (kstart - 1)`, `kstart = @startNumber` or 1, which is `startNumber - 1` for the first Segment;
- the `@duration` bullet (p. 86): the Segment number is `(k - 1) + NumberStart` (the text even brackets "[e.g. (k-1) + NumberStart]"), which is `startNumber` for the first Segment.

Settled by data, which the previous note had left "to confirm": `fixtures/dash/manifest.mpd` (`startNumber="1"`) names its first Segment `chunk-stream0-00001.m4s` (`tfdt` 0), and
`fixtures/dash/timeline-gpac/` (a different packager, GPAC; `startNumber="1"`, `$Number$`, `oracle.txt`) names it `seg-1-1.m4s` (`tfdt` 0); the first reading would make that Segment number 0. The second reading (`startNumber + k - 1`) is correct and the public note now says so,
citing the fixtures (and, per `docs/dash-runtime/DESIGN.md` section 11 item 1, livesim2 with `startNumber="0"`).

### D3. `$Time$` substitution when `PTO != 0` (standard-internal contradiction; public note fixed to state it)

Three statements, all verbatim-verified:

1. 5.3.9.5.3 (p. 85): "replacing the $Time$ identifier by MPD start time of this segment".
2. 5.3.9.4.4 Table 21 (the `$Time$` row): substituted with "the MPD start time of the Segment being accessed. For the Segment Timeline, this means ... the value of the SegmentTimeline@t attribute".
3. 5.3.9.6.1 (p. 89): "replacing the $Time$ identifier by the earliest presentation time obtained from the SegmentTimeline", and the sidx rule equates `S@t` with the `earliest_presentation_time`.

5.3.9.6.1 and Table 22 define the *MPD start time* of a timeline Segment as `S@t - @presentationTimeOffset`. Read literally, statement 1 (and the first sentence of 2) therefore substitute `S@t - PTO`, while the last sentence of 2 and
statement 3 substitute the raw `S@t`. They coincide only when `PTO = 0`, which is true of every real MPD committed so far (livesim2, ffmpeg, GPAC; the `$Time$` fixtures `fixtures/dash/livesim2/segtimeline_time_*.mpd` carry no PTO),
so no real capture resolves it. The public notes previously said "the value computed below, or the timeline `@t`", hiding the conflict. Now: substitute the raw `S@t` (two of the three statements, and the one that matches the sidx rule). OPEN for the owner:
if a real `$Time$` + `PTO != 0` asset is found, re-decide; until then a test pinning the raw-`S@t` reading must be marked as a documented choice, not a spec fact.

### D4. `@k` ordering constraint, 5.3.9.6.4 (p. 91): unreadable as printed

Printed: "The integer of the quotient of the value of @d and the value of @k of any S element shall not exceed the quotient of @d and the value of @k minus 1 of any other S element in the Segment Timeline."
The right-hand side can be `d / (k - 1)` or `(d / k) - 1`. The previous public note stated the first reading as fact. It now quotes the sentence and records both readings; a validator must not enforce either. Segment sequences (`@k`) are profile-gated and
no profile implemented here allows them, so there is no near-term effect.

### D5. Conditions for a nominal `@d`, 5.3.9.6.1 (p. 88): wording (fixed)

The public note called it an exception for "the single Segment that simultaneously (a)...(c)" and merged two printed conditions. The print has four conjunctive conditions: `@availabilityTimeOffset` present, `@availabilityTimeComplete` false, the `S` element has the largest `S@t` in the MPD,
and the Segment's non-adjusted availability time is at or after the MPD publish time. The condition is on the `S` *element* (which may stand for a run), not on a single Segment. Reworded to four items.

### D6. Scope of `$Number$`/`$Time$` rules, 5.3.9.5.3 (precision, fixed)

The print says "contains or inherits a SegmentTemplate element with $Number$" (and with `$Time$`); the notes dropped "or inherits". Restored, since a template declared on the Period or AdaptationSet governs every Representation below it.

### D7. Period kind rules, 5.3.2.1 (pp. 33-34): presentation (fixed)

The five rules match the print, but the public note called them a "decision procedure ... in document order". The standard states them as a plain bullet list: they are not mutually exclusive and carry no precedence (a Period with `@start` and `@duration` followed by a Period with `@start` is regular by rule 1 and early terminated by rule 5).
The note now says so; no formula changed.

## Checked and found identical (no action)

- 5.3.9.5.3: `MPDstart(Number) = (Number - NumberStart) * @duration`, `Number = (k-1) + NumberStart`; single-segment rule (start 0); last-Segment duration from the Period duration; `@endNumber` (at most 1 from nominal, higher numbers invalid);
  `SegAvailStart = AST + PeriodStart + MPDstart + MPDduration`; `SegAvailEnd = SegAvailStart + MPDduration + @timeShiftBufferDepth`; `ATO` as the sum of all levels used to resolve the URL, adjusted start = start - ATO; the coverage rule for URL listing; Initialization availability (start `AST + PeriodStart`, end = latest Media Segment end).
- 5.3.9.6: expansion of `S` (`@t` default = previous `t + d*(r+1)`, first 0; MPD start = `t - PTO`; `r+1` Segments; negative `@r` to next `S@t` / Period end / next MPD update; no rounding rule printed); `@n` constraints; `@d` at most `maxSegmentDuration`; sidx equalities for `$Time$` (timescale, `earliest_presentation_time`, sum of subsegment durations); `@k` semantics (`floor(d/k)`); `$SubNumber$` rules; mutual exclusion of `@duration` and `SegmentTimeline` (it is in 5.3.9.2.1, p. 73, not in this clause).
- 5.3.2.1: all five rules, the early-available restrictions (not before a regular Period, announced resources must be available, may be removed by an update while no PeriodStart exists), the remote-Period remark, the PeriodStart formula.
