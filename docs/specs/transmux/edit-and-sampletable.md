# Edit-list and additional Sample Table boxes — transmux crate reference

Covers boxes appearing in the Edit Box (`edts`) and additional Sample Table Box (`stbl`)
children beyond the basics (`stts`, `stsc`, `stsz`, `stco`, `co64`).

Source: **ISO/IEC 14496-12:2015** — markdown transcription of the PDF, verified by
the project orchestrator.

---

## `edts` — Edit Box

Source: ISO/IEC 14496-12:2015 §8.6.5

Container: `trak`.  
Type: container box (no own fields; contains child boxes).  
Mandatory: No.  
Quantity: Zero or one.

The Edit Box maps the presentation timeline to the media timeline. In its absence, a
one-to-one mapping is implied. Contains exactly one `elst` sub-box.

---

## `elst` — Edit List Box

Source: ISO/IEC 14496-12:2015 §8.6.6

Container: `edts`.  
Type: full box.  
Mandatory: No.  
Quantity: Zero or one.

Contains an explicit list of edit segments that map the media timeline to the
presentation timeline. Each edit may define a normal playback segment, an empty
segment (offset), or a dwell (freeze-frame).

### Header

| Field           | Bytes | Type    | Description                                                        |
|-----------------|-------|---------|--------------------------------------------------------------------|
| `size`          | 4     | u32 BE  | Total size.                                                        |
| `type`          | 4     | fourCC  | `'elst'`                                                           |
| `version`       | 1     | u8      | 0 = 32-bit segment_duration/media_time; 1 = 64-bit.                |
| `flags`         | 3     | u24 BE  | Reserved (0).                                                      |
| `entry_count`   | 4     | u32 BE  | Number of edit entries.                                            |

### Per-entry (repeated `entry_count` times)

| Field                | Bytes | Type      | Description                                                         |
|----------------------|-------|-----------|---------------------------------------------------------------------|
| `segment_duration`   | 4/8   | u32/u64   | Duration of this edit segment in **movie** timescale units.         |
| `media_time`         | 4/8   | i32/i64   | Starting media time in media timescale (composition time). `-1` = empty edit. |
| `media_rate_integer` | 2     | i16 BE    | Relative playback rate. `0` = dwell; otherwise `1`.                 |
| `media_rate_fraction`| 2     | i16 BE    | Fractional part of rate (should be 0).                              |

**Size variant:** `version == 0` → 4-byte fields; `version == 1` → 8-byte fields.

---

## `ctts` — Composition Time to Sample Box

Source: ISO/IEC 14496-12:2015 §8.6.1.3

Container: `stbl`.  
Type: full box.  
Mandatory: No.  
Quantity: Zero or one.

Provides offsets between decoding time (DT) and composition time (CT) per sample:
`CT(n) = DT(n) + CTTS(n)`. Must be present only when DT and CT differ for any
sample. Version 0 uses unsigned offsets (DT ≤ CT); version 1 uses signed offsets.

### Header

| Field           | Bytes | Type    | Description                                                        |
|-----------------|-------|---------|--------------------------------------------------------------------|
| `size`          | 4     | u32 BE  | Total size.                                                        |
| `type`          | 4     | fourCC  | `'ctts'`                                                           |
| `version`       | 1     | u8      | 0 = unsigned sample_offset; 1 = signed.                            |
| `flags`         | 3     | u24 BE  | Reserved (0).                                                      |
| `entry_count`   | 4     | u32 BE  | Number of run-length encoded entries.                              |

### Per-entry (repeated `entry_count` times)

| Field           | Bytes | Type      | Description                                                    |
|-----------------|-------|-----------|----------------------------------------------------------------|
| `sample_count`  | 4     | u32 BE    | Number of consecutive samples with this offset.                |
| `sample_offset` | 4     | u32/i32 BE | Offset = CT − DT. Version 0: unsigned; version 1: signed.     |

---

## `stss` — Sync Sample Box

Source: ISO/IEC 14496-12:2015 §8.6.2

Container: `stbl`.  
Type: full box.  
Mandatory: No.  
Quantity: Zero or one.

Lists sample numbers that are sync samples (key frames). If absent, every sample is a
sync sample.

### Header

| Field           | Bytes | Type    | Description                                                        |
|-----------------|-------|---------|--------------------------------------------------------------------|
| `size`          | 4     | u32 BE  | Total size.                                                        |
| `type`          | 4     | fourCC  | `'stss'`                                                           |
| `version`       | 1     | u8      | Reserved (0).                                                      |
| `flags`         | 3     | u24 BE  | Reserved (0).                                                      |
| `entry_count`   | 4     | u32 BE  | Number of sync sample entries. Zero = no sync samples.             |

### Per-entry (repeated `entry_count` times)

| Field            | Bytes | Type    | Description                                                    |
|------------------|-------|---------|----------------------------------------------------------------|
| `sample_number`  | 4     | u32 BE  | 1-based sample number of a sync sample (strictly increasing).  |

---

## `sdtp` — Independent and Disposable Samples Box

Source: ISO/IEC 14496-12:2015 §8.6.4

Container: `stbl`.  
Type: full box.  
Mandatory: No.  
Quantity: Zero or one.

Provides per-sample dependency flags: whether a sample depends on others (I/P/B),
whether others depend on it (disposable), whether it has redundant codings, and
whether it is a "leading" sample (B-frame preceding the next I-frame).

### Header

| Field           | Bytes | Type    | Description                                                        |
|-----------------|-------|---------|--------------------------------------------------------------------|
| `size`          | 4     | u32 BE  | Total size.                                                        |
| `type`          | 4     | fourCC  | `'sdtp'`                                                           |
| `version`       | 1     | u8      | Reserved (0).                                                      |
| `flags`         | 3     | u24 BE  | Reserved (0).                                                      |

### Per-sample (repeated for `sample_count`, from `stsz`)

Each byte contains four 2-bit fields:

| Bit range | Field                     | Values                                             |
|-----------|---------------------------|----------------------------------------------------|
| `[7:6]`   | `is_leading`              | 0 = unknown; 1 = leading (depends before ref); 2 = not leading; 3 = leading (decodable). |
| `[5:4]`   | `sample_depends_on`       | 0 = unknown; 1 = depends on others; 2 = does not depend (I picture); 3 = reserved. |
| `[3:2]`   | `sample_is_depended_on`   | 0 = unknown; 1 = others may depend; 2 = no other depends (disposable); 3 = reserved. |
| `[1:0]`   | `sample_has_redundancy`   | 0 = unknown; 1 = has redundant coding; 2 = no redundant coding; 3 = reserved. |

**Note:** Number of entries equals `sample_count` from `stsz`. No per-sample count
field is stored in `sdtp` itself.

---

## `subs` — Sub-Sample Information Box

Source: ISO/IEC 14496-12:2015 §8.7.7

Container: `stbl` or `traf`.  
Type: full box.  
Mandatory: No.  
Quantity: Zero or more (multiple instances distinguished by `flags`).

Describes sub-sample structure: a contiguous range of bytes within a sample. Sparsely
coded — only entries with `subsample_count > 0` are listed.

### Header

| Field           | Bytes | Type    | Description                                                        |
|-----------------|-------|---------|--------------------------------------------------------------------|
| `size`          | 4     | u32 BE  | Total size.                                                        |
| `type`          | 4     | fourCC  | `'subs'`                                                           |
| `version`       | 1     | u8      | 0 = 16-bit subsample_size; 1 = 32-bit.                             |
| `flags`         | 3     | u24 BE  | Semantics defined per coding system; 0 if none.                    |
| `entry_count`   | 4     | u32 BE  | Number of entries with sub-sample info.                            |

### Per-entry (repeated `entry_count` times)

| Field             | Bytes | Type      | Description                                                      |
|-------------------|-------|-----------|------------------------------------------------------------------|
| `sample_delta`    | 4     | u32 BE    | Sample number delta from previous entry (first entry = from 0).  |
| `subsample_count` | 2     | u16 BE    | Number of sub-samples for this sample. 0 = none; no array follows. |

### Per-sub-sample (repeated `subsample_count` times, only if > 0)

| Field                     | Bytes | Type      | Description                                                    |
|---------------------------|-------|-----------|----------------------------------------------------------------|
| `subsample_size`          | 2/4   | u16/u32 BE | Size in bytes. Version 0: 16-bit; version 1: 32-bit.         |
| `subsample_priority`      | 1     | u8        | Degradation priority (higher = more important).                |
| `discardable`             | 1     | u8        | 0 = required; 1 = enhancement (e.g. SEI).                     |
| `codec_specific_parameters` | 4   | u32 BE    | Codec-defined; set to 0 if not used.                           |

---

## `saiz` — Sample Auxiliary Information Sizes Box

Source: ISO/IEC 14496-12:2015 §8.7.8

Container: `stbl` or `traf`.  
Type: full box.  
Mandatory: No.  
Quantity: Zero or more (paired with `saio` by `aux_info_type`).

Provides per-sample sizes for auxiliary information (e.g. encryption metadata).
Multiple instances allowed for different `aux_info_type` values.

### Header

| Field                    | Bytes | Type      | Description                                                  |
|--------------------------|-------|-----------|--------------------------------------------------------------|
| `size`                   | 4     | u32 BE    | Total size.                                                  |
| `type`                   | 4     | fourCC    | `'saiz'`                                                     |
| `version`                | 1     | u8        | Reserved (0).                                                |
| `flags`                  | 3     | u24 BE    | Bit 0: if set, `aux_info_type` and `aux_info_type_parameter` fields are present. |
| `aux_info_type`          | 4     | u32 BE    | **Conditional (flags & 1):** Identifies the auxiliary info type. |
| `aux_info_type_parameter`| 4     | u32 BE    | **Conditional (flags & 1):** Stream identifier within the type. |
| `default_sample_info_size`| 1    | u8        | Constant size if all samples match; 0 = variable sizes follow. |
| `sample_count`           | 4     | u32 BE    | Number of samples (must match sample count from `stsz` or `trun` sum). |

### Per-sample (only if `default_sample_info_size == 0`)

| Field                 | Bytes | Type | Description                                              |
|-----------------------|-------|------|----------------------------------------------------------|
| `sample_info_size[i]` | 1     | u8   | Size of auxiliary info for sample i, in bytes. Zero = no info. |

---

## `saio` — Sample Auxiliary Information Offsets Box

Source: ISO/IEC 14496-12:2015 §8.7.9

Container: `stbl` or `traf`.  
Type: full box.  
Mandatory: No.  
Quantity: Zero or more (paired with `saiz` by `aux_info_type`).

### Header

| Field                    | Bytes | Type      | Description                                                  |
|--------------------------|-------|-----------|--------------------------------------------------------------|
| `size`                   | 4     | u32 BE    | Total size.                                                  |
| `type`                   | 4     | fourCC    | `'saio'`                                                     |
| `version`                | 1     | u8        | 0 = 32-bit offsets; 1 = 64-bit offsets.                      |
| `flags`                  | 3     | u24 BE    | Bit 0: if set, `aux_info_type` and `aux_info_type_parameter` fields are present. |
| `aux_info_type`          | 4     | u32 BE    | **Conditional (flags & 1):** Identifies the auxiliary info type. |
| `aux_info_type_parameter`| 4     | u32 BE    | **Conditional (flags & 1):** Stream identifier within the type. |
| `entry_count`            | 4     | u32 BE    | 1 = contiguous aux info; or = number of chunks/runs.         |

### Per-offset (repeated `entry_count` times)

| Field     | Bytes | Type      | Description                                                    |
|-----------|-------|-----------|----------------------------------------------------------------|
| `offset`  | 4/8   | u32/u64 BE | File position. In `stbl`: absolute. In `traf`: relative to base offset from `tfhd`. |

---

## `btrt` — Bit Rate Box

Source: ISO/IEC 14496-12:2015 §8.5.2 (class `BitRateBox`)

Container: `SampleEntry` (used inside `stsd` sample entry boxes like `avc1`, `mp4a`).  
Type: basic box (no version/flags).  
Mandatory: No.  
Quantity: Zero or one.

Provides buffer size and bitrate information for the elementary stream.

| Field           | Bytes | Type    | Description                                                        |
|-----------------|-------|---------|--------------------------------------------------------------------|
| `size`          | 4     | u32 BE  | Total size.                                                        |
| `type`          | 4     | fourCC  | `'btrt'`                                                           |
| `bufferSizeDB`  | 4     | u32 BE  | Decoding buffer size in bytes.                                     |
| `maxBitrate`    | 4     | u32 BE  | Maximum rate in bits/sec over any 1-second window.                 |
| `avgBitrate`    | 4     | u32 BE  | Average rate in bits/sec over the entire presentation.             |
