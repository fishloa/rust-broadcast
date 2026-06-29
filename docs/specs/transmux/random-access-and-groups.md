# Random-access and Sample Group boxes — transmux crate reference

Covers the random-access index boxes at the end of a fragmented file (`mfra`, `tfra`,
`mfro`) and the generic sample-grouping mechanism (`sbgp`, `sgpd`) available in both
init segments and track fragments.

Source: **ISO/IEC 14496-12:2015** — markdown transcription of the PDF, verified by
the project orchestrator.

---

## `mfra` — Movie Fragment Random Access Box

Source: ISO/IEC 14496-12:2015 §8.8.9

Container: File (top-level).  
Type: container box (no own fields; contains child boxes).  
Mandatory: No.  
Quantity: Zero or one.

Provides a table of random-access points for files using movie fragments. Usually
placed at or near the end of the file. Contains one `tfra` per indexed track and
exactly one `mfro` as the last child.

---

## `tfra` — Track Fragment Random Access Box

Source: ISO/IEC 14496-12:2015 §8.8.10

Container: `mfra`.  
Type: full box.  
Mandatory: No.  
Quantity: Zero or one per track.

Each entry locates a sync sample (by `moof` offset + traf/trun/sample number) and its
presentation time. Not every sync sample need be listed. A zero `number_of_entry`
means every sample is a sync sample.

### Header

| Field                      | Bytes | Type     | Description                                                     |
|----------------------------|-------|----------|-----------------------------------------------------------------|
| `size`                     | 4     | u32 BE   | Total size.                                                     |
| `type`                     | 4     | fourCC   | `'tfra'`                                                        |
| `version`                  | 1     | u8       | 0 = 32-bit time/moof_offset; 1 = 64-bit.                        |
| `flags`                    | 3     | u24 BE   | Reserved (0).                                                   |
| `track_ID`                 | 4     | u32 BE   | Identifies the track.                                           |
| `reserved`                 | 4     | u32 BE   | Zero.                                                           |
| `length_size_of_traf_num`  | (2b)  | u2       | Length in bytes of `traf_number` field minus 1 (0..3 from value 0..3). |
| `length_size_of_trun_num`  | (2b)  | u2       | Length in bytes of `trun_number` field minus 1 (ditto).         |
| `length_size_of_sample_num`| (2b)  | u2       | Length in bytes of `sample_number` field minus 1 (ditto).       |
| `number_of_entry`          | 4     | u32 BE   | Number of entries. Zero = all samples are sync samples; no table follows. |

The four bit-fields `reserved`(26) + `length_size_of_traf_num`(2) +
`length_size_of_trun_num`(2) + `length_size_of_sample_num`(2) pack into a single
32-bit word following `track_ID`.

### Per-entry (repeated `number_of_entry` times)

| Field           | Bytes   | Type      | Description                                                    |
|-----------------|---------|-----------|----------------------------------------------------------------|
| `time`          | 4/8     | u32/u64 BE | Presentation time of sync sample, in `mdhd` timescale units. |
| `moof_offset`   | 4/8     | u32/u64 BE | Byte offset from file start to enclosing `moof`.             |
| `traf_number`   | 1..4    | u8..32 BE  | 1-based `traf` index within the `moof`. Value stored in `(length_size_of_traf_num + 1)` bytes. |
| `trun_number`   | 1..4    | u8..32 BE  | 1-based `trun` index within the `traf`. ditto.                |
| `sample_number` | 1..4    | u8..32 BE  | 1-based sample number within the `trun`. ditto.               |

**Size variant:** `version == 0` → 4-byte time/moof_offset; `version == 1` → 8-byte.

---

## `mfro` — Movie Fragment Random Access Offset Box

Source: ISO/IEC 14496-12:2015 §8.8.11

Container: `mfra` (last child).  
Type: full box.  
Mandatory: Yes (inside `mfra`).  
Quantity: Exactly one.

Duplicates the total size of the enclosing `mfra` box as a 32-bit field placed at its
end, enabling file scanners to locate `mfra` by reading backwards from EOF.

| Field           | Bytes | Type    | Description                                                        |
|-----------------|-------|---------|--------------------------------------------------------------------|
| `size`          | 4     | u32 BE  | Total size of this box.                                            |
| `type`          | 4     | fourCC  | `'mfro'`                                                           |
| `version`       | 1     | u8      | Reserved (0).                                                      |
| `flags`         | 3     | u24 BE  | Reserved (0).                                                      |
| `mfra_size`     | 4     | u32 BE  | Total size in bytes of the enclosing `mfra` box.                   |

---

## `sbgp` — Sample to Group Box

Source: ISO/IEC 14496-12:2015 §8.9.2

Container: `stbl` or `traf`.  
Type: full box.  
Mandatory: No.  
Quantity: Zero or more (at most one per `grouping_type` per container).

Assigns each sample to a sample group by run-length encoding. The
`group_description_index` refers to an entry in the corresponding `sgpd` box.

### Header

| Field                     | Bytes | Type      | Description                                                  |
|---------------------------|-------|-----------|--------------------------------------------------------------|
| `size`                    | 4     | u32 BE    | Total size.                                                  |
| `type`                    | 4     | fourCC    | `'sbgp'`                                                     |
| `version`                 | 1     | u8        | 0 = no grouping_type_parameter; 1 = with grouping_type_parameter. |
| `flags`                   | 3     | u24 BE    | Reserved (0).                                                |
| `grouping_type`           | 4     | u32 BE    | Identifies the grouping criterion (e.g. `'roll'`, `'tele'`).  |
| `grouping_type_parameter` | 4     | u32 BE    | **Version 1 only:** Sub-type of the grouping.                |
| `entry_count`             | 4     | u32 BE    | Number of run-length entries.                                |

### Per-entry (repeated `entry_count` times)

| Field                     | Bytes | Type      | Description                                                  |
|---------------------------|-------|-----------|--------------------------------------------------------------|
| `sample_count`            | 4     | u32 BE    | Number of consecutive samples in this group.                 |
| `group_description_index` | 4     | u32 BE    | Index into `sgpd` entries (1-based). `0` = member of no group. |

---

## `sgpd` — Sample Group Description Box

Source: ISO/IEC 14496-12:2015 §8.9.3

Container: `stbl` or `traf`.  
Type: full box.  
Mandatory: No.  
Quantity: Zero or more (one per distinct `grouping_type`).

Describes the properties of each sample group. The actual entry format is defined by
the grouping type (e.g. `'tele'` → temporal layers, `'roll'` → recovery points). This
box provides the abstract container; version 0 entries do not carry a size field and
are deprecated.

### Header

| Field                     | Bytes | Type      | Description                                                  |
|---------------------------|-------|-----------|--------------------------------------------------------------|
| `size`                    | 4     | u32 BE    | Total size.                                                  |
| `type`                    | 4     | fourCC    | `'sgpd'`                                                     |
| `version`                 | 1     | u8        | 0 = no grouping_type_parameter; 1 = with grouping_type_parameter. |
| `flags`                   | 3     | u24 BE    | Reserved (0).                                                |
| `grouping_type`           | 4     | u32 BE    | Identifies the grouping criterion; matches `sbgp.grouping_type`. |
| `grouping_type_parameter` | 4     | u32 BE    | **Version 1 only:** Sub-type of the grouping.                |
| `default_length`          | 4     | u32 BE    | **Version 1 only:** Constant entry length (0 = variable).    |
| `entry_count`             | 4     | u32 BE    | Number of sample group description entries.                  |

### Per-entry (repeated `entry_count` times)

| Field                     | Bytes     | Type      | Description                                                  |
|---------------------------|-----------|-----------|--------------------------------------------------------------|
| `description_length`      | 4         | u32 BE    | **Version 1 only, conditional:** Present when `default_length == 0`. Length of this entry in bytes. |
| `entry_data`              | variable  | bytes     | Group-description payload, defined by the grouping type.     |

**Note:** Version 0 entries have no length prefix and must be self-delimiting or
fixed-size (deprecated).
