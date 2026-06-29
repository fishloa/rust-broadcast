# Protection / restriction scheme boxes — transmux crate reference

Covers the box structure for signalling content protection (`sinf`, `frma`, `schm`,
`schi`) and restriction schemes (`rinf`, `stvi`) as defined in the base
ISO/IEC 14496-12:2015 specification.

Source: **ISO/IEC 14496-12:2015** — markdown transcription of the PDF, verified by
the project orchestrator.

**Note:** CENC-specific boxes (`tenc`, `pssh`, `senc`) are defined in
ISO/IEC 23001-7 and are NOT covered here.

---

## `sinf` — Protection Scheme Information Box

Source: ISO/IEC 14496-12:2015 §8.12.1

Container: Protected Sample Entry (e.g. `encv`, `enca`) or Item Protection Box (`ipro`).  
Type: container box (no own fields; contains child boxes).  
Mandatory: Yes (inside a protected sample entry).  
Quantity: One or more.

Contains all information needed to understand the encryption transform applied and
its parameters. Always contains a `frma` box; at least one signalling method must be
used (IPMP descriptors or Scheme signalling with `schm` + `schi`).

```
sinf
  frma   — Original Format Box (mandatory)
  schm   — Scheme Type Box (optional; used together with schi)
  schi   — Scheme Information Box (optional; used together with schm)
```

When more than one `sinf` is present, they are equivalent alternatives.

---

## `frma` — Original Format Box

Source: ISO/IEC 14496-12:2015 §8.12.2

Container: `sinf`, `rinf`, or `cinf`.  
Type: basic box (no version/flags).  
Mandatory: Yes (when used in protected/restricted sample entry).  
Quantity: Exactly one.

Contains the four-character-code of the **original** un-transformed sample entry type
(e.g. `'mp4v'` for protected MPEG-4 visual, `'avc1'` for protected AVC).

| Field         | Bytes | Type    | Description                                                        |
|---------------|-------|---------|--------------------------------------------------------------------|
| `size`        | 4     | u32 BE  | Total size.                                                        |
| `type`        | 4     | fourCC  | `'frma'`                                                           |
| `data_format` | 4     | fourCC  | Original un-transformed sample entry type (e.g. `'mp4v'`, `'avc1'`, `'mp4a'`). |

---

## `schm` — Scheme Type Box

Source: ISO/IEC 14496-12:2015 §8.12.5

Container: `sinf`, `rinf`, or `srpp`.  
Type: full box.  
Mandatory: Zero or one in `sinf`; exactly one in `rinf`/`srpp`.  
Quantity: Zero or one.

Identifies the protection or restriction scheme by fourCC and version.

### Header

| Field           | Bytes | Type      | Description                                                        |
|-----------------|-------|-----------|--------------------------------------------------------------------|
| `size`          | 4     | u32 BE    | Total size.                                                        |
| `type`          | 4     | fourCC    | `'schm'`                                                           |
| `version`       | 1     | u8        | Reserved (0).                                                      |
| `flags`         | 3     | u24 BE    | Bit 0 (`0x000001`): if set, `scheme_uri` field is present.         |
| `scheme_type`   | 4     | u32 BE    | FourCC identifying the scheme (e.g. `'cenc'`, `'cbcs'`, `'stvi'`). |
| `scheme_version`| 4     | u32 BE    | Version of the scheme used when the content was created.           |
| `scheme_uri`    | var   | string    | **Conditional (flags & 1):** Null-terminated UTF-8 URI pointing to scheme info. |

---

## `schi` — Scheme Information Box

Source: ISO/IEC 14496-12:2015 §8.12.6

Container: `sinf`, `rinf`, or `srpp`.  
Type: container box (holds boxes; own fields = header only).  
Mandatory: No (used together with `schm`).  
Quantity: Zero or one.

Acts as a container for scheme-specific sub-boxes (e.g. `tenc`, `pssh` per
ISO/IEC 23001-7). The content is defined entirely by the scheme declared in `schm`.

| Field           | Bytes | Type    | Description                                                        |
|-----------------|-------|---------|--------------------------------------------------------------------|
| `size`          | 4     | u32 BE  | Total size.                                                        |
| `type`          | 4     | fourCC  | `'schi'`                                                           |
| (child boxes)   | var   | Box[]   | Scheme-specific data boxes, format defined by the scheme.          |

---

## `rinf` — Restricted Scheme Information Box

Source: ISO/IEC 14496-12:2015 §8.15.3

Container: Restricted Sample Entry (`'resv'`) or un-restricted Sample Entry.  
Type: container box.  
Mandatory: Yes (inside `'resv'` sample entries).  
Quantity: Exactly one.

Contains information about a restriction scheme applied to the media. Similar
structure to `sinf` but always includes a `schm` box alongside `frma`.

```
rinf
  frma   — Original Format Box (mandatory)
  schm   — Scheme Type Box (mandatory)
  schi   — Scheme Information Box (optional)
```

| Field           | Bytes | Type    | Description                                                        |
|-----------------|-------|---------|--------------------------------------------------------------------|
| `size`          | 4     | u32 BE  | Total size.                                                        |
| `type`          | 4     | fourCC  | `'rinf'`                                                           |
| (child boxes)   | var   | Box[]   | Contains `frma`, `schm`, optionally `schi`.                        |

---

## `stvi` — Stereo Video Box

Source: ISO/IEC 14496-12:2015 §8.15.4

Container: `schi` (inside a restriction scheme where `scheme_type == 'stvi'`).  
Type: full box.  
Mandatory: Yes (when scheme type is `'stvi'`).  
Quantity: One.

Describes stereoscopic video arrangement: frame packing, left/right views, or
3D service-compatible arrangements.

### Header

| Field                     | Bytes | Type      | Description                                                  |
|---------------------------|-------|-----------|--------------------------------------------------------------|
| `size`                    | 4     | u32 BE    | Total size.                                                  |
| `type`                    | 4     | fourCC    | `'stvi'`                                                     |
| `version`                 | 1     | u8        | Reserved (0).                                                |
| `flags`                   | 3     | u24 BE    | Reserved (0).                                                |
| `reserved`                | 4     | u30       | Set to 0 (30-bit reserved field).                            |
| `single_view_allowed`     | (2b)  | u2        | 0 = stereo only; bit 0 = right view OK; bit 1 = left view OK.|
| `stereo_scheme`           | 4     | u32 BE    | 1 = frame packing (H.264 SEI); 2 = MPEG-2 arrangement type; 3 = ISO/IEC 23000-11. |
| `length`                  | 4     | u32 BE    | Byte count of `stereo_indication_type` field.                |
| `stereo_indication_type`  | var   | u8[]      | Scheme-specific data bytes (length = `length`).              |
| `any_box`                 | var   | Box[]     | Optional additional boxes (extension).                       |

The 30-bit `reserved` + 2-bit `single_view_allowed` pack into a single 32-bit word.
