# Table 23: switch_3 for 2-bits per pixel code

_Source: specs/etsi_en_300_743_v01.06.01_dvb_subtitling.pdf §7.2 (PDF pp. 27-53, 62)_


|  Value | Meaning  |
| --- | --- |
|  00 | end of 2-bit/pixel_code_string  |
|  01 | two pixels shall be set to pseudo colour (entry) '00'  |
|  10 | the following 6 bits contain run length coded pixel data  |
|  11 | the following 10 bits contain run length coded pixel data  |

run_length_12-27: Number of pixels minus 12 that shall be set to the pseudo-colour defined next.

run_length_29-284: Number of pixels minus 29 that shall be set to the pseudo-colour defined next.

## 7.2.5.2.2 4-bits per pixel code

Table 24 defines the syntax of the 4-bits per pixel code string.
