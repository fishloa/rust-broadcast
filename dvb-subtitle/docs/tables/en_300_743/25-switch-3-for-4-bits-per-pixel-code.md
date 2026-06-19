# Table 25: switch_3 for 4-bits per pixel code

_Source: specs/etsi_en_300_743_v01.06.01_dvb_subtitling.pdf §7.2 (PDF pp. 27-53, 62)_


|  Value | Meaning  |
| --- | --- |
|  00 | 1 pixel shall be set to pseudo-colour (entry) '0000'  |
|  01 | 2 pixels shall be set to pseudo-colour (entry) '0000'  |
|  10 | the following 8 bits contain run-length coded pixel-data  |
|  11 | the following 12 bits contain run-length coded pixel-data  |

run_length_4-7: Number of pixels minus 4 that shall be set to the pseudo-colour defined next.

run_length_9-24: Number of pixels minus 9 that shall be set to the pseudo-colour defined next.

run_length_25-280: Number of pixels minus 25 that shall be set to the pseudo-colour defined next.

## 7.2.5.2.3 8-bits per pixel code

Table 26 defines the syntax of the 8-bits per pixel code string.
