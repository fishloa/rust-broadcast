# ISO/IEC 14496-12 (ISOBMFF) — behavioural rules we depend on

Curated **semantic** rules (the prose, not just syntax) the workspace's MP4/fMP4/CMAF
crates must honour. Source: the full spec text at
`specs/fulltext/iso_iec_14496-12_isobmff_2015.md` (gitignored pdf2md of the copyrighted
`specs/iso_iec_14496-12_isobmff_2015.pdf`; regenerate with the pdf2md skill). Each rule
cites the spec § and the line in that full-text md. Consumers: the planned transmux /
fMP4-mux / `mp4-emsg` crates. Design decisions cite here — they are not asserted.

## Box / FullBox structure — §4.2 (fulltext L1254)

- **Box** = `size`(32) + `type`(32 four-CC), then fields/children. `size` is the **entire**
  box incl. header + children (L1266). `size==1` → real size in a `largesize`(64) that
  follows (L1292); `size==0` → box runs **to end of file** (normally only `mdat`). `type=='uuid'`
  → a 16-byte `usertype` extended type follows.
- **Big-endian**; sub-byte fields packed MSB-first (L1276).
- **FullBox** adds `version`(8) + `flags`(24) before its fields (L1298).
- **Unknown `type` → ignore + skip** (use `size` to advance) (L1294). **Unknown FullBox
  `version` → ignore + skip** (L1304). A parser must be box-order-tolerant on read but
  size-driven; a serializer must recompute `size` from contents (no stored-and-echoed length).

## File type — §4.3 (fulltext L1306)

- `ftyp` = `major_brand`(32) + `minor_version`(32) + `compatible_brands[]` to end (L1360).
- `ftyp` **shall precede any variable-length box** (movie/mdat/free) (L1326, L1776).
- `minor_version` is **informative only** — never use for conformance (L1342).
- A reader should accept a file marked compatible with **any** brand it implements (L1332).
- Absent `ftyp` → treat as `mp41`/0/`mp41` (L1316).

## Box order — §6.2.3 (fulltext L1746)

Mostly *recommendations* (read-tolerant), but two are **shall** and one is structural:
1. `ftyp` **shall** be before any variable-length box; only a fixed-size signature may precede (L1776).
2. Movie Fragment boxes **shall** be in `sequence_number` order (L1786, §8.8.5).
3. Recommended `stbl` child order: `stsd, stts, stsc, stsz, stco` (L1788); `tref`/`elst` before
   `mdia`; `hdlr` before `minf`; `dinf` before `stbl` (L1792). `mfra` last in file if present.
- Containment (Table 1, L1808): `moov`→`mvhd`,`trak*`; `trak`→`tkhd`,`tref`,`edts`(→`elst`),`mdia`;
  `mdia`→`mdhd`,`hdlr`,`minf`; `minf`→`vmhd`/`smhd`/…,`dinf`(→`dref`),`stbl`;
  `stbl`→`stsd`,`stts`,`ctts`,`cslg`,`stss`,`stsc`,`stsz`,`stco`/`co64`.

## Sample model — §8.5 (fulltext L2723)

- `stbl` holds **all** time + data indexing: locate a sample in time, type (sync?), size,
  container, offset (L2729).
- A data-referencing track **requires** `stsd`, `stsz`, `stsc`, `stco` (L2739). `stss` optional —
  **absent ⇒ every sample is a sync sample** (L2749).
- `stsd` ≥1 entry; entry form chosen by handler type (L2779). `SampleEntry` = 6 reserved bytes +
  `data_reference_index`(16) then codec-specific (e.g. `avc1`+`avcC`, `mp4a`+`esds`) (L2831).
  Note: `data_reference_index` is 16-bit so usable entry count ≪ 2³² (L2785).
- **Unrecognized `format` → do not decode the sample description or its samples** (L2787).

## Timing — §8.6.1 (fulltext L2877) — the model the transmux must preserve

- Two clocks per track on the **media timeline** (units = `mdhd.timescale`):
  - **DT** (decode time) from `stts` deltas: `DT(n+1)=DT(n)+stts(n)`, zero origin, deltas
    non-negative; **sum of all deltas = media duration** (L2974, L2980).
  - **CT** (composition/presentation time) = `DT(n) + ctts(n)` (L2887). `ctts` present **only if
    CT≠DT for some sample** (L3043); if all equal, `ctts` **must not** be present (L2889).
- `stts` deltas **strictly positive** except possibly the **last** (may be 0) — no two timestamps
  in a stream may be equal (L2893). Adding a sample may require giving the previously-last sample a
  real duration.
- `ctts` v0 = unsigned offsets (DT<CT); **v1 = signed** offsets (L3009). Each sample has a **unique
  CT** (L3021). With v1, `cslg` (`compositionToDTSShift`, least/greatest delta, start/end) may
  relate the two timelines (L3029, L3134); `cslg` in `stbl` = `moov` samples only, in `trep` =
  all later fragments (L3118).
- B-frame reorder is encoded by `ctts` offsets (Table 2/3 closed/open GOP, L2939/L2950) — the worked
  example is the reference for round-tripping DT/CT.
- **Non-output reference samples**: CT outside the output range + an edit list to exclude them;
  `ctts` v1 with `sample_offset = -2³¹`; `cslg.leastDecodeToDisplayDelta` excludes those (L2911).

## Edit list — §8.6.5/6 (fulltext L3297) — presentation↔media mapping

- `edts`/`elst` maps **presentation timeline → media timeline** (L3303). Absent ⇒ implicit 1:1,
  presentation starts at media start.
- Entry = `segment_duration` (in **mvhd** timescale) + `media_time` (in **media** timescale,
  composition time) + `media_rate` (L3379, L3392).
- `media_time == -1` ⇒ **empty edit** (offsets/delays the start). **Last edit shall never be empty**
  (L3392). `media_rate == 0` ⇒ **dwell** (hold one media point for the duration); else `media_rate==1`.
- **Initial empty edit** = a track start offset; **`segment_duration==0` non-empty edit** = pure
  composition→presentation offset (used in fragmented files / when `ctts` shifts time 0) (L3353).
  Recommended: an edit mapping first-CT→presentation 0 when composition offsets are used (L3361).
- Edits may fall **between** sample times → entering an edit may need back-up to a sync point +
  preroll, and first/last sample slicing (L3331). A transmux that moves samples must preserve or
  recompute `elst` rather than drop it.

## Movie fragments — §8.8 (fulltext L3773) — fMP4 / CMAF write model

- `mvex` (`trex` per track) declares fragment **defaults** (sample_duration/size/flags/desc index)
  (§8.8.3). `moof`→`mfhd`(`sequence_number`, in order, L3786)→`traf` per track.
- **`tfhd`** (§8.8.7, L3940) sets per-fragment defaults + the data anchor. tf_flags (L3960):
  - `0x000001` base-data-offset-present (explicit absolute anchor = a chunk offset, whole-file).
  - `0x000002` sample-description-index-present; `0x000008/10/20` default-sample-duration/size/flags.
  - `0x010000` duration-is-empty (no samples this interval); **error to mix with `moov` edit lists** (L3966).
  - `0x020000` **default-base-is-moof** — anchor = first byte of the enclosing `moof`. Required under
    `iso5`, **shall not** be used with pre-iso5 brands (breaks offset compat) (L3966, L3968).
  - **Anchor fallback** when no base-data-offset & no default-base-is-moof: first `traf` anchors at
    the **start of the enclosing `moof`**, subsequent `traf` at the end of the previous `traf`'s data
    (same data-reference) (L3962).
- **`trun`** (§8.8.8, L3985): a contiguous sample run. tr_flags (L4013):
  - `0x000001` data-offset-present (signed, relative to the `tfhd` base anchor; absent ⇒ run starts
    immediately after previous run / at base anchor, L4005).
  - `0x000004` first-sample-flags-present (overrides flags for first sample only — key+delta pattern;
    mutually exclusive with per-sample flags, L4015).
  - `0x000100/200/400/800` per-sample duration/size/flags/composition-time-offset present.
  - **Field/record sizing is flag-driven** — low byte = which optional fields, second byte = record
    layout (L3997). A serializer computes presence from flags; a parser must not assume a fixed record.
  - `sample_composition_time_offset` signed iff `version==1` (L4032); same signed-offset recommendation
    as `ctts` (L4017).
- **`tfdt`** (§8.8.12, L4146): absolute **base media decode time** of the first sample (decode order)
  of the fragment, on the media timeline — lets a reader seek without summing prior fragments (L4152).
  Shall sit **after `tfhd`, before the first `trun`** (L4162). Decode timeline is pre-edit-list (L4166).
- `mfra`/`tfra`/`mfro` (§8.8.9-11, L4046): optional random-access index, usually at EOF; `mfro` (last,
  fixed) carries `mfra` size for backward scan. **Hint only** — `trun`/`traf`/`trex` must be correct
  regardless (L4082). A transmux that rewrites offsets must rebuild or drop `mfra` (its offsets are
  whole-file).

## Segments & segment index — §8.16 (fulltext L6293) — DASH/CMAF delivery

- A **segment** need not be a compliant file (may lack `moov`) (L6295). `styp` = same layout as
  `ftyp` (box type `styp`) and **shall be first** in a segment if present; may be dropped on
  concatenation (L6317, L6325).
- **`sidx`** (§8.16.3, L6333) = compact time/byte index of one reference stream within a (sub)segment;
  generic enough to also index MPEG-2 TS (L6343). Syntax (L6503): `reference_ID`, `timescale`
  (ticks/s, matches `mdhd` for ISOBMFF), `earliest_presentation_time` + `first_offset` (32 or 64 by
  version), then `reference_count` × { `reference_type`(1) + `referenced_size`(31), `subsegment_duration`(32),
  `starts_with_SAP`(1) + `SAP_type`(3) + `SAP_delta_time`(28) }.
- **Anchor** for offsets = the **first byte after the `sidx` box** (single-file case) (L6413). `sidx`
  shall precede the material it documents (before any `moof` of that subsegment) (L6467).
- `reference_type==1` → points to a child `sidx`; `==0` → media (a `moof`) (L6521). `referenced_size`
  = bytes to the next referenced item. Subsegment durations sum to the parent duration (L6361).
- `earliest_presentation_time` / presentation times are **composition times after edit-list
  application** (movie timeline) (L6481). SAP signalling: type 1/2 = sync sample, type 3 = `rap `
  group, type 4 = `roll` group >0 (L6489). A transmux producing segments computes EPT from the first
  non-edit-omitted access unit and `referenced_size`/`first_offset` from the actual byte layout — never
  copies stale values.

## Code-conformance notes (tracked — NOT yet applied; for the future transmux crate)

1. Box parser: size-driven, skip-unknown (§4.2 L1294); serializer recomputes every `size`/`largesize`
   from contents — no stored-and-echoed length (the workspace no-raw-passthrough invariant).
2. fMP4 writer: `tfhd`/`trun` field presence derived from flags (§8.8.8 L3997); `tfdt` placed
   after `tfhd`, before `trun` (L4162); pick `default-base-is-moof` only with `iso5`+ brands (L3968).
3. Timeline preservation: a transmux that re-times/moves samples must carry or recompute `stts`/`ctts`/
   `elst`/`tfdt` consistently (§8.6.1, §8.6.6) — dropping `elst` or `ctts` corrupts A/V sync.
4. Segment index: rebuild `sidx` (EPT, sizes, offsets, SAP flags) from the produced byte layout;
   rebuild or drop `mfra`/`sidx` after any offset rewrite (§8.16.3 L6413, §8.8.9 L4082).
