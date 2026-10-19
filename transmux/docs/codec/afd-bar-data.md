# Active Format Description (AFD) and Bar Data

Primary-source transcription for display geometry (AFD phase, `docs/display-geometry/DESIGN.md`).
**Every table, syntax and quote below was checked against the rendered PDF page** (all five sources), not
only the markdown; the markdown of SMPTE ST 2016-1 Table 1 is garbled (two columns collapsed) and must NOT
be used. Page numbers are printed page numbers.

| Source | Edition | Private PDF |
|---|---|---|
| SMPTE ST 2016-1 | 2009 | `private/specs/smpte_st2016-1_2009_afd_bar_data.pdf` |
| SMPTE ST 2016-3 | 2009 | `private/specs/smpte_st2016-3_2009_afd_bar_data_anc.pdf` |
| ETSI TS 101 154 Annex B (normative) | V2.10.1 (2026-05) | `private/specs/etsi_ts_101_154_v02.10.01_av_coding_in_dvb.pdf` |
| ATSC A/53 Part 4 §6.2 | 2009 (7 Aug 2009) | `private/specs/atsc_a53_part4_2009_mpeg2_video.pdf` |
| ITU-T H.264 / H.265 SEI T.35 syntax | (V15) 08/2024 / (V11) 01/2026 | `itu_t_h264_avc.pdf` / `itu_t_h265_hevc.pdf` |

## 1. AFD codes — SMPTE ST 2016-1:2009 §5, Table 1 (printed p. 6)

AFD is "a 4-bit code describing a video picture in terms of the aspect ratio and other characteristics of
the active image within the coded frame" (§5, p. 5). The code is read **together with the aspect ratio of
the coded frame** (NOTE, p. 5). Table 1, as rendered (4:3 coded frame | 16:9 coded frame | code a3 a2 a1 a0):

| AFD code | In a 4:3 coded frame | In a 16:9 coded frame |
|---|---|---|
| `0000` | Undefined (see below) | Undefined (see below) |
| `0001` | Reserved | Reserved |
| `0010` | Letterbox 16:9 image, at top of the coded frame (see note 1) | Full frame 16:9 image, the same as the coded frame (see notes 1 and 2) |
| `0011` | Letterbox 14:9 image, at top of the coded frame (see note 1) | Pillarbox 14:9 image, horizontally centered in the coded frame (see notes 1 and 3) |
| `0100` | Letterbox image with an aspect ratio greater than 16:9, vertically centered in the coded frame (see note 1) | Letterbox image with an aspect ratio greater than 16:9, vertically centered in the coded frame (see note 1) |
| `0101` | Reserved | Reserved |
| `0110` | Reserved | Reserved |
| `0111` | Reserved | Reserved |
| `1000` | Full frame 4:3 image, the same as the coded frame | Full frame 16:9 image, the same as the coded frame |
| `1001` | Full frame 4:3 image, the same as the coded frame (see note 4) | Pillarbox 4:3 image, horizontally centered in the coded frame |
| `1010` | Letterbox 16:9 image, vertically centered in the coded frame with all image areas protected (see note 5) | Full frame 16:9 image, with all image areas protected (see note 5) |
| `1011` | Letterbox 14:9 image, vertically centered in the coded frame | Pillarbox 14:9 image, horizontally centered in the coded frame |
| `1100` | Reserved | Reserved |
| `1101` | Full frame 4:3 image, with alternative 14:9 center (see note 6) | Pillarbox 4:3 image, with alternative 14:9 center (see note 6) |
| `1110` | Letterbox 16:9 image, with alternative 14:9 center (see note 6) | Full frame 16:9 image, with alternative 14:9 center (see note 6) |
| `1111` | Letterbox 16:9 image, with alternative 4:3 center (see note 6) | Full frame 16:9 image, with alternative 4:3 center (see note 6) |

Text under Table 1 (p. 6), verbatim:

> If Bar Data is not present (see § 9.2.1), AFD '0000', indicates that exact information is not available
> and the active image should be assumed to be the same as the coded frame. AFD '0000', when accompanied
> by Bar Data, signals that the active image's aspect ratio is narrower than 16:9, but is not 4:3 or 14:9.
> As the exact aspect ratio cannot be conveyed by AFD alone, wherever possible, AFD '0000' should be
> accompanied by Bar Data to define the exact vertical or horizontal extent of the active image.
>
> AFD code '0100' signals that the active image aspect ratio is wider than 16:9. As the exact aspect ratio
> cannot be conveyed by AFD alone, wherever possible, AFD '0100' should be accompanied by Bar Data to
> define the exact vertical extent of the active image.
>
> AFD codes may be applicable to 4:3 coded frames, 16:9 coded frames, or both and may have different
> meanings for each. ... The AFD codes may also be used with coded frames of other aspect ratios but such
> use is outside the scope of this standard.

Notes (pp. 6-7), verbatim where they carry rules:

> 1 In determining AFD codes to originate, users are cautioned that emission systems in some areas of the
> world may not support some or all of AFD codes '0001' through '0111'.
> 2 In a 16:9 coded frame, AFD code '0010' represents the same image display as AFD code '1000'. AFD code
> '1000' is the preferred coding for a full frame 16:9 image. AFD code '0010' is not used in North America.
> 3 In a 16:9 coded frame, AFD code '0011' represents the same image display as AFD code '1011'. AFD code
> '1011' is the preferred coding for a horizontally centered 14:9 pillarbox image. AFD code '0011' is not
> used in North America.
> 4 In a 4:3 coded frame, AFD code '1001' represents the same image display as AFD code '1000'. AFD code
> '1000' is the preferred coding for a full frame 4:3 image.
> 5 AFD code '1010' is intended for use with images that may not fill the frame vertically but where all
> areas of the image should be displayed (e.g. an image where cropping is not permitted) and which
> therefore can only be adequately displayed in letterbox format.
> 6 The "alternative center" image refers to an area of essential picture information. The areas outside
> this area (shown gray in the illustrations in figures 1, 2, and 3) may optionally be cropped without
> significant loss to the viewer.
> 7 Aspect ratio information for the coded frame is not part of the AFD code and it is necessary to code
> this aspect ratio information separately - see § 5.2. The AFD and aspect ratio codes may be transported
> together as part of a larger data structure.

### §5.2 Aspect ratio of the coded frame (p. 10), verbatim

> The aspect ratio of the coded frame shall be signaled with a single bit (see § 9.1). When the aspect
> ratio is 16:9, the bit shall be set to logical one. When the aspect ratio is 4:3, the bit shall be set to
> logical zero.

### §5.1 Table 2 — Video format information (p. 10)

| Format | Applicable Production Standard | Pixels x Lines | Coded Pixels | First Field | Second Field | Frame |
|---|---|---|---|---|---|---|
| 480 Interlaced | SMPTE 125M | 720 x 480 | 0 - 719 | 23 - 262 | 286 - 525 | - |
| 480 Progressive | SMPTE 293M | 720 x 480 | 0 - 719 | - | - | 45 - 524 |
| 576 Interlaced | ITU-R BT.656 | 720 x 576 | 0 - 719 | 23 - 310 | 336 - 623 | - |
| 576 Progressive | ITU-R BT.1358 | 720 x 576 | 0 - 719 | - | - | 45 - 620 |
| 720 Progressive | SMPTE 296M | 1280 x 720 | 0 - 1279 | - | - | 26 - 745 |
| 1080 Interlaced | SMPTE 274M | 1920 x 1080 | 0 - 1919 | 21 - 560 | 584 - 1123 | - |
| 1080 Progressive | SMPTE 274M | 1920 x 1080 | 0 - 1919 | - | - | 42 - 1121 |

> NOTE - Table 2 is based on a subset of the formats in SMPTE RP 202, Table 1.
> In some compression systems, 704 pixels may be coded for the 480-line and 576-line formats; in this
> standard, 720 pixels shall be assumed for AFD and Bar Data calculations.
> If AFD calculations result in non-integer values for lines or pixel numbers, the values used should be
> rounded to the nearest whole number.

## 2. Bar Data — SMPTE ST 2016-1:2009 §6 and §9.2

§6 (p. 10): Bar Data "should be present whenever the active image does not extend to the full height or
width of the coded frame and AFD alone is insufficient to describe the extent of the image. Bar Data is
constrained (below) to be signaled in pairs, either top and bottom bars or left and right bars, but not both
pairs at once. Bars may be unequal in size. One bar of a pair may be zero width or height." AFD `0100` should
be accompanied by Top/Bottom Bar Data; AFD `0000` + Bar Data defines the width of an image narrower than 16:9
but not 4:3 or 14:9.

### Table 3 — Bar Data element syntax (p. 11)

```
Bar Data flags() {
    top_bar_flag      1  bslbf
    bottom_bar_flag   1  bslbf
    left_bar_flag     1  bslbf
    right_bar_flag    1  bslbf
}
Top bar() {     if (top_bar_flag == 1)     { marker_bits 2 '11'; line_number_end_of_top_bar        14 uimsbf } }
Bottom bar() {  if (bottom_bar_flag == 1)  { marker_bits 2 '11'; line_number_start_of_bottom_bar   14 uimsbf } }
Left bar() {    if (left_bar_flag == 1)    { marker_bits 2 '11'; pixel_number_end_of_left_bar      14 uimsbf } }
Right bar() {   if (right_bar_flag == 1)   { marker_bits 2 '11'; pixel_number_start_of_right_bar   14 uimsbf } }
```

Flag rules (§6.1.1-6.1.4, pp. 11-12): `top_bar_flag` — "If left_bar_flag is '1', this flag shall be set to
'0'"; `bottom_bar_flag` "shall have the same value as top_bar_flag"; `left_bar_flag` — "If top_bar_flag is '1',
this flag shall be set to '0'"; `right_bar_flag` "shall have the same value as left_bar_flag".

Values (§6.2-6.5, p. 12), each a 14-bit unsigned integer: `line_number_end_of_top_bar` = "the last line of a
horizontal letterbox bar area at the top of the reconstructed frame"; `line_number_start_of_bottom_bar` = "the
first line of a horizontal letterbox bar area at the bottom"; `pixel_number_end_of_left_bar` = "the last
horizontal luminance sample of a vertical pillarbox bar area at the left side"; `pixel_number_start_of_right_bar`
= "the first horizontal luma sample of a vertical pillarbox bar area at the right side". Lines numbered per
Table 2; pixels "numbered from zero, starting with the leftmost pixel".

§6.6 (p. 12): "For equipment generating Bar Data, when the Bar Data is present, line or pixel numbers shall be
signaled on a frame by frame basis, with one set of Bar Data numbers per frame. For interlaced video frames,
the line number for each bar may correspond to a line from either the first or the second field."

### Coding for transport — §9.1, §9.2 (pp. 13-14)

AFD information byte, Table 4 (p. 13): one byte; b7 `'0'` (reserved); b6-b3 = AFD code data bits `a3 a2 a1 a0`;
b2 = aspect ratio of the coded frame (`'1'` = 16:9, `'0'` = 4:3); b1-b0 `'00'` (reserved). "NOTE - The bit
positions for the AFD information byte are consistent with practice in the U.K."

Bar Data flags byte, Table 5 (p. 14): B7 top bar flag, B6 bottom bar flag, B5 left bar flag, B4 right bar flag
(`'1'` = that Bar Data present), B3-b0 reserved `'0000'`. "NOTE - In ATSC standard A/53, the four lsb's of the
byte coding the Bar Data flags are set to logical one in accordance with MPEG practice." "When the Bar Data flag
bits are all set to logical zero, there is no Bar Data present and the remaining 4 bytes of Bar Data should be
ignored by receiving equipment."

Bar Data values (§9.2.2, p. 14): two 16-bit fields, each "most significant bit in bit 'b7' of the first byte";
each "consist of fixed marker bits '11' in the two most significant positions followed by the line number /
pixel number information"; "If no valid Bar Data values ... are to be carried, then all bits of this field,
including the marker bits, shall be set to logical zero." Value 1 corresponds to the first (most significant)
flag bit set to one, Value 2 to the next.

### §7, §8, §9.3 (pp. 13-14)

§7: when a signal is converted from one format to another, the converting device "shall, if necessary, process
the associated AFD and Bar Data so that it is correct for the output video format, replacing the original";
Bar Data "does not need to be changed for a video compression format that uses horizontal sub-sampling";
pass/switch/record equipment leaves them unchanged.

§8: "Any combination of AFD and Bar Data may be present in a video signal. When AFD and Bar Data are present
together, AFD is used by DTV receivers in preference to Bar Data, except in the case of AFD '0000' and '0100'
where Bar Data is used in concert with AFD as described in § 6. In the absence of Bar Data or AFD information,
DTV receivers interpret the active format as being the same as the coded frame." Pan-Scan: respond to
Pan-Scan *or* to Bar Data/AFD, "but not to both sets of information simultaneously".

§9.3: "AFD and Bar Data, when coded as ancillary data, shall be signaled on a frame by frame basis"; first and
second field of a frame carry identical values.

## 3. AFD/Bar Data in a SMPTE 291M ANC packet — SMPTE ST 2016-3:2009 §4-5 (pp. 4-5)

> Each data packet shall comply with the format defined in SMPTE 291M for a type 2 ANC packet. ... The DID
> word shall be set to the value 41h. The SDID word shall be set to the value of 05h. DC is a count of the
> number of words in the UDW and shall be set to a fixed value of 8 (08h).

"Six bytes of AFD and Bar Data, as well as two reserved bytes" in eight 10-bit user data words; the data is in
bits b7-b0, "Bit b8 is even parity for bits b7 through b0 of the 10-bit data word, and bit b9 equals the
complement of bit b8." Table 1 — ANC data payload (p. 4), bits b7..b0 of each UDW:

| UDW | Function | b7 | b6 | b5 | b4 | b3 | b2 | b1 | b0 |
|---|---|---|---|---|---|---|---|---|---|
| 1 | AFD | '0' | a3 | a2 | a1 | a0 | AR | '0' | '0' |
| 2 | Reserved | '0' | '0' | '0' | '0' | '0' | '0' | '0' | '0' |
| 3 | Reserved | '0' | '0' | '0' | '0' | '0' | '0' | '0' | '0' |
| 4 | Bar Data Flags | Top | Bot. | Left | Right | '0' | '0' | '0' | '0' |
| 5 | Bar Data Value 1 | f15 | f14 | f13 | f12 | f11 | f10 | f9 | f8 |
| 6 | | f7 | f6 | f5 | f4 | f3 | f2 | f1 | f0 |
| 7 | Bar Data Value 2 | s15 | s14 | s13 | s12 | s11 | s10 | s9 | s8 |
| 8 | | s7 | s6 | s5 | s4 | s3 | s2 | s1 | s0 |

(each word also carries b9 = NOT b8 and b8 = P, the even parity of b7..b0). "NOTE - In ATSC standard A/53,
the byte coding the Bar Data flags differs from the byte in Table 1 in that it sets the unused bits to logical
one, following MPEG practice."

Location (§5, p. 5): "The AFD/Bar Data packet shall be located in the VANC space of every frame. For
interlaced video systems, it shall be located in the first field and in the second field. ... Multiple AFD and
Bar Data packets per interlaced field, per progressive frame, or per frame segment ... shall not occur ... The
AFD/Bar Data packet shall be associated with the video frame that is being described. When the ANC packets
defined in this standard are carried in a high definition signal, they shall be carried in the Y stream."

## 4. AFD / bar data in the video elementary stream

### 4.1 Common structures — ETSI TS 101 154 V2.10.1 Annex B.1-B.2 (printed pp. 235-236)

"Certain picture-related types of data may be carried in the video elementary stream. While the 'outer
wrapper' is codec dependent, the basic data structures are shared in common between MPEG-2, H.264/AVC, HEVC,
and VC-1." "Transmission of these descriptions and use of these descriptions by a receiver are both optional."

Table B.1 — Values for user_identifier (p. 235): `0x47413934` ('GA94') -> `DVB1_data()`; `0x44544731` ('DTG1') ->
`afd_data()`. ("Values of the user_identifier are registered with SMPTE-RA.")

Table B.2 — `afd_data()` syntax (p. 235):

```
afd_data() {
    '0'                         1  bslbf
    active_format_flag          1  bslbf
    reserved (set to '00 0001') 6  bslbf
    if (active_format_flag == 1) {
        reserved (set to '1111')  4  bslbf
        active_format             4  bslbf
    }
}
```

Table B.3 — `DVB1_data()` syntax: `user_data_type_code` (8, uimsbf) then `user_data_type_structure()`.
Table B.4 — user_data_type_code (p. 236): `0x00`-`0x02` Industry Reserved; `0x03` cc_data(); `0x04`, `0x05`
Industry Reserved; `0x06` **bar_data()**; `0x07` multi_region_disparity(); `0x08` Industry Reserved; `0x09`
ST2094-10_data(); `0x0A`-`0xFF` Industry Reserved.

### 4.2 `active_format` — Table B.5 (p. 237)

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

"AFD 0000 indicates that information is not available and is undefined. Unless bar data is available,
receivers and video equipment should interpret the active format as being the same as the coded frame." (p. 237)
The AFD "does not describe the aspect ratio of the coded frame (as this is described elsewhere in the MPEG-2,
H264/AVC, HEVC, or SMPTE VC-1 video syntax)" (B.3.0, p. 236).

Source aspect ratio for MPEG-2 (B.3.1, pp. 236-237): "If sequence_display_extension() is not present: source
aspect ratio = DAR. If sequence_display_extension() is present: source aspect ratio = DAR x
display_horizontal_size / display_vertical_size x vertical_size / horizontal_size." For H.264/AVC (B.3.2) the
receiver uses "the picture width, picture height, frame cropping information, and sample aspect ratio" from the
SPS / VUI; HEVC (B.3.6) likewise uses SPS size/shape and `aspect_ratio_idc`.

### 4.3 Bar data — Table B.7 (TS 101 154 p. 240) = A/53 Table 6.11 (printed p. 14)

```
bar_data() {
    top_bar_flag        1  bslbf
    bottom_bar_flag     1  bslbf
    left_bar_flag       1  bslbf
    right_bar_flag      1  bslbf
    reserved (set to '1111')  4
    if (top_bar_flag)    { marker_bits ('11') 2; line_number_end_of_top_bar        14 uimsbf }
    if (bottom_bar_flag) { marker_bits ('11') 2; line_number_start_of_bottom_bar   14 uimsbf }
    if (left_bar_flag)   { marker_bits ('11') 2; pixel_number_end_of_left_bar      14 uimsbf }
    if (right_bar_flag)  { marker_bits ('11') 2; pixel_number_start_of_right_bar   14 uimsbf }
}
```

Both documents use the same flag rules as ST 2016-1 §6.1 and the same value definitions (TS 101 154 p. 241 and A/53 pp. 14-15 rendered and compared). TS 101 154 also lists `additional_bar_data` — "Reserved for future DVB definition". A/53 names the
2-bit field `one_bits` (value '11'), TS 101 154 `marker_bits`. Line numbering: TS 101 154 Table B.8 / A/53
Table 6.12: 480 interlaced 4:3 SMPTE 125M; 480 interlaced 16:9 SMPTE 267M; 480 progressive SMPTE 293M; 720
progressive SMPTE 296M; 1080 interlaced and progressive SMPTE 274M (the 576-line rows of ST 2016-1 Table 2 are
not in these two tables). TS 101 154 B.4.0 NOTE 2: for HEVC "decimated" sub-rasters (clauses 5.14.2.2,
5.14.3.2) "AFD/Bar Data might not work properly ... the HEVC Default Display Window ... can be used as an
alternative"; B.4.2: "AFD data may not always exactly match bar data because AFD primarily deals with 4:3,
14:9, and 16:9 aspect ratios while bar data can represent nearly any aspect ratio."

### 4.4 MPEG-2 video — TS 101 154 B.6 (p. 243), A/53 Part 4 §6.2.2-6.2.4 (pp. 12-17)

Carried at picture level in `extension_and_user_data(2)`: `user_data_start_code` (32, = `0x0000 01B2`),
`user_identifier` (32), `user_structure()` (Table B.10 / A/53 Tables 6.7-6.9). "More than one picture user
data construct may follow any given picture header. However, no more than one picture user data construct using
the same user_identifier or user_data_type_code shall follow any given picture header." "Receiving devices are
expected to silently discard any unrecognized video user data."

**Persistence (the only normative persistence rules in these sources):**
- Bar data (TS 101 154 B.6.1, A/53 §6.2.3.2): "After any sequence_header() such bar data shall appear before
  the next picture_data() within extension_and_user_data(2). After introduction, such bar data shall remain
  in effect until: 1) the next sequence_header(); or 2) extension_and_user_data(2) containing a bar_data()
  structure which contains new bar data; or 3) extension_and_user_data(2) containing AFD per clause B.3.4."
  "After any sequence_header(), unless AFD data is present specifying otherwise, the absence of bar data shall
  indicate that the rectangular picture area containing useful information extends to the full height and
  width of the coded frame."
- AFD (A/53 §6.2.4, p. 15): "After any sequence_header() the default aspect ratio of the area of interest
  shall be that signaled by the parameters in the sequence_header() and sequence_display_extension()
  structures. After any sequence_header() the AFD, when present, shall appear before the next picture_data().
  After introduction, such an AFD shall remain in effect until the next sequence_header() or until a new AFD
  is introduced."

A/53 Table 6.13 (p. 16) `afd_data()` syntax is byte-identical to TS 101 154 Table B.2 (`zero`, `active_format_flag`,
`reserved` '00 0001', then `reserved` '1111' + `active_format`). **A/53 Table 6.14 (p. 17) differs from the DVB
table:** for `active_format` ('0000' undefined; '0001' reserved; **'0010'-'0011' "Not recommended"**; '0100'
aspect ratio greater than 16:9; '0101'-'0111' reserved; '1000' 4:3 / 16:9 full frame; '1001' 4:3 full frame /
4:3 pillarbox; '1010' 16:9 letterbox / 16:9 full frame; '1011' 14:9 letterbox / 14:9 pillarbox; '1100'
reserved; '1101'-'1111' as ST 2016-1) and the text "Use of either '0010' or '0011' is not recommended in the
ATSC television system. Values '0001', '0101' through '0111', and '1100' are reserved." A/53 §6.2.5: "When AFD
and bar data are present together, AFD should be used in preference to bar data, except in the cases of AFD
'0000' and '0100'."

### 4.5 H.264/AVC and HEVC — TS 101 154 B.7, B.8a (pp. 244-246)

Carried as the SEI message "User data registered by Recommendation ITU-T T.35" (H.264 clauses D.8.5/D.9.5
per TS 101 154; the H.264 edition checked here numbers them D.1.6 syntax / D.2.6 semantics). Table B.11
(p. 244):

```
user_data_registered_itu_t_t35(payloadSize) {
    itu_t_t35_country_code        b(8)    0xB5
    itu_t_t35_provider_code       u(16)   0x0031
    user_identifier               f(32)
    user_structure()
}
```

`user_identifier` "is used in addition to country and provider codes to definitively identify this as
Auxiliary Data" (NOTE, p. 244). HEVC (B.8a.1, p. 245): "When the 'User data registered by Recommendation
ITU-T T.35 SEI message' is present in an HEVC Bitstream, it shall be a prefix SEI message (i.e. nal_unit_type
shall be equal to PREFIX_SEI_NUT)"; syntax "shall be the same as for H.264/AVC" (B.8a.2). Support for encoding
and decoding is optional. MVC: AFD, bar data and cc data "shall be the same for both base and dependent view
bitstreams and may be transmitted in the MVC Stereo Base view bitstream" (B.7.3).

The codec-side SEI container, verified on the rendered pages:
- H.264 `sei_payload()` (D.1.1, p. 333): `payloadType == 4` -> `user_data_registered_itu_t_t35( payloadSize )`
  (and 5 -> `user_data_unregistered`, 6 -> `recovery_point`).
- H.265 `sei_payload()` (D.2.1, p. 290): `PREFIX_SEI_NUT`, `payloadType == 4` ->
  `user_data_registered_itu_t_t35( payloadSize )`.
- Syntax, identical in both (H.264 D.1.6 p. 339; H.265 D.2.6 p. 297):

```
user_data_registered_itu_t_t35( payloadSize ) {
    itu_t_t35_country_code                      b(8)
    if( itu_t_t35_country_code != 0xFF )
        i = 1
    else {
        itu_t_t35_country_code_extension_byte   b(8)
        i = 2
    }
    do {
        itu_t_t35_payload_byte                  b(8)
        i++
    } while( i < payloadSize )
}
```

- Semantics (H.264 D.2.6 p. 362; H.265 D.3.6 p. 334): "The ITU-T T.35 terminal provider code and terminal
  provider oriented code shall be contained in the first one or more bytes of the itu_t_t35_payload_byte, in
  the format specified by the Administration that issued the terminal provider code. Any remaining
  itu_t_t35_payload_byte data shall be data having syntax and semantics as specified by the entity identified
  by the ITU-T T.35 country code and terminal provider code."
  So for AFD: country `0xB5`, provider `0x0031`, then the 32-bit `user_identifier` and `afd_data()` / the
  `DVB1_data()` wrapper.

### 4.6 HEVC interplay with the Default Display Window — TS 101 154 (not page-rendered here)

Text-layer only (md lines ~5507-5569, v2.10.1): "HEVC IRDs shall support conformance cropping. Support of
Default Display Window VUI is strongly recommended. HEVC IRDs may ignore AFD/Bar Data signalling." and "If both
annex B AFD/Bar Data and Default Display Window are signalled in the HEVC Bitstream for the purpose of
signalling the active area, they are expected to describe the same active area." These clauses (5.14.1.5.x) were
NOT rendered or transcribed in full; read them from the PDF before relying on them.

## 5. WSS relationship (informative) — TS 101 154 B.9

AFD/bar data are "a super-set of the aspect ratio signalling specified in ETSI EN 300 294". Table B.13 maps
source aspect ratio + `active_format` to WSS codes (4:3: 1001->full format 4:3 `0001`; 1011->box 14:9 centre
`1000`; 0011->box 14:9 top `0100`; 1010->box 16:9 centre `1101`; 0010->box 16:9 top `0010`; 0100->box > 16:9
centre `1011`; 1101->full format 4:3 shoot-and-protect 14:9 `0111`; 16:9: 1010->full format 16:9 anamorphic
`1110`). Not page-rendered in this pass.

## Gaps (read before implementing)

1. **Persistence of AFD in H.264 and HEVC is NOT stated** in TS 101 154 Annex B (B.7, B.8a say nothing about
   how long an AFD/bar data SEI applies), nor in ST 2016-1/-3 (which are per-frame, "frame by frame basis").
   The only persistence rules are the MPEG-2 ones above ("until the next sequence_header() or a new AFD";
   the VC-1 rule in TS 101 154 B.8.1 is equivalent) and they are stated in terms of `sequence_header()`, which
   has no H.264/HEVC analogue. Any "sticky until replaced, regardless of IDR/sequence boundaries" model for
   H.264/HEVC is therefore an implementation choice, not a citation. ATSC A/72 (H.264 in ATSC) or A/341
   might state it; neither is in the private repo.
2. **CTA-CEB16-B** (receiver recommended practice), cited by both TS 101 154 (B.4.1) and A/53 (6.2.3.2.1,
   6.2.4.3) for how a receiver should *act* on AFD/bar data, is not available (paywalled). Receiver behaviour
   (crop vs letterbox decisions) is therefore not specified by any source we hold; only the signalled value is.
3. AFD in HEVC: TS 101 154 allows IRDs to ignore it; interaction with Default Display Window only partly read
   (§4.6).
4. The TS 101 154 `GA94` `DVB1_data()` / `cc_data()` structures (captions) are out of scope here.
