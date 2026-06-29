# Fragment-box / media-segment syntax — transmux crate reference

Covers all boxes in a **media segment**: the `styp`-optional + `moof` + `mdat` triplet written
for each segment in fMP4/CMAF/HLS streaming.

Source key:
- **[3GP-26244]** — 3GPP TS 26.244 V19.1.0 (2025-12), §13  
  `https://www.3gpp.org/ftp/Specs/archive/26_series/26.244/26244-j10.zip`
- **[W3C-MSE]** — W3C ISO BMFF Byte Stream Format Group Note (2024-07)  
  `https://www.w3.org/TR/mse-byte-stream-format-isobmff/`

---

## Media segment structure

Source: [W3C-MSE] §media-segment, [3GP-26244] §13.2

```
[styp]          optional — Segment Type Box
moof            Movie Fragment Box
  mfhd          Movie Fragment Header
  traf (×N)     Track Fragment (one per track in this segment)
    tfhd         Track Fragment Header
    tfdt         Track Fragment Decode Time
    trun (×M)    Track Fragment Run (one or more)
mdat (×P)       Media Data Box (one or more; holds the actual coded samples)
```

Constraints from [W3C-MSE]:
- `moof` must contain at least one `traf`.
- Each `traf` must contain exactly one `tfdt` (positioned after `tfhd`, before first `trun`).
- All `moof` boxes must use **movie-fragment relative addressing**: either  
  (a) all `traf` boxes have the `default-base-is-moof` flag set, or  
  (b) exactly one `traf` is present and `base-data-offset-present` flag is NOT set.
- External data references are forbidden in media segments.
- `mdat` box(es) must follow the `moof` they reference.

---

## `styp` — Segment Type Box

Source: [3GP-26244] §13.2

Container: top-level (first box in a segment file, if present).  
Type: basic box (no version/flags) — **same field layout as `ftyp`** with box type `'styp'`.

> "A segment type has the same format as an 'ftyp' box, except that it takes the box type 'styp'."  
> — 3GPP TS 26.244 §13.2

| Field               | Bytes    | Type     | Description                                                         |
|---------------------|----------|----------|---------------------------------------------------------------------|
| `size`              | 4        | u32 BE   | Total size.                                                         |
| `type`              | 4        | fourCC   | `'styp'`                                                            |
| `major_brand`       | 4        | fourCC   | Primary brand for this segment (e.g. `'cmfs'`, `'msdh'`).          |
| `minor_version`     | 4        | u32 BE   | Minor version of the major brand.                                   |
| `compatible_brands` | variable | fourCC[] | Array of compatible brand codes; length = `(size - 16) / 4`.       |

Brands in `styp` should include the brands from the init segment's `ftyp` and may add additional
segment-compatibility brands ([3GP-26244] §13.2).

---

## `mvex` — Movie Extends Box

Source: **ISO/IEC 14496-12 §8.8.1**

Container: `moov`.  
Type: container box (no own fields; contains child boxes).

The Movie Extends Box warns readers that Movie Fragment Boxes may be present in this file.
To know of all samples in the tracks, Movie Fragment Boxes must be found and scanned in order,
and their information logically added to that found in the Movie Box.

Required child:
- One or more `trex` — one per track in the `moov`.

---

## `trex` — Track Extends Box

Source: **ISO/IEC 14496-12 §8.8.3**

Container: `mvex`.  
Type: full box.

Sets up default values used by movie fragments. By setting defaults in this way, space and
complexity can be saved in each Track Fragment Header Box.

| Field                            | Bytes | Type   | Description                                                    |
|----------------------------------|-------|--------|----------------------------------------------------------------|
| `size`                           | 4     | u32 BE | Total size.                                                    |
| `type`                           | 4     | fourCC | `'trex'`                                                       |
| `version`                        | 1     | u8     | Reserved (0).                                                  |
| `flags`                          | 3     | u24 BE | Reserved (0).                                                  |
| `track_id`                       | 4     | u32 BE | Identifies the track; must be the track ID of a track in the `moov`. |
| `default_sample_description_index` | 4  | u32 BE | Default sample description index (1-based into `stsd`). |
| `default_sample_duration`        | 4     | u32 BE | Default sample duration in media timescale units. |
| `default_sample_size`            | 4     | u32 BE | Default sample size in bytes; 0 = variable per-sample. |
| `default_sample_flags`           | 4     | u32 BE | Default sample flags (see `sample_flags` structure below). |

---

## `moof` — Movie Fragment Box

Source: **ISO/IEC 14496-12 §8.8.4**

Container: top-level.  
Type: container box (no own fields; contains child boxes).

Movie fragments extend the presentation in time. They provide the information that would
previously have been in the Movie Box. The actual samples are in Media Data Boxes.

The Movie Fragment Box contains a Movie Fragment Header Box, and then one or more Track Fragment Boxes.

Required children:
- One `mfhd` — movie fragment header
- One or more `traf` — track fragment(s)

---

## `mfhd` — Movie Fragment Header Box

Source: **ISO/IEC 14496-12 §8.8.5**

Container: `moof`.  
Type: full box.

Contains a sequence number as a safety check. The sequence number usually starts at 1 and must
increase for each movie fragment in the file, in the order in which they occur.

| Field             | Bytes | Type   | Description                                                            |
|-------------------|-------|--------|------------------------------------------------------------------------|
| `size`            | 4     | u32 BE | Total size.                                                            |
| `type`            | 4     | fourCC | `'mfhd'`                                                               |
| `version`         | 1     | u8     | Reserved (0).                                                          |
| `flags`           | 3     | u24 BE | Reserved (0).                                                          |
| `sequence_number` | 4     | u32 BE | Ordinal number of this fragment, in increasing order; typically starts at 1. |

---

## `traf` — Track Fragment Box

Source: **ISO/IEC 14496-12 §8.8.6**

Container: `moof`.  
Type: container box (no own fields; contains child boxes).

Within the movie fragment, there is a set of track fragments. The track fragments in turn
contain zero or more track runs, each of which document a contiguous run of samples for that
track. Within these structures, many fields are optional and can be defaulted.

Required children (per W3C-MSE):
- One `tfhd` — track fragment header
- One `tfdt` — track fragment decode time (from [3GP-26244])
- Zero or more `trun` — track fragment run(s)

---

## `tfhd` — Track Fragment Header Box

Source: **ISO/IEC 14496-12 §8.8.7**

Container: `traf`.  
Type: full box.

Each movie fragment can add zero or more fragments to each track. The track fragment header
sets up information and defaults used for those runs of samples.

| Field                            | Bytes | Type    | Description                                                    |
|----------------------------------|-------|---------|----------------------------------------------------------------|
| `size`                           | 4     | u32 BE  | Total size.                                                    |
| `type`                           | 4     | fourCC  | `'tfhd'`                                                       |
| `version`                        | 1     | u8      | Reserved (0).                                                  |
| `flags`                          | 3     | u24 BE  | Track fragment flags (see `tf_flags` table below).            |
| `track_id`                       | 4     | u32 BE  | Identifies the track; must be a track ID from `moov`.         |
| `base_data_offset` (optional)    | 8     | u64 BE  | **Present only if `base-data-offset-present` flag set.** Explicit anchor for data offsets in each track run. If not provided, the base-data-offset for the first track in the movie fragment is the position of the first byte of the enclosing `moof`; for subsequent track fragments, it defaults to the end of data defined by the preceding fragment. |
| `sample_description_index` (opt) | 4     | u32 BE  | **Present only if `sample-description-index-present` flag set.** Overrides the default sample description index from the Track Extends Box. |
| `default_sample_duration` (opt)  | 4     | u32 BE  | **Present only if `default-sample-duration-present` flag set.** Overrides the default from Track Extends Box. |
| `default_sample_size` (opt)      | 4     | u32 BE  | **Present only if `default-sample-size-present` flag set.** Overrides the default from Track Extends Box. |
| `default_sample_flags` (opt)     | 4     | u32 BE  | **Present only if `default-sample-flags-present` flag set.** Overrides the default from Track Extends Box (see `sample_flags` structure below). |

### `tf_flags` — Track Fragment flags

| Bit       | Name                       | Description |
|-----------|----------------------------|-------------|
| 0x000001  | base-data-offset-present   | Indicates presence of `base_data_offset` field. |
| 0x000002  | sample-description-index-present | Indicates presence of `sample_description_index` field. |
| 0x000008  | default-sample-duration-present | Indicates presence of `default_sample_duration` field. |
| 0x000010  | default-sample-size-present | Indicates presence of `default_sample_size` field. |
| 0x000020  | default-sample-flags-present | Indicates presence of `default_sample_flags` field. |
| 0x010000  | duration-is-empty          | Indicates the duration is empty (no samples for this time interval). |

---

## `trun` — Track Fragment Run Box

Source: **ISO/IEC 14496-12 §8.8.8**

Container: `traf`.  
Type: full box.

Documents a contiguous run of samples for a track. A track run documents a contiguous set of
samples for a track. If the `duration-is-empty` flag is set in `tf_flags`, there are no track runs.

| Field                      | Bytes    | Type    | Description                                                    |
|----------------------------|----------|---------|----------------------------------------------------------------|
| `size`                     | 4        | u32 BE  | Total size.                                                    |
| `type`                     | 4        | fourCC  | `'trun'`                                                       |
| `version`                  | 1        | u8      | Reserved (0).                                                  |
| `flags`                    | 3        | u24 BE  | Track run flags (see `tr_flags` table below).                 |
| `sample_count`             | 4        | u32 BE  | Number of samples being added in this run; also the number of rows in the following sample table. |
| `data_offset` (optional)   | 4        | i32 BE  | **Present only if `data-offset-present` flag set.** Added to the implicit or explicit base-data-offset established in the track fragment header. If not present, data for this run starts immediately after data of the previous run (or at base-data-offset if this is the first run). |
| `first_sample_flags` (opt) | 4        | u32 BE  | **Present only if `first-sample-flags-present` flag set.** Provides flags for the first sample only of this run. If this flag and field are used, `sample-flags` shall not be present. |
| **Sample array** `[sample_count]` |   |    | Rows follow, with presence of each column determined by `tr_flags`. All fields in a row are optional. |

### Per-sample array columns (presence determined by flags):

| Column                           | Bytes | Type    | Condition               | Description |
|----------------------------------|-------|---------|-------------------------|-------------|
| `sample_duration`                | 4     | u32 BE  | if flag 0x000100 set    | Duration of this sample in media timescale units. |
| `sample_size`                    | 4     | u32 BE  | if flag 0x000200 set    | Size of this sample in bytes. |
| `sample_flags`                   | 4     | u32 BE  | if flag 0x000400 set    | Flags for this sample (see `sample_flags` structure below). Must not be present if `first-sample-flags-present` is set. |
| `sample_composition_time_offset` | 4     | i32 BE  | if flag 0x000800 set    | Composition time offset (e.g. as used for I/P/B video in MPEG). |

### `tr_flags` — Track Run flags

| Bit       | Name                               | Description |
|-----------|-----------------------------------|-------------|
| 0x000001  | data-offset-present               | Indicates presence of `data_offset` field. |
| 0x000004  | first-sample-flags-present        | Overrides default flags for the first sample only. |
| 0x000100  | sample-duration-present           | Each sample has its own duration; otherwise default is used. |
| 0x000200  | sample-size-present               | Each sample has its own size; otherwise default is used. |
| 0x000400  | sample-flags-present              | Each sample has its own flags; otherwise default is used. Must not be set if `first-sample-flags-present` is set. |
| 0x000800  | sample-composition-time-offsets-present | Each sample has a composition time offset. |

---

## `sample_flags` — 32-bit Sample Flags Structure

Used in: `trex.default_sample_flags`, `tfhd.default_sample_flags`, `trun.first_sample_flags`,
and per-sample `trun.sample_flags`.

| Bit range | Name                        | Size | Description |
|-----------|----------------------------|------|-------------|
| `[31:26]` | reserved                   | 6    | Reserved, set to 0. |
| `[25:24]` | sample_depends_on           | 2    | 0 = unknown, 1 = does depend on others, 2 = does not depend on others, 3 = reserved. |
| `[23:22]` | sample_is_depended_on       | 2    | 0 = unknown, 1 = is not disposable, 2 = is disposable, 3 = reserved. |
| `[21:20]` | sample_has_redundancy       | 2    | 0 = unknown, 1 = has redundancy, 2 = no redundancy, 3 = reserved. |
| `[19:17]` | sample_padding_value        | 3    | Padding bits added at end of sample; 0–7 bits. |
| `[16]`    | sample_is_difference_sample | 1    | When 1: signals a non-key or non-sync sample. When 0: sync/key sample. |
| `[15:0]`  | sample_degradation_priority | 16   | Degradation priority; higher values = more important / greater impact on decoded quality. |

---

## `tfdt` — Track Fragment Base Media Decode Time Box

Source: [3GP-26244] §13.5

Container: `traf` (after `tfhd`, before first `trun`).  
Type: full box.

Provides the absolute decode timestamp of the **first sample** in this track fragment, without
requiring the reader to accumulate all prior sample durations.  Required in every `traf` for
MSE/CMAF compatibility ([W3C-MSE]).

Syntax (from [3GP-26244] §13.5):

```
aligned(8) class TrackFragmentBaseMediaDecodeTimeBox
    extends FullBox('tfdt', version, 0) {
    if (version == 1) {
        unsigned int(64) baseMediaDecodeTime;
    } else {   // version == 0
        unsigned int(32) baseMediaDecodeTime;
    }
}
```

| Field                  | Bytes | Type    | Description                                                             |
|------------------------|-------|---------|-------------------------------------------------------------------------|
| `size`                 | 4     | u32 BE  | Total size.                                                             |
| `type`                 | 4     | fourCC  | `'tfdt'`                                                                |
| `version`              | 1     | u8      | 0 = 32-bit decode time; 1 = 64-bit decode time.                         |
| `flags`                | 3     | u24 BE  | Reserved (0).                                                           |
| `base_media_decode_time` | 4/8 | u32/u64 | Sum of the decode durations of all samples that preceded the first sample in this fragment. In the media's timescale. Does **not** include samples in this fragment. |

**Semantics note ([3GP-26244] §13.5):** The decode timeline is established before any edit-list
mapping.  For a track starting at presentation time 0, `base_media_decode_time` in the first
segment is typically 0 (or a positive encoder-delay offset).

**Version selection:** Use version 1 when `base_media_decode_time` exceeds 32-bit range.  For
90 kHz video timescale, 32 bits wraps at ~13.3 hours; use version 1 for long-form content.

---

## `mdat` — Media Data Box

Source: [QTFF] general atom model; [W3C-MSE]

Container: top-level (follows `moof`).  
Type: basic box.

| Field    | Bytes    | Type    | Description                                                            |
|----------|----------|---------|------------------------------------------------------------------------|
| `size`   | 4        | u32 BE  | Total size including header. Use `largesize` (size=1) if data > 4 GB. |
| `type`   | 4        | fourCC  | `'mdat'`                                                               |
| `data`   | variable | bytes   | Raw coded sample data (NAL units, AAC frames, AV1 OBUs, etc.), laid out as referenced by `trun` entries. |

There is no internal structure defined — the data is addressed externally by `trun` `data_offset`
and per-sample `sample_size` values.  One or more `mdat` boxes may follow a single `moof`
([W3C-MSE]), though a single `mdat` per `moof` is standard practice.
