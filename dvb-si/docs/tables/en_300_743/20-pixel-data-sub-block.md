# Table 20: Pixel-data sub-block

_Source: specs/etsi_en_300_743_v01.06.01_dvb_subtitling.pdf §7.2 (PDF pp. 27-53, 62)_


|  Syntax | Size | Type  |
| --- | --- | --- |
|  pixel-data_sub-block() { |  |   |
|  data_type | 8 | bslbf  |
|  if data_type =='0x10' { |  |   |
|  repeat { |  |   |
|  2-bit/pixel_code_string() |  |   |
|  } until (end of 2-bit/pixel_code_string) |  |   |
|  while (!bytealigned()) |  |   |
|  2_stuff_bits | 2 | bslbf  |
|  if data_type =='0x11' { |  |   |
|  repeat { |  |   |
|  4-bit/pixel_code_string() |  |   |
|  } until (end of 4-bit/pixel_code_string) |  |   |
|  if (!bytealigned()) |  |   |
|  4_stuff_bits | 4 | bslbf  |
|  } |  |   |
|  } |  |   |
|  if data_type =='0x12' { |  |   |
|  repeat { |  |   |
|  8-bit/pixel_code_string() |  |   |
|  } until (end of 8-bit/pixel_code_string) |  |   |
|  } |  |   |
|  if data_type =='0x20' |  |   |
|  2_to_4-bit_map-table | 16 | bslbf  |
|  if data_type =='0x21' |  |   |
|  2_to_8-bit_map-table | 32 | bslbf  |
|  if data_type =='0x22' |  |   |
|  4_to_8-bit_map-table | 128 | bslbf  |
|  } |  |   |

## Semantics:

data_type: Identifies the type of information contained in the pixel-data_sub-block according to table 21.
