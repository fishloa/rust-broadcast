# Table 21: Resolutions for Full-screen Display from HEVC UHDTV IRD

_Source: specs/etsi_ts_101_154_v02.10.01_av_coding_in_dvb.pdf — AVC/HEVC resolution+frame-rate constraint tables (8-12,19-21), AFD (B.11), AD_descriptor (E.1), DASH player conformance (L.1). Curated constraint/descriptor subset of a 357-page spec; full codec-constraint prose not transcribed._


|  Luminance resolution |   | Scan (interlace/ progressive) | Aspect ratio |   | Example up-sampling for 3 840 x 2 160 display  |   |
| --- | --- | --- | --- | --- | --- | --- |
|  Horizontal | Vertical |   | Coded Frame | Aspect_ratio_idc | Horizontal | Vertical  |
|  3 840 | 2 160 | P | 16x9 | 1 | 1 | 1  |
|  2 880 | 2 160 | P | 16x9 | 14 | x 4/3 | x 1  |
|  3 200 | 1 800 | P | 16x9 | 1 | x 6/5 | x 6/5  |
|  2 560 | 1 440 | P | 16x9 | 1 | x 3/2 | x 3/2  |

NOTE: High Dynamic Range and/or High Frame Rates Bitstreams as defined in clauses 5.14.4 and 5.14.5 are not intended to be used with non-square pixel formats (with aspect_ratio_idc not equal to "1"). Therefore, the use of non-square pixel formats in HEVC UHDTV Bitstreams might complicate any upgrade to HDR and/or HFR.

## 5.14.3.3 Colour Parameter Information

Encoding: The chromaticity co-ordinates of the ideal display, opto-electronic transfer characteristic of the source picture and matrix coefficients used in deriving luminance and chrominance signals from the red, green and blue primaries shall be explicitly signalled in the encoded HEVC Bitstream by setting the appropriate values for each of the following 3 parameters in the VUI: colour_primaries, transfer_characteristics, and matrix_coeffs.

HEVC UHDTV Bitstreams shall use Recommendation ITU-R BT.709 [13] or Recommendation ITU-R BT.2020 [36] non-constant luminance colorimetry.

BT.709 [13] colorimetry usage is signalled by setting colour_primaries to the value 1, transfer_characteristics to the value 1 and matrix_coeffs to the value 1.

BT.2020 [36] non-constant luminance colorimetry usage is signalled by setting colour_primaries to the value 9, transfer_characteristics to the value 14 and matrix_coeffs to the value 9.

Decoding: HEVC UHDTV IRDs shall be capable of decoding bitstreams that use Recommendation ITU-R BT.709 [13] or Recommendation ITU-R BT.2020 [36] non-constant luminance colorimetry. It is recommended that appropriate processing be included for the accurate representation of pictures using Recommendation ITU-R BT.709 [13] colorimetry.

NOTE 1: The HEVC UHDTV IRDs might not include appropriate processing for the accurate representation of pictures using Recommendation ITU-R BT.2020 [36] non-constant luminance colorimetry. DVB anticipates that BT.2020 colour primaries will be used together with future versions of the present document. Equipment makers should consider including the capability to map BT.2020 colour primaries for BT.709 displays.

NOTE 2: Where IRDs implement a transformation of the colour space of the coded bitstream to match the capabilities of the display (e.g. from a Recommendation ITU-R BT.2020 non-constant luminance [36] bitstream to a Recommendation ITU-R BT.709 [13] display), it is recommended that the colour space conversion does not:

- impose a hard limit such that all bitstream colours outside of the gamut of the display are placed on the outer boundary of the display gamut;
- linearly scale the wider gamut of the bitstream to fit within the gamut of the display.

NOTE 3: High Dynamic Range and/or High Frame Rates Bitstreams as defined in clauses 5.14.4 and 5.14.5 are not intended to be used with BT. 709 colour primaries. Therefore, the use of BT. 709 colour primaries in HEVC UHDTV Bitstreams might complicate any upgrade to HDR and/or HFR.

## 5.14.3.4 Backwards Compatibility

Decoding: HEVC UHDTV IRDs shall be capable of decoding any bitstream that a HEVC HDTV IRD is required to decode and resulting in the same displayed pictures as the HEVC HDTV IRD, as described in clause 5.14.2.



# B.7 Auxiliary Data and H264/AVC, MVC Stereo or SVC video

# B.7.1 Coding

The Auxiliary Data is carried in the data as Supplemental Enhancement Information in H.264/AVC's "User data registered by Recommendation ITU-T T.35 [19] SEI message" syntactic element (see clauses D.8.5 and D.9.5 of ISO/IEC 14496-10 [16]).

Encoding: Support for the encoding of Auxiliary Data is optional.

Decoding: Support for the decoding of Auxiliary Data is optional.

# B.7.2 Syntax and Semantics

The Auxiliary Data (AFD, bar data, caption data and multi_region_disparity) is carried in the video elementary stream as Supplemental Enhancement Information in H.264/AVC's "User data registered by Recommendation ITU-T T.35 SEI message" syntactic element [19]. The syntax of Auxiliary Data is illustrated in table B.11.
