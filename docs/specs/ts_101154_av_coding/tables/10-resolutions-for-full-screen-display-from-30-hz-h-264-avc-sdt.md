# Table 10: Resolutions for Full-screen Display from 30 Hz H.264/AVC SDTV IRD, and supported by 30 Hz H.264/AVC HDTV IRD, 60 Hz H.264/AVC HDTV IRD, 30 Hz SVC HDTV IRD and 60 Hz SVC HDTV IRD

_Source: specs/etsi_ts_101_154_v02.10.01_av_coding_in_dvb.pdf — AVC/HEVC resolution+frame-rate constraint tables (8-12,19-21), AFD (B.11), AD_descriptor (E.1), DASH player conformance (L.1). Curated constraint/descriptor subset of a 357-page spec; full codec-constraint prose not transcribed._


|  Coded Picture |   |   | Displayed Picture Horizontal up sampling  |   |
| --- | --- | --- | --- | --- |
|  Luminance resolution (horizontal × vertical) | Source Aspect Ratio | aspect_ratio_idc | 4:3 Monitors | 16:9 Monitors  |
|  720 × 480 | 4:3 | 3 | × 1 | × 3/4 (see note 1)  |
|   |  16:9 | 5 | × 4/3 (see note 2) | × 1  |
|  640 × 480 | 4:3 | 1 | × 9/8 | × 27/32 (see note 1)  |
|   |  16:9 | 14 | × 3/2 | × 9/8  |
|  544 × 480 | 4:3 | 5 | × 4/3 | × 1 (see note 1)  |
|   |  16:9 | 13 | × 16/9 (see note 2) | × 4/3  |
|  480 × 480 | 4:3 | 11 | × 3/2 | × 9/8 (see note 1)  |
|   |  16:9 | 7 | × 2 (see note 2) | × 3/2  |
|  352 × 480 | 4:3 | 7 | × 2 | × 3/2 (see note 1)  |
|   |  16:9 | 9 | × 8/3 (see note 2) | × 2  |



|  Coded Picture |   |   | Displayed Picture Horizontal up sampling  |   |
| --- | --- | --- | --- | --- |
|  Luminance resolution (horizontal × vertical) | Source Aspect Ratio | aspect_ratio_idc | 4:3 Monitors | 16:9 Monitors  |
|  352 × 240 | 4:3 16:9 | 3 5 | × 2 × 8/3 (see note 2) (and vertical up sampling × 2) | × 3/2 (see note 1) × 2 (and vertical up sampling × 2)  |
|  NOTE 1: Up sampling of 4:3 pictures for display on a 16:9 monitor is optional in the IRD, as 16:9 monitors can be switched to operate in 4:3 mode. NOTE 2: The up sampling with this value is applied to the pixels of the 16:9 picture to be displayed on a 4:3 monitor. NOTE 3: It is recommended that luminance resolution of 704 pixels represents the "middle" of the picture, and that it be decoded to a 720 pixels full-screen display by placing 8 pixels of padding at each side. It is recommended that luminance resolutions, such as 352 pixels, that are natural scalings of 704 pixels, be upscaled to 704 pixels and padded as above. It is recommended that all other resolutions be scaled as indicated by the table above. Where this does not result in the expected 720 pixels full-screen display, it is recommended that the result of the scaling be clipped or padded symmetrically as required to produce a 720 pixels full-screen display.  |   |   |   |   |

## 5.7 H.264/AVC HDTV IRDs and Bitstreams

## 5.7.1 Specifications common to all H.264/AVC HDTV IRDs and Bitstreams

## 5.7.1.0 Scope

The specification in this clause applies to the following IRDs and bitstreams:

- 25 Hz H.264/AVC HDTV IRD and Bitstream;
- 30 Hz H.264/AVC HDTV IRD and Bitstream;
- 50 Hz H.264/AVC HDTV IRD and Bitstream;
- 60 Hz H.264/AVC HDTV IRD and Bitstream.

## 5.7.1.1 Sequence Parameter Set and Picture Parameter Set

Encoding: In addition to the provisions set forth in Recommendation ITU-T H.264 / ISO/IEC 14496-10 [16], the following restrictions shall apply for the fields in the sequence parameter set:

profile_idc = 100 (High Profile [16])

constraint_set0_flag = 0

constraint_set1_flag = 0

constraint_set2_flag = 0

constraint_set3_flag = 0

gaps_in_frame_num_value_allowed_flag = 0 (gaps not allowed)

vui_parameters_present_flag = 1

## 5.7.1.2 Aspect ratio

Encoding: The source aspect ratio in H.264/AVC HDTV Bitstreams shall be 16:9.

The source aspect ratio information shall be derived from the aspect_ratio_idc value in the Video Usability Information (see values of aspect_ratio_idc in Recommendation ITU-T H.264 / ISO/IEC 14496-10 [16], table E-1).



The frame cropping information in the Sequence Parameter Set may be used when appropriate.

Decoding: H.264/AVC HDTV IRDs shall support decoding and displaying H.264/AVC HDTV Bitstreams with the values of aspect_ratio_idc as specified in table 11.

The source aspect ratio information shall be derived from the pic_height_in_map.units_minus1 and the pic_width_in_mbs_minus1 and the frame cropping information coded in the Sequence Parameter Set as well as the sample aspect ratio encoded with the aspect_ratio_idc value in the Video Usability Information (see values of aspect_ratio_idc in Recommendation ITU-T H.264 / ISO/IEC 14496-10 [16], table E-1).

H.264/AVC HDTV IRDs shall support frame cropping.

## 5.7.1.3 Colour Parameter Information

Encoding: The chromaticity co-ordinates of the ideal display, opto-electronic transfer characteristic of the source picture and matrix coefficients used in deriving luminance and chrominance signals from the red, green and blue primaries shall be explicitly signalled in the encoded H.264/AVC HDTV Bitstream by setting the appropriate values for each of the following 3 parameters in the VUI: colour_primaries, transfer_characteristics, and matrix_coefficients.

It is recommended that H.264/AVC HDTV bitstreams use either Recommendation ITU-R BT.709 [13] or IEC 61966-2-4 [31] colorimetry.

BT.709 [13] colorimetry usage is signalled by setting colour_primaries to the value 1, transfer_characteristics to the value 1 and matrix_coefficients to the value 1.

IEC 61966-2-4 [31] colorimetry usage is signalled by setting colour_primaries to the value 1, transfer_characteristics to the value 11 and matrix_coefficients to the value 1.

Decoding: H.264/AVC HDTV IRDs shall be capable of decoding bitstreams with any allowed values of colour_primaries, transfer_characteristics and matrix_coefficients. It is recommended that appropriate processing be included for the accurate representation of pictures using Recommendation ITU-R BT.709 [13] colorimetry.

H.264/AVC HDTV IRDs may be capable of decoding bitstreams that use IEC 61966-2-4 [31] colorimetry.

NOTE: The H.264/AVC HDTV IRD might not include appropriate processing for the accurate representation of pictures that use IEC 61966-2-4 [31] colorimetry.

## 5.7.1.4 Luminance resolution

Encoding: H.264/AVC HDTV Bitstreams shall represent video with luminance resolutions as shown in table 11. Non full-screen pictures may be encoded for display at less than full-size (when using one of the standard up-conversion ratios at the H.264/AVC HDTV IRD).

Decoding: H.264/AVC HDTV IRDs shall be capable of decoding pictures with luminance resolutions as shown in table 11 and applying up sampling to allow the decoded pictures to be displayed at full-screen size.
