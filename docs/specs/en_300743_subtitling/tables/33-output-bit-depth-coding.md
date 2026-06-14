# Table 33: Output bit-depth coding

_Source: specs/etsi_en_300_743_v01.06.01_dvb_subtitling.pdf §7.2 (PDF pp. 27-53, 62)_


|  Value | Output bit-depth  |
| --- | --- |
|  0x0 | 8  |
|  0x1 | 10  |
|  0x2 - 0x7 | Reserved  |

reserved_zero_future_use: This bit is reserved for future use. It shall be set to the value 0.

dynamic_range_and_colour_gamut: This eight-bit field shall be coded according to one of the entries in table 34.
