# Table 11: Resolutions for Full-screen Display from H.264/AVC HDTV IRD and SVC HDTV IRD

_Source: specs/etsi_ts_101_154_v02.10.01_av_coding_in_dvb.pdf — AVC/HEVC resolution+frame-rate constraint tables (8-12,19-21), AFD (B.11), AD_descriptor (E.1), DASH player conformance (L.1). Curated constraint/descriptor subset of a 357-page spec; full codec-constraint prose not transcribed._


|  Coded Picture  |   |   |   |
| --- | --- | --- | --- |
|  Luminance resolution (horizontal × vertical) | Source Aspect Ratio | aspect_ratio_idc | 16:9 Monitors Horizontal up sampling  |
|  1 920 × 1 080 | 16:9 | 1 | × 1  |
|  1 440 × 1 080 | 16:9 | 14 | × 4/3  |
|  1 280 × 1 080 | 16:9 | 15 | × 3/2  |
|  960 × 1 080 | 16:9 | 16 | × 2  |
|  1 280 × 720 | 16:9 | 1 | × 1  |
|  960 × 720 | 16:9 | 14 | × 4/3  |
|  640 × 720 | 16:9 | 16 | × 2  |



## 5.7.2 25 Hz H.264/AVC HDTV IRD and Bitstream

### 5.7.2.0 General

This clause specifies the 25 Hz H.264/AVC HDTV IRD and Bitstream. All specifications in clauses 5.5 and 5.7.1 shall apply. The specification in the remainder of this clause only applies to the 25 Hz H.264/AVC HDTV IRD and Bitstream.

### 5.7.2.1 Profile and level

Encoding: 25 Hz H.264/AVC HDTV Bitstreams shall comply with the High Profile Level 4 restrictions, as specified in Recommendation ITU-T H.264 / ISO/IEC 14496-10 [16].

The value of level_idc shall be equal to 30, 31, 32 or 40.

Decoding: 25 Hz H.264/AVC HDTV IRDs shall support the decoding of High Profile Level 4 bitstreams. This requirement includes support for High Profile and levels 3 to 4. Support for profiles and levels other than High Profile, Level 3 to 4 is optional. If the 25 Hz H.264/AVC HDTV IRD encounters an extension which it cannot decode, it shall discard the following data until the next start code prefix (to allow backward compatible extensions to be added in the future).

### 5.7.2.2 Frame rate

Encoding: The frame rate shall be 25 Hz or 50 Hz. This shall be indicated in the VUI by setting time_scale and num_units_in_tick according to table 12. Time_scale and num_units_in_tick define the picture rate of the video. The source video format for 50 Hz frame rate material shall be progressive. The source video format for 25 Hz frame rate material shall be interlaced or progressive.
