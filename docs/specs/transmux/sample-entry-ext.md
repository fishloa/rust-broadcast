# Sample-entry extension boxes — transmux crate reference

Covers optional boxes that appear inside a `VisualSampleEntry` (inside `stsd`) to
describe pixel aspect ratio, clean aperture, and colour information.

Source: **ISO/IEC 14496-12:2015** — markdown transcription of the PDF, verified by
the project orchestrator.

---

## `pasp` — Pixel Aspect Ratio Box

Source: ISO/IEC 14496-12:2015 §12.1.4

Container: `VisualSampleEntry` (inside `stsd`).  
Type: basic box (no version/flags).  
Mandatory: No.  
Quantity: Zero or one.

Overrides pixel aspect ratio declarations from codec-specific structures. Only the
ratio `hSpacing : vSpacing` matters; units are arbitrary.

| Field       | Bytes | Type    | Description                                                        |
|-------------|-------|---------|--------------------------------------------------------------------|
| `size`      | 4     | u32 BE  | Total size.                                                        |
| `type`      | 4     | fourCC  | `'pasp'`                                                           |
| `hSpacing`  | 4     | u32 BE  | Horizontal spacing (arbitrary units; must be positive).            |
| `vSpacing`  | 4     | u32 BE  | Vertical spacing (same units; must be positive).                   |

---

## `clap` — Clean Aperture Box

Source: ISO/IEC 14496-12:2015 §12.1.4

Container: `VisualSampleEntry` (inside `stsd`).  
Type: basic box (no version/flags).  
Mandatory: No.  
Quantity: Zero or one.

Describes the clean aperture of the video — the intended display region. Each
parameter is stored as a fraction `N/D` (two 32-bit unsigned integers).

| Field                   | Bytes | Type    | Description                                                     |
|-------------------------|-------|---------|-----------------------------------------------------------------|
| `size`                  | 4     | u32 BE  | Total size.                                                     |
| `type`                  | 4     | fourCC  | `'clap'`                                                        |
| `cleanApertureWidthN`   | 4     | u32 BE  | Numerator of clean aperture width in pixels.                    |
| `cleanApertureWidthD`   | 4     | u32 BE  | Denominator of clean aperture width (must be positive).         |
| `cleanApertureHeightN`  | 4     | u32 BE  | Numerator of clean aperture height in pixels.                   |
| `cleanApertureHeightD`  | 4     | u32 BE  | Denominator of clean aperture height (must be positive).        |
| `horizOffN`             | 4     | u32 BE  | Numerator of horizontal centre offset (may be negative as i32). |
| `horizOffD`             | 4     | u32 BE  | Denominator of horizontal offset (must be positive).            |
| `vertOffN`              | 4     | u32 BE  | Numerator of vertical centre offset (may be negative as i32).   |
| `vertOffD`              | 4     | u32 BE  | Denominator of vertical offset (must be positive).              |

---

## `colr` — Colour Information Box

Source: ISO/IEC 14496-12:2015 §12.1.5

Container: `VisualSampleEntry` (inside `stsd`).  
Type: basic box (no version/flags).  
Mandatory: No.  
Quantity: Zero or more (multiple instances, most accurate first).

Contains colour property information for the video stream. Two types are defined:
ICC profiles (binary) and nclx colour parameters.

| Field           | Bytes    | Type      | Description                                                    |
|-----------------|----------|-----------|----------------------------------------------------------------|
| `size`          | 4        | u32 BE    | Total size.                                                    |
| `type`          | 4        | fourCC    | `'colr'`                                                       |
| `colour_type`   | 4        | fourCC    | `'nclx'` (NCLX colour primaries) or `'rICC'`/`'prof'` (ICC profile). |
| `colour_primary`| 2        | u16 BE    | **If colour_type == 'nclx':** Colour primaries (ITU-T H.273).  |
| `transfer_characteristics` | 2 | u16 BE    | **If colour_type == 'nclx':** Transfer characteristics (ITU-T H.273). |
| `matrix_coefficients` | 2   | u16 BE    | **If colour_type == 'nclx':** Matrix coefficients (ITU-T H.273). |
| `full_range_flag` | 1       | u8        | **If colour_type == 'nclx':** Bit 7: 1 = full range; bits [6:0] = 0. |
| `icc_profile`    | variable | u8[]      | **If colour_type == 'rICC'/'prof':** The ICC colour profile binary data. |

**Note:** The `colr` box syntax in the ISO 14496-12:2015 source cited the full
field table for the `nclx` variant and the ICC profile binary data variant. The
exact bit-width of `full_range_flag` is 7 reserved bits + 1 flag bit, stored as a
single byte.
