# VUI colour signalling: syntax and Tables E-2..E-5 / E.2..E.5

Source of truth for `transmux::ir::display`'s colour types
(`VideoFormat`, `ColourPrimaries`, `TransferCharacteristics`,
`MatrixCoefficients`, `ColourDescription`, `VideoSignalType`). Used by
`DisplayGeometry::from_avc_sps` / `from_hevc_sps` via the VUI walks in
`transmux/src/sps.rs`.

Every table below was read off the **rendered PDF page** (`pdftoppm -r 90`),
not only the `private/specs-md` markdown, because that markdown is badly
mangled for these tables (see "Markdown garbling" at the end). The row
count is stated per table so a re-verification is cheap.

| Spec | File (`private/specs/`) | PDF pages checked |
|---|---|---|
| ITU-T H.264 (V15, 08/2024) | `itu_t_h264_avc.pdf` | 453 (printed 429): §E.1.1 syntax; 456 (432): §E.2.1, Table E-2; 457-458 (433-434): Table E-3; 458-460 (434-436): Table E-4; 464-466 (440-442): §E.2.1 `matrix_coefficients`, Table E-5 |
| ITU-T H.265 (V11, 01/2026) | `itu_t_h265_hevc.pdf` | 445 (427): §E.2.1 syntax; 450 (432): Table E.2; 451-452 (433-434): Table E.3; 452-453 (434-435): Table E.4; 459-460 (441-442): Table E.5 |

## Syntax (H.264 §E.1.1 PDF p453; H.265 §E.2.1 PDF p445)

Identical in both, and it sits **after** `aspect_ratio_info_present_flag` and
**before** `chroma_loc_info_present_flag`:

```
overscan_info_present_flag         u(1)
if( overscan_info_present_flag )
  overscan_appropriate_flag        u(1)
video_signal_type_present_flag     u(1)
if( video_signal_type_present_flag ) {
  video_format                     u(3)
  video_full_range_flag            u(1)
  colour_description_present_flag  u(1)
  if( colour_description_present_flag ) {
    colour_primaries               u(8)
    transfer_characteristics       u(8)
    matrix_coefficients            u(8)     // H.265 spells it matrix_coeffs
  }
}
```

Inference when `video_signal_type_present_flag` is 0 (H.264 §E.2.1 p456,
H.265 p450): `video_format` 5, `video_full_range_flag` 0,
`colour_primaries` 2, `transfer_characteristics` 2, `matrix_coefficients` 2.
Reserved values in H.265 "shall be interpreted as ... equivalent to the value
2" (H.265 §E.3.1 p450); H.264 says the same for
`transfer_characteristics` only (p458). `transmux` records what was
**signalled** and exposes the inferred defaults through accessors.

## Table E-2 (H.264, p456) / Table E.2 (H.265, p450): `video_format`

8 rows in both. **The specs differ**: H.264 defines 6 and 7 as `Reserved`
(decoders do not prescribe an interpretation); H.265 marks the same two rows
reserved "for future use" and says decoders "shall interpret the values 6 and
7 ... as equivalent to the value 5". `transmux` exposes 0..=6 as named rows
and 7 as `Reserved(7)` (see the type's doc for the reason).

| video_format | Meaning (H.264 Table E-2) | Meaning (H.265 Table E.2) |
|---|---|---|
| 0 | Component | Component |
| 1 | PAL | PAL |
| 2 | NTSC | NTSC |
| 3 | SECAM | SECAM |
| 4 | MAC | MAC |
| 5 | Unspecified video format | Unspecified video format |
| 6 | Reserved | Reserved (interpret as 5) |
| 7 | Reserved | Reserved (interpret as 5) |

## Table E-3 (H.264 pp457-458) / Table E.3 (H.265 pp451-452): `colour_primaries`

**Identical rows in both specs** (13 defined rows + 3 reserved ranges).

| Value | Primaries | Informative remark |
|---|---|---|
| 0 | Reserved | For future use by ITU-T \| ISO/IEC |
| 1 | Rec. ITU-R BT.709-6 | also BT.1361-0 conventional gamut (historical), IEC 61966-2-1 sRGB or sYCC, IEC 61966-2-4, SMPTE RP 177 (1993) Annex B |
| 2 | Unspecified | Image characteristics are unknown or are determined by the application |
| 3 | Reserved | For future use by ITU-T \| ISO/IEC |
| 4 | Rec. ITU-R BT.470-7 System M (historical) | NTSC 1953, FCC Title 47 CFR (2016) 73.682 (a) (20) |
| 5 | Rec. ITU-R BT.470-7 System B, G (historical) | BT.601-7 625, BT.1358-0 625 (historical), BT.1700-0 625 PAL and 625 SECAM |
| 6 | Rec. ITU-R BT.601-7 525 | BT.1358-1 525 or 625 (historical), BT.1700-0 NTSC, SMPTE 170M (2004) (functionally the same as the value 7) |
| 7 | SMPTE 240M (1999, historical) | (functionally the same as the value 6) |
| 8 | Generic film (colour filters using Illuminant C) | Wratten 58/47/25 |
| 9 | Rec. ITU-R BT.2020-2 | Rec. ITU-R BT.2100-2 |
| 10 | SMPTE ST 428-1 (2006) | (CIE 1931 XYZ as in ISO 11664-1) |
| 11 | SMPTE RP 431-2 (2011) | SMPTE ST 2113 (2018) "P3DCI" |
| 12 | SMPTE EG 432-1 (2010) | SMPTE ST 2113 (2018) "P3D65" |
| 13..21 | Reserved | For future use by ITU-T \| ISO/IEC |
| 22 | EBU Tech. 3213-E (1975) |  |
| 23..255 | Reserved | For future use by ITU-T \| ISO/IEC |

## Table E-4 (H.264 pp458-460) / Table E.4 (H.265 pp452-453): `transfer_characteristics`

**Identical rows in both specs** (19 defined rows 0..=18; the bytes are
`Reserved` 19..=255). Only the value and the characteristic are reproduced;
the transfer functions are not used by `transmux`.

| Value | Transfer characteristic | Informative remark |
|---|---|---|
| 0 | Reserved | For future use by ITU-T \| ISO/IEC |
| 1 | V = a * Lc^0.45 - (a - 1), V = 4.500 * Lc | Rec. ITU-R BT.709-6; BT.1361-0 conventional gamut (historical) (functionally the same as the values 6, 14, and 15) |
| 2 | Unspecified | Image characteristics are unknown or are determined by the application |
| 3 | Reserved | For future use by ITU-T \| ISO/IEC |
| 4 | Assumed display gamma 2.2 | BT.470-7 System M (historical), NTSC 1953, FCC 73.682 (a) (20), BT.1700-0 625 PAL and 625 SECAM |
| 5 | Assumed display gamma 2.8 | BT.470-7 System B, G (historical) |
| 6 | V = a * Lc^0.45 - (a - 1), V = 4.500 * Lc | BT.601-7 525 or 625, BT.1358-1 525 or 625 (historical), BT.1700-0 NTSC, SMPTE 170M (2004) (functionally the same as the value 1) |
| 7 | V = a * Lc^0.45 - (a - 1), V = 4.0 * Lc | SMPTE 240M (1999, historical) |
| 8 | V = Lc | Linear transfer characteristics |
| 9 | V = 1.0 + Log10(Lc) / 2, V = 0.0 | Logarithmic transfer characteristic (100:1 range) |
| 10 | V = 1.0 + Log10(Lc) / 2.5, V = 0.0 | Logarithmic transfer characteristic (100 * Sqrt(10) : 1 range) |
| 11 | piecewise a * Lc^0.45 form, signed | IEC 61966-2-4 |
| 12 | piecewise BT.1361 form, signed | BT.1361-0 extended colour gamut system (historical) |
| 13 | IEC 61966-2-1 sRGB curve (matrix_coefficients 0) / sYCC curve (5) / signed otherwise | IEC 61966-2-1 sRGB (with matrix_coefficients equal to 0), IEC 61966-2-1 sYCC (with matrix_coefficients equal to 5) |
| 14 | V = a * Lc^0.45 - (a - 1), V = 4.500 * Lc | Rec. ITU-R BT.2020-2 (10 bit system) (functionally the same as the values 1, 6, and 15) |
| 15 | V = a * Lc^0.45 - (a - 1), V = 4.500 * Lc | Rec. ITU-R BT.2020-2 (12 bit system) (functionally the same as the values 1, 6, and 14) |
| 16 | PQ: V = ((c1 + c2 * Lo^n) / (1 + c3 * Lo^n))^m | SMPTE ST 2084 (2014) for 10, 12, 14, and 16-bit systems; BT.2100-2 perceptual quantization (PQ) system |
| 17 | V = (48 * Lo / 52.37)^(1 / 2.6) | SMPTE ST 428-1 (2006) |
| 18 | HLG: V = a * Ln(12 * Lc - b) + c, V = Sqrt(3) * Lc^0.5 | ARIB STD-B67, BT.2100-2 hybrid log-gamma (HLG) system |
| 19..255 | Reserved | For future use by ITU-T \| ISO/IEC |

## Table E-5 (H.264 pp464-466) / Table E.5 (H.265 pp459-460): `matrix_coefficients`

**Identical rows in both specs** (18 defined rows 0..=17; `Reserved`
18..=255). H.264 labels row 0 `GBR`; H.265 labels it `Identity`.

| Value | Matrix (H.264 Table E-5 / H.265 Table E.5) | Informative remark |
|---|---|---|
| 0 | GBR / Identity | identity matrix; GBR (RGB), may also be used for YZX |
| 1 | KR = 0.2126; KB = 0.0722 | BT.709-6, BT.1361-0 conventional and extended gamut (historical), IEC 61966-2-4 xvYCC709, SMPTE RP 177 (1993) Annex B |
| 2 | Unspecified | Image characteristics are unknown or are determined by the application |
| 3 | Reserved | For future use by ITU-T \| ISO/IEC |
| 4 | KR = 0.30; KB = 0.11 | FCC Title 47 CFR (2016) 73.682 (a) (20) |
| 5 | KR = 0.299; KB = 0.114 | BT.470-7 System B, G (historical), BT.601-7 625, BT.1358-0 625 (historical), BT.1700-0 625 PAL and 625 SECAM, IEC 61966-2-1 (sYCC), IEC 61966-2-4 xvYCC601 (functionally the same as the value 6) |
| 6 | KR = 0.299; KB = 0.114 | BT.601-7 525, BT.1358-1 525 or 625, BT.1700-0 NTSC, SMPTE 170M (2004) (functionally the same as the value 5) |
| 7 | KR = 0.212; KB = 0.087 | SMPTE 240M (1999, historical) |
| 8 | YCgCo or YCgCo-R | Equations E-41..E-47 (YCgCo) / E-48..E-55 (YCgCo-R) |
| 9 | KR = 0.2627; KB = 0.0593 | BT.2020-2 non-constant luminance system, BT.2100-2 Y'CbCr |
| 10 | KR = 0.2627; KB = 0.0593 | BT.2020-2 constant luminance system |
| 11 | Y'D'ZD'X | SMPTE ST 2085 (2015) |
| 12 | chromaticity-derived non-constant luminance system | Equations E-29..E-34 |
| 13 | chromaticity-derived constant luminance system | Equations E-29..E-34 |
| 14 | ICtCp | BT.2100-2 ICtCp (PQ for transfer 16, HLG for transfer 18) |
| 15 | IPT-C2 | SMPTE IPT-PQ-C2 |
| 16 | YCgCo-Re | Equations E-48..E-55 |
| 17 | YCgCo-Ro | Equations E-48..E-55 |
| 18..255 | Reserved | For future use by ITU-T \| ISO/IEC |

H.264 adds a constraint (p460): `matrix_coefficients` shall not be 0, 16, or
17 unless `BitDepthC == BitDepthY` and `chroma_format_idc == 3`; 8 is
restricted similarly. That is a conformance rule for encoders; `transmux`
keeps the signalled value.

## Markdown garbling corrected (not copied)

The `private/specs-md/itu_t_h264_avc.md` / `itu_t_h265_hevc.md` conversions
lose the table structure in these tables; the following were read off the
rendered page instead:

- **Table E-3**: rows 1..=12 are each a multi-line cell (primary/green/blue/
  red/white with x, y). The md splits them into separate rows, merges the
  informative remark of one row into the next (`Rec. ITU-R BT.470-7 System M
  (historical)` appears on the "green" row), and for row 11 pushes
  `red 0.680 0.320` / `white 0.314 0.351` out as a bare paragraph between rows
  11 and 12. No code point is lost.
- **Table E-4**: rows 2, 3 and 11 lost their leading pipe and merged into the
  row-1 prose paragraph, so a naive md scrape misses `2 Unspecified` and
  `3 Reserved`. Row 13's first column ("13" and the "If matrix_coefficients is
  equal to 0" condition) is interleaved with its equations.
- **Table E-5**: the table header lost its column separators (`Value Matrix
  Informative remark` with no pipes) and rows 0..=7, 9..=15 lost their pipes
  and merged into one paragraph; only rows 16, 17 and 18..=255 survived as
  table rows.
- **Table E-2** is intact in the md. H.265 Table E.2, E.3, E.4, E.5: the md
  repeats the E.3 and E.5 header rows mid-table (page break), and the E.5
  equations for YCgCo-R cite "Equations E-48 to E-55" in H.264 but "E-41 to
  E-55" in H.265 (H.265 numbering); neither affects the labels.
- H.265 §E.2.1 spells the middle colour element `matrix_coeffs`; H.264 spells
  it `matrix_coefficients`. Same 8-bit field, same table.
