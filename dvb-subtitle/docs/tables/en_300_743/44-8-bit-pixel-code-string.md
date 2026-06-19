# Table 44: 8-bit/pixel_code_string()

_Source: specs/etsi_en_300_743_v01.06.01_dvb_subtitling.pdf §7.2 (PDF pp. 27-53, 62)_


|  Value | Meaning  |
| --- | --- |
|  00000001 | one pixel in colour 1  |
|  To | to  |
|  11111111 | one pixel in colour 255  |
|  00000000 0LLLLLLL | L pixels (1-127) in colour 0 (L > 0)  |
|  00000000 1LLLLLLL CCCCCCCC | L pixels (3-127) in colour C (L > 2)  |
|  00000000 00000000 | end of 8-bit/pixel_code_string  |
