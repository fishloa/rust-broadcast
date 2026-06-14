# Table 42: 2-bit/pixel_code_string()

_Source: specs/etsi_en_300_743_v01.06.01_dvb_subtitling.pdf §7.2 (PDF pp. 27-53, 62)_


|  Value | Meaning  |
| --- | --- |
|  01 | one pixel in colour 1  |
|  10 | one pixel in colour 2  |
|  11 | one pixel in colour 3  |
|  00 01 | one pixel in colour 0  |
|  00 00 01 | two pixels in colour 0  |
|  00 1L LL CC | L pixels (3..10) in colour C  |
|  00 00 10 LL LL CC | L pixels (12..27) in colour C  |
|  00 00 11 LL LL LL LL CC | L pixels (29..284) in colour C  |
|  00 00 00 | end of 2-bit/pixel_code_string  |
|  NOTE: Runs of 11 pixels and 28 pixels can be coded as one pixel plus a run of 10 pixels and 27 pixels, respectively.  |   |

The structure of the 4-bit/pixel_code_string is shown in table 43.
