# Table 8: Resolutions for Full-screen Display from 25 Hz H.264/AVC SDTV IRD and supported by 25 Hz H.264/AVC HDTV IRD, 50 Hz H.264/AVC HDTV IRD, 25 Hz SVC HDTV IRD and 50 Hz SVC HDTV IRD

_Source: specs/etsi_ts_101_154_v02.10.01_av_coding_in_dvb.pdf — AVC/HEVC resolution+frame-rate constraint tables (8-12,19-21), AFD (B.11), AD_descriptor (E.1), DASH player conformance (L.1). Curated constraint/descriptor subset of a 357-page spec; full codec-constraint prose not transcribed._


|  Coded Picture |   |   | Displayed Picture Horizontal up sampling  |   |
| --- | --- | --- | --- | --- |
|  Luminance resolution (horizontal × vertical) | Source Aspect Ratio | Aspect_ratio_idc | 4:3 Monitors | 16:9 Monitors  |
|  720 × 576 | 4:3 | 2 | × 1 | × 3/4 (see note 1)  |
|   |  16:9 | 4 | × 4/3 (see note 2) | × 1  |
|  544 × 576 | 4:3 | 4 | × 4/3 | × 1 (see note 1)  |
|   |  16:9 | 12 | × 16/9 (see note 2) | × 4/3  |
|  480 × 576 | 4:3 | 10 | × 3/2 | × 9/8 (see note 1)  |
|   |  16:9 | 6 | × 2 (see note 2) | × 3/2  |
|  352 × 576 | 4:3 | 6 | × 2 | × 3/2 (see note 1)  |
|   |  16:9 | 8 | × 8/3 (see note 2) | × 2  |
|  352 × 288 | 4:3 | 2 | × 2 | × 3/2 (see note 1)  |
|   |  16:9 | 4 | × 8/3 (see note 2) (and vertical up sampling × 2) | × 2 (and vertical up sampling × 2)  |
|  NOTE 1: Up sampling of 4:3 pictures for display on a 16:9 monitor is optional in the IRD, as 16:9 monitors can be switched to operate in 4:3 mode. NOTE 2: The up sampling with this value is applied to the pixels of the 16:9 picture to be displayed on a 4:3 monitor. NOTE 3: It is recommended that luminance resolution of 704 pixels represents the "middle" of the picture, and that it be decoded to a 720 pixels full-screen display by placing 8 pixels of padding at each side. It is recommended that luminance resolutions, such as 352 pixels, that are natural scalings of 704 pixels, be upscaled to 704 pixels and padded as above. It is recommended that all other resolutions be scaled as indicated by the table above. Where this does not result in the expected 720 pixels full-screen display, it is recommended that the result of the scaling be clipped or padded symmetrically as required to produce a 720 pixels full-screen display. NOTE 4: The 16x9 picture comprises only the 702 pixels in the centre of the 720 pixel wide digital line. To avoid aspect ratio distortions and blanking or padding pixels appearing on the left and right of the screen, it is recommended that the remaining 18 pixels are not displayed (see EBU Technical Recommendation R92 [i.31].  |   |   |   |   |

# 5.6.3 30 Hz H.264/AVC SDTV IRD and Bitstream

# 5.6.3.0 General

This clause specifies the  $30\mathrm{Hz}$  H.264/AVC SDTV IRD and Bitstream. All specifications in clauses 5.5 and 5.6.1 shall apply. The specification in the remainder of this clause only applies to the  $30\mathrm{Hz}$  H.264/AVC SDTV IRD and Bitstream.

# 5.6.3.1 Colour Parameter Information

Encoding: The chromaticity co-ordinates of the ideal display, opto-electronic transfer characteristic of the source picture and matrix coefficients used in deriving luminance and chrominance signals from the red, green and blue primaries shall be explicitly signalled in the encoded H.264/AVC Bitstream by setting the appropriate values for each of the following 3 parameters in the VUI: colour_primaries, transfer_characteristics, and matrix_coefficients.

It is recommended that Recommendation ITU-R BT.1700 [25], Part A colorimetry is used for video of all other vertical resolutions in the H.264/AVC Bitstream, which is signalled by setting colour_primaries to the value 6, transfer_characteristics to the value 6 and matrix_coefficients to the value 6.

Decoding: The  $30\mathrm{Hz}$  H.264/AVC SDTV IRD shall be capable of decoding bitstreams with any allowed values of colour_primaries, transfer_characteristics and matrix_coefficients. It is recommended that appropriate processing be included for the accurate representation of pictures using Recommendation ITU-R BT.1700 [25], Part A colorimetry.

NOTE: Previous editions of the present document referenced SMPTE ST 170 colorimetry [i.9]. Recommendation ITU-R BT.1700 [25], Part A references SMPTE ST 170 [i.9].



## 5.6.3.2 Frame rate

Encoding: The frame rate shall be 24 000/1 001, 24, 30 000/1 001, 30 Hz. This shall be indicated in the VUI by setting time_scale and num_units_in_tick according to table 9. Time_scale and num_units_in_tick define the picture rate of the video.
