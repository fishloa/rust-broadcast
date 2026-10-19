# MPEG-2 video (ITU-T H.262 | ISO/IEC 13818-2:1995) — display-related syntax and semantics

Groundwork for display geometry on MPEG-2 video (Phase 3b: AFD/bar data in picture user data, and MPEG-2
sequence geometry/colour). Complements `mpeg2-video-h262.md` (config subset). Source:
`private/specs/itu_t_h262_mpeg2_video.pdf` (H.262 (1995 E), 1995 edition). **Every table, syntax block and quote
below was checked against the rendered PDF page.** Page numbers are printed page numbers (PDF page = printed + 10).

## Syntax (printed pp. 26-28)

`sequence_header()` (p. 26): `sequence_header_code` 32, `horizontal_size_value` 12, `vertical_size_value` 12,
`aspect_ratio_information` 4, `frame_rate_code` 4, `bit_rate_value` 18, `marker_bit` 1, `vbv_buffer_size_value` 10,
`constrained_parameters_flag` 1, `load_intra_quantiser_matrix` 1 [+ `intra_quantiser_matrix[64]` 8x64],
`load_non_intra_quantiser_matrix` 1 [+ `non_intra_quantiser_matrix[64]` 8x64], `next_start_code()`.

`extension_and_user_data( i )` (p. 26):

```
extension_and_user_data( i ) {
    while ( ( nextbits() == extension_start_code ) || ( nextbits() == user_data_start_code ) ) {
        if ( ( i != 1 ) && ( nextbits() == extension_start_code ) )
            extension_data( i )
        if ( nextbits() == user_data_start_code )
            user_data()
    }
}
```

`extension_data( i )` (p. 27): `while ( nextbits() == extension_start_code ) { extension_start_code (32);
if (i == 0) { /* follows sequence_extension() */ if ( nextbits() == "Sequence Display Extension ID" )
sequence_display_extension() else sequence_scalable_extension() }; (NOTE - i never takes the value 1 because
extension_data() never follows a group_of_pictures_header()); if (i == 2) { /* follows picture_coding_extension()
*/ quant_matrix_extension() | copyright_extension() | picture_display_extension() |
picture_spatial_scalable_extension() | picture_temporal_scalable_extension() } }`.

`user_data()` (p. 27): `user_data_start_code` 32 (bslbf); `while( nextbits() != '0000 0000 0000 0000 0000 0001' ) {
user_data 8 (uimsbf) }`; `next_start_code()`.

`sequence_display_extension()` (p. 28):

```
sequence_display_extension() {
    extension_start_code_identifier   4  uimsbf
    video_format                      3  uimsbf
    colour_description                1  uimsbf
    if ( colour_description ) {
        colour_primaries              8  uimsbf
        transfer_characteristics      8  uimsbf
        matrix_coefficients           8  uimsbf
    }
    display_horizontal_size          14  uimsbf
    marker_bit                        1  bslbf
    display_vertical_size            14  uimsbf
    next_start_code()
}
```

(`sequence_extension()`, p. 28, carries `horizontal_size_extension` 2 and `vertical_size_extension` 2 — the
coded size is `(extension << 12) | value`, already used by `mpeg2-video-h262.md`.)

## aspect_ratio_information — Table 6-3 and the SAR relation (printed p. 41)

> aspect_ratio_information - This is a four-bit integer defined in Table 6-3.

| aspect_ratio_information | Sample Aspect Ratio | DAR |
|---|---|---|
| 0000 | Forbidden | Forbidden |
| 0001 | 1.0 (Square Sample) | - |
| 0010 | - | 3 ÷ 4 |
| 0011 | - | 9 ÷ 16 |
| 0100 | - | 1 ÷ 2.21 |
| 0101 ... 1111 | - | Reserved |

> aspect_ratio_information either specifies that the "Sample Aspect Ratio" (SAR) of the reconstructed frame is 1,0
> (square samples) or alternatively it gives the "Display Aspect Ratio" (DAR).
> - If sequence_display_extension() is not present, then it is intended that the entire reconstructed frame is
>   intended to be mapped to the entire active region of the display. The sample aspect ratio may be calculated as
>   follows: **SAR = DAR x horizontal_size / vertical_size**. NOTE - In this case horizontal_size and vertical_size
>   are constrained by the SAR of the source and the DAR selected.
> - If sequence_display_extension() is present then the sample aspect ratio may be calculated as follows:
>   **SAR = DAR x display_horizontal_size / display_vertical_size**.

(Note the table lists DAR as height-over-width style fractions "3 ÷ 4", "9 ÷ 16": the printed form is the
ratio as the spec writes it; the usual 4:3 / 16:9 / 2.21:1 reading is width:height. The SAR formula as printed
multiplies DAR by size ratios; no source here resolves the notation beyond the printed text, so an implementation
must test it against real MPEG-2 captures, not assume.)

## Sequence display extension semantics (printed pp. 44-48)

p. 44: "This Specification does not define the display process. The information in this extension does not affect
the decoding process and may be ignored by decoders that conform to this Specification."

`video_format` (3 bits, Table 6-6; "If the sequence_display_extension() is not present in the bitstream, then the
video format may be assumed to be 'Unspecified video format'"): 000 Component; 001 PAL; 010 NTSC; 011 SECAM;
100 MAC; 101 Unspecified video format; 110, 111 Reserved.

`colour_description`: "A flag which if set to '1' indicates the presence of colour_primaries,
transfer_characteristics and matrix_coefficients in the bitstream."

**Inferred defaults differ from H.264/H.265** (printed pp. 44, 47): "In the case that sequence_display_extension() is
not present in the bitstream or colour_description is zero the chromaticity is assumed to be that corresponding to
colour_primaries having the value 1" and likewise transfer_characteristics value **1** and matrix_coefficients value
**1** — i.e. **BT.709 is the MPEG-2 inferred colour**, whereas H.264/H.265 infer "unspecified" (2/2/2)
(`vui-colour-tables.md`). A player must not apply the H.264 default to an MPEG-2 stream.

Table 6-7 colour primaries (p. 45): 0 (Forbidden); 1 ITU-R BT.709 (green 0.300/0.600, blue 0.150/0.060, red
0.640/0.330, white D65 0.3127/0.3290); 2 Unspecified Video ("Image characteristics are unknown"); 3 Reserved;
4 ITU-R BT.470-2 System M (green 0.21/0.71, blue 0.14/0.08, red 0.67/0.33, white C 0.310/0.316); 5 ITU-R BT.470-2
System B, G (green 0.29/0.60, blue 0.15/0.06, red 0.64/0.33, white D65 0.313/0.329); 6 SMPTE 170M (green
0.310/0.595, blue 0.155/0.070, red 0.630/0.340, white D65 0.3127/0.3290); 7 SMPTE 240M (1987) (same primaries as 6,
white D65 0.3127/0.3291); 8-255 Reserved.

Table 6-8 transfer characteristics (p. 46): 0 (Forbidden); 1 ITU-R BT.709 (V = 1.099 Lc^0.45 - 0.099 for 1 >= Lc >=
0.018, V = 4.500 Lc for 0.018 > Lc >= 0); 2 Unspecified Video; 3 Reserved; 4 ITU-R BT.470-2 System M (assumed
display gamma 2.2); 5 ITU-R BT.470-2 System B, G (assumed display gamma 2.8); 6 SMPTE 170M (same curve as 1);
7 SMPTE 240M (1987) (V = 1.1115 Lc^0.45 - 0.1115 for Lc >= 0.0228, V = 4.0 Lc below); 8 Linear (V = Lc); 9-255
Reserved.

Table 6-9 matrix coefficients (p. 47): 0 (Forbidden); 1 ITU-R BT.709 (E'Y = 0.7154 E'G + 0.0721 E'B + 0.2125 E'R;
E'PB = -0.386 E'G + 0.500 E'B - 0.115 E'R; E'PR = -0.454 E'G - 0.046 E'B + 0.500 E'R); 2 Unspecified Video;
3 Reserved; 4 FCC (0.59 / 0.11 / 0.30; PB -0.331 / 0.500 / -0.169; PR -0.421 / -0.079 / 0.500); 5 ITU-R BT.470-2
System B, G (0.587 / 0.114 / 0.299; PB -0.331 / 0.500 / -0.169; PR -0.419 / -0.081 / 0.500); 6 SMPTE 170M (same as 5);
7 SMPTE 240M (1987) (0.701 / 0.087 / 0.212; PB -0.384 / 0.500 / -0.116; PR -0.445 / -0.055 / 0.500); 8-255 Reserved.
Quantisation: Y = 219 E'Y + 16, Cb = 224 E'PB + 128, Cr = 224 E'PR + 128 (p. 47 NOTE: output sample values are
limited to [0:255]). H.262 has **no** full-range flag.

Compared with the H.264/H.265 tables, the MPEG-2 value sets are **subsets** (primaries 1-7, transfer 1-8, matrix
1-7) with the same meanings where defined; **MPEG-2 transfer 4/5 give "assumed display gamma 2.2/2.8" and matrix 4
is "FCC"**, the same code points H.273 labels BT.470-6 System M/B,G and FCC. The shared `ColourPrimaries` /
`TransferCharacteristics` / `MatrixCoefficients` enums therefore cover MPEG-2 values 1-7/1-8/1-7 unchanged.

`display_horizontal_size` / `display_vertical_size` (p. 48), verbatim: "together define a rectangle which may be
considered as the 'intended display's' active rectangle. If this rectangle is smaller than the encoded frame size,
then the display process may be expected to display only a portion of the encoded frame. Conversely if the display
rectangle is larger than the encoded frame size, then the display process may be expected to display the
reconstructed frames on a portion of the display device rather than on the whole display device.
display_horizontal_size shall be in the same units as horizontal_size (samples of the encoded frames).
display_vertical_size shall be in the same units as vertical_size (lines of the encoded frames).
display_horizontal_size and display_vertical_size do not affect the decoding process but may be used by the display
process that is not standardised in this Specification."

## What this settles for Phase 3b, and what it does not

1. AFD for MPEG-2 is interpreted against the *source aspect ratio* = `DAR x display_horizontal_size /
   display_vertical_size x vertical_size / horizontal_size` when `sequence_display_extension()` is present, else the
   DAR (ETSI TS 101 154 B.3.1 / A/53 §6.2.4.2, `afd-bar-data.md`); this file supplies the DAR table and the extension
   syntax those formulas use.
2. AFD/bar data user data sits in `extension_and_user_data(2)` after the picture header (`user_data()`,
   start code `0x000001B2`); `extension_and_user_data(0)` follows the sequence extension.
3. MPEG-2 colour: inferred BT.709 (1/1/1) when absent; value sets are subsets of the shared tables.

## Not covered

- ISO/IEC 13818-2 editions after 1995 (e.g. the 2000 edition and corrigenda); only the 1995 text is held.
- `picture_display_extension()` (frame centre offsets / pan-scan vectors) syntax and semantics: referenced above but
  not transcribed (relevant to ETSI TS 101 154 B.3.5, "Relationship with Pan Vectors").
- The DAR notation question under Table 6-3 (see the note above).
