# Table 9: Time_scal and num_units_in_tick for Progressive and Interlace Frame Rates for 30 Hz H.264/AVC SDTV

_Source: specs/etsi_ts_101_154_v02.10.01_av_coding_in_dvb.pdf — AVC/HEVC resolution+frame-rate constraint tables (8-12,19-21), AFD (B.11), AD_descriptor (E.1), DASH player conformance (L.1). Curated constraint/descriptor subset of a 357-page spec; full codec-constraint prose not transcribed._


|  Frame Rate | Interlaced or Progressive | time_scale | Num_units_in_tick  |
| --- | --- | --- | --- |
|  24 000/ 1 001 | P | 48 000 | 1 001  |
|  24 | P | 48 | 1  |
|  30 000/ 1 001 | P | 60 000 | 1 001  |
|  30 | P | 60 | 1  |
|  30 000/ 1 001 | I | 60 000 | 1 001  |
|  30 | I | 60 | 1  |

Decoding: The 30 Hz H.264/AVC SDTV IRD shall support decoding and displaying video with a frame rate of 24 000/1 001, 24, 30 000/1 001 or 30 Hz within the constraints of Main Profile at Level 3. Support of other frame rates is optional.

## 5.6.3.3 Luminance resolution

Encoding: 30 Hz H.264/AVC SDTV Bitstreams shall represent video with luminance resolutions as shown in table 10. Non full-screen pictures may be encoded for display at less than full-size (when using one of the standard up-conversion ratios at the 30 Hz H.264/AVC SDTV IRD).

Decoding: 30 Hz H.264/AVC SDTV IRDs shall be capable of decoding pictures with luminance resolutions as shown in table 10 and applying up sampling to allow the decoded pictures to be displayed at full-screen size. In addition, 30 Hz H.264/AVC SDTV IRDs shall be capable of decoding lower picture resolutions and displaying them at less than full-size after using one of the standard up-conversions, e.g. a horizontal resolution of 704 pixels within the 720 pixels full-screen display.
