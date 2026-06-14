# Table 43: 4-bit/pixel_code_string()

_Source: specs/etsi_en_300_743_v01.06.01_dvb_subtitling.pdf §7.2 (PDF pp. 27-53, 62)_


|  Value | Meaning  |
| --- | --- |
|  0001 | one pixel in colour 1  |
|  To | to  |
|  1111 | one pixel in colour 15  |
|  0000 1100 | one pixel in colour 0  |
|  0000 1101 | two pixels in colour 0  |
|  0000 0LLL | L pixels (3..9) in colour 0 (L>0)  |
|  0000 10LL CCCC | L pixels (4..7) in colour C  |
|  0000 1110 LLLL CCCC | L pixels (9..24) in colour C  |
|  0000 1111 LLLL LLLL CCCC | L pixels (25..280) in colour C  |
|  0000 0000 | end of 4-bit/pixel_code_string  |
|  NOTE: Runs of 8 pixels in a colour not equal to '0' can be coded as one pixel plus a run of 7 pixels.  |   |

The structure of the 8-bit/pixel_code_string is shown in table 44.
