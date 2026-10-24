---
topic: ISO/IEC 23009-1:2022 (Ed.5) clause 6 - Segment formats (own-words implementer notes)
clauses: 6.1-6.4.7.2.3
printed_pages: 159-172
pdf_pages: 167-180
verbatim_source: private/specs-verified/dash/23009-1/06-segment-formats.md
status: derived from the page-verified private transcription; not a copy of the spec text
---

# 6 Segment formats

Segment formats are specified for two containers: ISO base media file format (ISO/IEC 14496-12, 6.3) and MPEG-2 TS (ISO/IEC 13818-1, 6.4). Guidelines for other formats are in annex F (not transcribed). Segments are the resources behind HTTP-URLs of the MPD (or data URLs).

## 6.2 Segment types (generic)

| Type | Rule summary |
|---|---|
| Initialization Segment | carries initialisation info for accessing a Representation; contains no media data with an assigned presentation time; format is media-format specific and must be defined by every format that allows/needs one |
| Media Segment | holds encoded media components; see below |
| Index Segment | mainly indexing information for one or more Media Segments; media-format specific |
| Bitstream Switching Segment | data needed to switch to the Representation it belongs to; media-format specific |
| Missing Content Segment (6.2.6) | extends the Segment timeline when data is missing/partial; see below |

### Media Segment (6.2.3)

A Media Segment shall: contain whole access units; carry enough information to present each component time-accurately (needed for seamless switching and joint presentation); express all Media Presentation times relative to the Period start compensated by `@presentationTimeOffset` (accurately, so Representations of a Period align; see [`07-combined-semantics.md`](07-combined-semantics.md)); if it is the first Media Segment of the Representation contain only streams starting with a SAP of type 1 or 2. It should say how to access the media (exact presentation time, index). It need not start with a SAP (an MPD attribute can promise that it does). It may be split into Subsegments by a Segment Index (inside the Media Segment or in a dedicated Index Segment).

### Subsegments and Segment Index (6.2.3.2-6.2.3.3)

- A Subsegment contains whole access units; formats may restrict boundaries. The Segment Index gives, per Subsegment and per indexed stream, the presentation time range and byte range, plus earliest presentation time of access units and presentation time of the first SAP.
- If an index exists for at least one stream, every access unit of any non-indexed stream shall be a SAP of type 1, and per Subsegment each non-indexed stream has exactly one access unit with presentation time <= the Subsegment's earliest presentation time.
- When several streams are indexed in one index, they should index the same number of Subsegments.
- With no Segment Index the Media Segment is one Subsegment. A Subsegment may be subdivided by further indexes; a Subsegment with media data only is a *Media Subsegment*; a Subsegment Index (`ssix`) can index levels inside a Media Subsegment.
- Generic mechanism: the `sidx` box of ISO/IEC 14496-12: `earliest_presentation_time` gives the Subsegment's earliest presentation time; byte range from `first_offset` + `reference_size`; two `sidx` boxes describing the same byte range shall have identical `reference_size` and `first_offset` pointing to the same byte offset.

### Missing Content Segment (6.2.6)

Contains no or only some of the media samples of the timeline span it stands for. If it is not decodable, it shall have major brand `miss`; it may carry an `emsg` (e.g. an MPD validity expiry event). It shall contain exactly one `sidx` whose values describe the Segment Timeline exactly: `earliest_presentation_time` = `S@t` and the subsegment durations = `S@d` of the MPD's SegmentTimeline - so a client can extend the timeline even without media.

## 6.3 ISO BMFF segment formats

- All Segment formats of 6.3 are made of boxes per ISO/IEC 14496-12. Defined: Initialization Segments, Media Segments, Self-Initializing Media Segments. Bitstream Switching Segments and Index Segments are **not** defined for ISO BMFF.
- Media Subsegment = self-contained set of one or more consecutive movie fragments (`moof` + its `mdat`(s)); an `mdat` with data referenced by a `moof` follows that `moof` and precedes the next `moof` of the same track. `reference_type` in the describing `sidx` shall be 0 for a Media Subsegment. SAP types, `sidx`, `ssix`: as in ISO/IEC 14496-12 (when a Segment Index/Subsegment Index is provided these boxes shall be used).

### Resynchronization Point for ISO BMFF (6.3.2.5, Table 51)

A Resync Point is the start of a (constrained) ISO BMFF Segment. Allowed top-level boxes (all at nesting level 0) in this order:

| Box | Cardinality | Defined in | Constraint | Gloss |
|---|---|---|---|---|
| styp | 0/1 | 14496-12 | DASH/CMAF constraints | segment type; signals compatibility with a CMAF chunk |
| prft | 0/1 | 14496-12 | DASH/CMAF | producer reference time |
| emsg | * | 23009-1 | DASH/CMAF | event message |
| free | * | 14496-12 | none | free box |
| skip | * | 14496-12 | none | skip box |
| moof | 1 | 14496-12 | DASH/CMAF | movie fragment and its contents |
| mdat | 1 | 14496-12 | DASH/CMAF | media data |

("0/1" = at most one; "*" = any number including none.) Property definitions: Index = byte offset of the first octet of the constrained segment; Time = smallest value, over all samples in the segment, of the combination of decode time, composition offset and edit list; SAP type per 4.5.2; the marker (see [`05-3-13-resynchronization.md`](05-3-13-resynchronization.md)) is present if the segment starts with `styp`, the box after `styp` is one of `prft`, `emsg`, `free`, `skip`, `moof`, and the Table 51 cardinality/ordering holds.

### Initialization Segment (6.3.3)

Conforms to ISO BMFF; shall contain `ftyp` and `moov`, no `moof`; may contain other boxes such as `pdin`. The tracks in `moov` have no samples (`entry_count` of `stts`, `stsc`, `stco` = 0) so `moov` is small. `mvex` shall be inside `moov` (signals movie fragments and sets defaults). Informative: the media engine should not need MPD data to decode the streams.

### Media Segment types (6.3.4) - brand / requirement table

Segments may conform to several types; conformance is expressed by compatible brands (and major brand if applicable) in `styp`. All Media Segments conform to the Delivery Unit type. Unless said otherwise boxes are those of ISO/IEC 14496-12.

| Type (clause) | Brand | Requirements |
|---|---|---|
| Delivery Unit Media Segment (6.3.4.2) | `dums` (may) | one or more whole self-contained movie fragments (`moof` + `mdat` holding all samples without external data references used by its track runs); each `moof` has at least one track fragment; no external data references, flag `default-base-is-moof` set, `data-offset` used, `base-data-offset-present` not used (movie-fragment-relative addressing); no absolute byte offsets; per movie fragment, track extents should be about equal so durations do not drift |
| Simple (6.3.4.3) | `msdh` (if `styp` present, shall carry it) | conforms to Delivery Unit; each `traf` contains `tfdt` (`tfad` of 3GPP TS 26.244 may also exist - clients are discouraged from applying both alignments); may contain one or more `sidx`, the first placed before any `moof` and documenting the entire Segment; redundant samples are ignored when deciding overlap (earliest presentation time computed without them) |
| Indexed (6.3.4.4) | `msix` (shall) | conforms to Delivery Unit and each `moof` is immediately followed by its `mdat`; one or more `sidx`, the first before any `moof`, documenting Subsegments that span the composition time of the whole Segment |
| Sub-Indexed (6.3.4.5) | `sims` (shall) | conforms to Indexed (the print cites 6.3.4.3); an `ssix` immediately follows the `sidx` documenting the same Subsegment; that `sidx` indexes only Media Subsegments |
| Random Access (6.3.4.6) | - (none stated) | conforms to Simple; first access unit of each movie fragment corresponds to I_sau of a SAP of type 1, 2 or 3; enough information to access the media (e.g. necessary encryption info combined with the Initialization Segment) |
| Switching Media Segments | listed in 6.3.4.1 but no section defines them | - |

### Self-initializing Media Segment (6.3.5)

- General (`dsms` brand): conforms to ISO BMFF. Since a Representation has only one such Segment, switching happens inside it (at a Subsegment starting with a SAP).
- Indexed self-initializing (`dash` brand, shall carry): the concatenation of an Initialization Segment and one Indexed Media Segment, with no `styp` before the media part.

## 6.4 MPEG-2 TS segment formats

Defined: Initialization, Media, Bitstream Switching and Index Segments; MPEG-2-TS-specific boxes in 6.4.7. (MPEG-2 TS inside ISO BMFF is covered by 6.3, not here.)

- **Subsegment**: an indexed set of access units consecutive in decode order, complete for the indexed stream (`reference_ID` = its PID), possibly with incomplete PES packets of other streams. Access units travel in PES packets; PES packets in TS packets of one PID.
- **Media stream = Elementary Stream; SAP = Elementary Stream Random Access Point**. I_SAU = position of the first (sync) byte of a TS packet of that stream's PID, which starts a PES packet that holds the access point; that PES packet has only whole access units and shall contain a PTS. Informative: normally `payload_unit_start_indicator`=1, `adaptation_field_control`='11', `random_access_indicator`=1 (for SAP types 1-3 it is commonly 1 unless the packet has no PES payload). The first packet of the PCR PID is at or before the TS packet at the smallest I_SAP; if PCR rides on a media PID the first packet of the Representation after initialisation data carries a PCR (a PCR-only adaptation-field packet may be inserted before the smallest I_SAU); with a `pcrb` box in an Index Segment PCR can be inferred.
- **Segment Index**: `sidx` (14496-12) is used; `reference_ID` shall be the indexed stream's PID; all media offsets point to the first (sync) byte of a TS packet; times in `sidx` use the `timescale` field, not 90 kHz. **Subsegment Index** (`ssix`): offsets also point to a TS packet sync byte.

### Initialization information and Initialization Segment (6.4.3)

- Untimed initialisation info: PAT, CAT, PMT, EMM, other PSI; time-varying: needed to start playout but different between at least two Subsegments/Segments. Mandatory initialisation information, in this order: PAT (untimed unless it changes within the Representation), PMT (same), PCR (time-varying). If conditional access is used, the ECM is mandatory untimed info if unchanged for the whole Period, otherwise mandatory time-varying info.
- Initialization Segment: valid MPEG-2 TS; concatenated with any Media Segment it must not change the presentation duration; contains the mandatory untimed info and no time-varying info (no PCR-bearing packets; ECM only if constant over the Representation; PSI tables only if constant over the Representation); only complete sections. It may be absent, then all Media Segments are self-initializing; with one in use not all initialisation info has to be in it, only the concatenation must be complete.

### Media Segments (6.4.4)

- Basic Media Segment: valid TS; concatenating consecutive Segments of one Representation must also give a valid TS (5.3.5.1); complete TS packets; exactly one program; all time-varying initialisation info between I_SAP and I_SAU and/or in the Index Segment; no dependence on initialisation info in earlier Segments. Should contain only complete PES packets, sections, access units (each PES packet should be whole access units).
- Content protection: info needed to decrypt (or find it, e.g. ECM) shall precede the encrypted packets, in the same (Sub)Segment and/or the Initialization Segment; a Subsegment may lack a preceding ECM only if an Index Segment lets the client find it. Sub-Representations can be arranged so ECM is in each that needs it (e.g. by giving ECM its own level).
- Self-initializing Media Segment: a basic one that also contains all mandatory untimed and timed initialisation info; all of it should come before any media data.

### Bitstream Switching Segment (6.4.5)

Valid TS; when concatenated with any Media Segment it must not alter the Media Presentation timeline for that segment; any initialisation info in it is identical to the Initialization Segment's. Informative: use it where non-conforming behaviour (e.g. continuity counter errors, differing initialisation info) is expected at a switch point.

### Index Segments (6.4.6) and the `pcrb` box (6.4.7)

- Index Segments are sequences of ISO BMFF boxes (not valid ISO BMFF files; a client needs only `styp`, `sidx`, `ssix`). Recommended for at least one media stream. Other boxes may appear but must not carry information needed to interpret `styp`/`sidx`/`ssix`.

| Type | Brand | Rules |
|---|---|---|
| Single Index Segment (6.4.6.2) | `sisx` | starts with `styp` carrying it; indexes exactly one Media Segment with one or more `sidx`; optional `ssix` following the `sidx` of the same Subsegment (no other `sidx` between); optional `pcrb` following the `sidx` it describes (covers every subsegment of the last `sidx`) |
| Representation Index Segment (6.4.6.3) | `risx` | starts with `styp` carrying it; each Media Segment indexed by one or more contiguous `sidx`; each `sidx` may be followed by `ssix` and/or `pcrb`; the per-Segment indexes are concatenated in order and preceded by one `sidx` indexing the Index Segment, with one loop entry per Media Segment |
| Subsegment Index Segment (6.4.6.4) | `ssss` | a Single or Representation Index Segment that also has `ssix` immediately after the `sidx` of the same Subsegment (its `reference_type` = 0); `pcrb`, if present, follows the `ssix` |

Figure 6 (printed p. 171): a Representation Index Segment for four Subsegments S0-S3 with an `ssix` per `sidx` indexing two temporal layers (I frames L0 and P frames L1, L2 positions) for trick modes.

**`pcrb` (MPEG-2 TS PCR information box, 6.4.7.2)**: box type `pcrb`, container File, optional, at most one. Syntax: `unsigned int(32) subsegment_count;` then for each of `subsegment_count`: `unsigned int(42) pcr; unsigned int(6) pad = 0;`. `subsegment_count` is positive and shall equal `reference_count` of the last `sidx`. `pcr` is the PCR value corresponding to the first (sync) byte of the first TS packet of the corresponding Media Subsegment (it differs from a PCR carried in that packet itself because ISO/IEC 13818-1 defines PCR relative to the byte containing the last bit of `program_clock_reference_base`).

## Quirks

- 6.3.4.1 lists "Switching Media Segments" among the types but no subclause defines them; 6.3.4.5 points at 6.3.4.3 for the Indexed type (it is 6.3.4.4) and 7.3.4 points at 6.3.4.4 for Sub-Indexed (it is 6.3.4.5).
- 6.3.4.3 NOTE on `tfad`: both alignments (`tfdt` and `tfad`) would double-correct.
- 6.2.3.2 contains "the values their first_offset field shall point to the same byte offset" (word missing).
- Table 51's header cell reads "NL 0" (nesting level 0); the table includes one row with a two-paragraph description (styp).

Verbatim text: see `private/specs-verified/dash/23009-1/06-segment-formats.md` (printed pp. 159-172).
