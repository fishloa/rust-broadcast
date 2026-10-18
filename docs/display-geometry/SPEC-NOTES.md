# Display geometry: spec transcription notes

Companion to [`DESIGN.md`](DESIGN.md). Only what the design depends on is
transcribed. Every table here was checked against the **rendered PDF page**
(`pdftoppm -r 90`), not only the markdown transcription, because the
`private/specs-md` text of several of these tables is mangled (rows of
H.264 Table E-1 1..7 are merged into one paragraph; TS 101 154 Table B.5 has
split cells). Page numbers are PDF page indices in `private/specs/*.pdf`.

| Source | File | Pages checked |
|---|---|---|
| ITU-T H.264 (V15, 08/2024) | `itu_t_h264_avc.pdf` | 455 (printed 431): §E.2.1, Table E-1 |
| ITU-T H.265 (V11, 01/2026) | `itu_t_h265_hevc.pdf` | 449 (printed 431): Table E.1 |
| ETSI TS 101 154 V2.10.1 (2026-05) | `etsi_ts_101_154_v02.10.01_av_coding_in_dvb.pdf` | 235-237: Tables B.1-B.5; 238-241 (text layer): Table B.7; 243-246 (text layer): Table B.11, §B.8a |

## 1. Sample aspect ratio (H.264 §E.2.1 Table E-1; H.265 §E.3.1 Table E.1)

Syntax (H.264 §E.1.1, H.265 §E.2.1, identical prefix):

```
aspect_ratio_info_present_flag   u(1)
if( aspect_ratio_info_present_flag ) {
  aspect_ratio_idc               u(8)
  if( aspect_ratio_idc == Extended_SAR )   // H.265 spells it EXTENDED_SAR
    sar_width                    u(16)
    sar_height                   u(16)
}
```

Table E-1 (H.264) and Table E.1 (H.265) carry the **same** values (both
rendered pages checked; the informative "examples of use" column is omitted
here except where it explains a fixture):

| aspect_ratio_idc | SAR | informative example kept for the fixtures |
|---|---|---|
| 0 | Unspecified | |
| 1 | 1:1 ("square") | 1920x1080 16:9 cropped from 1920x1088 |
| 2 | 12:11 | 720x576 4:3 frame with horizontal overscan |
| 3 | 10:11 | 720x480 4:3 frame with horizontal overscan |
| 4 | 16:11 | **720x576 16:9 frame with horizontal overscan** (krone fixture) |
| 5 | 40:33 | 720x480 16:9 frame with horizontal overscan |
| 6 | 24:11 | |
| 7 | 20:11 | |
| 8 | 32:11 | |
| 9 | 80:33 | |
| 10 | 18:11 | |
| 11 | 15:11 | |
| 12 | 64:33 | |
| 13 | 160:99 | |
| 14 | 4:3 | **1440x1080 16:9 frame without horizontal overscan** (tsduck test-092) |
| 15 | 3:2 | |
| 16 | 2:1 | |
| 17..254 | Reserved | |
| 255 | Extended_SAR | (melodie fixture: 64:45) |

Semantics (both specs, verbatim in substance):

- `aspect_ratio_idc` absent => inferred 0.
- "sar_width and sar_height shall be relatively prime or equal to 0. When
  aspect_ratio_idc is equal to 0 or sar_width is equal to 0 or sar_height is
  equal to 0, the sample aspect ratio shall be considered unspecified".
- H.265 only, additionally: "Values of aspect_ratio_idc in the range of 17 to
  254, inclusive, are reserved ... Decoders shall interpret values of
  aspect_ratio_idc in the range of 17 to 254, inclusive, as equivalent to the
  value 0." H.264 Table E-1 marks 17..254 Reserved but the rendered page has no
  equivalent sentence; the design applies the H.265 rule to both (documented
  as such, not presented as an H.264 requirement).
- Table E-1 NOTE 1: "with horizontal overscan" means a 704-wide area of a
  720-wide frame is the 4:3/16:9 picture. This is why 720x576 16:9 is coded as
  16:11 (idc 4) by one broadcaster and as 64:45 (Extended_SAR, 720 x 64/45 =
  1024) by another: both fixtures are legitimate and must both be modelled
  exactly, never "normalised".
- H.265 Table E.1 NOTE 2: the example dimensions are those of the
  conformance cropping window when `field_seq_flag` is 0 and "would have twice
  the height" when `field_seq_flag` is 1 (field-coded HEVC: the SAR applies to
  the luma samples of each field picture).

## 2. Video signal type and colour description (H.264 §E.2.1; H.265 §E.3.1)

```
video_signal_type_present_flag     u(1)
if( video_signal_type_present_flag ) {
  video_format                     u(3)
  video_full_range_flag            u(1)
  colour_description_present_flag  u(1)
  if( colour_description_present_flag ) {
    colour_primaries               u(8)
    transfer_characteristics       u(8)
    matrix_coefficients            u(8)
  }
}
```

- `video_format` absent => inferred 5 (Unspecified). Table E-2 (H.264,
  rendered PDF p456 / printed 432): 0 Component, 1 PAL, 2 NTSC, 3 SECAM,
  4 MAC, 5 Unspecified, 6..7 Reserved. **Checked on the rendered page** for
  Phase 1b; the md table is intact, unlike Tables E-3..E-5 (see below).
- `video_full_range_flag` absent => inferred 0.
- `colour_primaries` absent => inferred 2 (Unspecified). Code-point tables:
  H.264 Tables E-3/E-4/E-5 (PDF pp ~457-465), H.265 Tables E.3/E.4/E.5. These
  are long; Phase 1b transcribed them, row by row, from the rendered pages
  into `transmux/docs/codec/vui-colour-tables.md` (H.264 pp457-460 /
  464-466; H.265 pp451-453 / 459-460). The enum labels come from those
  tables, never from memory or from ffprobe's names.
- `overscan_info_present_flag` / `overscan_appropriate_flag` precede
  `video_signal_type_present_flag` (both specs).

## 3. Cropping / output window

### H.264 §7.4.2.1.1 (md lines 4532-4551)

```
CropUnitX = 1,          CropUnitY = 2 - frame_mbs_only_flag             if ChromaArrayType == 0   (7-19, 7-20)
CropUnitX = SubWidthC,  CropUnitY = SubHeightC * (2 - frame_mbs_only_flag) otherwise              (7-21, 7-22)
CroppedWidth  = PicWidthInSamplesL - CropUnitX * (frame_crop_left_offset + frame_crop_right_offset)        (7-23)
CroppedHeight = 16 * FrameHeightInMbs - CropUnitY * (frame_crop_top_offset + frame_crop_bottom_offset)     (7-24)
```

- ChromaArrayType = `chroma_format_idc` if `separate_colour_plane_flag` == 0,
  else 0 (§7.4.2.1.1, md line 4342-4346).
- Rectangle: x from `CropUnitX * frame_crop_left_offset` to
  `PicWidthInSamplesL - (CropUnitX * frame_crop_right_offset + 1)`, y from
  `CropUnitY * frame_crop_top_offset` to
  `16 * FrameHeightInMbs - (CropUnitY * frame_crop_bottom_offset + 1)`,
  inclusive.
- Range constraint: `frame_crop_left_offset` in `0 ..= PicWidthInSamplesL /
  CropUnitX - (frame_crop_right_offset + 1)`; top likewise. A violating SPS
  is non-conforming (edge case E3 in the design).

### H.265 §7.4.3.2.1 conformance window (md lines 4509-4513)

- Window: x from `SubWidthC * conf_win_left_offset` to
  `pic_width_in_luma_samples - (SubWidthC * conf_win_right_offset + 1)`;
  y from `SubHeightC * conf_win_top_offset` to
  `pic_height_in_luma_samples - (SubHeightC * conf_win_bottom_offset + 1)`.
- "The value of SubWidthC * (conf_win_left_offset + conf_win_right_offset)
  shall be less than pic_width_in_luma_samples" (height likewise).

### H.265 §E.3.1 default display window (md lines 20536-20553)

- `default_display_window_flag`, then `def_disp_win_{left,right,top,bottom}_offset`
  ue(v), inferred 0 when absent.
- Offsets are **added to** the conformance window offsets (E-78..E-81):
  `leftOffset = conf_win_left_offset + def_disp_win_left_offset`, etc.; the
  window is `SubWidthC * leftOffset ..= pic_width_in_luma_samples -
  (SubWidthC * rightOffset + 1)` and likewise vertically.
- Semantics: "the area that is within the conformance cropping window and that
  is suggested to be displayed in the absence of any alternative indication".
  It is a **suggestion**, not decoder output cropping.

## 4. Parameter-set activation (when a geometry change can take effect)

- H.264 §7.4.1.2.1 (md line 4140-4142): an SPS is activated when a PPS that
  refers to it is activated (by a slice) or by a buffering-period SEI; "An
  activated sequence parameter set RBSP shall remain active for the entire
  coded video sequence." NOTE 3: an SPS "can only be activated by a buffering
  period SEI message when the buffering period SEI message is part of an IDR
  access unit". "Any sequence parameter set NAL unit containing the value of
  seq_parameter_set_id for the active sequence parameter set RBSP for a coded
  video sequence shall have the same content as that of the active sequence
  parameter set RBSP ... unless it follows the last access unit of the coded
  video sequence and precedes the first VCL NAL unit ... of another coded video
  sequence."
- H.265 §7.4.3.x (md lines 4303-4309): SPS activated by PPS activation or by
  an active parameter sets SEI; "An activated SPS RBSP for the base layer shall
  remain active for the entire coded video sequence (CVS)". NOTE 2: "an IRAP
  access unit with NoRaslOutputFlag equal to 1 begins a new CVS". Same
  same-content rule as H.264 (md line 4305). VPS: same activation pattern
  (md line 4309); the VPS carries no display-geometry syntax.

Consequence used by the design: in a conforming stream, SPS-derived geometry
(SAR, crop, colour, full range) changes **only** at an IDR (H.264) or an IRAP
with NoRaslOutputFlag = 1 (H.265).

## 5. AFD and bar data (ETSI TS 101 154 Annex B)

### Table B.1 user_identifier (PDF p235)

| user_identifier | user_structure() |
|---|---|
| 0x47413934 ('GA94') | DVB1_data() |
| 0x44544731 ('DTG1') | afd_data() |

### Table B.2 afd_data() (PDF p235)

```
afd_data() {
  '0'                         1  bslbf
  active_format_flag          1  bslbf
  reserved (set to '00 0001') 6  bslbf
  if( active_format_flag == 1 ) {
    reserved (set to '1111')  4  bslbf
    active_format             4  bslbf
  }
}
```

### Table B.3 / B.4 DVB1_data() (PDF pp235-236)

`DVB1_data() { user_data_type_code 8 uimsbf; user_data_type_structure() }`;
`user_data_type_code` 0x03 cc_data(), **0x06 bar_data()**, 0x07
multi_region_disparity(), 0x09 ST2094-10_data(); all others industry
reserved.

### Table B.5 active_format (PDF p237)

| active_format | Aspect ratio of the "area of interest" |
|---|---|
| 0000 | AFD unknown (see below) |
| 0001 | Reserved |
| 0010 | box 16:9 (top) |
| 0011 | box 14:9 (top) |
| 0100 | box > 16:9 (centre) |
| 0101 to 0111 | Reserved |
| 1000 | Active format is the same as the coded frame |
| 1001 | 4:3 (centre) |
| 1010 | 16:9 (centre) |
| 1011 | 14:9 (centre) |
| 1100 | Reserved |
| 1101 | 4:3 (with shoot and protect 14:9 centre) |
| 1110 | 16:9 (with shoot and protect 14:9 centre) |
| 1111 | 16:9 (with shoot and protect 4:3 centre) |

- "AFD 0000 indicates that information is not available and is undefined.
  Unless bar data is available, receivers and video equipment should interpret
  the active format as being the same as the coded frame." With bar data:
  aspect narrower than 16:9 but not 4:3 or 14:9.
- "AFD 0100, which should be accompanied by bar data, signals that the image's
  aspect ratio is wider than 16:9".
- §B.3.0: "The AFD itself does not describe the aspect ratio of the coded
  frame". §B.3.2 (AVC) / §B.3.6 (HEVC): it is used "in conjunction with ...
  picture width, picture height, frame cropping information, and sample aspect
  ratio".

### Table B.7 bar_data() (text layer, PDF ~p240)

```
bar_data() {
  top_bar_flag 1, bottom_bar_flag 1, left_bar_flag 1, right_bar_flag 1
  reserved (set to '1111') 4
  if( top_bar_flag )    { marker_bits '11' 2; line_number_end_of_top_bar      14 uimsbf }
  if( bottom_bar_flag ) { marker_bits '11' 2; line_number_start_of_bottom_bar 14 uimsbf }
  if( left_bar_flag )   { marker_bits '11' 2; pixel_number_end_of_left_bar    14 uimsbf }
  if( right_bar_flag )  { marker_bits '11' 2; pixel_number_start_of_right_bar 14 uimsbf }
}
```

"Bar data is constrained ... to be signalled in pairs, either top and bottom
bars or left and right bars, but not both pairs at once." Table B.8 (line
number designation) is **not** transcribed here; it must be before bar data is
given a typed coordinate meaning.

### Carriage

| Codec | Wrapper | Clause |
|---|---|---|
| MPEG-2 video | picture `user_data()`: `user_data_start_code` 0x000001B2 + `user_identifier` + `user_structure()` in `extension_and_user_data(2)` | B.6.2, Table B.10 |
| H.264/AVC | SEI `user_data_registered_itu_t_t35` (payloadType 4): `itu_t_t35_country_code` 0xB5, `itu_t_t35_provider_code` 0x0031, `user_identifier`, `user_structure()` | B.7.2, Table B.11 |
| H.265/HEVC | same as AVC; "it shall be a prefix SEI message (i.e. nal_unit_type shall be equal to PREFIX_SEI_NUT)" | B.8a |

The SEI wrapper itself: H.264 §D.1.6 / H.265 §D.2.6
`user_data_registered_itu_t_t35(payloadSize) { itu_t_t35_country_code b(8);
if (!= 0xFF) i = 1 else { itu_t_t35_country_code_extension_byte b(8); i = 2 }
do { itu_t_t35_payload_byte b(8) } while (i < payloadSize) }`. TS 101 154's
provider code and user_identifier are the first `itu_t_t35_payload_byte`s.
`transmux/src/nal.rs` already walks exactly this header for GA94 captions
(`append_if_atsc_cc_data`).

### Persistence (what the specs available here actually say)

| Codec | Rule | Source |
|---|---|---|
| MPEG-2 | "After any sequence_header() the default aspect ratio of the area of interest shall be that signaled by the parameters in the sequence_header() and sequence_display_extension() ... After introduction, such an AFD shall remain in effect until the next sequence_header() or until a new AFD is introduced." | ATSC A/53 Part 4 (2009) §6.2.4 (md line 720-724) |
| MPEG-2 bar data | remains in effect until next `sequence_header()`, new `bar_data()`, or AFD per B.3.4 | TS 101 154 §B.6.1 |
| VC-1 | AFD/bar data persists until next sequence start or a new AFD / different bar data | TS 101 154 §B.8.1 |
| **H.264 / H.265** | **No persistence rule in any spec available in `private/specs-md`.** TS 101 154 §B.7 / §B.8a define carriage only. The AVC rule lives in ATSC A/72 Part 1 and SMPTE ST 2016-1, neither of which is transcribed. | gap |

## 6. ISO BMFF visual sample entry boxes

Only what `transmux/docs/codec/isobmff-visual-ext.md` already transcribes
(ISO/IEC 14496-12:2015 §12.1.4.2/§12.1.4.3/§12.1.5): `pasp` (hSpacing,
vSpacing u32, both positive, only the ratio matters), `clap` (eight u32 N/D
fractions, centre-relative), `colr` (`nclx`: colour_primaries u16,
transfer_characteristics u16, matrix_coefficients u16, full_range_flag u1 +
7 reserved). §12.1.5.1: "If colour information is supplied in both this box,
and also in the video bitstream, this box takes precedence, and over-rides the
information in the bitstream." The transcription starts mid-sentence: the
§12.1.4.1 definition (box placement, and whether `pasp`/`clap` take precedence
over VUI) and §8.3.2 (`tkhd` width/height = visual presentation size) are
**not** transcribed. See the gaps list in DESIGN.md §12.

## 7. Manifests

- DVB-DASH, ETSI TS 103 285 V1.4.1 (md lines 1045-1072): for video Adaptation
  Sets `@par` "should be present"; for each video Representation `@sar` "should
  be present or inherited"; "for which the picture aspect ratio is other than
  16:9, at least one of the attributes @par and @sar shall be present or
  inherited". The attribute *syntax* is ISO/IEC 23009-1, not transcribed.
- HLS, RFC 8216 §4.3.4.2 / draft-pantos-hls-rfc8216bis: `RESOLUTION` is "the
  optimal pixel resolution at which to display all the video in the Variant
  Stream" (no SAR attribute exists). rfc8216bis `VIDEO-RANGE` (SDR/HLG/PQ) is
  derived from the TransferCharacteristics code point ([CICP] 1, 6, 13, 14, 15
  => SDR; 18 => HLG; see the draft for PQ).

## 8. DVB component descriptor (EN 300 468 §6.2.8, Table 26)

Carried in SDT and EIT (EN 300 468 V1.20.1 Table 12, md line 2392), not the
PMT. "When used in the SDT and EIT these component stream characteristics ...
describe the editorial characteristics of the event or service. These are
intended to be used by receivers to inform the user" (md line 2700). Table 26
rows for H.264 SD video distinguish "4:3 aspect ratio", "16:9 aspect ratio",
"> 16:9 aspect ratio" per frame rate. It is event-granular, not frame-accurate.
Already parsed by `dvb-si` (`descriptors/component.rs`).

## 9. Colour tables: markdown garbling found in Phase 1b

Read from the **rendered** pages, not the md (details per table in
`transmux/docs/codec/vui-colour-tables.md`):

- **H.264 Table E-3 (md)**: each row 1..=12 is one multi-line cell in the PDF
  (primary/green/blue/red/white with x, y); the md splits them into separate
  rows, merges one row's informative remark into the next, and for row 11
  emits `red 0.680 0.320` / `white 0.314 0.351` as a bare paragraph between
  rows 11 and 12. No code point is lost.
- **H.264 Table E-4 (md)**: `2 Unspecified` and `3 Reserved` lost their pipes
  and merged into row 1's prose, so a naive md scrape **misses two defined
  rows**.
- **H.264 Table E-5 (md)**: the header lost its pipes (`Value Matrix
  Informative remark`) and rows 0..=7, 9..=15 merged into one paragraph; only
  16, 17 and 18..=255 survive as table rows. A naive scrape would treat 8
  (YCgCo) and, worse, the BT.601/709 rows as absent.
- **H.265 Tables E.3/E.5 (md)**: the header row is repeated mid-table at the
  page break; E.5's YCgCo-R equation references differ from H.264's (H.265
  numbering). Neither affects a label.
- **Naming**: H.265 §E.2.1 spells the middle colour element `matrix_coeffs`,
  H.264 `matrix_coefficients`; same `u(8)` field and same table.
- **Row differences**: only `video_format` 6 and 7 differ in meaning between
  the specs (H.264 "Reserved" with no interpretation; H.265 "interpret as 5").
  Tables E-3/E-4/E-5 and E.3/E.4/E.5 define the same rows.

