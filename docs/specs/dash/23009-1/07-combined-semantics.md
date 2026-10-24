---
topic: ISO/IEC 23009-1:2022 (Ed.5) clause 7 - Combined semantics of MPD and Segment formats: timeline, authoring rules (own-words implementer notes)
clauses: 7.1-7.4.4
printed_pages: 172-178
pdf_pages: 180-186
verbatim_source: private/specs-verified/dash/23009-1/07-combined-semantics.md
status: own-words notes derived from the private verbatim conversion (pdf2md hybrid, exit 0, rendered-page verified, independently verified); not a copy of the spec text
---

# 7 Combined semantics of MPD and Segment formats

Authoring rules that tie the MPD (clause 5, 5.15) to the Segment formats (clause 6). 7.2 is general, 7.3 ISO BMFF, 7.4 MPEG-2 TS; other formats: annex F. Informative note in 7.1: representation metadata may be repeated inside the media streams; the two should agree, and if they do not the value in the stream wins (especially for decoding); MPD metadata is chiefly for the client's selection, stream metadata for the decoder.

## 7.2 General

### 7.2.1 Media Presentation timeline

Formulas (normative):

- Let T_O = `@presentationTimeOffset` of the Representation and T_P the presentation time of an access unit as signalled in the media. Its Media Presentation time relative to *PeriodStart* is
  **T_M = T_P - T_O**.
- A Representation should cover all presentation times T_P >= T_O. If smaller values occur (negative `@eptDelta`), presentation of that Media Segment is expected only for T_P >= T_O.
- Segment start times in the MPD:
  - with `@duration` addressing, the MPD start time of a Segment (5.3.9.5.3) should approximate T_M; the difference between MPD start time and presentation time shall not exceed 50 % of (`@duration` / `@timescale`);
  - with a SegmentTimeline, the MPD start time shall equal exactly the earliest presentation time T_M of the Segment.
- Dynamic MPD with `MPD@suggestedPresentationDelay`: wall-clock presentation time of an access unit = `MPD@availabilityStartTime` + *PeriodStart* + T_M + `MPD@suggestedPresentationDelay` (the print spells the first attribute `availibilityStartTime`).
- Period boundaries: playout may have to be adjusted at the end of a Period to match the next *PeriodStart*; Segments whose real media is longer or shorter than the Period duration cause overlaps or gaps respectively.
- Outage handling (informative/should): authors should address encoder or input failures with redundancy or outage/blackout signals, may use empty or zero-duration Segments where the format allows, may signal gaps in the timeline (explicitly with SegmentTimeline, or by leaving gaps), and if nothing helps may terminate the Period early (5.3.2); a client is expected to ride through such outages and continue with the next Period.
- Format-specific presentation-time definitions may exist (7.3.2, 7.4.2).

Note: the *availability* derivations (Segment availability start/end, `availabilityTimeOffset`) are not in clause 7; see 5.3.9.5.3 ([`05-3-9-5-segment-information-derivation.md`](05-3-9-5-segment-information-derivation.md)).

### 7.2.2 Segment Index

Where the Media Segments of an Adaptation Set's Representations carry a Segment Index: the order in which Segment Index boxes for different media streams first appear defines an ordering of the components, identical for all Segments of all Representations of the set (so a component with an index in one Segment has an index in all Segments of the set), and non-indexed streams have the same access-unit duration in all Representations of the set.

### 7.2.3-7.2.4 Segment / Subsegment alignment

The requirements of 5.3.3.2 apply.

## 7.3 ISO BMFF based Media Presentation

### 7.3.1 General

- MPD per 5.1; MIME type per annex C; 7.2 applies; `@mimeType` of each Representation per RFC 4337, extra parameters per RFC 6381; if present `@SegmentProfiles` is a whitespace-separated list of segment profile identifiers.
- Allowed Segment types: Initialization (6.3.3), Media (6.3.4.2), Self-Initializing (6.3.5).
- If a Representation has more than one Media Segment: an Initialization Segment shall be present; Media Segments shall not be self-initializing (format 6.3.4); the last Media Segment may carry the `lmsg` compatible brand and non-last ones shall not.
- If it has exactly one Media Segment: either one Initialization Segment plus one non-self-initializing Media Segment, or one Self-Initializing Media Segment. Index Segments may be present.
- Sub-Representations: rules of 7.3.4.

### 7.3.2 Media presentation timeline

T_P is relative to the movie timeline, i.e. composition times after applying any edit list. Recommended: `@timescale` equals the `timescale` of the Media Header Box of a present track; with a `sidx`, the track whose `sidx` appears first in the Media Segment should define `@timescale`.

### 7.3.3 Authoring rules for specific MPD attributes

- Segments starting with a SAP: nothing beyond 5.3.3.2 and 6.3.2.2.
- `@bitstreamSwitching` = true: 5.3.3.2 conditions apply and **no** Bitstream Switching Segment is present; consequences: track IDs of the same media component are identical across the Adaptation Set's Representations; the conditions for `@segmentAlignment` = true hold; the conditions for `@startWithSAP` = 2, or (all Representations share `@mediaStreamStructureId` and `@startWithSAP` = 3) hold.

### 7.3.4 Sub-Representations

If a `SubRepresentation` with `@level` exists, the Media Segments conform to the Sub-Indexed Media Segment type (the print cites 6.3.4.4; it is 6.3.4.5) and the Initialization Segment contains the Level Assignment (`leva`) box. `@level` ties the Sub-Representation to a level in the Subsegment Index. Media data should be ordered so each level enhances the lower ones.

### 7.3.5 SegmentTimeline without Segment Index

With a SegmentTimeline and `$Time$` templating and no `sidx` in the Media Segment: exactly one track and one movie-fragment header; the `baseMediaDecodeTime` of the first `tfdt` equals the Segment's earliest presentation time (usable to form the URL); the sum of the `sample_duration` values of the `trun` boxes of the single `traf` equals the Segment's presentation duration, so the next Segment's address can be derived without a new MPD.

## 7.4 MPEG-2 TS based Media Presentation

### 7.4.1 General

MPD per 5.2 (print), MIME type per annex C, 7.2 applies; Representation `@mimeType` shall be `video/mp2t`. Allowed Segment types: Initialization (6.4.3), Media (6.4.4), Bitstream Switching (6.4.5), Index (6.4.6). `@segmentProfiles` may be absent and is ignored if present. With a SegmentTimeline and `$Time$` templating the Segment Index shall be present.

### 7.4.2 Media presentation timeline

- T_P is the PTS carried in the TS. For one Representation let PTS(i) be the PTS of the i-th access unit and PTS_A(i) the same unwrapped across 33-bit rollovers (as if PTS had unlimited bits).
- **T_P(i) = [PTS_A(i) - PTS_0] * S / 90 000**, with PTS_0 typically PTS(0) and S the timescale (S is the `timescale` of the `sidx` when an Index Segment is used); PTS_0 can be derived from `@presentationTimeOffset` with suitable scaling.
- At a discontinuity PTS_A(i) assumes relative timing is kept: the correction is the gap between the first PCR seen after the discontinuity and the PCR value extrapolated from the earlier PCR rate. Recommended: start a new Period to reset `@presentationTimeOffset` at discontinuities.
- Recommended: `@timescale` matches the elementary-stream clock S; with a `sidx`, the stream whose `sidx` appears first in the Index Segment defines `@timescale`.

### 7.4.3 Authoring rules for specific MPD attributes

- Segments starting with SAPs: nothing beyond 5.3.3.2 and 6.4.2.2.
- `@segmentAlignment` = true: requirements of 5.3.2 and 5.3.3.2 plus each Media Segment contains only complete PES packets and sections and only complete access units per PID, and the first PES packet has a PTS.
- `@subsegmentAlignment` not false: 5.3.3.2 semantics apply; a Subsegment contains only complete PES packets and sections per PID and the first PES packet of each elementary stream has a PTS.
- `@bitstreamSwitching` = true: 5.3.3.2 conditions must hold for the whole sequence i = 1..M and for any consecutive run i_S..i_E; a Bitstream Switching Segment may be present (signalled by `BitstreamSwitching` in the Segment Information) such that Media Segment i of Representation X + Bitstream Switching Segment of Y + Media Segment i+1 of Y form a valid MPEG-2 TS; consequences: the `@startWithSAP` = 2 conditions (or shared `@mediaStreamStructureId` and `@startWithSAP` = 3 on the set), the `@segmentAlignment` = true conditions, and PCR present in the Segment before the first byte of TS payload carrying media (not inferred from `pcrb`).

### 7.4.4 Sub-Representations

If a `SubRepresentation` with `@level` is present, an Index Segment shall be present in the format of 6.4.6.4 whose Subsegment Index box has at least one entry for `@level` and for each `@dependencyLevel`; the remaining SubRepresentation attributes should let the data be distinguished from the containing Representation (no inband level assignment exists in MPEG-2 TS). If the Subsegment Index is used for temporal sub-sequences, PCR should precede the first media bytes of the indexed range, and encryption keys should not change within a Subsegment.

## Quirks

- 7.1 says the formats are "defined in Clauses 5 and 5.15" (5.15 is part of clause 5); 7.4.1 cites 5.2 for the MPD description where 7.3.1 cites 5.1.
- Informative note 7.1 contains "it is is important" (doubled word).
- `@SegmentProfiles` (capital S) in 7.3.1 versus `@segmentProfiles` in 7.4.1.

Verbatim text: see `private/specs-verified/dash/23009-1/07-combined-semantics.md` (printed pp. 172-178).
