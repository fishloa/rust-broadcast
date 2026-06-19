# Table 34: Dynamic range and colour gamut coding

_Source: specs/etsi_en_300_743_v01.06.01_dvb_subtitling.pdf §7.2 (PDF pp. 27-53, 62)_


|  Value | Dynamic range and colour gamut  |
| --- | --- |
|  0x00 | SDR; ITU-R BT.709 [10]  |
|  0x01 | SDR; ITU-R BT.2020-2 [11]  |
|  0x02 | HDR; ITU-R BT.2100-1 [12] PQ  |
|  0x03 | HDR; ITU-R BT.2100-1 [12] HLG  |
|  0x04 - 0xFF | Reserved  |

luma-value: This field indicates the luma output value of the CLUT entry.

chroma1-value: This field indicates the first chroma output value of the CLUT entry.

chroma2-value: This field indicates the second chroma output value of the CLUT entry.



# 11 Structure of the pixel code strings (informative)

The structure of the 2-bit/pixel_code_string is shown in table 42.
