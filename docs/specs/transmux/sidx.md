# `sidx` — Segment Index Box

Source: **[3GP-26244]** — 3GPP TS 26.244 V19.1.0 (2025-12), §13.4  
`https://www.3gpp.org/ftp/Specs/archive/26_series/26.244/26244-j10.zip`

Container: top-level (segment or file, before the `moof` it indexes).  
Type: full box.

---

## Overview

The Segment Index Box (`sidx`) provides a compact random-access index for one reference track
within a media segment.  It maps time ranges to byte ranges within the segment, optionally
indicating whether each subsegment starts with a Stream Access Point (SAP / random access point).

Key properties ([3GP-26244] §13.4):
- Each `sidx` documents how a (sub)segment is divided into one or more subsegments; each
  subsegment is a contiguous byte range.
- **Anchor point** for a `sidx`: the first byte *after* that `sidx` box.
- `first_offset` gives the byte distance from the anchor to the first referenced box.
- When `reference_type == 0`, the entry points to a `moof` + `mdat` pair (leaf subsegment).
- When `reference_type == 1`, the entry points to a nested `sidx` (hierarchical / daisy-chain).
- The first `sidx` in a segment, for a given track, SHALL document the entirety of that track.
- Subsegments are contiguous in presentation time and contiguous in byte offset.

---

## Syntax

From [3GP-26244] §13.4:

```c
aligned(8) class SegmentIndexBox extends FullBox('sidx', version, 0) {
    unsigned int(32) reference_ID;
    unsigned int(32) timescale;
    if (version == 0) {
        unsigned int(32) earliest_presentation_time;
        unsigned int(32) first_offset;
    } else {
        unsigned int(64) earliest_presentation_time;
        unsigned int(64) first_offset;
    }
    unsigned int(16) reserved = 0;
    unsigned int(16) reference_count;
    for (i = 1; i <= reference_count; i++) {
        bit(1)           reference_type;
        unsigned int(31) referenced_size;
        unsigned int(32) subsegment_duration;
        bit(1)           starts_with_SAP;
        unsigned int(3)  SAP_type;
        unsigned int(28) SAP_delta_time;
    }
}
```

---

## Field table

| Field                      | Bytes    | Type    | Description                                                                |
|----------------------------|----------|---------|----------------------------------------------------------------------------|
| `size`                     | 4        | u32 BE  | Total size of this box.                                                    |
| `type`                     | 4        | fourCC  | `'sidx'`                                                                   |
| `version`                  | 1        | u8      | 0 = 32-bit timestamps; 1 = 64-bit timestamps.                              |
| `flags`                    | 3        | u24 BE  | Reserved (0).                                                              |
| `reference_ID`             | 4        | u32 BE  | `track_id` of the reference track this index covers.  If nested (referenced from a parent `sidx`), value SHALL match the parent's `reference_ID`. |
| `timescale`                | 4        | u32 BE  | Ticks per second for all time/duration fields in this box.  Recommended: match the reference track's `mdhd.timescale`. |
| `earliest_presentation_time` | 4/8   | u32/u64 | Earliest presentation time of any sample of the reference track in the **first** subsegment.  In `timescale` units. |
| `first_offset`             | 4/8      | u32/u64 | Byte distance from the first byte after this box (the anchor) to the first byte of the first referenced box. |
| `reserved`                 | 2        | u16     | Reserved, set to zero.                                                     |
| `reference_count`          | 2        | u16 BE  | Number of reference entries that follow.                                   |

### Per-reference entry (8 bytes each)

| Sub-field             | Bits  | Description                                                                 |
|-----------------------|-------|-----------------------------------------------------------------------------|
| `reference_type`      | 1     | `0` = reference to a `moof` (leaf media subsegment); `1` = reference to a nested `sidx` box. |
| `referenced_size`     | 31    | Byte count of the referenced box (from its first byte to the first byte of the next reference or end of indexed range). |
| `subsegment_duration` | 32    | Duration of the referenced subsegment in `timescale` units.  When `reference_type == 1`, this is the sum of all `subsegment_duration` values in the referenced `sidx`. |
| `starts_with_SAP`     | 1     | `1` = the subsegment starts with a Stream Access Point. See SAP semantics table below. |
| `SAP_type`            | 3     | SAP type (1–6 as defined in [3GP-26244] Annex G.6 via TS 26.247), or `0` = unknown. |
| `SAP_delta_time`      | 28    | When the subsegment contains a SAP, this is `TSAP − earliest_presentation_time` of the subsegment (in `timescale` units).  Zero when no SAP is present or when the SAP starts at the earliest presentation time. |

### SAP semantics table ([3GP-26244] §13.4 Table 13.1)

| `starts_with_SAP` | `SAP_type` | `reference_type` | Meaning                                                                                   |
|:-----------------:|:----------:|:----------------:|-------------------------------------------------------------------------------------------|
| 0                 | 0          | 0 or 1           | No SAP information provided.                                                              |
| 0                 | 1–6        | 0 (media)        | Subsegment contains (but may not start with) a SAP of the given type; `SAP_delta_time` locates it. |
| 0                 | 1–6        | 1 (index)        | All referenced subsegments contain a SAP of at most the given type; no unknown-type SAPs. |
| 1                 | 0          | 0 (media)        | Subsegment starts with a SAP of unknown type.                                             |
| 1                 | 0          | 1 (index)        | All referenced subsegments start with a SAP (possibly of unknown type).                   |
| 1                 | 1–6        | 0 (media)        | Subsegment starts with a SAP of the given type.                                           |
| 1                 | 1–6        | 1 (index)        | All referenced subsegments start with a SAP of at most the given type; no unknown-type SAPs. |

---

## Usage notes for DASH / HLS

- **DASH:** `sidx` is required by most DASH profiles for on-demand content; enables efficient
  byte-range requests.  Place the `sidx` at the start of each segment (after optional `styp`)
  to allow servers to seek without scanning.
- **HLS (CMAF):** `sidx` is optional but recommended for low-latency chunk-level addressing.
- **Multiple tracks:** A segment may have multiple independent top-level `sidx` boxes, one per
  track.  For tracks that are not indexed, the first `sidx` in the segment serves as the byte-
  range reference for all non-indexed tracks ([3GP-26244] §13.4).
- **Hierarchical indexing:** A two-level hierarchy (top-level `sidx` referencing per-chunk
  `sidx` boxes) supports sub-segment random access without reading all chunk-level data upfront.

---

## Validate-by-golden-byte strategy

Since `sidx` byte ranges can be verified independently of the coded media:

1. Mux a known test clip to fMP4 using FFmpeg (`ffmpeg -i input.ts -c copy -movflags frag_keyframe+empty_moov+default_base_moof output.mp4`).
2. Extract the `sidx` with `MP4Box -info output.mp4` or `mp4dump output.mp4`.
3. Use `ffprobe -show_packets output.mp4` to get per-segment byte offsets.
4. Cross-validate: `earliest_presentation_time` in each `sidx` reference should match the PTS
   of the first sample in the corresponding `moof`, and `referenced_size` should match the byte
   distance between consecutive `moof` boxes.
