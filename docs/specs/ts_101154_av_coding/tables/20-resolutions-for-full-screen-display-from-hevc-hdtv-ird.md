# Table 20: Resolutions for Full-screen Display from HEVC HDTV IRD

_Source: specs/etsi_ts_101_154_v02.10.01_av_coding_in_dvb.pdf — AVC/HEVC resolution+frame-rate constraint tables (8-12,19-21), AFD (B.11), AD_descriptor (E.1), DASH player conformance (L.1). Curated constraint/descriptor subset of a 357-page spec; full codec-constraint prose not transcribed._


|  Luminance resolution |   | Scan (interface/ progressive) | Aspect ratio |   | Example up-sampling for 1 920 x 1 080 display  |   |
| --- | --- | --- | --- | --- | --- | --- |
|  Horizontal | Vertical |   | Coded Frame | Aspect_ratio_idc | Horizontal | Vertical  |
|  1 920 | 1 080 | I and P | 16:9 | 1 | x 1 | x 1  |
|  1 440 | 1 080 | I and P | 16:9 | 14 | x 4/3 | x 1  |
|  1 600 | 900 | P | 16:9 | 1 | x 6/5 | x 6/5  |
|  1 280 | 720 | P | 16:9 | 1 | x 3/2 | x 3/2  |
|  960 | 720 | P | 16:9 | 14 | x 2 | x 3/2  |
|  960 | 540 | P | 16:9 | 1 | x 2 | x 2  |

# 5.14.2.3 Colour Parameter Information

Encoding: The chromaticity co-ordinates of the ideal display, opto-electronic transfer characteristic of the source picture and matrix coefficients used in deriving luminance and chrominance signals from the red, green and blue primaries shall be explicitly signalled in the encoded HEVC HDTV Bitstream by setting the appropriate values for each of the following 3 parameters in the VUI: colour_primaries, transfer_characteristics, and matrix_coeffs.

HEVC HDTV Bitstreams shall use Recommendation ITU-R BT.709 [13] or optionally IEC 61966-2-4 [31] colorimetry for luminance resolutions shown in table 20. For other luminance resolutions, usage of Recommendation ITU-R BT.709 [13] should be used except for interlaced resolutions with heights of 576 or 480 lines where Recommendation ITU-R BT.601-7 [38] is appropriate.

NOTE: Interlaced Standard Definition resolutions are not currently defined for HEVC bitstreams and IRDs in the present document. Nonetheless, the above clause states that Recommendation ITU-R BT.601-7 [38] colorimetry should be used if encoders support those resolutions.

Recommendation ITU-R BT.709 [13] colorimetry usage is signalled by setting colour_primaries to the value 1, transfer_characteristics to the value 1 and matrix_coeffs to the value 1.

IEC 61966-2-4 [31] colorimetry usage is signalled by setting colour_primaries to the value 1, transfer_characteristics to the value 11 and matrix_coeffs to the value 1.

Decoding: HEVC HDTV IRDs shall be capable of decoding bitstreams that use Recommendation ITU-R BT.709 [13] colorimetry. It is recommended that appropriate processing be included for the accurate representation of pictures using Recommendation ITU-R BT.709 [13] colorimetry.

HEVC HDTV IRDs may be capable of decoding bitstreams that use IEC 61966-2-4 [31] colorimetry.

# 5.14.3 HEVC UHDTV IRDs and Bitstreams

# 5.14.3.0 General

This clause specifies the HEVC UHDTV IRDs and Bitstreams. All specifications in clause 5.14.1 shall apply. The specification in the remainder of this clause only applies to the HEVC UHDTV IRDs and Bitstreams.

# 5.14.3.1 Profile, tier and level

Encoding: In addition to the provisions set forth in Recommendation ITU-T H.265 / ISO/IEC 23008-2 [35], the following restrictions shall apply for the fields in the sequence parameter set:

bit_depth_luma_minus8 = 0 or 2

bit_depth_chroma_minus8 = bit_depth_luma_minus8

vui_parameters_present_flag = 1



$$
\text{sps\_extension\_present\_flag} = 0
$$

In addition to the provisions set forth in Recommendation ITU-T H.265 / ISO/IEC 23008-2 [35], the following restrictions shall apply for the fields in the profile_tier_level syntax structure in the sequence parameter set:

$$
\begin{array}{l}
\text{general\_tier\_flag} = 0 \\
\text{general\_profile\_idc} = 2 \text{ (Main 10 profile)}
\end{array}
$$

HEVC UHDTV Bitstreams shall obey the limits in Recommendation ITU-T H.265 / ISO/IEC 23008-2 [35], table A.1 and table A.2 associated with Level 5.1.

general_level_idc shall be less than or equal to 153 (level 5.1), unless the HEVC Bitstream is a HEVC temporal video sub-bitstream. In this case, sps_max_sub_layers_minus1 shall be greater than "0", sub_layer_level_present_flag[i] where 'i' is equal to temporal_id_max carried within the HEVC Video Descriptor shall be equal to "1", and sub_layer_level_idc[i] where 'i' is equal to temporal_id_max carried within the HEVC Video Descriptor shall be less than or equal to "153" (level 5.1).

It is recommended that bitstreams which are compliant with the Main profile set general_profile_compatibility_flag[1] to 1.

NOTE 1: In the Main 10 Profile, chroma_format_idc is equal to '1'.

Decoding: HEVC UHDTV IRDs shall support the decoding of Main 10 Profile and Main Profile, Main Tier, Level 5.1 bitstreams within the constraints of the present document.

If temporal extensions are added in future versions of the present document, general_level_idc may be greater than 153 (level 5.1). When sps_max_sub_layers_minus1 is greater than "0", IRDs may ignore general_level_idc and shall make use of the sub_layer_level_idc[i] syntax element, where 'i' is equal to temporal_id_max carried within the HEVC Video Descriptor, to determine whether a bitstream or sub-bitstream can be decoded.

HEVC UHDTV IRDs may ignore sequence parameter set extensions signalled by sps_extension_present_flag set to "1".

NOTE 2: HEVC UHDTV IRDs are not required to decode and display correctly HEVC Bitstreams or HEVC temporal video sub-bitstreams that do not obey the constraints and limits associated with the Main or Main 10 Profile, Main Tier, Level 5.1.

NOTE 3: High Dynamic Range and/or High Frame Rates Bitstreams as defined in clauses 5.14.4 and 5.14.5 are not intended to be used with a coding bit depth of 8 bits. Therefore, the use of a coding bit depth of 8 bits in HEVC UHDTV Bitstreams might complicate any upgrade to HDR and/or HFR.

## 5.14.3.2 Luminance resolution

Encoding: HEVC UHDTV encoders shall, as a minimum, represent video with the luminance resolutions shown in table 21 and the luminance resolutions shown in table 20, where luminance resolution is to be understood as the video resolution after conformance cropping. Pictures may be down-scaled and encoded at less than full size using the reciprocal of the scaling ratios shown in those two tables. Additional luminance resolutions may be supported, but they shall be square pixel formats indicated by aspect_ratio_idc equal to "1".

Where non 16:9 sources are re-formatted and encoded within a 16:9 frame, AFD/bar data defined in clause B.3 and default display window defined in clause 5.14.1.5.6 should be included within the bitstream to assist the IRD in displaying the content.

Decoding: HEVC UHDTV IRDs shall be capable of decoding pictures with luminance resolutions shown in table 21 and luminance resolutions shown in table 20, where luminance resolution is to be understood as the video resolution after conformance cropping. HEVC IRDs shall be able to reconstruct the image size to allow the decoded pictures to be displayed at full-screen size.
