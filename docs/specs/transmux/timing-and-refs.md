# Timing and track-reference boxes — transmux crate reference

Covers the Producer Reference Time Box (`prft`) and Track Reference Box (`tref`).

Source: **ISO/IEC 14496-12:2015** — markdown transcription of the PDF, verified by
the project orchestrator.

---

## `prft` — Producer Reference Time Box

Source: ISO/IEC 14496-12:2015 §8.16.5

Container: File (top-level).  
Type: full box.  
Mandatory: No.  
Quantity: Zero or more (at most one per movie fragment).

Supplies relative wall-clock times (NTP-synchronised) at which a movie fragment was
produced, enabling clients to synchronise consumption with production rates.

The box must follow any `styp`/`sidx` in the segment and occur before the movie
fragment to which it refers.

| Field                | Bytes | Type      | Description                                                    |
|----------------------|-------|-----------|----------------------------------------------------------------|
| `size`               | 4     | u32 BE    | Total size.                                                    |
| `type`               | 4     | fourCC    | `'prft'`                                                       |
| `version`            | 1     | u8        | 0 = 32-bit media_time; 1 = 64-bit.                             |
| `flags`              | 3     | u24 BE    | Reserved (0).                                                  |
| `reference_track_ID` | 4     | u32 BE    | Track ID of the reference track for this timestamp.            |
| `ntp_timestamp`      | 8     | u64 BE    | UTC time in NTP format (seconds + fraction, RFC 5905).         |
| `media_time`         | 4/8   | u32/u64 BE | Corresponding media time in the reference track's timescale. Version 0: 32-bit; version 1: 64-bit. |

---

## `tref` — Track Reference Box

Source: ISO/IEC 14496-12:2015 §8.3.3

Container: `trak`.  
Type: container box (no own fields; contains child `TrackReferenceTypeBox` boxes).  
Mandatory: No.  
Quantity: Zero or one.

Provides typed references from the containing track to other tracks in the
presentation. The container holds zero or more `TrackReferenceTypeBox` sub-boxes,
one per reference type.

### Child: `TrackReferenceTypeBox` (one sub-box per reference type)

Each sub-box uses the reference type fourCC as its box type.

| Field        | Bytes    | Type      | Description                                                    |
|--------------|----------|-----------|----------------------------------------------------------------|
| `size`       | 4        | u32 BE    | Total size.                                                    |
| `type`       | 4        | fourCC    | Reference type (e.g. `'hint'`, `'cdsc'`, `'hind'`, `'subt'`). |

### Per-reference payload (track_ID array)

| Field         | Bytes    | Type      | Description                                                    |
|---------------|----------|-----------|----------------------------------------------------------------|
| `track_IDs[]` | variable | u32 BE[]  | Array of track IDs being referenced (one or more). Padding to fill the box size. |

### Standard reference types (non-exhaustive)

| Type       | Meaning                                                      |
|------------|--------------------------------------------------------------|
| `'hint'`   | Referenced tracks contain original media for this hint track. |
| `'cdsc'`   | This track describes (contains metadata for) the referenced track. |
| `'font'`   | This track uses fonts carried/defined in the referenced track. |
| `'hind'`   | This track depends on the referenced hint track.              |
| `'vdep'`   | This track contains auxiliary depth video for the referenced track. |
| `'vplx'`   | This track contains auxiliary parallax video for the referenced track. |
| `'subt'`   | This track contains subtitle/timed-text for the referenced track or its alternate group. |
