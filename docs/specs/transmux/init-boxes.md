# Init-segment box syntax — transmux crate reference

Covers all boxes that appear in the **init segment** (the `ftyp + moov` blob written once before
any media segments).  Every field table is traced to an ALLOWED free source; no paid ISO standard
was consulted.

Source key used throughout:
- **[QTFF]** — Apple QuickTime File Format specification, markdown API  
  `https://developer.apple.com/tutorials/data/documentation/quicktime-file-format/<Slug>.md`  
  (retrieved 2026-06-29)
- **[3GP-26244]** — 3GPP TS 26.244 V19.1.0 (2025-12), §5.4 / §13  
  `https://www.3gpp.org/ftp/Specs/archive/26_series/26.244/26244-j10.zip`
- **[W3C-MSE]** — W3C ISO BMFF Byte Stream Format Group Note (2024-07)  
  `https://www.w3.org/TR/mse-byte-stream-format-isobmff/`

---

## Box header (universal)

Every ISOBMFF box begins with this layout.  Source: [QTFF] general atom description.

| Field         | Bytes | Type    | Description                                                                |
|---------------|-------|---------|----------------------------------------------------------------------------|
| `size`        | 4     | u32 BE  | Total byte count of this box including header. `0` = extends to EOF. `1` = extended size follows. |
| `type`        | 4     | fourCC  | Box type identifier (ASCII four-character code).                           |
| `largesize`   | 8     | u64 BE  | Present **only** when `size == 1`. Actual total size including all header bytes. |

**Full box** (version + flags prefix, used by most leaf boxes):

| Field     | Bytes | Type   | Description                                      |
|-----------|-------|--------|--------------------------------------------------|
| `size`    | 4     | u32 BE | Total size including header and version/flags.   |
| `type`    | 4     | fourCC | Box type.                                        |
| `version` | 1     | u8     | Box version (usually 0 or 1).                    |
| `flags`   | 3     | u24 BE | Box-specific flags bitfield.                     |

---

## `ftyp` — File Type Box

Source: [QTFF] `File_type_compatibility_atom.md`

Container: top-level (first significant box in file).  
Type: basic box (no version/flags).

The `ftyp` box identifies the file format brand and lists compatible brands.  It must appear
**before** `moov`.  For CMAF/HLS fMP4 output, `major_brand` is typically `'cmf2'` or `'mp42'`
with `'isom'`, `'iso6'`, `'cmfc'` in compatible brands.

| Field               | Bytes    | Type      | Description                                                          |
|---------------------|----------|-----------|----------------------------------------------------------------------|
| `size`              | 4        | u32 BE    | Total size of this box.                                              |
| `type`              | 4        | fourCC    | `'ftyp'`                                                             |
| `major_brand`       | 4        | fourCC    | Primary brand identifier (e.g. `'mp42'`, `'cmf2'`, `'qt  '`).       |
| `minor_version`     | 4        | u32 BE    | Version of the major brand specification.                            |
| `compatible_brands` | variable | fourCC[]  | Array of 4-byte brand codes; length = `(size - 16) / 4` entries.    |

---

## `moov` — Movie Container Box

Source: [QTFF] `Movie_atoms.md`, [3GP-26244] §5.4.10, [W3C-MSE]

Container: top-level.  
Type: container box (no own fields; contains child boxes).

Required children for fragmented output ([W3C-MSE]):
- `mvhd` — movie header
- One or more `trak` — one per elementary stream
- `mvex` — declares the file uses movie fragments

```
moov
  mvhd
  trak (per ES)
    tkhd
    mdia
      mdhd
      hdlr
      minf
        vmhd | smhd
        dinf
          dref
            url
        stbl
          stsd
          stts
          stsc
          stsz
          stco | co64
  mvex
    trex (per trak)
```

---

## `mvhd` — Movie Header Box

Source: [QTFF] `Movie_header_atom.md`

Container: `moov`.  
Type: full box (version + flags).

| Field               | Bytes | Type      | Description                                                                      |
|---------------------|-------|-----------|----------------------------------------------------------------------------------|
| `size`              | 4     | u32 BE    | Total size.                                                                      |
| `type`              | 4     | fourCC    | `'mvhd'`                                                                         |
| `version`           | 1     | u8        | Box version (0 = 32-bit times/duration; 1 = 64-bit).                            |
| `flags`             | 3     | u24 BE    | Reserved (0).                                                                    |
| `creation_time`     | 4/8   | u32/u64   | Creation time in seconds since 1904-01-01 00:00:00 UTC. Size depends on version. |
| `modification_time` | 4/8   | u32/u64   | Last modification time (same epoch). Size depends on version.                    |
| `timescale`         | 4     | u32 BE    | Time scale: ticks per second for all time/duration values in this movie.         |
| `duration`          | 4/8   | u32/u64   | Duration of the entire movie in `timescale` units. Size depends on version.      |
| `preferred_rate`    | 4     | fixed16.16| Playback rate; `0x00010000` = normal (1.0).                                      |
| `preferred_volume`  | 2     | fixed8.8  | Volume; `0x0100` = full volume.                                                  |
| `reserved`          | 10    | bytes     | Reserved, set to zero.                                                           |
| `matrix`            | 36    | fixed     | 3×3 transformation matrix (identity = `[0x10000,0,0,0,0x10000,0,0,0,0x40000000]`). |
| `preview_time`      | 4     | u32 BE    | Start time of the movie preview (QuickTime legacy; set 0 for ISOBMFF).           |
| `preview_duration`  | 4     | u32 BE    | Duration of the movie preview (QuickTime legacy; set 0).                         |
| `poster_time`       | 4     | u32 BE    | Time of the movie poster (QuickTime legacy; set 0).                              |
| `selection_time`    | 4     | u32 BE    | Start time of the current selection (QuickTime legacy; set 0).                   |
| `selection_duration`| 4     | u32 BE    | Duration of the current selection (QuickTime legacy; set 0).                     |
| `current_time`      | 4     | u32 BE    | Current playback position (QuickTime legacy; set 0).                             |
| `next_track_id`     | 4     | u32 BE    | Next available `track_id`; must be > all existing track IDs; never 0.            |

**Note (ISOBMFF fMP4):** Version 0 is typical for most content; use version 1 when `duration`
exceeds 32-bit range (> ~4.3 billion ticks).  The QuickTime-specific preview/poster/current
fields are unused in ISOBMFF output and must be set to zero.

---

## `trak` — Track Container Box

Source: [QTFF] `Track_atoms.md`

Container: `moov`.  
Type: container box.

Contains `tkhd` + `mdia`.  One `trak` per elementary stream (video, audio, subtitle).

---

## `tkhd` — Track Header Box

Source: [QTFF] `Track_header_atom.md`

Container: `trak`.  
Type: full box (version + flags).

| Field               | Bytes | Type      | Description                                                                    |
|---------------------|-------|-----------|--------------------------------------------------------------------------------|
| `size`              | 4     | u32 BE    | Total size.                                                                    |
| `type`              | 4     | fourCC    | `'tkhd'`                                                                       |
| `version`           | 1     | u8        | 0 = 32-bit times; 1 = 64-bit times.                                            |
| `flags`             | 3     | u24 BE    | Bit 0 = track enabled; bit 1 = track in movie; bit 2 = track in preview. Typical: `0x000003`. |
| `creation_time`     | 4/8   | u32/u64   | Creation time since 1904-01-01 UTC.                                            |
| `modification_time` | 4/8   | u32/u64   | Modification time.                                                             |
| `track_id`          | 4     | u32 BE    | Unique 1-based track identifier; must not be 0 or reused.                      |
| `reserved`          | 4     | bytes     | Reserved, set to zero.                                                         |
| `duration`          | 4/8   | u32/u64   | Duration in **movie** timescale units.                                         |
| `reserved_2`        | 8     | bytes     | Reserved, set to zero.                                                         |
| `layer`             | 2     | i16 BE    | Visual ordering; lower = closer to viewer. Normally 0.                         |
| `alternate_group`   | 2     | i16 BE    | Group of alternate tracks (0 = not part of a group).                           |
| `volume`            | 2     | fixed8.8  | Audio volume; `0x0100` = full. Set to 0 for non-audio tracks.                  |
| `reserved_3`        | 2     | bytes     | Reserved, set to zero.                                                         |
| `matrix`            | 36    | fixed     | 3×3 transformation matrix (typically identity).                                |
| `width`             | 4     | fixed16.16| Visual presentation width in pixels. Set to 0 for audio.                       |
| `height`            | 4     | fixed16.16| Visual presentation height in pixels. Set to 0 for audio.                      |

---

## `mdia` — Media Container Box

Source: [QTFF] `Media_atoms.md`

Container: `trak`.  
Type: container box.

Contains: `mdhd`, `hdlr`, `minf`.

---

## `mdhd` — Media Header Box

Source: [QTFF] `Media_header_atom.md`

Container: `mdia`.  
Type: full box (version + flags).

| Field               | Bytes | Type    | Description                                                              |
|---------------------|-------|---------|--------------------------------------------------------------------------|
| `size`              | 4     | u32 BE  | Total size.                                                              |
| `type`              | 4     | fourCC  | `'mdhd'`                                                                 |
| `version`           | 1     | u8      | 0 = 32-bit times; 1 = 64-bit.                                            |
| `flags`             | 3     | u24 BE  | Reserved (0).                                                            |
| `creation_time`     | 4/8   | u32/u64 | Creation time since 1904-01-01 UTC.                                      |
| `modification_time` | 4/8   | u32/u64 | Modification time.                                                       |
| `timescale`         | 4     | u32 BE  | Media time scale: ticks per second for all durations in this media.      |
| `duration`          | 4/8   | u32/u64 | Duration in **media** timescale units.                                   |
| `language`          | 2     | u16 BE  | Packed ISO 639-2/T language code (5-bit chars, bit 15 = 0). `0x0000` = unspecified; `0x55c4` = 'und'. |
| `quality`           | 2     | u16 BE  | Media playback quality (QuickTime; set 0 for ISOBMFF).                  |

---

## `hdlr` — Handler Reference Box

Source: [QTFF] `Handler_reference_atom.md`

Container: `mdia` (or `meta`).  
Type: full box (version + flags).

In ISOBMFF usage, `component_type`, `component_manufacturer`, `component_flags`, and
`component_flags_mask` are all set to zero; only `component_subtype` (ISOBMFF: `handler_type`)
carries meaning.

| Field                      | Bytes    | Type   | Description                                                              |
|----------------------------|----------|--------|--------------------------------------------------------------------------|
| `size`                     | 4        | u32 BE | Total size.                                                              |
| `type`                     | 4        | fourCC | `'hdlr'`                                                                 |
| `version`                  | 1        | u8     | Reserved (0).                                                            |
| `flags`                    | 3        | u24 BE | Reserved (0).                                                            |
| `component_type`           | 4        | fourCC | Reserved (0 in ISOBMFF; `'mhlr'` in QuickTime media tracks).            |
| `component_subtype`        | 4        | fourCC | Handler type: `'vide'` (video), `'soun'` (audio), `'subt'` (subtitle), `'text'` (timed text), `'meta'` (metadata). |
| `component_manufacturer`   | 4        | bytes  | Reserved (0).                                                            |
| `component_flags`          | 4        | u32 BE | Reserved (0).                                                            |
| `component_flags_mask`     | 4        | u32 BE | Reserved (0).                                                            |
| `component_name`           | variable | bytes  | Null-terminated UTF-8 string; human-readable handler description. May be empty (`\0`). |

---

## `minf` — Media Information Container Box

Source: [QTFF] `Video_media_information_atom.md`, `Sound_media_information_atom.md`

Container: `mdia`.  
Type: container box.

For **video** tracks, required children:
- `vmhd` — video media information header
- `dinf` — data information
- `stbl` — sample table

For **audio** tracks:
- `smhd` — sound media information header
- `dinf`
- `stbl`

---

## `vmhd` — Video Media Information Header Box

Source: [QTFF] `Video_media_information_header_atom.md`

Container: `minf` (video tracks only).  
Type: full box.

| Field           | Bytes | Type   | Description                                                         |
|-----------------|-------|--------|---------------------------------------------------------------------|
| `size`          | 4     | u32 BE | Total size.                                                         |
| `type`          | 4     | fourCC | `'vmhd'`                                                            |
| `version`       | 1     | u8     | Reserved (0).                                                       |
| `flags`         | 3     | u24 BE | `0x000001` (QuickTime); set `0x000000` for ISOBMFF.                 |
| `graphics_mode` | 2     | u16 BE | QuickTime transfer mode; `0x0000` = copy. Set 0 for ISOBMFF.        |
| `opcolor`       | 6     | u16×3  | Red, green, blue for graphics mode transfer operation. Set 0.       |

---

## `smhd` — Sound Media Information Header Box

Source: [QTFF] `Sound_media_information_header_atom.md`

Container: `minf` (audio tracks only).  
Type: full box.

| Field      | Bytes | Type    | Description                                               |
|------------|-------|---------|-----------------------------------------------------------|
| `size`     | 4     | u32 BE  | Total size.                                               |
| `type`     | 4     | fourCC  | `'smhd'`                                                  |
| `version`  | 1     | u8      | Reserved (0).                                             |
| `flags`    | 3     | u24 BE  | Reserved (0).                                             |
| `balance`  | 2     | fixed8.8| Stereo balance: `-1.0` = left, `0.0` = centre, `+1.0` = right. Set `0x0000` (centre). |
| `reserved` | 2     | bytes   | Reserved (0).                                             |

---

## `dinf` — Data Information Container Box

Source: [QTFF] `Data_information_atom.md`

Container: `minf`.  
Type: container box (no own fields).

Contains exactly one `dref` box that lists the data reference(s) for this track.

---

## `dref` — Data Reference Box

Source: [QTFF] `Data_information_atom.md` (container description; sub-field page empty)

Container: `dinf`.  
Type: full box.

The `dref` box lists the media data references.  For self-contained files the single entry is a
`url ` box with the `self-contained` flag (`0x000001`) set, meaning the media data is in the same
file.

| Field         | Bytes    | Type   | Description                                                    |
|---------------|----------|--------|----------------------------------------------------------------|
| `size`        | 4        | u32 BE | Total size including all entries.                              |
| `type`        | 4        | fourCC | `'dref'`                                                       |
| `version`     | 1        | u8     | Reserved (0).                                                  |
| `flags`       | 3        | u24 BE | Reserved (0).                                                  |
| `entry_count` | 4        | u32 BE | Number of data reference entries that follow.                  |
| `entries`     | variable | box[]  | Array of data entry boxes (`url ` or `urn `).                  |

### `url ` — Data Entry URL Box (child of `dref`)

| Field     | Bytes    | Type   | Description                                                         |
|-----------|----------|--------|---------------------------------------------------------------------|
| `size`    | 4        | u32 BE | Total size.                                                         |
| `type`    | 4        | fourCC | `'url '` (note trailing space).                                     |
| `version` | 1        | u8     | Reserved (0).                                                       |
| `flags`   | 3        | u24 BE | `0x000001` = self-contained (media data in same file; no URL string follows). |
| `location`| variable | UTF-8  | URL string (null-terminated) **only** when `flags == 0x000000`. Absent for self-contained files. |

---

## `stbl` — Sample Table Container Box

Source: [QTFF] `Sample_table_atom.md`

Container: `minf`.  
Type: container box.

Required children (for non-empty tracks): `stsd`, `stts`, `stsc`, `stsz`, `stco` or `co64`.

| Child box | Purpose                                           |
|-----------|---------------------------------------------------|
| `stsd`    | Sample descriptions (codec config + sample entry) |
| `stts`    | Decoding time-to-sample table                     |
| `ctts`    | Composition offset table (optional)               |
| `stss`    | Sync sample table (optional; if absent all samples are sync) |
| `stsc`    | Sample-to-chunk mapping                           |
| `stsz`    | Per-sample sizes                                  |
| `stco`    | 32-bit chunk offsets                              |
| `co64`    | 64-bit chunk offsets (alternative to `stco`)      |

**Note for fragmented files:** In the `moov` init segment, the sample table boxes in each `trak`
are present but **empty** (all counts set to 0). All actual sample data lives in the `moof`/`mdat`
pairs of media segments ([3GP-26244] §5.4.10, [W3C-MSE]).

---

## `stsd` — Sample Description Box

Source: [QTFF] `Sample_description_atom.md`

Container: `stbl`.  
Type: full box.

| Field                   | Bytes    | Type    | Description                                                     |
|-------------------------|----------|---------|-----------------------------------------------------------------|
| `size`                  | 4        | u32 BE  | Total size.                                                     |
| `type`                  | 4        | fourCC  | `'stsd'`                                                        |
| `version`               | 1        | u8      | Reserved (0).                                                   |
| `flags`                 | 3        | u24 BE  | Reserved (0).                                                   |
| `entry_count`           | 4        | u32 BE  | Number of sample description entries that follow.               |
| `sample_description[]`  | variable | entry[] | Array of sample entries. First 4 fields of each entry are universal (see below). |

### Universal sample entry header fields

| Field               | Bytes | Type    | Description                                            |
|---------------------|-------|---------|--------------------------------------------------------|
| `size`              | 4     | u32 BE  | Size of this sample description entry.                 |
| `data_format`       | 4     | fourCC  | Codec/format code: `'avc1'`, `'hvc1'`, `'mp4a'`, `'av01'`, etc. |
| `reserved`          | 6     | bytes   | Must be zero.                                          |
| `data_reference_index` | 2  | u16 BE  | 1-based index into the `dref` entry list.              |

Codec-specific fields follow the universal header.  See `codec-config.md` for `avc1`/`hvc1`/`mp4a`/`av01`.

---

## `stts` — Decoding Time-to-Sample Box

Source: [QTFF] `Time-to-sample_atom.md`

Container: `stbl`.  
Type: full box.

Maps a sample index to a decode timestamp via a run-length-encoded table of (count, delta) pairs.
Empty (entry\_count = 0) in fragmented-file init segments.

| Field         | Bytes    | Type   | Description                                                          |
|---------------|----------|--------|----------------------------------------------------------------------|
| `size`        | 4        | u32 BE | Total size.                                                          |
| `type`        | 4        | fourCC | `'stts'`                                                             |
| `version`     | 1        | u8     | Reserved (0).                                                        |
| `flags`       | 3        | u24 BE | Reserved (0).                                                        |
| `entry_count` | 4        | u32 BE | Number of entries in the table.                                      |
| `entries`     | 8×N      | struct | Array of `entry_count` entries (see below).                          |

Each entry (8 bytes):

| Sub-field      | Bytes | Type   | Description                                              |
|----------------|-------|--------|----------------------------------------------------------|
| `sample_count` | 4     | u32 BE | Number of consecutive samples with this decode duration. |
| `sample_delta` | 4     | u32 BE | Decode duration of each sample, in media timescale units. |

---

## `stsc` — Sample-to-Chunk Box

Source: [QTFF] `Sample-to-chunk_atom.md`

Container: `stbl`.  
Type: full box.

Maps samples to chunks using a compact run-length table.  Empty in fragmented-file init segments.

| Field         | Bytes    | Type   | Description                                       |
|---------------|----------|--------|---------------------------------------------------|
| `size`        | 4        | u32 BE | Total size.                                       |
| `type`        | 4        | fourCC | `'stsc'`                                          |
| `version`     | 1        | u8     | Reserved (0).                                     |
| `flags`       | 3        | u24 BE | Reserved (0).                                     |
| `entry_count` | 4        | u32 BE | Number of entries in the table.                   |
| `entries`     | 12×N     | struct | Array of `entry_count` entries (see below).       |

Each entry (12 bytes):

| Sub-field               | Bytes | Type   | Description                                                        |
|-------------------------|-------|--------|--------------------------------------------------------------------|
| `first_chunk`           | 4     | u32 BE | 1-based index of the first chunk to which this entry applies.      |
| `samples_per_chunk`     | 4     | u32 BE | Number of samples in each chunk in this run.                       |
| `sample_description_index` | 4 | u32 BE | 1-based index into the `stsd` table for samples in these chunks.  |

---

## `stsz` — Sample Size Box

Source: [QTFF] `Sample_size_atom.md`

Container: `stbl`.  
Type: full box.

If all samples have the same size, `sample_size` is non-zero and the table is omitted.
If `sample_size == 0`, the per-sample table follows.  Empty in fragmented-file init segments.

| Field           | Bytes    | Type   | Description                                                       |
|-----------------|----------|--------|-------------------------------------------------------------------|
| `size`          | 4        | u32 BE | Total size.                                                       |
| `type`          | 4        | fourCC | `'stsz'`                                                          |
| `version`       | 1        | u8     | Reserved (0).                                                     |
| `flags`         | 3        | u24 BE | Reserved (0).                                                     |
| `sample_size`   | 4        | u32 BE | Constant sample size; `0` = variable (per-sample table follows).  |
| `sample_count`  | 4        | u32 BE | Total number of samples in the media.                             |
| `entry_size[]`  | 4×N      | u32 BE | Per-sample sizes; present only when `sample_size == 0`.           |

---

## `stco` — Chunk Offset Box (32-bit)

Source: [QTFF] `Chunk_offset_atom.md`

Container: `stbl`.  
Type: full box.

File offsets of each chunk.  Offsets are **absolute file offsets**, not relative to any container.
Offsets must be updated if the `moov` box is moved.  Use `co64` when any offset exceeds 32 bits.
Empty in fragmented-file init segments.

| Field         | Bytes    | Type   | Description                                      |
|---------------|----------|--------|--------------------------------------------------|
| `size`        | 4        | u32 BE | Total size.                                      |
| `type`        | 4        | fourCC | `'stco'`                                         |
| `version`     | 1        | u8     | Reserved (0).                                    |
| `flags`       | 3        | u24 BE | Reserved (0).                                    |
| `entry_count` | 4        | u32 BE | Number of chunk offsets.                         |
| `chunk_offset[]` | 4×N   | u32 BE | Array of 32-bit absolute file offsets.           |

---

## `co64` — Chunk Offset Box (64-bit)

Source: [QTFF] `Chunk_offset_atom.md` (notes on `co64` variant)

Container: `stbl`.  
Type: full box.  Alternative to `stco` for files where chunk offsets exceed 32-bit range.

| Field            | Bytes | Type   | Description                                      |
|------------------|-------|--------|--------------------------------------------------|
| `size`           | 4     | u32 BE | Total size.                                      |
| `type`           | 4     | fourCC | `'co64'`                                         |
| `version`        | 1     | u8     | Reserved (0).                                    |
| `flags`          | 3     | u24 BE | Reserved (0).                                    |
| `entry_count`    | 4     | u32 BE | Number of chunk offsets.                         |
| `chunk_offset[]` | 8×N   | u64 BE | Array of 64-bit absolute file offsets.           |

---

## `mvex` — Movie Extends Box

Source: [W3C-MSE] §init-segment, [3GP-26244] §5.4.10

Container: `moov`.  
Type: container box.

Presence of `mvex` signals that the file uses movie fragments.  The `moov` box must contain
`mvex` when writing fMP4/CMAF/HLS segments.  `mvex` contains one `trex` per track.

**GAP — field-level syntax defined in ISO/IEC 14496-12 §8.8.1 (paid standard).**  
What is documented here is from the free sources:
- Required in `moov` for fragmented files ([W3C-MSE], [3GP-26244]).
- Must contain one `trex` per track.
- May contain `mehd` (movie extends header, sets overall duration for fragmented files).

---

## `trex` — Track Extends Box

Source: [W3C-MSE] §init-segment (structural requirement)

Container: `mvex`.  
Type: full box.

Sets the default values used by track fragment run boxes (`trun`) when per-sample overrides are
not provided.

**GAP — field-level syntax defined in ISO/IEC 14496-12 §8.8.3 (paid standard).**  
What is documented from free sources:
- One `trex` per track, identified by `track_id`.
- Provides defaults for: sample duration, sample size, sample flags, sample composition time offset.
- Reference implementations: FFmpeg `libavformat/movenc.c` (`mov_write_trex_tag`),
  MP4Box `gpac/src/isomedia/isom_write.c`.
