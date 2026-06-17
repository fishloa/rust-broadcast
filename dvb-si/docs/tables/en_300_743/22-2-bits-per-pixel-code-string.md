# Table 22: 2-bits per pixel code string

_Source: specs/etsi_en_300_743_v01.06.01_dvb_subtitling.pdf §7.2 (PDF pp. 27-53, 62)_


|  Syntax | Size | Type  |
| --- | --- | --- |
|  2-bit/pixel_code_string() { |  |   |
|  if (next_bits(2) != '00') { |  |   |
|  2-bitpixel-code | 2 | bslbf  |
|  } else { |  |   |
|  2-bit_zero | 2 | bslbf  |
|  switch_1 | 1 | bslbf  |
|  if (switch_1 == '1') { |  |   |
|  run_length_3-10 | 3 | uimsbf  |
|  2-bitpixel-code | 2 | bslbf  |
|  } else { |  |   |
|  switch_2 | 1 | bslbf  |
|  if (switch_2 == '0') { |  |   |
|  switch_3 | 2 | bslbf  |
|  if (switch_3 == '10') { |  |   |
|  run_length_12-27 | 4 | uimsbf  |
|  2-bitpixel-code | 2 | bslbf  |
|  } |  |   |
|  if (switch_3 == '11') { |  |   |
|  run_length_29-284 | 8 | uimsbf  |
|  2-bitpixel-code | 2 | bslbf  |
|  } |  |   |
|  } |  |   |
|  } |  |   |
|  } |  |   |

## Semantics:

2-bitpixel-code: A 2-bit code, specifying the pseudo-colour of a pixel as either an entry number of a CLUT with four entries or an entry number of a map-table.
2-bit_zero: A 2-bit field filled with '00'.



switch_1: A 1-bit switch that identifies the meaning of the following fields.

run_length_3-10: Number of pixels minus 3 that shall be set to the pseudo-colour defined next.

switch_2: A 1-bit switch. If set to '1', it signals that one pixel shall be set to pseudo-colour (entry) '00', else it indicates the presence of the following fields.

switch_3: A 2-bit switch that may signal one of the properties listed in table 23.
