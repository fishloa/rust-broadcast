# Output cropping: H.264 frame cropping, H.265 conformance window + default display window, H.266 conformance windows

Primary-source transcription for display geometry (crop phase, `docs/display-geometry/DESIGN.md`).
**Every syntax table, equation and quote below was checked against the rendered PDF page.** The only
exceptions are named where they occur (the lines of a table that sit on a page not shown, and the
Pan-Scan equations, which are referenced but not transcribed). Page numbers are
the **printed** page numbers of the cited edition; PDF page = printed + 24 (H.264), + 18 (H.265), + 14
(H.266).

| Spec | Edition | Private PDF |
|---|---|---|
| ITU-T H.264 | (V15) (08/2024) | `private/specs/itu_t_h264_avc.pdf` |
| ITU-T H.265 | (V11) (01/2026) | `private/specs/itu_t_h265_hevc.pdf` |
| ITU-T H.266 | (V4) (01/2026) | `private/specs/itu_t_h266_vvc.pdf` |

## 1. SubWidthC / SubHeightC

### H.264 Table 6-1 (printed p. 23)

The variables SubWidthC and SubHeightC depend on the chroma format; "An entry marked as "-" in Table 6-1
denotes an undefined value for SubWidthC or SubHeightC."

| chroma_format_idc | separate_colour_plane_flag | Chroma Format | SubWidthC | SubHeightC |
|---|---|---|---|---|
| 0 | 0 | monochrome | - | - |
| 1 | 0 | 4:2:0 | 2 | 2 |
| 2 | 0 | 4:2:2 | 2 | 1 |
| 3 | 0 | 4:4:4 | 1 | 1 |
| 3 | 1 | 4:4:4 | - | - |

`ChromaArrayType` (H.264 §7.4.2.1.1, printed p. 77): "If separate_colour_plane_flag is
equal to 0, ChromaArrayType is set equal to chroma_format_idc. Otherwise (separate_colour_plane_flag is
equal to 1), ChromaArrayType is set equal to 0." When `chroma_format_idc` is absent it is inferred to be 1
(4:2:0).

### H.265 Table 6-1 (printed p. 22)

| chroma_format_idc | separate_colour_plane_flag | Chroma format | SubWidthC | SubHeightC |
|---|---|---|---|---|
| 0 | 0 | monochrome | 1 | 1 |
| 1 | 0 | 4:2:0 | 2 | 2 |
| 2 | 0 | 4:2:2 | 2 | 1 |
| 3 | 0 | 4:4:4 | 1 | 1 |
| 3 | 1 | 4:4:4 | 1 | 1 |

**Difference:** H.264 leaves monochrome and separate-plane 4:4:4 *undefined* ("-"); H.265 defines them as
1 x 1. H.264 instead gives monochrome its own crop unit rule (§2 below, `CropUnitX = 1`).

### H.266 Table 2 (printed p. 20)

| sps_chroma_format_idc | Chroma format | SubWidthC | SubHeightC |
|---|---|---|---|
| 0 | Monochrome | 1 | 1 |
| 1 | 4:2:0 | 2 | 2 |
| 2 | 4:2:2 | 2 | 1 |
| 3 | 4:4:4 | 1 | 1 |

## 2. H.264 frame cropping

### Syntax — `seq_parameter_set_data()` (printed p. 47; descriptor ue(v)/u(1), category 0)

```
pic_width_in_mbs_minus1               ue(v)
pic_height_in_map_units_minus1        ue(v)
frame_mbs_only_flag                   u(1)
if( !frame_mbs_only_flag )
    mb_adaptive_frame_field_flag      u(1)
direct_8x8_inference_flag             u(1)
frame_cropping_flag                   u(1)
if( frame_cropping_flag ) {
    frame_crop_left_offset            ue(v)
    frame_crop_right_offset           ue(v)
    frame_crop_top_offset             ue(v)
    frame_crop_bottom_offset          ue(v)
}
vui_parameters_present_flag           u(1)
if( vui_parameters_present_flag )
    vui_parameters( )
```

### Derived picture size (printed p. 81)

```
PicWidthInMbs = pic_width_in_mbs_minus1 + 1                            (7-13)
PicWidthInSamplesL = PicWidthInMbs * 16                                (7-14)
PicWidthInSamplesC = PicWidthInMbs * MbWidthC                          (7-15)
PicHeightInMapUnits = pic_height_in_map_units_minus1 + 1               (7-16)
PicSizeInMapUnits = PicWidthInMbs * PicHeightInMapUnits                (7-17)
FrameHeightInMbs = ( 2 - frame_mbs_only_flag ) * PicHeightInMapUnits   (7-18)
```

### Semantics (printed p. 82), verbatim

> frame_cropping_flag equal to 1 specifies that the frame cropping offset parameters follow next in the
> sequence parameter set. frame_cropping_flag equal to 0 specifies that the frame cropping offset
> parameters are not present.
>
> frame_crop_left_offset, frame_crop_right_offset, frame_crop_top_offset, frame_crop_bottom_offset
> specify the samples of the pictures in the coded video sequence that are output from the decoding
> process, in terms of a rectangular region specified in frame coordinates for output.
>
> The variables CropUnitX and CropUnitY are derived as follows:
> - If ChromaArrayType is equal to 0, CropUnitX and CropUnitY are derived as:
>   `CropUnitX = 1` (7-19); `CropUnitY = 2 - frame_mbs_only_flag` (7-20)
> - Otherwise (ChromaArrayType is equal to 1, 2, or 3), CropUnitX and CropUnitY are derived as:
>   `CropUnitX = SubWidthC` (7-21); `CropUnitY = SubHeightC * ( 2 - frame_mbs_only_flag )` (7-22)
>
> The variables CroppedWidth and CroppedHeight are derived as follows:
> `CroppedWidth = PicWidthInSamplesL - CropUnitX * ( frame_crop_left_offset + frame_crop_right_offset )` (7-23)
> `CroppedHeight = 16 * FrameHeightInMbs - CropUnitY * ( frame_crop_top_offset + frame_crop_bottom_offset )` (7-24)
>
> The frame cropping rectangle contains luma samples with horizontal frame coordinates from
> CropUnitX * frame_crop_left_offset to PicWidthInSamplesL - ( CropUnitX * frame_crop_right_offset + 1 )
> and vertical frame coordinates from CropUnitY * frame_crop_top_offset to
> ( 16 * FrameHeightInMbs ) - ( CropUnitY * frame_crop_bottom_offset + 1 ), inclusive. The value of
> frame_crop_left_offset shall be in the range of 0 to ( PicWidthInSamplesL / CropUnitX ) -
> ( frame_crop_right_offset + 1 ), inclusive; and the value of frame_crop_top_offset shall be in the
> range of 0 to ( 16 * FrameHeightInMbs / CropUnitY ) - ( frame_crop_bottom_offset + 1 ), inclusive.
>
> When frame_cropping_flag is equal to 0, the values of frame_crop_left_offset, frame_crop_right_offset,
> frame_crop_top_offset, and frame_crop_bottom_offset shall be inferred to be equal to 0.
>
> When ChromaArrayType is not equal to 0, the corresponding specified samples of the two chroma arrays
> are the samples having frame coordinates ( x / SubWidthC, y / SubHeightC ), where ( x, y ) are the
> frame coordinates of the specified luma samples.
>
> For decoded fields, the specified samples of the decoded field are the samples that fall within the
> rectangle specified in frame coordinates.

(Worked example, derived from (7-21)/(7-22), not quoted: the interlaced case matters because
`frame_mbs_only_flag = 0` doubles `CropUnitY`. For 4:2:0 (`SubHeightC = 2`) a 1088-line picture is cropped to
1080 (8 lines) with `frame_crop_bottom_offset = 2` when `frame_mbs_only_flag = 0` (`CropUnitY = 2*2 = 4`),
but with `frame_crop_bottom_offset = 4` when `frame_mbs_only_flag = 1` (`CropUnitY = 2*1 = 2`).)

## 3. H.265 conformance cropping window

### Syntax — `seq_parameter_set_rbsp()` (printed pp. 35-36)

```
sps_video_parameter_set_id            u(4)
sps_max_sub_layers_minus1             u(3)
sps_temporal_id_nesting_flag          u(1)
profile_tier_level( 1, sps_max_sub_layers_minus1 )
sps_seq_parameter_set_id              ue(v)
chroma_format_idc                     ue(v)
if( chroma_format_idc == 3 )
    separate_colour_plane_flag        u(1)
pic_width_in_luma_samples             ue(v)
pic_height_in_luma_samples            ue(v)
conformance_window_flag               u(1)
if( conformance_window_flag ) {
    conf_win_left_offset              ue(v)
    conf_win_right_offset             ue(v)
    conf_win_top_offset               ue(v)
    conf_win_bottom_offset            ue(v)          // next page, printed p. 36
}
bit_depth_luma_minus8                 ue(v)
```

### Semantics (printed p. 76), verbatim

> pic_width_in_luma_samples specifies the width of each decoded picture in units of luma samples.
> pic_width_in_luma_samples shall not be equal to 0 and shall be an integer multiple of MinCbSizeY.
> pic_height_in_luma_samples specifies the height ... (same constraint).
>
> conformance_window_flag equal to 1 indicates that the conformance cropping window offset parameters
> follow next in the SPS. conformance_window_flag equal to 0 indicates that the conformance cropping
> window offset parameters are not present.
>
> conf_win_left_offset, conf_win_right_offset, conf_win_top_offset and conf_win_bottom_offset specify
> the samples of the pictures in the CVS that are output from the decoding process, in terms of a
> rectangular region specified in picture coordinates for output. When conformance_window_flag is equal
> to 0, the values of conf_win_left_offset, conf_win_right_offset, conf_win_top_offset and
> conf_win_bottom_offset are inferred to be equal to 0.
>
> The conformance cropping window contains the luma samples with horizontal picture coordinates from
> SubWidthC * conf_win_left_offset to pic_width_in_luma_samples - ( SubWidthC * conf_win_right_offset + 1 )
> and vertical picture coordinates from SubHeightC * conf_win_top_offset to
> pic_height_in_luma_samples - ( SubHeightC * conf_win_bottom_offset + 1 ), inclusive.
>
> The value of SubWidthC * ( conf_win_left_offset + conf_win_right_offset ) shall be less than
> pic_width_in_luma_samples, and the value of SubHeightC * ( conf_win_top_offset + conf_win_bottom_offset )
> shall be less than pic_height_in_luma_samples.
>
> When ChromaArrayType is not equal to 0, the corresponding specified samples of the two chroma arrays
> are the samples having picture coordinates ( x / SubWidthC, y / SubHeightC ), where ( x, y ) are the
> picture coordinates of the specified luma samples.
>
> NOTE 3 - The conformance cropping window offset parameters are only applied at the output. All
> internal decoding processes are applied to the uncropped picture size.

(H.265 has no field/frame `CropUnit` doubling: the units are `SubWidthC` / `SubHeightC` only.)

### Default display window (VUI) — syntax printed pp. 427-428, semantics p. 445

Syntax fragment (`vui_parameters()`, p. 428; only the part on that page is transcribed, the earlier VUI
fields are on p. 427 and are not needed here):

```
neutral_chroma_indication_flag        u(1)
field_seq_flag                        u(1)
frame_field_info_present_flag         u(1)
default_display_window_flag           u(1)
if( default_display_window_flag ) {
    def_disp_win_left_offset          ue(v)
    def_disp_win_right_offset         ue(v)
    def_disp_win_top_offset           ue(v)
    def_disp_win_bottom_offset        ue(v)
}
vui_timing_info_present_flag          u(1)
```

Semantics, verbatim (p. 445):

> default_display_window_flag equal to 1 indicates that the default display window parameters follow next
> in the VUI. default_display_window_flag equal to 0 indicates that the default display window parameters
> are not present. The default display window parameters identify the area that is within the
> conformance cropping window and that is suggested to be displayed in the absence of any alternative
> indication (provided within the bitstream or by external means not specified in this Specification) of
> preferred display characteristics.
>
> def_disp_win_left_offset, def_disp_win_right_offset, def_disp_win_top_offset and
> def_disp_win_bottom_offset specify the samples of the pictures in the CVS that are within the default
> display window, in terms of a rectangular region specified in picture coordinates for display. When
> default_display_window_flag is equal to 0, the values of def_disp_win_left_offset,
> def_disp_win_right_offset, def_disp_win_top_offset and def_disp_win_bottom_offset are inferred to be
> equal to 0.
>
> The following variables are derived from the default display window parameters:
> `leftOffset = conf_win_left_offset + def_disp_win_left_offset` (E-78)
> `rightOffset = conf_win_right_offset + def_disp_win_right_offset` (E-79)
> `topOffset = conf_win_top_offset + def_disp_win_top_offset` (E-80)
> `bottomOffset = conf_win_bottom_offset + def_disp_win_bottom_offset` (E-81)
>
> The default display window contains the luma samples with horizontal picture coordinates from
> SubWidthC * leftOffset to pic_width_in_luma_samples - ( SubWidthC * rightOffset + 1 ) and vertical
> picture coordinates from SubHeightC * topOffset to pic_height_in_luma_samples -
> ( SubHeightC * bottomOffset + 1 ), inclusive.
>
> The value of SubWidthC * ( leftOffset + rightOffset ) shall be less than pic_width_in_luma_samples and
> the value of SubHeightC * ( topOffset + bottomOffset ) shall be less than pic_height_in_luma_samples.

So the default display window offsets are **relative to the conformance window** (they add), and the DDW
is a *suggestion* ("absence of any alternative indication"), while the conformance window is the output
crop. ETSI TS 101 154 says HEVC IRDs "shall support conformance cropping" and Default Display Window
support is "strongly recommended" (`afd-bar-data.md` §5; TS 101 154 §5.14.1.5.6 — not transcribed here).

## 4. H.266 conformance windows (SPS and PPS)

### SPS syntax (printed p. 40)

```
sps_pic_width_max_in_luma_samples     ue(v)
sps_pic_height_max_in_luma_samples    ue(v)
sps_conformance_window_flag           u(1)
if( sps_conformance_window_flag ) {
    sps_conf_win_left_offset          ue(v)
    sps_conf_win_right_offset         ue(v)
    sps_conf_win_top_offset           ue(v)
    sps_conf_win_bottom_offset        ue(v)
}
```

### SPS semantics (printed p. 105), verbatim

> sps_conformance_window_flag equal to 1 indicates that the conformance cropping window offset parameters
> follow next in the SPS. sps_conformance_window_flag equal to 0 indicates that the conformance cropping
> window offset parameters are not present in the SPS.
>
> sps_conf_win_left_offset, sps_conf_win_right_offset, sps_conf_win_top_offset, and
> sps_conf_win_bottom_offset specify the cropping window that is applied to pictures with
> pps_pic_width_in_luma_samples equal to sps_pic_width_max_in_luma_samples and
> pps_pic_height_in_luma_samples equal to sps_pic_height_max_in_luma_samples. When
> sps_conformance_window_flag is equal to 0, the values of sps_conf_win_left_offset,
> sps_conf_win_right_offset, sps_conf_win_top_offset, and sps_conf_win_bottom_offset are inferred to be
> equal to 0.
>
> The conformance cropping window contains the luma samples with horizontal picture coordinates from
> SubWidthC * sps_conf_win_left_offset to sps_pic_width_max_in_luma_samples -
> ( SubWidthC * sps_conf_win_right_offset + 1 ) and vertical picture coordinates from
> SubHeightC * sps_conf_win_top_offset to sps_pic_height_max_in_luma_samples -
> ( SubHeightC * sps_conf_win_bottom_offset + 1 ), inclusive.

The same clause continues on printed p. 106 (rendered and checked): "The value of SubWidthC *
( sps_conf_win_left_offset + sps_conf_win_right_offset ) shall be less than
sps_pic_width_max_in_luma_samples, and the value of SubHeightC * ( sps_conf_win_top_offset +
sps_conf_win_bottom_offset ) shall be less than sps_pic_height_max_in_luma_samples." / "When
sps_chroma_format_idc is not equal to 0, the corresponding specified samples of the two chroma arrays are
the samples having picture coordinates ( x / SubWidthC, y / SubHeightC ), where ( x, y ) are the picture
coordinates of the specified luma samples." / "NOTE 4 - The conformance cropping window offset parameters
are only applied at the output. All internal decoding processes are applied to the uncropped picture
size."

### PPS syntax (printed pp. 45-46)

```
pps_pic_width_in_luma_samples         ue(v)
pps_pic_height_in_luma_samples        ue(v)
pps_conformance_window_flag           u(1)
if( pps_conformance_window_flag ) {
    pps_conf_win_left_offset          ue(v)
    pps_conf_win_right_offset         ue(v)
    pps_conf_win_top_offset           ue(v)
    pps_conf_win_bottom_offset        ue(v)
}
```

### PPS semantics (printed pp. 119-120), verbatim

> pps_conformance_window_flag equal to 1 specifies that the conformance cropping window offset parameters
> follow next in the PPS. pps_conformance_window_flag equal to 0 specifies that the conformance cropping
> window offset parameters are not present in the PPS.
>
> When pps_pic_width_in_luma_samples is equal to sps_pic_width_max_in_luma_samples and
> pps_pic_height_in_luma_samples is equal to sps_pic_height_max_in_luma_samples, the value of
> pps_conformance_window_flag shall be equal to 0.
>
> pps_conf_win_left_offset, pps_conf_win_right_offset, pps_conf_win_top_offset, and
> pps_conf_win_bottom_offset specify the samples of the pictures in the CLVS that are output from the
> decoding process, in terms of a rectangular region specified in picture coordinates for output.
>
> When pps_conformance_window_flag is equal to 0, the following applies:
> - If pps_pic_width_in_luma_samples is equal to sps_pic_width_max_in_luma_samples and
>   pps_pic_height_in_luma_samples is equal to sps_pic_height_max_in_luma_samples, the values of
>   pps_conf_win_left_offset, pps_conf_win_right_offset, pps_conf_win_top_offset, and
>   pps_conf_win_bottom_offset are inferred to be equal to sps_conf_win_left_offset,
>   sps_conf_win_right_offset, sps_conf_win_top_offset, and sps_conf_win_bottom_offset, respectively.
> - Otherwise, the values of pps_conf_win_left_offset, pps_conf_win_right_offset, pps_conf_win_top_offset,
>   and pps_conf_win_bottom_offset are inferred to be equal to 0.
>
> The conformance cropping window contains the luma samples with horizontal picture coordinates from
> SubWidthC * pps_conf_win_left_offset to pps_pic_width_in_luma_samples -
> ( SubWidthC * pps_conf_win_right_offset + 1 ) and vertical picture coordinates from
> SubHeightC * pps_conf_win_top_offset to pps_pic_height_in_luma_samples -
> ( SubHeightC * pps_conf_win_bottom_offset + 1 ), inclusive.
>
> The value of SubWidthC * ( pps_conf_win_left_offset + pps_conf_win_right_offset ) shall be less than
> pps_pic_width_in_luma_samples, and the value of SubHeightC * ( pps_conf_win_top_offset +
> pps_conf_win_bottom_offset ) shall be less than pps_pic_height_in_luma_samples.
>
> NOTE 3 - The conformance cropping window offset parameters are only applied at the output. All
> internal decoding processes are applied to the uncropped picture size.
>
> Let ppsA and ppsB be any two PPSs referring to the same SPS. It is a requirement of bitstream
> conformance that, when ppsA and ppsB have the same values of pps_pic_width_in_luma_samples and
> pps_pic_height_in_luma_samples, respectively, ppsA and ppsB shall have the same values of
> pps_conf_win_left_offset, pps_conf_win_right_offset, pps_conf_win_top_offset, and
> pps_conf_win_bottom_offset, respectively.

**Consequence for a VVC reader:** the *effective* output crop of a picture is the **PPS** window (inferred
from the SPS window only when the picture is full-size; inferred 0 otherwise), so it can change with
reference picture resampling at the same SPS. An SPS-only crop parse is wrong for RPR streams.

## 5. What the crop rectangles are NOT (important for the display model)

- All three are **output** crops: "only applied at the output. All internal decoding processes are applied
  to the uncropped picture size" (H.265 NOTE 3, H.266 NOTE 3). Coded size != output size.
- They are expressed in luma samples **before** pixel-aspect correction. The sample aspect ratio (SPS VUI
  `aspect_ratio_idc` / `sar_width:sar_height`) applies to the cropped samples; none of these clauses
  states how crop interacts with SAR, `clap`, or `tkhd` (see `isobmff-14496-12-2012-visual.md` for the
  box-level statements and the DVB rule "nominal display size in square pixels after decoding, H.264/AVC
  cropping, and rescaling", `isobmff-tkhd.md` §2).
- H.264 has **no** default display window; the H.264 VUI carries no crop.
- Pan-scan SEI rectangles are defined *relative to the cropping rectangle* (H.264 D.2.4, equation on
  printed p. 362; H.265 D.3.4, printed p. 334: both reference `frame_crop_*` / `conf_win_*`); not transcribed.

## Not covered

- Order in which crop, SAR rescale, `clap` and `tkhd` presentation size are applied: no source states it.
- H.264 `vui_parameters()` fields other than the colour/aspect ones already in `h264-sps.md`.
- AFD / bar data (separate document: `afd-bar-data.md`).
